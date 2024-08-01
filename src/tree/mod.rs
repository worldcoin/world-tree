pub mod block_scanner;
pub mod config;
pub mod error;
pub mod identity_tree;
pub mod newtypes;
pub mod service;
pub mod tree_manager;

mod tasks;

use std::path::Path;
use std::sync::Arc;
use std::{process, thread};

use ethers::providers::Middleware;
use semaphore::generic_storage::MmapVec;
use semaphore::lazy_merkle_tree::LazyMerkleTree;
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::info;

use self::error::WorldTreeResult;
use self::identity_tree::{IdentityTree, InclusionProof};
pub use self::newtypes::{ChainId, LeafIndex, NodeIndex};
use self::tree_manager::{BridgedTree, CanonicalTree, TreeManager};

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
}

impl<M> WorldTree<M>
where
    M: Middleware + 'static,
{
    pub fn new(
        tree_depth: usize,
        canonical_tree_manager: TreeManager<M, CanonicalTree>,
        bridged_tree_managers: Vec<TreeManager<M, BridgedTree>>,
        cache_dir: &Path,
    ) -> WorldTreeResult<Self> {
        let mut chain_ids = vec![];

        chain_ids.push(canonical_tree_manager.chain_id);
        for tree_manager in &bridged_tree_managers {
            chain_ids.push(tree_manager.chain_id);
        }

        let chain_ids = chain_ids.into_iter().map(ChainId).collect::<Vec<_>>();
        let identity_tree = IdentityTree::new_with_cache_unchecked(
            tree_depth, cache_dir, &chain_ids,
        )?;

        let world_tree = Self {
            identity_tree: Arc::new(RwLock::new(identity_tree)),
            canonical_tree_manager,
            bridged_tree_managers,
        };

        let tree = world_tree.identity_tree.clone();
        let cache_dir = cache_dir.to_owned();

        tokio::task::spawn_blocking(move || {
            info!("Validating trees");
            let start = std::time::Instant::now();
            let tree = tree.blocking_read();

            // Scoped threads to validate all trees in parallel
            thread::scope(|s| {
                let mut join_handles = vec![];
                let j = s.spawn(|| {
                    let now = std::time::Instant::now();
                    tracing::info!("Validating canonical tree");

                    let res = tree.canonical_tree.validate();
                    let is_valid = res.is_ok();
                    let elapsed = now.elapsed();

                    tracing::info!(
                        ?elapsed,
                        is_valid,
                        "Validated canonical tree"
                    );

                    res
                });
                join_handles.push(j);

                for (chain_id, tree) in tree.trees.iter() {
                    let j = s.spawn(move || {
                        let now = std::time::Instant::now();
                        tracing::info!(%chain_id, "Validating tree");

                        let res = tree.validate();
                        let is_valid = res.is_ok();
                        let elapsed = now.elapsed();

                        tracing::info!(%chain_id, ?elapsed, is_valid, "Validated tree");

                        res
                    });
                    join_handles.push(j);
                }

                for handle in join_handles {
                    match handle.join() {
                        Err(e) => {
                            tracing::error!("Tree validation failed: {e:?}");
                            tracing::info!("Deleting cache and exiting");
                            std::fs::remove_dir_all(cache_dir).unwrap();
                            process::exit(1);
                        }
                        Ok(Err(validation_error)) => {
                            tracing::error!(
                                "Tree validation failed: {validation_error:?}"
                            );
                            tracing::info!("Deleting cache and exiting");
                            std::fs::remove_dir_all(cache_dir).unwrap();
                            process::exit(1);
                        }
                        _ => {
                            // validation succeeded
                        }
                    }
                }
            });

            let elapsed = start.elapsed();
            info!(?elapsed, "Validation complete");
        });

        Ok(world_tree)
    }

    /// Spawns tasks to synchronize the state of the world tree and listen for state changes across all chains
    pub async fn spawn(&self) -> Vec<JoinHandle<WorldTreeResult<()>>> {
        let (leaf_updates_tx, leaf_updates_rx) =
            tokio::sync::mpsc::channel(100);
        let (bridged_root_tx, bridged_root_rx) =
            tokio::sync::mpsc::channel(100);

        // Spawn the tree managers to listen to the canonical and bridged trees for updates
        let mut handles = vec![];
        handles.push(self.canonical_tree_manager.spawn(leaf_updates_tx));

        if !self.bridged_tree_managers.is_empty() {
            for bridged_tree in self.bridged_tree_managers.iter() {
                handles.push(bridged_tree.spawn(bridged_root_tx.clone()));
            }

            // Spawn a task to handle bridged updates, updating the tree with the latest root across all chains and applying
            // pending updates when a new common root is bridged to all chains
            handles.push(tokio::task::spawn(tasks::handle_bridged_updates(
                self.identity_tree.clone(),
                bridged_root_rx,
            )));
        }

        // Spawn a task to handle canonical updates, appending new identity updates to `pending_updates` as they arrive
        handles.push(tokio::task::spawn(tasks::handle_canonical_updates(
            self.canonical_tree_manager.chain_id,
            self.identity_tree.clone(),
            leaf_updates_rx,
        )));

        handles
    }

    /// Returns an inclusion proof for a given identity commitment.
    /// If a chain ID is provided, the proof is generated for the given chain.
    pub async fn inclusion_proof(
        &self,
        identity_commitment: Hash,
        chain_id: Option<ChainId>,
    ) -> WorldTreeResult<Option<InclusionProof>> {
        let identity_tree = self.identity_tree.read().await;

        let inclusion_proof =
            identity_tree.inclusion_proof(identity_commitment, chain_id)?;

        Ok(inclusion_proof)
    }
}
