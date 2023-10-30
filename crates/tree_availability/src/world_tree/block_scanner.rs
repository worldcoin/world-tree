use std::sync::atomic::{AtomicU64, Ordering};

use ethers::providers::Middleware;
use ethers::types::{
    Address, BlockNumber, Filter, FilterBlockOption, Log, Topic, ValueOrArray,
};

pub struct BlockScanner<M> {
    middleware: M,
    current_block: AtomicU64,
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

        metrics::gauge!(
            "tree_availability.block_scanner.latest_block",
            latest_block as f64
        );

        let current_block = self.current_block.load(Ordering::SeqCst);

        if current_block >= latest_block {
            return Ok(Vec::new());
        }

        let from_block = current_block;
        let to_block = latest_block.min(from_block + self.window_size);

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

        Ok(logs)
    }
}
