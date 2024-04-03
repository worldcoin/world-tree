use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ethers::providers::Middleware;
use ethers::types::{BlockNumber, Filter, Log};
use futures::stream::FuturesUnordered;
use futures::StreamExt;

/// The `BlockScanner` utility tool enables allows parsing arbitrary onchain events
#[derive(Debug)]
pub struct BlockScanner<M: Middleware> {
    /// The onchain data provider
    pub middleware: Arc<M>,
    /// The block from which to start parsing a given event
    pub last_synced_block: AtomicU64,
    /// The maximum block range to parse
    window_size: u64,
    /// Filter specifying the address and topics to match on when scanning
    filter: Filter,
}

impl<M> BlockScanner<M>
where
    M: Middleware + Send + Sync + Debug,
{
    /// Initializes a new `BlockScanner`
    pub const fn new(
        middleware: Arc<M>,
        window_size: u64,
        current_block: u64,
        filter: Filter,
    ) -> Self {
        Self {
            middleware,
            last_synced_block: AtomicU64::new(current_block),
            window_size,
            filter,
        }
    }

    /// Retrieves events matching the specified address and topics from the last synced block to the latest block, stepping by `window_size`.
    /// Note that the logs are unsorted and should be handled accordingly.
    pub async fn next(&self) -> Result<Vec<Log>, M::Error> {
        let latest_block = self.middleware.get_block_number().await?.as_u64();
        let mut last_synced_block =
            self.last_synced_block.load(Ordering::SeqCst);

        let mut tasks = FuturesUnordered::new();
        while last_synced_block < latest_block {
            let from_block = last_synced_block + 1;
            let to_block = (from_block + self.window_size).min(latest_block);

            tracing::debug!(?from_block, ?to_block, "Scanning blocks");

            let filter = self
                .filter
                .clone()
                .from_block(BlockNumber::Number(from_block.into()))
                .to_block(BlockNumber::Number(to_block.into()));

            let middleware = self.middleware.clone();
            tasks.push(async move { middleware.get_logs(&filter).await });

            last_synced_block = to_block;
        }

        //Sort all of the results
        let mut aggregated_logs = vec![];

        while let Some(result) = tasks.next().await {
            let logs = result?;

            aggregated_logs.extend(logs);
        }

        self.last_synced_block
            .store(last_synced_block, Ordering::SeqCst);

        tracing::info!(?last_synced_block, "Last synced block updated");

        Ok(aggregated_logs)
    }
}
