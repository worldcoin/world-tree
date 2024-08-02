use super::error::WorldTreeResult;
use crate::tree::tree_manager::BLOCK_SCANNER_SLEEP_TIME;
use crate::util::retry;
use ethers::providers::Middleware;
use ethers::types::{BlockNumber, Filter, Log};
use futures::{stream, Stream};
use std::fmt::Debug;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

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

    // TODO: implement reverse range

    pub fn block_stream(
        &self,
    ) -> impl Stream<Item: Future<Output = WorldTreeResult<Vec<Log>>> + Send> + '_
    {
        stream::unfold(
            (self.start_block, 0),
            move |(next_block, mut latest)| async move {
                let to_block = loop {
                    let try_to = next_block + self.window_size;
                    // Update the latest block number only if required
                    if try_to > latest {
                        let middleware = self.middleware.clone();
                        latest =
                            retry(
                                Duration::from_millis(100),
                                Some(Duration::from_secs(60)),
                                move || {
                                    let middleware = middleware.clone();
                                    async move {
                                        middleware.get_block_number().await
                                    }
                                },
                            )
                            .await
                            .expect("failed to fetch latest block after retry")
                            .as_u64();
                        if latest < next_block {
                            tokio::time::sleep(Duration::from_secs(
                                BLOCK_SCANNER_SLEEP_TIME,
                            ))
                            .await;
                            continue;
                        } else {
                            break (try_to).min(latest);
                        }
                    }

                    break (try_to).min(latest);
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
                            tracing::trace!(?chain_id, ?last_synced_block,);
                            let logs = middleware.get_logs(&filter).await?;
                            Ok(logs)
                        }
                    },
                );

                Some((fut, (to_block + 1, latest)))
            },
        )
    }
}
