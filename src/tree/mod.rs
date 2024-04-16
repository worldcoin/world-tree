pub mod block_scanner;
pub mod config;
pub mod error;
pub mod identity_tree;
pub mod service;
pub mod tree_manager;

use std::collections::{BTreeMap, HashMap};
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use ethers::providers::Middleware;
use ethers::types::{Log, U256};
use ruint::Uint;
use semaphore::cascading_merkle_tree::CascadingMerkleTree;
use semaphore::generic_storage::{GenericStorage, MmapVec};
use semaphore::lazy_merkle_tree::LazyMerkleTree;
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::PoseidonHash;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Receiver;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::instrument;

use self::identity_tree::{IdentityTree, InclusionProof, LeafUpdates, Root};
use self::tree_manager::{
    extract_identity_updates, BridgedTree, CanonicalTree, TreeManager,
};
use crate::abi::IBridgedWorldID;
use crate::tree::identity_tree::flatten_leaf_updates;

pub type PoseidonTree<Version> = LazyMerkleTree<PoseidonHash, Version>;
pub type Hash = <PoseidonHash as Hasher>::Hash;

/// The `WorldTree` syncs and maintains the state of the onchain Merkle tree representing all unique humans across multiple chains
/// and is also able to deliver an inclusion proof for a given identity commitment across any tracked chain
pub struct WorldTree<M: Middleware + 'static> {
    /// The identity tree is the main data structure that holds the state of the tree including latest roots, leaves, and an in-memory representation of the tree
    pub identity_tree: Arc<RwLock<IdentityTree<MmapVec<Hash>>>>,
    /// Responsible for listening to state changes to the tree on mainnet
    pub canonical_tree_manager: TreeManager<M, CanonicalTree>,
    /// Responsible for listening to state changes state changes to bridged WorldIDs
    pub bridged_tree_manager: Vec<TreeManager<M, BridgedTree>>,
    /// Mapping of chain Id -> root hash, representing the latest root for each chain
    pub chain_state: Arc<RwLock<HashMap<u64, Root>>>,
    /// Flag to indicate if the tree is synced to the latest block on startup. Once the tree is initially synced to the chain tip, this field is set to true
    pub synced: AtomicBool,
}

