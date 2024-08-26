use std::marker::PhantomData;
use std::ops::Deref;

use axum::async_trait;
use ethers::types::{H256, U64};
use sqlx::{Acquire, Executor, PgConnection, PgPool, Postgres, Transaction};

use crate::tree::Hash;

pub struct Db<T> {
    acquire: T,
}

#[derive(Debug)]
pub struct PoolProxy {
    pool: PgPool,
}

impl<T> Db<T> {
    pub fn from_executor(acquire: T) -> Self {
        Self { acquire }
    }
}

impl Db<PgPool> {
    pub async fn new(database_url: &str) -> Result<Self, sqlx::Error> {
        // Create a new database connection pool
        let pool = PgPool::connect(database_url).await?;

        // Auto-apply migrations
        sqlx::migrate!("./migrations").run(&pool).await?;

        Ok(Self { acquire: pool })
    }

    pub async fn begin<'a>(
        &'a self,
    ) -> Result<Db<Transaction<'a, Postgres>>, sqlx::Error> {
        let tx = self.acquire.begin().await?;

        Ok(Db::from_executor(tx))
    }
}

impl<'a> Db<Transaction<'a, Postgres>> {
    pub async fn commit(self) -> Result<(), sqlx::Error> {
        self.acquire.commit().await
    }

    pub async fn rollback(self) -> Result<(), sqlx::Error> {
        self.acquire.rollback().await
    }
}

// #[async_trait::async_trait]
// pub trait DbExecutor {
//     type ExecutorType: Executor<'static, Database = Postgres>;

//     // The "virtual" method that needs to be implemented
//     fn executor(&mut self) -> &mut Self::ExecutorType;

//     // High-level provided method: Fetch user info by ID
//     async fn fetch_num(&mut self) -> sqlx::Result<Option<i32>> {
//         let x: Option<(i32,)> = sqlx::query_as("SELECT id FROM users")
//             .fetch_optional(self.executor())
//             .await?;

//         Ok(x.map(|(id,)| id))
//     }
// }

// // Implementing DbExecutor for a PgPool (connection pool)
// #[async_trait::async_trait]
// impl DbExecutor for PgPool {
//     type ExecutorType = PgPool;

//     fn executor(&mut self) -> &mut Self::ExecutorType {
//         self
//     }
// }

// // Implementing DbExecutor for a PoolConnection
// #[async_trait::async_trait]
// impl DbExecutor for PgConnection {
//     type ExecutorType = PgConnection;

//     fn executor(&mut self) -> &mut Self::ExecutorType {
//         self
//     }
// }

// // Implementing DbExecutor for a Transaction
// #[async_trait::async_trait]
// impl<'a> DbExecutor for sqlx::Transaction<'a, Postgres> {
//     type ExecutorType = sqlx::Transaction<'a, Postgres>;

//     fn executor(&mut self) -> &mut Self::ExecutorType {
//         self
//     }
// }

