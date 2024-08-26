use std::ops::{Deref, DerefMut};

use async_trait::async_trait;
use ethers::types::{H256, U64};
use sqlx::{Acquire, Error, PgPool, Postgres};

use crate::tree::Hash;

pub struct Db {
    pool: PgPool,
}

impl Db {
    pub async fn new(database_url: &str) -> Result<Self, Error> {
        let pool = PgPool::connect(database_url).await?;
        Ok(Self { pool })
    }

    pub async fn migrate(&self) -> Result<(), Error> {
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
pub trait DbMethods<'c>:
    Acquire<'c, Database = Postgres> + Send + Sync + Sized
{
    async fn insert_tx(
        self,
        chain_id: u64,
        block_number: U64,
        tx_hash: H256,
    ) -> Result<i64, sqlx::Error> {
        let mut conn = self.acquire().await?;

        let row: (i64,) = sqlx::query_as(
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

        Ok(row.0)
    }

    async fn insert_canonical_update(
        self,
        pre_root: Hash,
        post_root: Hash,
        tx_id: i64,
    ) -> Result<(), sqlx::Error> {
        let mut conn = self.acquire().await?;

        sqlx::query(
            r#"
            INSERT INTO canonical_updates (pre_root, post_root, tx_id)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(pre_root.as_le_bytes().as_ref())
        .bind(post_root.as_le_bytes().as_ref())
        .bind(tx_id)
        .execute(&mut *conn)
        .await?;

        Ok(())
    }

    async fn insert_bridged_update(
        self,
        root: Hash,
        tx_id: i64,
    ) -> Result<(), sqlx::Error> {
        let mut conn = self.acquire().await?;

        sqlx::query(
            r#"
            INSERT INTO bridged_updates (root, tx_id)
            VALUES ($1, $2)
            "#,
        )
        .bind(root.as_le_bytes().as_ref())
        .bind(tx_id)
        .execute(&mut *conn)
        .await?;

        Ok(())
    }

    async fn bulk_insert_leaves(
        self,
        leaves: &[(u64, Hash)],
    ) -> Result<(), sqlx::Error> {
        let mut conn = self.acquire().await?;

        // TODO: Use query builder
        for (leaf_idx, leaf) in leaves {
            sqlx::query(
                r#"
                INSERT INTO leaf_updates (leaf_idx, leaf)
                VALUES ($1, $2)
                "#,
            )
            .bind(*leaf_idx as i64)
            .bind(leaf.as_le_bytes().as_ref())
            .execute(&mut *conn)
            .await?;
        }

        Ok(())
    }

    async fn fetch_latest_block_number(
        self,
        chain_id: u64,
    ) -> Result<Option<U64>, sqlx::Error> {
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
}

// Blanket implementation for all types that satisfy the trait bounds
impl<'c, T> DbMethods<'c> for T where
    T: Acquire<'c, Database = Postgres> + Send + Sync + Sized
{
}

#[cfg(test)]
mod tests {
    use rand::{Rng, SeedableRng};
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres;

    use super::*;

    fn random_leaves() -> impl Iterator<Item = Hash> {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(42);

        std::iter::from_fn(move || Some(Hash::from(rng.gen::<u64>())))
    }

    #[tokio::test]
    async fn db_operations() -> eyre::Result<()> {
        let container = postgres::Postgres::default().start().await?;
        let db_host = container.get_host().await?;
        let db_port = container.get_host_port_ipv4(5432).await?;

        // Get the connection string to the test database
        let db_url = format!(
            "postgres://postgres:postgres@{db_host}:{db_port}/postgres",
        );

        // Create a database connection pool
        let db = Db::new(&db_url).await?;
        db.migrate().await?;

        let mut tx = db.begin().await?;

        let chain_id = 1;
        let block_number = U64::from(11);
        let tx_hash = H256::from_low_u64_be(1);

        let pre_root = Hash::from(1u64);
        let post_root = Hash::from(2u64);

        let leaves: Vec<_> = random_leaves().take(10).collect();

        let tx_id = tx.insert_tx(chain_id, block_number, tx_hash).await?;

        tx.insert_canonical_update(pre_root, post_root, tx_id)
            .await?;

        tx.insert_bridged_update(post_root, tx_id).await?;

        // Test bulk inserting leaves
        tx.bulk_insert_leaves(&[(0, leaves[0]), (0, leaves[1])])
            .await?;

        tx.commit().await?;

        // Test fetching the latest block number
        let latest_block = db.fetch_latest_block_number(chain_id).await?;
        assert_eq!(latest_block, Some(block_number));

        Ok(())
    }
}
