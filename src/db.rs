use std::ops::{Deref, DerefMut};

use async_trait::async_trait;
use ethers::types::{H256, U64};
use sqlx::migrate::MigrateDatabase;
use sqlx::{Acquire, PgPool, Postgres};

use crate::tree::config::DbConfig;
use crate::tree::Hash;

pub struct Db {
    pool: PgPool,
}

impl Db {
    pub async fn init(config: &DbConfig) -> sqlx::Result<Self> {
        if config.create
            && !Postgres::database_exists(&config.connection_string).await?
        {
            Postgres::create_database(&config.connection_string).await?;
        }

        let db = Self::new(&config.connection_string).await?;

        if config.migrate {
            db.migrate().await?;
        }

        Ok(db)
    }

    pub async fn new(database_url: &str) -> sqlx::Result<Self> {
        let pool = PgPool::connect(database_url).await?;

        Ok(Self { pool })
    }

    pub async fn migrate(&self) -> sqlx::Result<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }
}

impl Deref for Db {
    type Target = PgPool;

    fn deref(&self) -> &Self::Target {
        &self.pool
    }
}

impl DerefMut for Db {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.pool
    }
}

#[async_trait]
pub trait DbMethods<'c>: Acquire<'c, Database = Postgres> + Sized {
    async fn insert_tx(
        self,
        chain_id: u64,
        block_number: U64,
        tx_hash: H256,
    ) -> sqlx::Result<i64> {
        let mut conn = self.acquire().await?;

        let (id,): (i64,) = sqlx::query_as(
            r#"
            INSERT INTO tx (chain_id, block_number, tx_hash)
            VALUES ($1, $2, $3)
            RETURNING id
            "#,
        )
        .bind(chain_id as i64)
        .bind(block_number.as_u64() as i64)
        .bind(tx_hash.as_bytes())
        .fetch_one(&mut *conn)
        .await?;

        Ok(id)
    }

    async fn insert_update(
        self,
        pre_root: Hash,
        post_root: Hash,
        tx_id: i64,
    ) -> sqlx::Result<i64> {
        let mut conn = self.acquire().await?;

        let (id,): (i64,) = sqlx::query_as(
            r#"
            INSERT INTO updates (pre_root, post_root, tx_id)
            VALUES ($1, $2, $3)
            RETURNING id
            "#,
        )
        .bind(pre_root.as_le_bytes().as_ref())
        .bind(post_root.as_le_bytes().as_ref())
        .bind(tx_id)
        .fetch_one(&mut *conn)
        .await?;

        Ok(id)
    }

    async fn insert_root(self, root: Hash, tx_id: i64) -> sqlx::Result<()> {
        let mut conn = self.acquire().await?;

        sqlx::query(
            r#"
            INSERT INTO roots (root, tx_id)
            VALUES ($1, $2)
            "#,
        )
        .bind(root.as_le_bytes().as_ref())
        .bind(tx_id)
        .execute(&mut *conn)
        .await?;

        Ok(())
    }

    async fn insert_leaf_updates(
        self,
        start_id: u64,
        leaf_updates: &[(u64, Hash)],
    ) -> sqlx::Result<()> {
        let mut conn = self.acquire().await?;

        // TODO: Use query builder
        for (idx, (leaf_idx, leaf)) in leaf_updates.into_iter().enumerate() {
            let id = start_id + idx as u64;

            sqlx::query(
                r#"
                INSERT INTO leaf_updates (id, leaf_idx, leaf)
                VALUES ($1, $2, $3)
                "#,
            )
            .bind(id as i64)
            .bind(*leaf_idx as i64)
            .bind(leaf.as_le_bytes().as_ref())
            .execute(&mut *conn)
            .await?;
        }

        Ok(())
    }

    async fn get_last_leaf_update_id(self) -> sqlx::Result<u64> {
        let mut conn = self.acquire().await?;

        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT id
            FROM leaf_updates
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_one(&mut *conn)
        .await?;

        Ok(row.0 as u64)
    }