impl<'c, A> Db<A> {
    // pub async fn insert_tx(
    //     self,
    //     chain_id: u64,
    //     block_number: U64,
    //     tx_hash: H256,
    // ) -> Result<i64, sqlx::Error> {
    //     let mut conn = self.acquire.acquire().await?;

    //     let row: (i64,) = sqlx::query_as(
    //         r#"
    //         INSERT INTO tx (chain_id, block_number, tx_hash)
    //         VALUES ($1, $2, $3)
    //         RETURNING id
    //         "#,
    //     )
    //     .bind(chain_id as i64)
    //     .bind(block_number.as_u64() as i64)
    //     .bind(tx_hash.as_bytes())
    //     .fetch_one(&mut conn)
    //     .await?;

    //     // let row: (i64,) = sqlx::query_as(
    //     //     r#"
    //     //     INSERT INTO tx (chain_id, block_number, tx_hash)
    //     //     VALUES ($1, $2, $3)
    //     //     RETURNING id
    //     //     "#,
    //     // )
    //     // .bind(chain_id as i64)
    //     // .bind(block_number.as_u64() as i64)
    //     // .bind(tx_hash.as_bytes());
    //     // // .fetch_one(conn)
    //     // // .await?;

    //     Ok(row.0)
    // }

    // pub async fn insert_canonical_update(
    //     &self,
    //     pre_root: Hash,
    //     post_root: Hash,
    //     tx_id: i64,
    // ) -> Result<(), sqlx::Error> {
    //     sqlx::query(
    //         r#"
    //         INSERT INTO canonical_updates (pre_root, post_root, tx_id)
    //         VALUES ($1, $2, $3)
    //         "#,
    //     )
    //     .bind(pre_root.as_le_bytes().as_ref())
    //     .bind(post_root.as_le_bytes().as_ref())
    //     .bind(tx_id)
    //     .execute(&self.acquire)
    //     .await?;

    //     Ok(())
    // }

    // pub async fn insert_bridged_update(
    //     &self,
    //     root: Hash,
    //     tx_id: i64,
    // ) -> Result<(), sqlx::Error> {
    //     sqlx::query(
    //         r#"
    //         INSERT INTO bridged_updates (root, tx_id)
    //         VALUES ($1, $2)
    //         "#,
    //     )
    //     .bind(root.as_le_bytes().as_ref())
    //     .bind(tx_id)
    //     .execute(&self.acquire)
    //     .await?;

    //     Ok(())
    // }

    // pub async fn bulk_insert_leaves(
    //     &self,
    //     leaves: &[(u64, Hash)],
    // ) -> Result<(), sqlx::Error> {
    //     // TODO: Use query builder
    //     for (leaf_idx, leaf) in leaves {
    //         sqlx::query(
    //             r#"
    //             INSERT INTO leaf_updates (leaf_idx, leaf)
    //             VALUES ($1, $2)
    //             "#,
    //         )
    //         .bind(*leaf_idx as i64)
    //         .bind(leaf.as_le_bytes().as_ref())
    //         .execute(&self.acquire)
    //         .await?;
    //     }

    //     Ok(())
    // }

    // pub async fn fetch_latest_block_number(
    //     &self,
    //     chain_id: u64,
    // ) -> Result<Option<U64>, sqlx::Error> {
    //     let row: Option<(i64,)> = sqlx::query_as(
    //         r#"
    //         SELECT block_number
    //         FROM tx
    //         WHERE chain_id = $1
    //         ORDER BY block_number DESC
    //         LIMIT 1
    //         "#,
    //     )
    //     .bind(chain_id as i64)
    //     .fetch_optional(&self.acquire)
    //     .await?;

    //     Ok(row.map(|r| U64::from(r.0 as u64)))
    // }
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

    // #[tokio::test]
    // async fn db_operations() -> eyre::Result<()> {
    //     let container = postgres::Postgres::default().start().await?;
    //     let db_host = container.get_host().await?;
    //     let db_port = container.get_host_port_ipv4(5432).await?;

    //     // Get the connection string to the test database
    //     let db_url = format!(
    //         "postgres://postgres:postgres@{db_host}:{db_port}/postgres",
    //     );

    //     // Create a database connection pool
    //     let db = Db::new(&db_url).await?;

    //     let db_tx = db.acquire.begin().await?;
    //     // let db_tx = Db { acquire: &mut db_tx };

    //     // let db_tx = db.begin().await?;
    //     // let db_tx = db;

    //     let chain_id = 1;
    //     let block_number = U64::from(11);
    //     let tx_hash = H256::from_low_u64_be(1);

    //     let pre_root = Hash::from(1u64);
    //     let post_root = Hash::from(2u64);

    //     let leaves: Vec<_> = random_leaves().take(10).collect();

    //     let tx_id = db_tx.insert_tx(chain_id, block_number, tx_hash).await?;

    //     // db_tx.insert_canonical_update(pre_root, post_root, tx_id)
    //     //     .await?;

    //     // db_tx.insert_bridged_update(post_root, tx_id).await?;

    //     // // Test bulk inserting leaves
    //     // db_tx.bulk_insert_leaves(&[(0, leaves[0]), (0, leaves[1])])
    //     //     .await?;

    //     // db_tx.commit().await?;

    //     // // Test fetching the latest block number
    //     // let latest_block = db.fetch_latest_block_number(chain_id).await?;
    //     // assert_eq!(latest_block, Some(block_number));

    //     Ok(())
    // }
}
