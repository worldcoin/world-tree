use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use alloy::eips::BlockNumberOrTag;
use alloy::network::Ethereum;
use alloy::providers::Provider;
use alloy::rpc::types::{Filter, Log};
use alloy::transports::Transport;
use futures::{stream, Stream};

use super::error::WorldTreeResult;
use crate::util::retry;

pub const BLOCK_SCANNER_SLEEP_TIME: u64 = 5;

/// The `BlockScanner` utility tool enables allows parsing arbitrary onchain events
#[derive(Debug)]
pub struct BlockScanner<T, P, N = Ethereum>
where
    T: Transport + Clone,
    P: Provider<T> + 'static,
{
    /// The onchain data provider
    pub provider: Arc<P>,
    /// The block from which to start parsing a given event
    pub start_block: u64,
    /// The maximum block range to parse
    window_size: u64,
    /// Filter specifying the address and topics to match on when scanning
    filter: Filter,
    chain_id: u64,
    _marker: PhantomData<(T, N)>,
}

impl<T, P> BlockScanner<T, P>
where
    T: Transport + Clone,
    P: Provider<T>,
{
    /// Initializes a new `BlockScanner`
    pub async fn new(
        provider: Arc<P>,
        window_size: u64,
        start_block: u64,
        filter: Filter,
    ) -> WorldTreeResult<Self> {
        let chain_id = provider.get_chain_id().await?;
        Ok(Self {
            provider,
            start_block,
            window_size,
            filter,
            chain_id,
            _marker: PhantomData,
        })
    }

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
                        let provider = self.provider.clone();
                        let finalized = retry(
                            Duration::from_millis(100),
                            Some(Duration::from_secs(60)),
                            move || {
                                let provider = provider.clone();
                                async move {
                                    provider
                                        .get_block_by_number(
                                            BlockNumberOrTag::Finalized,
                                            false,
                                        )
                                        .await
                                }
                            },
                        )
                        .await
                        .expect("failed to fetch latest block after retry")
                        .expect("no finalized block found");

                        latest = finalized.header.number;

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
                let filter = Arc::new(
                    self.filter
                        .clone()
                        .from_block(BlockNumberOrTag::from(next_block))
                        .to_block(BlockNumberOrTag::from(to_block)),
                );
                let last_synced_block = next_block;

                let provider = self.provider.clone();
                let chain_id = self.chain_id;

                // This future is yielded from the stream
                // and is awaited on by the caller
                let fut = retry(
                    Duration::from_millis(100),
                    Some(Duration::from_secs(60)),
                    move || {
                        let provider = provider.clone();
                        let filter = filter.clone();
                        async move {
                            tracing::trace!(?chain_id, ?last_synced_block,);
                            let logs = provider.get_logs(&filter).await?;
                            Ok(logs)
                        }
                    },
                );

                Some((fut, (to_block + 1, latest)))
            },
        )
    }
}