    async fn insert_leaf_batch(
        self,
        update_id: i64,
        start_id: u64,
        end_id: u64,
    ) -> sqlx::Result<()> {
        let mut conn = self.acquire().await?;

        sqlx::query(
            r#"
            INSERT INTO leaf_batches (update_id, start_id, end_id)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(update_id)
        .bind(start_id as i64)
        .bind(end_id as i64)
        .execute(&mut *conn)
        .await?;

        Ok(())
    }

    async fn fetch_latest_block_number(
        self,
        chain_id: u64,
    ) -> sqlx::Result<Option<U64>> {
        let mut conn = self.acquire().await?;

        let row: Option<(i64,)> = sqlx::query_as(
            r#"
            SELECT block_number
            FROM tx
            WHERE chain_id = $1
            ORDER BY block_number DESC
            LIMIT 1
            "#,
        )
        .bind(chain_id as i64)
        .fetch_optional(&mut *conn)
        .await?;

        Ok(row.map(|r| U64::from(r.0 as u64)))
    }

    async fn fetch_next_updates(
        self,
        root: Hash,
    ) -> sqlx::Result<Vec<(u64, Hash)>> {
        let mut conn = self.acquire().await?;

        let updates: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            r#"
            SELECT
                leaf_updates.leaf_idx,
                leaf_updates.leaf
            FROM updates
            JOIN leaf_batches ON updates.id = leaf_batches.update_id
            JOIN leaf_updates ON leaf_updates.id BETWEEN leaf_batches.start_id AND leaf_batches.end_id
            WHERE updates.pre_root = $1
            ORDER BY leaf_updates.id ASC
            "#,
        )
        .bind(root.as_le_bytes().as_ref())
        .fetch_all(&mut *conn)
        .await?;

        Ok(updates
            .into_iter()
            .map(|(idx, leaf)| {
                (idx as u64, Hash::try_from_le_slice(&leaf).unwrap())
            })
            .collect())
    }

    async fn fetch_latest_common_root(self) -> sqlx::Result<Hash> {
        let mut conn = self.acquire().await?;

        let (latest_hash,): (Vec<u8>,) = sqlx::query_as(
            r#"
            WITH latest AS (
                SELECT tx.chain_id, MAX(updates.id) as id
                FROM roots
                JOIN updates ON roots.root = updates.post_root
                JOIN tx ON roots.tx_id = tx.id
                GROUP BY tx.chain_id
            ),
            latest_common AS (
                SELECT MIN(latest.id) as id
                FROM latest
            )
            SELECT updates.post_root
            FROM latest_common
            JOIN updates ON latest_common.id = updates.id
            "#,
        )
        .fetch_one(&mut *conn)
        .await?;

        Ok(Hash::try_from_le_slice(&latest_hash).unwrap())
    }
}

