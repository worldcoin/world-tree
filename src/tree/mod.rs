pub mod block_scanner;
pub mod error;
pub mod service;
pub mod tree_data;
pub mod tree_updater;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use error::TreeAvailabilityError;
use ethers::providers::Middleware;
use ethers::types::H160;
use semaphore::lazy_merkle_tree::{Canonical, LazyMerkleTree};
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::task::JoinHandle;
use tracing::instrument;

use self::tree_data::TreeData;
use self::tree_updater::TreeUpdater;

pub type PoseidonTree<Version> = LazyMerkleTree<PoseidonHash, Version>;
pub type Hash = <PoseidonHash as Hasher>::Hash;

/// An abstraction over a tree with a history of changes
///
/// In our data model the `tree` is the oldest available tree.
/// The entires in `tree_history` represent new additions to the tree.
pub struct WorldTree<M: Middleware> {
    /// All the leaves of the tree and their corresponding root hash
    pub tree_data: Arc<TreeData>,
    /// The object in charge of syncing the tree from calldata
    pub tree_updater: Arc<TreeUpdater<M>>,
    /// Boolean to indicate when the tree state is synced wth the chain head upon spawning the `WorldTree`.
    pub synced: Arc<AtomicBool>,
}

impl<M: Middleware> WorldTree<M> {
    /// Initializes a new instance of `WorldTree`.
    ///
    /// # Arguments
    ///
    /// * `tree` - The `PoseidonTree` used for the merkle tree representation.
    /// * `tree_history_size` - The number of historical tree roots to keep in memory.
    /// * `address` - The smart contract address of the `WorldIDIdentityManager`.
    /// * `creation_block` - The block number at which the contract was deployed.
    /// * `middleware` - Provider to interact with Ethereum.
    pub fn new(
        tree: PoseidonTree<Canonical>,
        tree_history_size: usize,
        address: H160,
        creation_block: u64,
        window_size: u64,
        middleware: Arc<M>,
    ) -> Self {
        Self {
            tree_data: Arc::new(TreeData::new(tree, tree_history_size)),
            tree_updater: Arc::new(TreeUpdater::new(
                address,
                creation_block,
                window_size,
                middleware,
            )),
            synced: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Spawns a task that continually syncs the `TreeData` to the state at the chain head.
    #[instrument(skip(self))]
    pub fn spawn(&self) -> JoinHandle<Result<(), TreeAvailabilityError<M>>> {
        let tree_data = self.tree_data.clone();
        let tree_updater = self.tree_updater.clone();

        tracing::info!("Spawning thread to sync tree");
        let synced = self.synced.clone();

        tokio::spawn(async move {
            let start = tokio::time::Instant::now();

            tree_updater.sync_to_head(&tree_data).await?;

            let sync_time = start.elapsed();

            tracing::info!(?sync_time, "WorldTree synced to chain head");
            synced.store(true, Ordering::Relaxed);

            loop {
                tree_updater.sync_to_head(&tree_data).await?;

                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        })
    }
}
