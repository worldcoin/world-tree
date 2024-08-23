use std::marker::PhantomData;
use std::sync::Arc;

use ethers::providers::Middleware;
use ethers::types::{Filter, ValueOrArray, H160, H256};
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use super::block_scanner::BlockScanner;
use super::error::WorldTreeResult;

mod bridged;
mod canonical;

pub use self::bridged::{BridgeTreeUpdate, BridgedTree};
pub use self::canonical::{CanonicalChainUpdate, CanonicalTree};
pub const BLOCK_SCANNER_SLEEP_TIME: u64 = 5;

pub trait TreeVersion: Default {
    type ChannelData;

    fn spawn<M: Middleware + 'static>(
        tx: Sender<Self::ChannelData>,
        block_scanner: Arc<BlockScanner<M>>,
    ) -> JoinHandle<WorldTreeResult<()>>;

    fn tree_changed_signature() -> H256;
}

pub struct TreeManager<M: Middleware + 'static, T: TreeVersion> {
    pub address: H160,
    pub block_scanner: Arc<BlockScanner<M>>,
    pub chain_id: u64,
    _tree_version: PhantomData<T>,
}

impl<M, T> TreeManager<M, T>
where
    M: Middleware + 'static,
    T: TreeVersion,
{
    pub async fn new(
        address: H160,
        window_size: u64,
        last_synced_block: u64,
        middleware: Arc<M>,
    ) -> WorldTreeResult<Self> {
        let chain_id = middleware.get_chainid().await?.as_u64();

        let filter = Filter::new()
            .address(address)
            .topic0(ValueOrArray::Value(T::tree_changed_signature()));

        let block_scanner = Arc::new(
            BlockScanner::new(
                middleware,
                window_size,
                last_synced_block,
                filter,
            )
            .await?,
        );

        Ok(Self {
            address,
            block_scanner,
            chain_id,
            _tree_version: PhantomData,
        })
    }

    pub fn spawn(
        &self,
        tx: Sender<T::ChannelData>,
    ) -> JoinHandle<WorldTreeResult<()>> {
        T::spawn(tx, self.block_scanner.clone())
    }
}
