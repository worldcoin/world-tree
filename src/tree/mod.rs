pub mod block_scanner;
pub mod error;
pub mod identity_tree;
pub mod service;
pub mod tree_data;
pub mod tree_updater;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use error::TreeAvailabilityError;
use ethers::providers::Middleware;
use ethers::types::H160;
use semaphore::dynamic_merkle_tree::DynamicMerkleTree;
use semaphore::lazy_merkle_tree::{Canonical, LazyMerkleTree};
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::sync::mpsc::Sender;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::instrument;

use self::block_scanner::BlockScanner;
use self::tree_data::TreeData;
use self::tree_updater::TreeUpdater;

pub type PoseidonTree<Version> = LazyMerkleTree<PoseidonHash, Version>;
pub type Hash = <PoseidonHash as Hasher>::Hash;

pub const SYNC_TO_HEAD_SLEEP_SECONDS: u64 = 5;

/// An abstraction over a tree with a history of changes
///
/// In our data model the `tree` is the oldest available tree.
/// The entires in `tree_history` represent new additions to the tree.
pub struct WorldTree<M: Middleware> {
    // pub identity_tree: IdentityTree,
    // /// All the leaves of the tree and their corresponding root hash
    // pub tree_data: Arc<RwLock<TreeData>>,

    // /// The object in charge of syncing the tree from calldata
    // pub tree_updater: Arc<TreeUpdater<M>>,
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
            tree_data: Arc::new(RwLock::new(TreeData::new(
                tree,
                tree_history_size,
            ))),
            tree_updater: Arc::new(TreeUpdater::new(
                address,
                creation_block,
                window_size,
                middleware,
            )),
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

                tokio::time::sleep(Duration::from_secs(
                    SYNC_TO_HEAD_SLEEP_SECONDS,
                ))
                .await;
            }
        })
    }
}

pub type TreeUpdates = HashMap<usize, Hash>;

//TODO: this will replace WorldTree
pub struct IdentityTree<M: Middleware> {
    pub canonical_tree: DynamicMerkleTree<PoseidonHash>,
    pub tree_updates: BTreeMap<Root, TreeUpdates>,
    pub tree_manager: TreeManager<M>,
    pub chain_state: HashMap<usize, Root>,

    pub leaves: HashSet<Hash>,
}

impl<M> IdentityTree<M>
where
    M: Middleware,
{
    async fn spawn() {

        // create some channel

        //get the latest root from the bridged trees

        // (optional restore from cache for canonical tree)
        // sync the canonical tree manager to the latest root, update the tree updates once reached the gcr across all chains

        //spawn the canonical tree manager

        // spawn the bridged tree manager

        // some loop and tokio select! where we handle the bridged tree messages and the canonical tree messages
        // the canonical tree messages will append to the tree state
        // the bridged tree messages will update the root for the chain, as well as check the lcd across all of the roots and if there is a new common root,
        // the logic will advance the state of the canonical tree by consuming the next tree state into the canonical tree.
    }
}

pub struct TreeManager<M: Middleware> {
    pub canonical_tree_manager: CanonicalTreeManager<M>,
    pub bridged_tree_manager: Vec<BridgedTreeManager<M>>,
}

pub struct CanonicalTreeManager<M: Middleware> {
    pub identity_update_tx: Sender<(Root, TreeUpdates)>,
    pub tree_address: H160,
    pub middleware: Arc<M>,
}

pub struct BridgedTreeManager<M: Middleware> {
    pub root_tx: Sender<Root>,
    pub tree_address: H160,
    pub middleware: Arc<M>,
}

pub struct Root {
    pub root: Hash,
    pub block_timestamp: u64,
}

//TODO: ord the root by block timestamp so that they can have order in tree state
