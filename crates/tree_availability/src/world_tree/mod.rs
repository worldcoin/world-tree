pub mod abi;
pub mod block_scanner;
pub mod tree_data;
pub mod tree_updater;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use ethers::providers::Middleware;
use ethers::types::H160;
use semaphore::lazy_merkle_tree::{Canonical, LazyMerkleTree};
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::task::JoinHandle;

use self::tree_data::TreeData;
use self::tree_updater::TreeUpdater;
use crate::error::TreeAvailabilityError;

pub type PoseidonTree<Version> = LazyMerkleTree<PoseidonHash, Version>;
pub type Hash = <PoseidonHash as Hasher>::Hash;

/// An abstraction over a tree with a history of changes
///
/// In our data model the `tree` is the oldest available tree.
/// The entires in `tree_history` represent new additions to the tree.
pub struct WorldTree<M: Middleware> {
    pub tree_data: Arc<TreeData>,
    pub tree_updater: Arc<TreeUpdater<M>>,
}

impl<M: Middleware> WorldTree<M> {
    pub fn new(
        tree: PoseidonTree<Canonical>,
        tree_history_size: usize,
        address: H160,
        creation_block: u64,
        middleware: Arc<M>,
    ) -> Self {
        Self {
            tree_data: Arc::new(TreeData::new(tree, tree_history_size)),
            tree_updater: Arc::new(TreeUpdater::new(
                address,
                creation_block,
                middleware,
            )),
        }
    }

    pub async fn spawn(
        &self,
    ) -> JoinHandle<Result<(), TreeAvailabilityError<M>>> {
        let tree_data = self.tree_data.clone();
        let tree_updater = self.tree_updater.clone();

        tracing::info!("Spawning thread to sync tree");
        tokio::spawn(async move {
            tree_updater.sync_to_head(&tree_data).await?;
            tree_updater.synced.store(true, Ordering::Relaxed);

            loop {
                tree_updater.sync_to_head(&tree_data).await?;

                // Sleep a little to unblock the executor
                // tokio::time::sleep(Duration::from_secs(5)).await;
            }
        })
    }
}
