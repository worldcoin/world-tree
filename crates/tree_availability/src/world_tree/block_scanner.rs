use std::sync::atomic::{AtomicU64, Ordering};

use ethers::providers::Middleware;
use ethers::types::{
    Address, BlockNumber, Filter, FilterBlockOption, Log, Topic, ValueOrArray,
};

pub struct BlockScanner<M> {
    middleware: M,
    pub last_synced_block: AtomicU64,
    window_size: u64,
}

impl<M> BlockScanner<M>
where
    M: Middleware,
{
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

    pub async fn next(
        &self,
        address: Option<ValueOrArray<Address>>,
        topics: [Option<Topic>; 4],
    ) -> Result<Vec<Log>, M::Error> {
        let latest_block = self.middleware.get_block_number().await?.as_u64();

        let last_synced_block = self.last_synced_block.load(Ordering::SeqCst);

        if last_synced_block >= latest_block {
            return Ok(Vec::new());
        }

        let from_block = last_synced_block + 1;
        let to_block = latest_block.min(from_block + self.window_size);

        tracing::info!("Scanning from {} to {}", from_block, to_block);

        let new_synced_block = to_block;

        let from_block = Some(BlockNumber::Number(from_block.into()));
        let to_block = Some(BlockNumber::Number(to_block.into()));

        let logs = self
            .middleware
            .get_logs(&Filter {
                block_option: FilterBlockOption::Range {
                    from_block,
                    to_block,
                },
                address,
                topics,
            })
            .await?;

        self.last_synced_block
            .store(new_synced_block, Ordering::SeqCst);

        tracing::info!("Last synced block updated to {new_synced_block}");

        Ok(logs)
    }
}
