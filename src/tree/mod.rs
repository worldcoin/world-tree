pub mod block_scanner;
pub mod config;
pub mod error;
pub mod identity_tree;
pub mod newtypes;
pub mod service;
pub mod tree_manager;

mod tasks;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use ethers::providers::Middleware;
use semaphore::generic_storage::MmapVec;
use semaphore::lazy_merkle_tree::LazyMerkleTree;
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use self::error::WorldTreeError;
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

    /// Mapping of chain Id -> the latest observed root
    ///
    /// This mapping is used to monitor if observed chains
    /// are synced with the canonical chain
    pub chain_state: Arc<RwLock<HashMap<u64, Hash>>>,
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
    ) -> Result<Vec<JoinHandle<Result<(), WorldTreeError<M>>>>, WorldTreeError<M>>
    {
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
                self.chain_state.clone(),
                bridged_root_rx,
            )));
        }

        // Spawn a task to handle canonical updates, appending new identity updates to `pending_updates` as they arrive
        handles.push(tokio::task::spawn(tasks::handle_canonical_updates(
            self.canonical_tree_manager.chain_id,
            self.identity_tree.clone(),
            self.chain_state.clone(),
            leaf_updates_rx,
        )));

        Ok(handles)
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