impl<M> WorldTree<M>
where
    M: Middleware + 'static,
{
    pub fn new(
        tree_depth: usize,
        canonical_tree_manager: TreeManager<M, CanonicalTree>,
        bridged_tree_manager: Vec<TreeManager<M, BridgedTree>>,
        cache: &PathBuf,
    ) -> eyre::Result<Self> {
        let identity_tree =
            IdentityTree::new_with_cache(tree_depth, cache.to_owned())?;

        Ok(Self {
            identity_tree: Arc::new(RwLock::new(identity_tree)),
            canonical_tree_manager,
            bridged_tree_manager,
            chain_state: Arc::new(RwLock::new(HashMap::new())),
            synced: AtomicBool::new(false),
        })
    }

    /// Spawns tasks to synchronize the state of the world tree and listen for state changes across all chains
    pub async fn spawn(
        &self,
    ) -> eyre::Result<Vec<JoinHandle<eyre::Result<()>>>> {
        let start_time = Instant::now();

        // Sync the identity tree to the chain tip, also updating the chain_state with the latest roots on all chains
        tracing::info!("Syncing to head");
        self.sync_to_head().await?;
        tracing::info!("Synced to head in {:?} seconds", start_time.elapsed());

        let (leaf_updates_tx, leaf_updates_rx) =
            tokio::sync::mpsc::channel(100);
        let (bridged_root_tx, bridged_root_rx) =
            tokio::sync::mpsc::channel(100);

        // Spawn the tree managers to listen to the canonical and bridged trees for updates
        let mut handles = vec![];
        handles.push(self.canonical_tree_manager.spawn(leaf_updates_tx));

        if !self.bridged_tree_manager.is_empty() {
            for bridged_tree in self.bridged_tree_manager.iter() {
                handles.push(bridged_tree.spawn(bridged_root_tx.clone()));
            }

            // Spawn a task to handle bridged updates, updating the tree with the latest root across all chains and applying
            // pending updates when a new common root is bridged to all chains
            handles.push(self.handle_bridged_updates(bridged_root_rx));
        }

        // Spawn a task to handle canonical updates, appending new identity updates to `pending_updates` as they arrive
        handles.push(self.handle_canonical_updates(leaf_updates_rx));

        Ok(handles)
    }

    /// Spawns a task to handle updates to the canonical tree on mainnet.
    /// All updates are added to `pending_updates` and the mainnet root is updated with the latest root
    fn handle_canonical_updates(
        &self,
        mut leaf_updates_rx: Receiver<(Root, LeafUpdates)>,
    ) -> JoinHandle<eyre::Result<()>> {
        let canonical_chain_id = self.canonical_tree_manager.chain_id;
        let identity_tree = self.identity_tree.clone();
        let chain_state = self.chain_state.clone();

        tokio::spawn(async move {
            while let Some((root, leaf_updates)) = leaf_updates_rx.recv().await
            {
                let mut identity_tree = identity_tree.write().await;
                identity_tree.append_updates(root, leaf_updates);

                // Update the root for the canonical chain
                chain_state.write().await.insert(canonical_chain_id, root);
            }

            Err(eyre::eyre!("Leaf updates channel closed"))
        })
    }

    /// Spawns a task to handle updates to the bridged trees
    /// If an update results in an updated common root across all chains, all pending updates up to the previous root are applied to the tree
    fn handle_bridged_updates(
        &self,
        mut bridged_root_rx: Receiver<(u64, Hash)>,
    ) -> JoinHandle<eyre::Result<()>> {
        let identity_tree = self.identity_tree.clone();
        let chain_state = self.chain_state.clone();

        tokio::spawn(async move {
            while let Some((chain_id, bridged_root)) =
                bridged_root_rx.recv().await
            {
                let mut identity_tree = identity_tree.write().await;
                // We can use expect here because the root will always be in tree updates before the root is bridged to other chains
                let root_nonce = identity_tree
                    .roots
                    .get(&bridged_root)
                    .expect("Could not get root update");
                let new_root = Root {
                    hash: bridged_root,
                    nonce: *root_nonce,
                };

                // Get the oldest root across all chains
                let mut chain_state = chain_state.write().await;
                let oldest_root = chain_state
                    .values()
                    .min()
                    .expect("No roots in chain state");

                // Collect all chain ids where the root is the oldest root
                let oldest_chain_ids: Vec<u64> = chain_state
                    .iter()
                    .filter(|&(_, v)| v == oldest_root)
                    .map(|(&k, _)| k)
                    .collect();

                // If only one chain contains the oldest root the root update is for the oldest chain
                // apply the updates to the oldest root
                if oldest_chain_ids.len() == 1 {
                    if chain_id == oldest_chain_ids[0] {
                        identity_tree.apply_updates_to_root(&oldest_root);
                    }
                }

                // Update chain state with the new root
                chain_state.insert(chain_id, new_root);
            }

            Err(eyre::eyre!("Bridged root channel closed"))
        })
    }

    /// Fetches the latest root for all bridged chains
    /// Returns a HashMap<Hash, Vec<u64>> representing a given root and the chain IDs where the root is the latest on that chain
    async fn latest_bridged_roots(
        &self,
    ) -> eyre::Result<HashMap<Hash, Vec<u64>>> {
        // Get the latest root for all bridged chains and set the last synced block to the current block
        let futures =
            self.bridged_tree_manager
                .iter()
                .map(|tree_manager| async move {
                    let bridged_world_id = IBridgedWorldID::new(
                        tree_manager.address,
                        tree_manager.block_scanner.middleware.clone(),
                    );

                    let block_number = tree_manager
                        .block_scanner
                        .middleware
                        .get_block_number()
                        .await?
                        .as_u64();

                    let root: U256 = bridged_world_id
                        .latest_root()
                        .block(block_number)
                        .await?;

                    // Set the latest block number for the tree manager
                    tree_manager.block_scanner.next_block.store(
                        block_number + 1,
                        std::sync::atomic::Ordering::SeqCst,
                    );

                    eyre::Result::<_, eyre::Report>::Ok((
                        tree_manager.chain_id,
                        Uint::<256, 4>::from_limbs(root.0),
                    ))
                });

        let roots = futures::future::try_join_all(futures)
            .await?
            .into_iter()
            .collect::<Vec<(u64, Hash)>>();

        // Group roots by hash for easier indexing when finding the latest root for multiple chains
        let mut grouped_roots = HashMap::new();
        for (chain_id, root) in roots {
            let chain_ids = grouped_roots.entry(root).or_insert_with(Vec::new);
            chain_ids.push(chain_id);
        }

        Ok(grouped_roots)
    }

    /// Syncs the world tree to the latest block on mainnet, updating the canonical tree and bridged trees from identity updates extracted from logs
    #[instrument(skip(self))]
    pub async fn sync_to_head(&self) -> eyre::Result<()> {
        // Get logs from the canonical tree on mainnet
        let logs = self.get_canonical_logs().await?;

        // Extract identity updates from the logs and build the tree from the updates
        let identity_updates = extract_identity_updates(
            &logs,
            self.canonical_tree_manager.block_scanner.middleware.clone(),
        )
        .await?;

        self.build_tree_from_updates(identity_updates).await?;

        Ok(())
    }

    async fn get_canonical_logs(&self) -> eyre::Result<Vec<Log>> {
        let identity_tree = self.identity_tree.read().await;

        // Get all logs from the mainnet tree starting from the last synced block, up to the chain tip
        let all_logs = self.canonical_tree_manager.block_scanner.next().await?;
        if all_logs.is_empty() {
            return Err(eyre::eyre!("No logs found for canonical tree"));
        }

        // If the tree is populated, only process logs that are newer than the latest root
        let logs = if identity_tree.leaves.is_empty() {
            all_logs
        } else {
            // Split the logs
            let latest_root = identity_tree.tree.root();

            let mut pivot = all_logs.len();
            for log in all_logs.iter().rev() {
                let post_root = Hash::from_be_bytes(log.topics[3].0);
                if post_root == latest_root {
                    break;
                }
                pivot -= 1;
            }

            let (_, new_logs) = all_logs.split_at(pivot);
            new_logs.into()
        };

        Ok(logs)
    }

    async fn build_tree_from_updates(
        &self,
        identity_updates: BTreeMap<Root, LeafUpdates>,
    ) -> eyre::Result<()> {
        // Initialize the state of `self.roots` and `self.chain_state` with the latest roots from the identity updates
        self.initialize_roots(&identity_updates).await?;

        // The "canonical" tree is comprised of identity updates included in the most recent common root across all chains
        // All updates that have not yet been bridged to all chains are considered "pending" updates
        // We split up the updates into canonical and pending groups in order to build the canonical tree in memory and store the pending updates in the tree_updates map
        let (canonical_updates, pending_updates) =
            self.split_updates_at_canonical_root(identity_updates).await;

        // Build the tree from leaves extracted from the canonical updates
        self.build_canonical_tree(canonical_updates).await;

        // Apply any pending updates that have not been bridged to all chains yet
        if !pending_updates.is_empty() {
            let mut identity_tree = self.identity_tree.write().await;
            for (root, leaves) in pending_updates {
                identity_tree.append_updates(root, leaves);
            }
        }

        // Ensure that the identity tree root matches the canonical root
        let chain_state = self.chain_state.read().await;
        let canonical_root = chain_state
            .get(&self.canonical_tree_manager.chain_id)
            .expect("Could not get canonical root");

        let identity_tree = self.identity_tree.read().await;
        if identity_tree.tree.root() != canonical_root.hash {
            return Err(eyre::eyre!(
                "Identity tree root does not match canonical root"
            ));
        }

        Ok(())
    }

    /// Initializes `roots` and `chain_state` with the latest roots from the identity updates
    async fn initialize_roots(
        &self,
        identity_updates: &BTreeMap<Root, LeafUpdates>,
    ) -> eyre::Result<()> {
        let mut identity_tree = self.identity_tree.write().await;
        let mut chain_state = self.chain_state.write().await;

        // Get the latest root from all bridged chains
        let latest_bridged_roots = self.latest_bridged_roots().await?;

        if identity_updates.is_empty() {
            // @dev If there are no identity updates, meaning that the in-memory tree's latest root matches the onchain root across all chains.
            // In this case, we can set all chains to the latest root, with the root nonce set to 0. When the next canonical update is received,
            // the root for the canonical chain_id will be updated and once the new root is bridged to all chains,
            // the pending tree_updates will be applied and the root with nonce 0 will no longer be in the chain state hashmap.
            let latest_root = identity_tree.tree.root();

            let chain_ids = latest_bridged_roots
                .get(&latest_root)
                .expect("No bridged roots match the latest canonical root");

            // Ensure that all bridged chains have the same root
            if chain_ids.len() != self.bridged_tree_manager.len() {
                return Err(eyre::eyre!(
                    "Identity updates are empty but bridged roots are not the same",
                ));
            }

            let root = Root {
                hash: latest_root,
                nonce: 0,
            };

            // Update chain_state for all roots
            for chain_id in chain_ids {
                chain_state.insert(*chain_id, root);
            }
            chain_state.insert(self.canonical_tree_manager.chain_id, root);

            // Note that we do not need to insert the root into roots since it is already in the canonical tree.
            // The roots hashmap is only used when applying updates to the tree.
        } else {
            // Update chain state for bridged roots
            for root in identity_updates.keys() {
                if let Some(chain_ids) = latest_bridged_roots.get(&root.hash) {
                    for chain_id in chain_ids {
                        chain_state.insert(*chain_id, *root);
                    }
                    identity_tree.roots.insert(root.hash, root.nonce);
                }
            }

            // Update chain state for mainnet root
            let latest_mainnet_root =
                identity_updates.keys().last().expect("No updates");

            identity_tree
                .roots
                .insert(latest_mainnet_root.hash, latest_mainnet_root.nonce);

            chain_state.insert(
                self.canonical_tree_manager.chain_id,
                *latest_mainnet_root,
            );
        }

        Ok(())
    }

    /// Builds the canonical tree from identity updates
    pub async fn build_canonical_tree(
        &self,
        identity_updates: BTreeMap<Root, LeafUpdates>,
    ) {
        let mut identity_tree = self.identity_tree.write().await;

        // Flatten the leaves and build the canonical tree
        let flattened_leaves = flatten_leaf_updates(identity_updates);

        // Update the latest leaves and collect the canonical leaf values
        let canonical_leaves = flattened_leaves
            .iter()
            .map(|(idx, hash)| {
                if hash != &Hash::ZERO {
                    identity_tree.leaves.insert(*hash, idx.into());
                } else {
                    identity_tree.leaves.remove(hash);
                }

                *hash
            })
            .collect::<Vec<_>>();

        // Build the tree from leaves
        tracing::info!(num_new_leaves = ?canonical_leaves.len(), "Building the canonical tree");

        identity_tree.tree.extend_from_slice(&canonical_leaves);
    }

    /// Splits the identity updates into canonical and pending updates based on the oldest root in the chain state
    async fn split_updates_at_canonical_root(
        &self,
        mut identity_updates: BTreeMap<Root, LeafUpdates>,
    ) -> (BTreeMap<Root, LeafUpdates>, BTreeMap<Root, LeafUpdates>) {
        let chain_state = self.chain_state.read().await;
        let (_, oldest_root) = chain_state
            .iter()
            .min_by_key(|&(_, v)| v)
            .expect("No roots in chain state");

        //TODO: this can be more efficient
        let roots = identity_updates.keys().cloned().collect::<Vec<_>>();

        // Find the root at which to split the updates. `chain_state` holds the state of the latest roots for all chains.
        // The oldest common root across all chains signifies the point at which the updates should be split into canonical and pending updates
        let mut pivot = roots.len();
        for (idx, root) in roots.iter().enumerate().rev() {
            if root == oldest_root {
                pivot = idx + 1;
                break;
            }
        }

        // If the oldest root is the latest root, all updates are canonical
        if pivot == roots.len() {
            (identity_updates, BTreeMap::new())
        } else {
            // If there are pending updates, split off at the next root after the oldest root,
            // since `split_off` returns everything after the given key, including the key
            let pending_updates = identity_updates.split_off(&roots[pivot]);

            (identity_updates, pending_updates)
        }
    }

    /// Returns an inclusion proof for a given identity commitment.
    /// If a chain ID is provided, the proof is generated for the given chain.
    pub async fn inclusion_proof(
        &self,
        identity_commitment: Hash,
        chain_id: Option<ChainId>,
    ) -> eyre::Result<Option<InclusionProof>> {
        let chain_state = self.chain_state.read().await;

        let root = if let Some(chain_id) = chain_id {
            chain_state.get(&chain_id)
        } else {
            return Err(eyre::eyre!("Chain ID not found"));
        };

        let inclusion_proof = self
            .identity_tree
            .read()
            .await
            .inclusion_proof(identity_commitment, root)?;

        dbg!(&inclusion_proof);

        Ok(inclusion_proof)
    }
}

macro_rules! primitive_newtype {
    (pub struct $outer:ident($tname:ty)) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            Serialize,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Deserialize,
        )]
        pub struct $outer(pub $tname);

        impl std::fmt::Display for $outer {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl Deref for $outer {
            type Target = $tname;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl DerefMut for $outer {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
        impl From<$tname> for $outer {
            fn from(value: $tname) -> Self {
                $outer(value)
            }
        }

        impl From<$outer> for $tname {
            fn from(value: $outer) -> Self {
                value.0
            }
        }

        impl From<&$outer> for $tname {
            fn from(value: &$outer) -> Self {
                value.0
            }
        }
    };
}

primitive_newtype!(pub struct ChainId(u64));
primitive_newtype!(pub struct NodeIndex(u32));
primitive_newtype!(pub struct LeafIndex(u32));
