pub mod builder;
pub mod canonical;
pub mod derived;

use std::cmp::min;
use std::collections::VecDeque;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

use chrono::Utc;
use ethers::providers::Middleware;
use ethers::types::H160;
use semaphore::lazy_merkle_tree::{Canonical, Derived, LazyMerkleTree};
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::{PoseidonHash, Proof};
use semaphore::{lazy_merkle_tree, Field};
use serde::Serialize;
use thiserror::Error;
use tracing::{info, warn};

pub type PoseidonTree<Version> = LazyMerkleTree<PoseidonHash, Version>;
pub type Hash = <PoseidonHash as Hasher>::Hash;

/// The most important public-facing type of this library. Exposes a type-safe
/// API for working with versioned trees. It uses interior mutability and
/// cloning it only gives a new handle on the underlying shared memory.
pub struct WorldTree<T: TreeReader + TreeWriter, M: Middleware> {
    pub address: H160,
    pub tree: Arc<RwLock<T>>,
    pub last_synced_block: u64,
    pub tree_history: Arc<VecDeque<TreeData<Derived>>>, //TODO: will probably need some arc rwlock
    pub middleware: Arc<M>,
}

impl<T: TreeReader + TreeWriter, M: Middleware> WorldTree<T, M> {
    pub fn new(
        address: H160,
        tree: Arc<RwLock<T>>,
        last_synced_block: u64,
        middleware: Arc<M>,
    ) -> Self {
        Self {
            address,
            tree,
            middleware,
            tree_history: Arc::new(VecDeque::new()),
            last_synced_block,
        }
    }
}

// impl<T: TreeReader + TreeWriter, M: Middleware> WorldTree<T, M> {
//     fn get_data(&self) -> MutexGuard<T> {
//         self.0.lock().expect("no lock poisoning")
//     }
// }

/// The public-facing API for reading from a tree version. It is implemented for
/// all versions. This being a trait allows us to hide some of the
/// implementation details.
pub trait TreeReader {
    /// Returns the current tree root.q
    fn get_root(&self) -> Hash;

    /// Returns the next free leaf.
    fn next_leaf(&self) -> usize;

    /// Returns the merkle proof and element at the given leaf.
    fn get_leaf_and_proof(&self, leaf: usize) -> (Hash, Hash, Proof);

    /// Gets the leaf value at a given index.
    fn get_proof(&self, leaf: usize) -> (Hash, Proof);

    fn get_leaf(&self, leaf: usize) -> Hash;

    fn commitments_by_indices(&self, indices: impl IntoIterator<Item = usize>) -> Vec<Hash>;
}

/// Write operations that should be available for all tree versions.
pub trait TreeWriter {
    /// Updates the tree with the given element at the given leaf index.
    fn update(&mut self, item: TreeItem) -> Hash;
}

impl<T: TreeVersion> TreeReader for TreeData<T> {
    /// Returns the current tree root.
    fn get_root(&self) -> Hash {
        self.tree.root()
    }

    /// Returns the next free leaf.
    fn next_leaf(&self) -> usize {
        self.next_leaf
    }

    /// Returns the merkle proof and element at the given leaf.
    fn get_leaf_and_proof(&self, leaf: usize) -> (Hash, Hash, Proof) {
        let proof = self.tree.proof(leaf);
        let leaf = self.tree.get_leaf(leaf);

        (leaf, self.tree.root(), proof)
    }

    /// Gets the leaf value at a given index.
    fn get_proof(&self, leaf: usize) -> (Hash, Proof) {
        let proof = self.tree.proof(leaf);

        (self.tree.root(), proof)
    }

    //TODO: docs
    fn get_leaf(&self, leaf: usize) -> Hash {
        self.tree.get_leaf(leaf)
    }

    //TODO: docs
    fn commitments_by_indices(&self, indices: impl IntoIterator<Item = usize>) -> Vec<Hash> {
        let mut commitments = vec![];

        for idx in indices {
            commitments.push(self.tree.get_leaf(idx));
        }

        commitments
    }
}

/// The marker trait for linear ordering of tree versions. It also defines the
/// marker for underlying tree storage.
pub trait TreeVersion
where
    Self: lazy_merkle_tree::VersionMarker,
{
}

/// Underlying data structure for a tree version. It holds the tree itself, the
/// next leaf (only used in the latest tree), a pointer to the next version (if
/// exists) and the metadata specified by the version marker.
///
pub struct TreeData<T: TreeVersion> {
    pub tree: PoseidonTree<T>,
    pub next_leaf: usize,
}

impl<T: TreeVersion> TreeData<T> {
    pub fn new(tree: PoseidonTree<T>, next_leaf: usize) -> Self {
        Self { tree, next_leaf }
    }
}

pub struct TreeItem {
    pub leaf_index: usize,
    pub element: Hash,
}

impl TreeItem {
    pub const fn new(leaf_index: usize, element: Hash) -> Self {
        Self {
            leaf_index,
            element,
        }
    }
}

// TODO: Move this to inclusion proof module
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InclusionProof {
    pub root: Field,
    pub proof: Proof,
    pub message: Option<String>,
}
