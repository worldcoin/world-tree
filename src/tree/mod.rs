pub mod block_scanner;
pub mod config;
pub mod error;
pub mod identity_tree;
pub mod service;
pub mod tree_manager;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use ethers::providers::Middleware;
use rayon::iter::{Either, IntoParallelIterator, ParallelIterator};
use semaphore::generic_storage::MmapVec;
use semaphore::lazy_merkle_tree::LazyMerkleTree;
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::sync::mpsc::Receiver;
use tokio::sync::{broadcast, RwLock};
use tokio::task::JoinHandle;

use self::error::WorldTreeError;
use self::identity_tree::{IdentityTree, InclusionProof, LeafUpdates, Root};
use self::tree_manager::{BridgedTree, CanonicalTree, TreeManager};
use crate::tree::identity_tree::flatten_leaf_updates;

pub mod newtypes;

pub use self::newtypes::{ChainId, LeafIndex, NodeIndex};

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
    pub bridged_tree_managers: Vec<TreeManager<M, BridgedTree>>,
    /// Mapping of chain Id -> root hash, representing the latest root for each chain
    pub chain_state: Arc<RwLock<HashMap<u64, Root>>>,
}

impl<M> WorldTree<M>
where
    M: Middleware + 'static,
{
    pub fn new(
        tree_depth: usize,
        canonical_tree_manager: TreeManager<M, CanonicalTree>,
        bridged_tree_managers: Vec<TreeManager<M, BridgedTree>>,
        cache: &PathBuf,
    ) -> Result<Self, WorldTreeError<M>> {
        let identity_tree =
            IdentityTree::new_with_cache(tree_depth, cache.to_owned())?;

        Ok(Self {
            identity_tree: Arc::new(RwLock::new(identity_tree)),
            canonical_tree_manager,
            bridged_tree_managers,
            chain_state: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Spawns tasks to synchronize the state of the world tree and listen for state changes across all chains
    pub async fn spawn(
        &self,
        cancel_tx: broadcast::Sender<()>,
    ) -> Result<Vec<JoinHandle<Result<(), WorldTreeError<M>>>>, WorldTreeError<M>>
    {
        // let start_time = Instant::now();
        // Sync the identity tree to the chain tip, also updating the chain_state with the latest roots on all chains
        // tracing::info!("Syncing to head");
        // self.sync_to_head().await?;
        // tracing::info!(
        //     sync_time = start_time.elapsed().as_millis(),
        //     "Synced to head"
        // );

        let (leaf_updates_tx, leaf_updates_rx) =
            tokio::sync::mpsc::channel(100);
        let (bridged_root_tx, bridged_root_rx) =
            tokio::sync::mpsc::channel(100);

        // Spawn the tree managers to listen to the canonical and bridged trees for updates
        let mut handles = vec![];
        handles.push(
            self.canonical_tree_manager
                .spawn(leaf_updates_tx, cancel_tx.subscribe()),
        );

        if !self.bridged_tree_managers.is_empty() {
            for bridged_tree in self.bridged_tree_managers.iter() {
                handles.push(
                    bridged_tree
                        .spawn(bridged_root_tx.clone(), cancel_tx.subscribe()),
                );
            }

            // Spawn a task to handle bridged updates, updating the tree with the latest root across all chains and applying
            // pending updates when a new common root is bridged to all chains
            handles.push(self.handle_bridged_updates(
                bridged_root_rx,
                cancel_tx.subscribe(),
            ));
        }

        // Spawn a task to handle canonical updates, appending new identity updates to `pending_updates` as they arrive
        handles.push(
            self.handle_canonical_updates(
                leaf_updates_rx,
                cancel_tx.subscribe(),
            ),
        );

        Ok(handles)
    }

    /// All updates are added to `pending_updates` and the mainnet root is updated with the latest root
    fn handle_canonical_updates(
        &self,
        leaf_updates_rx: Receiver<(Root, LeafUpdates)>,
        cancel_rx: broadcast::Receiver<()>,
    ) -> JoinHandle<Result<(), WorldTreeError<M>>> {
        // If there are no bridged trees, apply canonical updates to the tree as they arrive
        if self.bridged_tree_managers.is_empty() {
            self.apply_canonical_updates(leaf_updates_rx, cancel_rx)
        } else {
            // Otherwise, append canonical updates to `tree_updates`
            // which will be applied to the tree once the root is bridged to all chains
            self.append_canonical_updates(leaf_updates_rx, cancel_rx)
        }
    }

    // Appends canonical updates to `tree_updates` as they arrive
    fn append_canonical_updates(
        &self,
        mut leaf_updates_rx: Receiver<(Root, LeafUpdates)>,
        mut cancel_rx: broadcast::Receiver<()>,
    ) -> JoinHandle<Result<(), WorldTreeError<M>>> {
        let canonical_chain_id = self.canonical_tree_manager.chain_id;
        let identity_tree = self.identity_tree.clone();
        let chain_state = self.chain_state.clone();

        // If we are monitoring bridged chains, apply canonical updates to tree updates until the root is bridged to all chains
        tokio::spawn(async move {
            loop {
                let (new_root, leaf_updates) = tokio::select! {
                    res = leaf_updates_rx.recv() => {
                        match res {
                            Some((new_root, leaf_updates)) => (new_root, leaf_updates),
                            None => break,
                        }
                    }
                    _ = cancel_rx.recv() => {
                        break
                    }
                };

                tracing::info!(
                    ?new_root,
                    "Leaf updates received, appending tree updates"
                );
                let mut identity_tree = identity_tree.write().await;

                identity_tree.append_updates(new_root, leaf_updates)?;

                // Update the root for the canonical chain
                chain_state
                    .write()
                    .await
                    .insert(canonical_chain_id, new_root);
            }

            Err(WorldTreeError::LeafChannelClosed)
        })
    }

    // Applies canonical updates to the tree as they arrive
    fn apply_canonical_updates(
        &self,
        mut leaf_updates_rx: Receiver<(Root, LeafUpdates)>,
        mut cancel_rx: broadcast::Receiver<()>,
    ) -> JoinHandle<Result<(), WorldTreeError<M>>> {
        let canonical_chain_id = self.canonical_tree_manager.chain_id;
        let identity_tree = self.identity_tree.clone();
        let chain_state: Arc<RwLock<HashMap<u64, Root>>> =
            self.chain_state.clone();

        tokio::spawn(async move {
            loop {
                let (new_root, leaf_updates) = tokio::select! {
                    res = leaf_updates_rx.recv() => {
                        match res {
                            Some((new_root, leaf_updates)) => (new_root, leaf_updates),
                            None => break,
                        }
                    }
                    _ = cancel_rx.recv() => {
                        break
                    }
                };

                tracing::info!(
                    ?new_root,
                    "Leaf updates received, applying to the canonical tree"
                );

                match leaf_updates {
                    LeafUpdates::Insert(leaves) => {
                        let mut identity_tree = identity_tree.write().await;

                        // Sort the leaf updates by index
                        let mut leaves = leaves
                            .into_iter()
                            .map(|(idx, hash)| (idx.0, hash))
                            .collect::<Vec<_>>();

                        leaves.sort_by_key(|(idx, _)| *idx);

                        identity_tree.extend_from_slice(&leaves);
                    }
                    LeafUpdates::Delete(leaves) => {
                        let mut identity_tree = identity_tree.write().await;

                        for (leaf_idx, _) in leaves {
                            identity_tree.remove(leaf_idx.0 as usize);
                        }
                    }
                }

                // Update the root for the canonical chain
                chain_state
                    .write()
                    .await
                    .insert(canonical_chain_id, new_root);
            }

            Err(WorldTreeError::LeafChannelClosed)
        })
    }

    /// Spawns a task to handle updates to the bridged trees
    /// If an update results in an updated common root across all chains, all pending updates up to the previous root are applied to the tree
    fn handle_bridged_updates(
        &self,
        mut bridged_root_rx: Receiver<(u64, Hash)>,
        mut cancel_rx: broadcast::Receiver<()>,
    ) -> JoinHandle<Result<(), WorldTreeError<M>>> {
        let identity_tree = self.identity_tree.clone();
        let chain_state = self.chain_state.clone();

        tokio::spawn(async move {
            loop {
                let (chain_id, bridged_root) = tokio::select! {
                    res = bridged_root_rx.recv() => {
                        match res {
                            Some((chain_id, bridged_root)) => (chain_id, bridged_root),
                            None => break,
                        }
                    }
                    _ = cancel_rx.recv() => {
                        break
                    }
                };

                tracing::info!(?chain_id, root = ?bridged_root, "Bridged root received");

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

                // Update chain state with the new root
                let mut chain_state = chain_state.write().await;
                chain_state.insert(chain_id, new_root);

                let greatest_common_root = chain_state
                    .values()
                    .min()
                    .expect("No roots in chain state");

                // If the current tree root is less than the greatest common root, apply updates up to the common root across all chains
                if identity_tree.tree.root() < greatest_common_root.hash {
                    tracing::info!(
                        ?greatest_common_root,
                        "Applying updates to the canonical tree"
                    );

                    // Apply updates up to the common root
                    identity_tree.apply_updates_to_root(greatest_common_root);
                }
            }

            Err(WorldTreeError::BridgedRootChannelClosed)
        })
    }

    /// Builds the canonical tree from identity updates
    pub async fn build_canonical_tree(
        &self,
        identity_updates: BTreeMap<Root, LeafUpdates>,
    ) -> Result<(), WorldTreeError<M>> {
        let mut identity_tree = self.identity_tree.write().await;

        // Flatten the leaves and build the canonical tree
        let flattened_leaves = flatten_leaf_updates(identity_updates);

        // Update the latest leaves
        for (idx, hash) in flattened_leaves.iter() {
            if hash != &Hash::ZERO {
                identity_tree.leaves.insert(*hash, idx.into());
            } else {
                identity_tree.leaves.remove(hash);
            }
        }

        // Build the tree from leaves
        tracing::info!(num_new_leaves = ?flattened_leaves.len(), "Building the canonical tree");

        // If the tree is empty, we insert all of the canonical leaf updates
        if identity_tree.tree.num_leaves() == 0 {
            let canonical_leaves = flattened_leaves
                .iter()
                .map(|(_, hash)| *hash)
                .collect::<Vec<_>>();

            identity_tree.tree.extend_from_slice(&canonical_leaves);
        } else {
            // If the tree is already populated, we insert the new insertions
            // and then apply the deletions by setting the leaf idx to Hash::ZERO

            // Sort leaf updates into insertions and deletions
            let (insertions, deletions): (Vec<Hash>, Vec<usize>) =
                flattened_leaves.into_par_iter().partition_map(
                    |(leaf_idx, value)| {
                        if value != Hash::ZERO {
                            Either::Left(value)
                        } else {
                            Either::Right(leaf_idx.0 as usize)
                        }
                    },
                );

            // Extend the tree with the new insertions
            identity_tree.tree.extend_from_slice(&insertions);

            // Update the tree with the deletions
            for leaf_idx in deletions {
                identity_tree.tree.set_leaf(leaf_idx, Hash::ZERO);
            }
        }

        Ok(())
    }

    /// Returns an inclusion proof for a given identity commitment.
    /// If a chain ID is provided, the proof is generated for the given chain.
    pub async fn inclusion_proof(
        &self,
        identity_commitment: Hash,
        chain_id: Option<ChainId>,
    ) -> Result<Option<InclusionProof>, WorldTreeError<M>> {
        let chain_state = self.chain_state.read().await;

        let root = if let Some(chain_id) = chain_id {
            let root = chain_state
                .get(&chain_id)
                .ok_or(WorldTreeError::ChainIdNotFound)?;

            Some(root)
        } else {
            None
        };

        let inclusion_proof = self
            .identity_tree
            .read()
            .await
            .inclusion_proof(identity_commitment, root)?;

        Ok(inclusion_proof)
    }

    /// Computes the updated root given a set of identity commitments.
    /// If a chain ID is provided, the updated root is calculated from the latest root on the specified chain.
    /// If no chain ID is provided, the updated root is calculated from the latest root bridged to all chains.
    pub async fn compute_root(
        &self,
        identity_commitements: &[Hash],
        chain_id: Option<ChainId>,
    ) -> Result<Hash, WorldTreeError<M>> {
        let chain_state = self.chain_state.read().await;

        let root = if let Some(chain_id) = chain_id {
            let root = chain_state
                .get(&chain_id)
                .ok_or(WorldTreeError::ChainIdNotFound)?;

            Some(root)
        } else {
            None
        };

        let updated_root = self
            .identity_tree
            .read()
            .await
            .compute_root(identity_commitements, root)?;

        Ok(updated_root)
    }
}
