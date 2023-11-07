use std::sync::atomic::{AtomicU64, Ordering};

use ethers::providers::Middleware;
use ethers::types::{
    Address, BlockNumber, Filter, FilterBlockOption, Log, Topic, ValueOrArray,
};

/// The `BlockScanner` utility tool enables allows parsing arbitrary onchain events
pub struct BlockScanner<M> {
    /// The onchain data provider
    middleware: M,
    /// The block from which to start parsing a given event
    pub last_synced_block: AtomicU64,
    /// The maximum block range to parse
    window_size: u64,
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
    ) -> Self {
        Self {
            middleware,
            last_synced_block: AtomicU64::new(current_block),
            window_size,
        }
    }

    /// Retrieves events matching the specified address and topics from the last synced block to the latest block.
    ///
    /// # Arguments
    ///
    /// * `address` - Optional address to target when fetching logs.
    /// * `topics` - Optional topics to target when fetching logs, enabling granular filtering when looking for specific event signatures or topic values.
    pub async fn next(
        &self,
        address: Option<ValueOrArray<Address>>,
        topics: [Option<Topic>; 4],
    ) -> Result<Vec<Log>, M::Error> {
        let latest_block = self.middleware.get_block_number().await?.as_u64();

        metrics::gauge!(
            "tree_availability.block_scanner.latest_block",
            latest_block as f64
        );

        let last_synced_block = self.last_synced_block.load(Ordering::SeqCst);

        if last_synced_block >= latest_block {
            return Ok(Vec::new());
        }

        let from_block = last_synced_block + 1;
        let to_block = latest_block.min(from_block + self.window_size);

        tracing::info!(?from_block, ?to_block, "Scanning blocks");

        let logs = self
            .middleware
            .get_logs(&Filter {
                block_option: FilterBlockOption::Range {
                    from_block: Some(BlockNumber::Number(from_block.into())),
                    to_block: Some(BlockNumber::Number(to_block.into())),
                },
                address,
                topics,
            })
            .await?;

        self.last_synced_block.store(to_block, Ordering::SeqCst);

        tracing::info!(?to_block, "Current block updated");

        Ok(logs)
    }
}