// Blanket implementation for all types that satisfy the trait bounds
impl<'c, T> DbMethods<'c> for T where
    T: Acquire<'c, Database = Postgres> + Send + Sync + Sized
{
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rand::{Rng, SeedableRng};
    use testcontainers::runners::AsyncRunner;
    use testcontainers::ContainerAsync;
    use testcontainers_modules::postgres;

    use super::*;

    fn random_leaves() -> impl Iterator<Item = Hash> {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(42);

        std::iter::from_fn(move || Some(Hash::from(rng.gen::<u64>())))
    }

    async fn setup() -> eyre::Result<(Db, ContainerAsync<postgres::Postgres>)> {
        let container = postgres::Postgres::default().start().await?;
        let db_host = container.get_host().await?;
        let db_port = container.get_host_port_ipv4(5432).await?;

        // Get the connection string to the test database
        let db_url = format!(
            "postgres://postgres:postgres@{db_host}:{db_port}/postgres",
        );

        println!("export DATABASE_URL={}", db_url);

        // Create a database connection pool
        let db = Db::init(&DbConfig {
            connection_string: db_url.clone(),
            create: true,
            migrate: true,
        })
        .await?;

        Ok((db, container))
    }

    fn rand_tx() -> H256 {
        let mut rng = rand::thread_rng();
        rng.gen()
    }

    #[tokio::test]
    async fn db_operations() -> eyre::Result<()> {
        let (db, _container) = setup().await?;

        let mut tx = db.begin().await?;

        let chain_id = 1;
        let block_number = U64::from(11);
        let tx_hash = H256::from_low_u64_be(1);

        let pre_root = Hash::from(1u64);
        let post_root = Hash::from(2u64);

        let leaves: Vec<_> = random_leaves().take(10).collect();

        let tx_id = tx.insert_tx(chain_id, block_number, tx_hash).await?;

        tx.insert_update(pre_root, post_root, tx_id).await?;

        tx.insert_root(post_root, tx_id).await?;

        // Test bulk inserting leaves
        tx.insert_leaf_updates(0, &[(0, leaves[0]), (0, leaves[1])])
            .await?;

        tx.insert_leaf_batch(tx_id, 0, 1).await?;

        tx.commit().await?;

        // Test fetching the latest block number
        let latest_block = db.fetch_latest_block_number(chain_id).await?;
        assert_eq!(latest_block, Some(block_number));

        Ok(())
    }

    #[tokio::test]
    async fn latest_common_root() -> eyre::Result<()> {
        let (db, _container) = setup().await?;

        let chain_1_id = 1;
        let chain_2_id = 2;
        let chain_3_id = 3;

        let roots = vec![
            Hash::from(0u64),
            Hash::from(1u64),
            Hash::from(2u64),
            Hash::from(3u64),
            Hash::from(4u64),
        ];

        let canonical_tx_1_id =
            db.insert_tx(chain_1_id, 1.into(), rand_tx()).await?;

        let canonical_tx_2_id =
            db.insert_tx(chain_1_id, 2.into(), rand_tx()).await?;

        let canonical_tx_3_id =
            db.insert_tx(chain_1_id, 3.into(), rand_tx()).await?;

        // Canonical updates
        let _update_1 = db
            .insert_update(roots[0], roots[1], canonical_tx_1_id)
            .await?;
        let _update_2 = db
            .insert_update(roots[1], roots[2], canonical_tx_2_id)
            .await?;
        let _update_3 = db
            .insert_update(roots[2], roots[3], canonical_tx_3_id)
            .await?;

        db.insert_root(roots[1], canonical_tx_1_id).await?;
        db.insert_root(roots[2], canonical_tx_2_id).await?;
        db.insert_root(roots[3], canonical_tx_3_id).await?;

        // Chain 2 -> root[2]
        let tx_2_id = db.insert_tx(chain_2_id, 2.into(), rand_tx()).await?;
        db.insert_root(roots[2], tx_2_id).await?;

        // Chain 3 -> root[1]
        let tx_3_id = db.insert_tx(chain_3_id, 3.into(), rand_tx()).await?;
        db.insert_root(roots[1], tx_3_id).await?;

        // LCR == root[1]
        let lcr = db.fetch_latest_common_root().await?;
        assert_eq!(lcr, roots[1]);

        // Chain 3 -> root[2]
        let tx_4_id = db.insert_tx(chain_3_id, 4.into(), rand_tx()).await?;
        db.insert_root(roots[2], tx_4_id).await?;

        // LCR == root[2]
        let lcr = db.fetch_latest_common_root().await?;
        assert_eq!(lcr, roots[2]);

        // Chain 2 -> root[3]
        let tx_5_id = db.insert_tx(chain_2_id, 5.into(), rand_tx()).await?;
        db.insert_root(roots[3], tx_5_id).await?;

        // Chain 3 -> root[3]
        let tx_6_id = db.insert_tx(chain_3_id, 6.into(), rand_tx()).await?;
        db.insert_root(roots[3], tx_6_id).await?;

        // LCR == root[3]
        let lcr = db.fetch_latest_common_root().await?;
        assert_eq!(lcr, roots[3]);

        Ok(())
    }
}
