use std::fmt::Debug;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use ethers::providers::Middleware;
use ethers::types::{BlockNumber, Filter, Log};
use futures::{stream, Stream};

use crate::tree::tree_manager::BLOCK_SCANNER_SLEEP_TIME;
use crate::util::retry;

use super::error::WorldTreeResult;

/// The `BlockScanner` utility tool enables allows parsing arbitrary onchain events
#[derive(Debug)]
pub struct BlockScanner<M: Middleware + 'static> {
    /// The onchain data provider
    pub middleware: Arc<M>,
    /// The block from which to start parsing a given event
    pub start_block: u64,
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
        start_block: u64,
        filter: Filter,
    ) -> WorldTreeResult<Self> {
        let chain_id = middleware.get_chainid().await?.as_u64();
        Ok(Self {
            middleware,
            start_block,
            window_size,
            filter,
            chain_id,
        })
    }

    pub fn block_stream(
        &self,
    ) -> impl Stream<Item: Future<Output = WorldTreeResult<Vec<Log>>> + Send> + '_
    {
        stream::unfold(self.start_block, move |mut next_block| async move {
            // This is executed before the item is yielded
            let to_block = loop {
                match self.middleware.get_block_number().await {
                    Ok(latest) if latest.as_u64() < next_block => {
                        tokio::time::sleep(Duration::from_secs(
                            BLOCK_SCANNER_SLEEP_TIME,
                        ))
                        .await;
                        continue;
                    }
                    Ok(latest) => {
                        break (next_block + self.window_size)
                            .min(latest.as_u64());
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_secs(
                            BLOCK_SCANNER_SLEEP_TIME,
                        ))
                        .await;
                        continue;
                    }
                }
            };

            let filter = self
                .filter
                .clone()
                .from_block(BlockNumber::Number((next_block).into()))
                .to_block(BlockNumber::Number(to_block.into()));

            let last_synced_block = next_block;

            let middleware = self.middleware.clone();
            let chain_id = self.chain_id;

            // This future is yielded from the stream
            // and is awaited on by the caller
            let fut = retry(
                Duration::from_millis(100),
                Some(Duration::from_secs(60)),
                move || {
                    let filter = filter.clone();
                    let middleware = middleware.clone();
                    async move {
                        tracing::trace!(?chain_id, ?last_synced_block);
                        Ok(middleware.get_logs(&filter).await?)
                    }
                },
            );

            next_block = to_block + 1;

            Some((fut, next_block))
        })
    }
}
