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
    pub hash: Hash,
    pub block_number: u64,
}

impl Ord for Root {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.block_number.cmp(&other.block_number)
    }
}

impl std::hash::Hash for Root {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

pub struct IdentityTree {
    pub tree: DynamicMerkleTree<PoseidonHash>,
    pub tree_updates: BTreeMap<Root, IdentityUpdates>,
    pub leaves: HashMap<Hash, usize>,
}

impl IdentityTree {
    pub fn new(tree_depth: usize) -> Self {
        let tree = DynamicMerkleTree::new((), tree_depth, &Hash::ZERO);

        Self {
            tree,
            tree_updates: BTreeMap::new(),
            leaves: HashMap::new(),
        }
    }
    //TODO: get proof

    pub fn insert(&mut self, index: usize, value: Hash) {
        self.leaves.insert(value, index);
        // We can expect here because the `reallocate` implementation for Vec<H::Hash> as DynamicTreeStorage does not fail
        self.tree.push(value).expect("Failed to insert into tree");
    }

    pub fn insert_many(&mut self, values: &[(usize, Hash)]) {
        for (index, value) in values {
            self.insert(*index, *value);
        }
    }

    pub fn remove(&mut self, index: usize) {
        let leaf = self.tree.get_leaf(index);
        self.leaves.remove(&leaf);
        self.tree.set_leaf(index, Hash::ZERO);
    }

    pub fn remove_many(&mut self, indices: &[usize]) {
        for index in indices {
            self.remove(*index);
        }
    }

    pub fn insert_many_leaves(&mut self, leaves: &[(usize, Hash)]) {
        for (index, value) in leaves {
            self.leaves.insert(*value, *index);
        }
    }

    pub fn remove_many_leaves(&mut self, leaves: &[Hash]) {
        for value in leaves {
            self.leaves.remove(&value);
        }
    }

    //TODO: function to append updates. We need to decide if the updates are going to be be all intermediate nodes from all prev updates or just from prev update to this one.
    //NOTE: it depends on if we want to just have it ready to index or flatten when a proof comes in, probably ready to index im thinking

    // Applies updates up to the specified root, inclusive
    pub fn apply_updates_to_root(&mut self, root: Hash) -> eyre::Result<()> {
        let flattened_updates = flatten_updates(
            &self.tree_updates,
            Some(Root {
                hash: root,
                block_number: 0,
            }),
        )?;

        //TODO: delete many

        //TODO: insert many

        //TODO: split off at new oldest root

        Ok(())
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
