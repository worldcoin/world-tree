use std::sync::Arc;

use ethers::abi::RawLog;
use ethers::contract::EthEvent;
use ethers::providers::Middleware;
use ethers::types::{H256, U64};
use eyre::ContextCompat;
use futures::{StreamExt, TryStreamExt};
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use super::TreeVersion;
use crate::abi::RootAddedFilter;
use crate::tree::block_scanner::BlockScanner;
use crate::tree::error::WorldTreeResult;
use crate::tree::Hash;

#[derive(Default)]
pub struct BridgedTree;

#[derive(Debug, Clone)]
pub struct BridgeTreeUpdate {
    pub chain_id: u64,
    pub root: Hash,
    pub tx_hash: H256,
    pub block_number: U64,
}

impl TreeVersion for BridgedTree {
    type ChannelData = BridgeTreeUpdate;

    fn spawn<M: Middleware + 'static>(
        tx: Sender<Self::ChannelData>,
        block_scanner: Arc<BlockScanner<M>>,
    ) -> JoinHandle<WorldTreeResult<()>> {
        tokio::spawn(async move {
            let chain_id =
                block_scanner.middleware.get_chainid().await?.as_u64();

            tracing::info!(?chain_id, "Starting bridged tree manager");
            block_scanner
                .block_stream()
                // Retrieve logs concurrently
                // Setting this too high can cause a 502
                .buffered(10)
                // Process logs sequentially
                .try_for_each(|logs| async {
                    for log in logs {
                        // Extract metadata
                        let tx_hash =
                            log.transaction_hash.context("Missing tx_hash")?;
                        let block_number =
                            log.block_number.context("Missing block_number")?;

                        // Extract the root from the RootAdded log
                        let data =
                            RootAddedFilter::decode_log(&RawLog::from(log))?;
                        let root = Hash::from_limbs(data.root.0);

                        tracing::info!(?chain_id, ?root, "Root updated");
                        tx.send(BridgeTreeUpdate {
                            chain_id,
                            root,
                            tx_hash,
                            block_number,
                        })
                        .await?;
                    }
                    Ok(())
                })
                .await?;
            Ok(())
        })
    }

    fn tree_changed_signature() -> H256 {
        RootAddedFilter::signature()
    }
}
