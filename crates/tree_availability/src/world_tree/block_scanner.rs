use std::sync::atomic::{AtomicU64, Ordering};

use ethers::providers::Middleware;
use ethers::types::{
    Address, BlockNumber, Filter, FilterBlockOption, Log, Topic, ValueOrArray,
};

pub struct BlockScanner<M> {
    middleware: M,
    pub current_block: AtomicU64,
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
            current_block: AtomicU64::new(current_block),
            window_size,
        }
    }

    pub async fn next(
        &self,
        address: Option<ValueOrArray<Address>>,
        topics: [Option<Topic>; 4],
    ) -> Result<Vec<Log>, M::Error> {
        let latest_block = self.middleware.get_block_number().await?.as_u64();

        let current_block = self.current_block.load(Ordering::SeqCst);

        if current_block >= latest_block {
            return Ok(Vec::new());
        }

        //TODO: next should maybe sync to chain head or maybe this logic can be outside of the block scanner
        //TODO: but we need some way to create futures ordered until we get the chain head?

        let from_block = current_block;
        let to_block = latest_block.min(from_block + self.window_size);

        tracing::info!("Scanning from {} to {}", current_block, latest_block);

        let next_current_block = to_block + 1;

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

        self.current_block
            .store(next_current_block, Ordering::SeqCst);

        tracing::info!("Current block updated to {next_current_block}");

        Ok(logs)
    }
}
