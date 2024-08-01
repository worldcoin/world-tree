use std::future::Future;
use std::time::Duration;

use tracing::{error, warn};

pub async fn retry<S, F, T, E>(
    mut backoff: Duration,
    limit: Option<Duration>,
    f: S,
) -> Result<T, E>
where
    F: Future<Output = Result<T, E>> + Send + 'static,
    S: Fn() -> F + Send + Sync + 'static,
    E: std::fmt::Debug,
{
    loop {
        match f().await {
            Ok(res) => return Ok(res),
            Err(e) => {
                warn!("{e:?}");
                if let Some(limit) = limit {
                    if backoff > limit {
                        error!("Retry limit reached: {e:?}");
                        return Err(e);
                    }
                }
                tokio::time::sleep(backoff).await;
                backoff *= 2;
            }
        }
    }
}
