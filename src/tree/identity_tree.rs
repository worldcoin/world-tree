use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use anyhow::Context;
use ethers::providers::{Middleware, MiddlewareError};
use ethers::types::{Log, Selector, H160, U256};
use ruint::Uint;
use semaphore::dynamic_merkle_tree::DynamicMerkleTree;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::sync::mpsc::Sender;
use tracing::instrument;

use super::block_scanner::BlockScanner;
use super::tree_manager::{
    BridgedTree, CanonicalTree, TreeManager, TreeVersion,
};
use super::Hash;
use crate::abi::IBridgedWorldID;
use crate::tree::tree_manager::extract_identity_updates;

pub type IdentityUpdates = HashMap<u32, Hash>;

#[derive(PartialEq, PartialOrd, Eq, Clone, Copy)]
pub struct Root {
    pub root: Hash,
    pub block_number: u64,
}

impl Ord for Root {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.block_number.cmp(&other.block_number)
    }
}

impl std::hash::Hash for Root {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.root.hash(state);
    }
}

pub struct IdentityTree {
    pub tree: DynamicMerkleTree<PoseidonHash>,
    pub tree_updates: BTreeMap<Root, IdentityUpdates>,
    pub leaves: HashSet<Hash>,
}

impl IdentityTree {
    pub fn new(tree_depth: usize) -> Self {
        let tree = DynamicMerkleTree::new((), tree_depth, &Hash::ZERO);

        Self {
            tree,
            tree_updates: BTreeMap::new(),
            leaves: HashSet::new(),
        }

        //TODO: get proof
    }
}

pub fn flatten_updates(
    identity_updates: &BTreeMap<Root, HashMap<u32, Hash>>,
    root: Option<Root>,
) -> eyre::Result<HashMap<u32, &Hash>> {
    let mut flattened_updates = HashMap::new();

    let bound = if let Some(root) = root {
        std::ops::Bound::Included(root)
    } else {
        std::ops::Bound::Unbounded
    };

    let sub_tree = identity_updates.range((std::ops::Bound::Unbounded, bound));

    // Iterate in reverse over the sub-tree to ensure the latest updates are applied first
    for (_, updates) in sub_tree.rev() {
        for (index, hash) in updates.iter() {
            flattened_updates.entry(*index).or_insert(hash);
        }
    }

    Ok(flattened_updates)
}
