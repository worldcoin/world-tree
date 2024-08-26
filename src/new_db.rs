use axum::async_trait;
use sqlx::{Acquire, Postgres};

#[async_trait]
pub trait Database<'c>:
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
impl<'c, T> Database<'c> for T where
    T: Acquire<'c, Database = Postgres> + Send + Sync
{
}

#[cfg(test)]
mod tests {
    use sqlx::{Connection, PgConnection, PgPool};

    use super::*;

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

        let id = conn.fetch_user_id().await.unwrap();
        assert_eq!(id, None);

        let id = conn.fetch_user_id().await.unwrap();
        assert_eq!(id, None);
    }

    #[tokio::test]
    async fn works_for_transaction() {
        let pool =
            PgPool::connect("postgres://postgres:password@localhost/testdb")
                .await
                .unwrap();
        let mut tx = pool.begin().await.unwrap();

        let id = tx.fetch_user_id().await.unwrap();
        assert_eq!(id, None);

        let id = tx.fetch_user_id().await.unwrap();
        assert_eq!(id, None);

        tx.commit().await.unwrap();
    }
}
