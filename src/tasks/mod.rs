use std::time::Duration;

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::task::{JoinError, JoinHandle};
use tracing::info;

pub mod ingest;
pub mod observe;
pub mod update;

pub async fn monitor_tasks(
    mut handles: FuturesUnordered<JoinHandle<()>>,
    shutdown_delay: Duration,
) -> Result<(), JoinError> {
    while let Some(result) = handles.next().await {
        if let Err(error) = result {
            tracing::error!(?error, "Task panicked");
            // abort all other tasks
            for handle in handles.iter() {
                handle.abort();
            }
            info!("All tasks aborted");
            // Give tasks a few seconds to get to an await point
            tokio::time::sleep(shutdown_delay).await;
            return Err(error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use std::time::Instant;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_monitor_tasks() {
        let shutdown_delay = Duration::from_millis(100);

        let panic_handle = tokio::spawn(async {
            panic!("Task failed");
        });

        let return_handle = tokio::spawn(async {});

        let run_time = Duration::from_millis(100);
        let run_handle = tokio::spawn(async {
            sleep(Duration::from_secs(1)).await;
        });

        let handles = FuturesUnordered::from_iter([
            panic_handle,
            return_handle,
            run_handle,
        ]);

        let start = Instant::now();
        assert!(monitor_tasks(handles, shutdown_delay).await.is_err());

        let elapsed = start.elapsed();
        assert!(elapsed >= shutdown_delay);
        assert!(elapsed <= shutdown_delay + run_time);
    }
}
