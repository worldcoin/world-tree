use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};

use semaphore::dynamic_merkle_tree::DynamicMerkleTree;
use semaphore::poseidon_tree::{PoseidonHash, Proof};
use semaphore::Field;
use serde::{Deserialize, Deserializer, Serialize};

use super::Hash;

pub type LeafUpdates = HashMap<u32, Hash>;
pub type NodeUpdates = HashMap<u32, Hash>;

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
    pub tree_updates: BTreeMap<Root, NodeUpdates>,
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

    pub fn inclusion_proof(
        &self,
        leaf: Hash,
        root: Option<&Root>,
    ) -> eyre::Result<Option<InclusionProof>> {
        // Get leaf index from leaves

        todo!()
    }

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

    fn leaf_to_arr_index(&self, leaf_idx: u32) -> u32 {
        let total_nodes = 1 << self.tree.depth() + 1;
        let num_leaves = 1 << self.tree.depth();
        total_nodes - num_leaves + leaf_idx
    }

    // Appends new leaf updates and newly calculated intermediate nodes to the tree updates
    pub fn append_updates(&mut self, mut updates: LeafUpdates) {
        // Flatten the last updates and the new leaf updates
        if let Some(last_update) = self.tree_updates.iter().last() {
            for (node_idx, value) in last_update.1 {
                updates.entry(*node_idx).or_insert(*value);
            }
        }

        // Calculate the affected nodes from the new leaf updates
        //NOTE:TODO: With this approach, when there are two updates that are siblings, we are doing duplicate work
        for (leaf_idx, value) in updates {
            // Collect the affected intermediate nodes

            // While index > 0{
            // Get the current idx sibling coordinates

            // Fetch the sibling from the updates. If not present, get the sibling from the tree

            // Calculate the parent hash and insert the value into the updates

            // }
        }

        //TODO: calculate root and insert into tree updates
    }

    // Applies updates up to the specified root, inclusive
    pub fn apply_updates_to_root(&mut self, root: &Root) -> eyre::Result<()> {
        // Get the update at the specified root and apply to the tree
        let update = self.tree_updates.get(&root).expect("TODO: handle error");

        for (node_idx, value) in update {
            // TODO: set node value
        }

        // Separate leaf updates from intermediate node updates

        // Split leaf insertions and deletions

        // Insert leaves

        // Delete leaves

        // Split off tree updates at the new root

        Ok(())
    }

    pub fn get_root_update_by_hash(&self, hash: &Hash) -> Option<&Root> {
        let target_root = Root {
            hash: *hash,
            block_number: 0,
        };

        if let Some((root, _)) = self.tree_updates.get_key_value(&target_root) {
            Some(root)
        } else {
            None
        }
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InclusionProof {
    pub root: Field,
    //TODO: Open a PR to semaphore-rs to deserialize proof instead of implementing deserialization here
    #[serde(deserialize_with = "deserialize_proof")]
    pub proof: Proof,
}

impl InclusionProof {
    pub fn new(root: Field, proof: Proof) -> InclusionProof {
        Self { root, proof }
    }
}

fn deserialize_proof<'de, D>(deserializer: D) -> Result<Proof, D::Error>
where
    D: Deserializer<'de>,
{
    // let value: Value = Deserialize::deserialize(deserializer)?;
    // if let Value::Array(array) = value {
    //     let mut branches = vec![];
    //     for value in array {
    //         let branch = serde_json::from_value::<Branch>(value)
    //             .map_err(serde::de::Error::custom)?;
    //         branches.push(branch);
    //     }

    //     Ok(semaphore::merkle_tree::Proof(branches))
    // } else {
    //     Err(D::Error::custom("Expected an array"))
    // }

    todo!()
}
