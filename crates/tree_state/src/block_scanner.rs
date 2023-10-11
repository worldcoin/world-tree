use ethers::providers::Middleware;
use ethers::types::{
    Address, BlockNumber, Filter, FilterBlockOption, Log, Topic, ValueOrArray,
};

pub struct BlockScanner<M> {
    middleware: M,
    current_block: u64,
    window_size: u64,
}

impl<M> BlockScanner<M>
where
    M: Middleware,
    <M as Middleware>::Error: 'static,
{
    pub const fn new(
        middleware: M,
        window_size: u64,
        current_block: u64,
    ) -> Self {
        Self {
            middleware,
            current_block,
            window_size,
        }
    }

    pub async fn next(
        &mut self,
        address: Option<ValueOrArray<Address>>,
        topics: [Option<Topic>; 4],
    ) -> Result<Vec<Log>, M::Error> {
        let latest_block = self.middleware.get_block_number().await?.as_u64();

        if self.current_block >= latest_block {
            return Ok(Vec::new());
        }

        let from_block = self.current_block;
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

        self.current_block = next_current_block;

        Ok(logs)
    }
}
