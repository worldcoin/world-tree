use std::marker::PhantomData;
use std::ops::DerefMut;

use sqlx::{Acquire, Executor, PgConnection, PgPool, Postgres, Transaction};

pub struct Db<T, E> {
    inner: T,
    _e: PhantomData<E>,
}

impl<T, E> Db<T, E> {
    pub fn new(inner: T) -> Self {
        Db {
            inner,
            _e: PhantomData,
        }
    }
}

impl<'c, T, E> Db<T, E>
where
    T: Acquire<'c, Database = Postgres> + Send,
    <T as Acquire<'c>>::Connection: AsMut<E> + Send,
    for<'a> &'a mut E: Executor<'a, Database = Postgres>,
{
    pub async fn fetch_user_id(self) -> sqlx::Result<Option<i32>> {
        let mut conn = self.inner.acquire().await?;
        let executor: &mut E = conn.as_mut();

        let x: Option<(i32,)> = sqlx::query_as("SELECT id FROM users")
            .fetch_optional(executor)
            .await?;

        Ok(x.map(|(id,)| id))
    }
}

#[cfg(test)]
mod tests {
    use sqlx::Connection;

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
    }

    #[tokio::test]
    async fn works_for_connection() {
        let mut conn = PgConnection::connect(
            "postgres://postgres:password@localhost/testdb",
        )
        .await
        .unwrap();
        let db: Db<&mut PgConnection, PgConnection> = Db::new(&mut conn);

        let id = db.fetch_user_id().await.unwrap();
        assert_eq!(id, None);
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
    }
}
