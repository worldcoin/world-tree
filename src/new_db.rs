use std::ops::{Deref, DerefMut};

use async_trait::async_trait;
use sqlx::{Acquire, Error, PgPool, Postgres};

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
    async fn fetch_user_id(self) -> sqlx::Result<Option<i32>> {
        let mut conn = self.acquire().await?;

        let x: Option<(i32,)> = sqlx::query_as("SELECT id FROM users")
            .fetch_optional(&mut *conn)
            .await?;

        Ok(x.map(|(id,)| id))
    }
}

// Blanket implementation for all types that satisfy the trait bounds
impl<'c, T> DbMethods<'c> for T where
    T: Acquire<'c, Database = Postgres> + Send + Sync + Sized
{
}

#[cfg(test)]
mod tests {
    use sqlx::{Connection, PgConnection};

    use super::*;

    #[tokio::test]
    async fn works_for_db() {
        let db = Db::new("postgres://postgres:password@localhost/testdb")
            .await
            .unwrap();
        db.migrate().await.unwrap();

        let id = db.fetch_user_id().await.unwrap();
        assert_eq!(id, None);

        let id = db.fetch_user_id().await.unwrap();
        assert_eq!(id, None);
    }

    #[tokio::test]
    async fn works_for_pool() {
        let pool =
            PgPool::connect("postgres://postgres:password@localhost/testdb")
                .await
                .unwrap();

        let id = pool.fetch_user_id().await.unwrap();
        assert_eq!(id, None);

        let id = pool.fetch_user_id().await.unwrap();
        assert_eq!(id, None);
    }

    #[tokio::test]
    async fn works_for_connection() {
        let mut conn = PgConnection::connect(
            "postgres://postgres:password@localhost/testdb",
        )
        .await
        .unwrap();

        let id = (&mut conn).fetch_user_id().await.unwrap();
        assert_eq!(id, None);

        let id = (&mut conn).fetch_user_id().await.unwrap();
        assert_eq!(id, None);
    }

    #[tokio::test]
    async fn works_for_transaction() {
        let db = Db::new("postgres://postgres:password@localhost/testdb")
            .await
            .unwrap();
        let mut tx = db.begin().await.unwrap();

        let id = (&mut tx).fetch_user_id().await.unwrap();
        assert_eq!(id, None);

        let id = (&mut tx).fetch_user_id().await.unwrap();
        assert_eq!(id, None);

        tx.commit().await.unwrap();
    }
}
