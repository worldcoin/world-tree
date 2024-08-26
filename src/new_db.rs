use sqlx::{Acquire, Postgres};

#[derive(Clone, Copy)]
pub struct Db<T> {
    inner: T,
}

impl<T> Db<T> {
    pub fn new(inner: T) -> Self {
        Db { inner }
    }
}

impl<'c, T> Db<T>
where
    T: Acquire<'c, Database = Postgres> + Send,
{
    pub async fn fetch_user_id(self) -> sqlx::Result<Option<i32>> {
        let mut conn = self.inner.acquire().await?;
        let executor = conn.as_mut();

        let x: Option<(i32,)> = sqlx::query_as("SELECT id FROM users")
            .fetch_optional(executor)
            .await?;

        Ok(x.map(|(id,)| id))
    }
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

        let db = Db::new(&pool);

        let id = db.fetch_user_id().await.unwrap();
        assert_eq!(id, None);

        // Can reuse because db &Pool is Copy and fetch_user_id borrows self
        let id = db.fetch_user_id().await.unwrap();
        assert_eq!(id, None);
    }

    #[tokio::test]
    async fn works_for_connection() {
        let mut conn = PgConnection::connect(
            "postgres://postgres:password@localhost/testdb",
        )
        .await
        .unwrap();

        let db = Db::new(&mut conn);

        let id = db.fetch_user_id().await.unwrap();
        assert_eq!(id, None);

        // Doesn't work because &mut T is neither Clone nor Copy
        // let id = db.fetch_user_id().await.unwrap();
        // assert_eq!(id, None);
    }

    #[tokio::test]
    async fn works_for_transaction() {
        let pool =
            PgPool::connect("postgres://postgres:password@localhost/testdb")
                .await
                .unwrap();
        let mut tx = pool.begin().await.unwrap();

        let db = Db::new(&mut tx);

        let id = db.fetch_user_id().await.unwrap();
        assert_eq!(id, None);

        // Doesn't work because &mut T is neither Clone nor Copy
        // let id = db.fetch_user_id().await.unwrap();
        // assert_eq!(id, None);

        // This works though
        let id = Db::new(&mut tx).fetch_user_id().await.unwrap();
        assert_eq!(id, None);
    }
}
