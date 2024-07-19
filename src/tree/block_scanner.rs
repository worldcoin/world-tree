use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ethers::providers::Middleware;
use ethers::types::{BlockNumber, Filter, Log};

/// The `BlockScanner` utility tool enables allows parsing arbitrary onchain events
#[derive(Debug)]
pub struct BlockScanner<M: Middleware + 'static> {
    /// The onchain data provider
    pub middleware: Arc<M>,
    /// The block from which to start parsing a given event
    pub next_block: AtomicU64,
    /// The maximum block range to parse
    window_size: u64,
    /// Filter specifying the address and topics to match on when scanning
    filter: Filter,
    chain_id: u64,
}

impl<M> BlockScanner<M>
where
    M: Middleware + Send + Sync + Debug,
{
    /// Initializes a new `BlockScanner`
    pub async fn new(
        middleware: Arc<M>,
        window_size: u64,
        current_block: u64,
        filter: Filter,
    ) -> Result<Self, M::Error> {
        let chain_id = middleware.get_chainid().await?.as_u64();
        Ok(Self {
            middleware,
            next_block: AtomicU64::new(current_block),
            window_size,
            filter,
            chain_id,
        })
    }

    /// Retrieves events matching the specified address and topics from the last synced block to the latest block, stepping by `window_size`.
    /// Note that the logs are unsorted and should be handled accordingly.
    pub async fn next(&self) -> Result<(usize, Vec<Log>), M::Error> {
        let latest_block = self.middleware.get_block_number().await?.as_u64();
        let mut next_block = self.next_block.load(Ordering::SeqCst);

        let to_block = (next_block + self.window_size).min(latest_block);

        let num_blocks =
            if let Some(num_blocks) = to_block.checked_sub(next_block) {
                num_blocks as usize
            } else {
                return Ok((0, vec![]));
            };

        let filter = self
            .filter
            .clone()
            .from_block(BlockNumber::Number(next_block.into()))
            .to_block(BlockNumber::Number(to_block.into()));

        let middleware = self.middleware.clone();

        let logs = middleware.get_logs(&filter).await?;

        next_block = to_block + 1;
        self.next_block.store(next_block, Ordering::SeqCst);
        tracing::debug!(chain_id = ?self.chain_id, last_synced_block = ?next_block - 1, "Last synced block updated");

        return Ok((num_blocks, logs));
    }
}
