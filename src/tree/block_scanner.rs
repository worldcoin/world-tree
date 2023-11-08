use std::sync::atomic::{AtomicU64, Ordering};

use ethers::providers::Middleware;
use ethers::types::{
    Address, BlockNumber, Filter, FilterBlockOption, Log, Topic, ValueOrArray,
    H160,
};

/// The `BlockScanner` utility tool enables allows parsing arbitrary onchain events
pub struct BlockScanner<M> {
    /// The onchain data provider
    middleware: M,
    /// The block from which to start parsing a given event
    pub last_synced_block: AtomicU64,
    /// The maximum block range to parse
    window_size: u64,
    //TODO:
    filter: Filter,
}

impl<M> BlockScanner<M>
where
    M: Middleware,
{
    /// Initializes a new `BlockScanner`
    pub const fn new(
        middleware: M,
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
    pub async fn next(&self) -> Result<Vec<Log>, M::Error> {
        let latest_block = self.middleware.get_block_number().await?.as_u64();
        let mut last_synced_block =
            self.last_synced_block.load(Ordering::SeqCst);
        let mut logs = Vec::new();

        while last_synced_block < latest_block {
            let from_block = last_synced_block + 1;
            let to_block = (from_block + self.window_size).min(latest_block);

            tracing::info!(?from_block, ?to_block, "Scanning blocks");

            let filter = self
                .filter
                .clone()
                .from_block(BlockNumber::Number(from_block.into()))
                .to_block(BlockNumber::Number(to_block.into()));

            //TODO: can probably also use futures ordered here to get all of the logs quickly
            logs.extend(self.middleware.get_logs(&filter).await?);

            last_synced_block = to_block;
        }

        self.last_synced_block
            .store(last_synced_block, Ordering::SeqCst);

        tracing::info!(?last_synced_block, "Last synced block updated");

        Ok(logs)
    }
}
