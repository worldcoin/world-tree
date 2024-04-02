use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap, VecDeque};

use semaphore::dynamic_merkle_tree::DynamicMerkleTree;
use semaphore::poseidon_tree::{PoseidonHash, Proof};
use semaphore::Field;
use serde::{Deserialize, Deserializer, Serialize};

use super::Hash;

pub type StorageUpdates = HashMap<u32, Hash>;
pub type Leaves = HashMap<u32, Hash>;

pub enum LeafUpdates {
    Insert(Leaves),
    Delete(Leaves),
}

impl Into<Leaves> for LeafUpdates {
    fn into(self) -> Leaves {
        match self {
            LeafUpdates::Insert(updates) => updates,
            LeafUpdates::Delete(updates) => updates,
        }
    }
}

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
    pub tree_updates: BTreeMap<Root, StorageUpdates>,
    pub leaves: HashMap<Hash, u32>,
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

    pub fn insert(&mut self, index: u32, value: Hash) {
        self.leaves.insert(value, index);
        // We can expect here because the `reallocate` implementation for Vec<H::Hash> as DynamicTreeStorage does not fail
        self.tree.push(value).expect("Failed to insert into tree");
    }

    pub fn insert_many(&mut self, values: &[(u32, Hash)]) {
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

    pub fn insert_many_leaves(&mut self, leaves: &[(u32, Hash)]) {
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
    pub fn append_updates(&mut self, root: Root, leaves: LeafUpdates) {
        // Update leaves
        match leaves {
            LeafUpdates::Insert(ref updates) => {
                for (idx, val) in updates.iter() {
                    self.leaves.insert(*val, *idx);
                }
            }
            LeafUpdates::Delete(ref updates) => {
                for (_, val) in updates.iter() {
                    self.leaves.remove(val);
                }
            }
        }

        let mut updates: NodeUpdates = leaves.into();

        // Flatten the last updates and the new leaf updates
        if let Some(last_update) = self.tree_updates.iter().last() {
            for (node_idx, value) in last_update.1 {
                updates.entry(*node_idx).or_insert(*value);
            }
        }

        // Calculate the affected nodes from the new leaf updates
        //NOTE:TODO: With this approach, when there are two updates that are siblings, we are doing duplicate work

        let mut affected_nodes = VecDeque::new();

        // 1. Add all leaf updates to the affected_ndoes queue before flattening the updates
        // 2. Flatten the updates NOTE: this actually wont work since you need to know if the parent is in the updates hashmap
        // 3. For each node in the queue
        // - check if the parent is already in the updates hashmap, if so continue to the next node in the queue
        // - if not, check if the node is in the updates hashmap
        // - if not, get it from the tree
        // - calculate the parent hash and insert it into the updates hashmap
        // - queue the parent index in the queue

        // 4. stop when you have calculated the new root

        //NOTE: make a note about having a different algo for deletions since it will be unlikely that two deletions will be siblings and you can just do the dup work in the very small chance they are

        // //NOTE: you can go through reverse
        // for (leaf_idx, value) in updates {

        for (leaf_idx, value) in updates {
            // Collect the affected intermediate nodes

            // While index > 0{
            // Get the current idx sibling coordinates

            // Fetch the sibling from the updates. If not present, get the sibling from the tree

            // Calculate the parent hash and insert the value into the updates
        }

        //TODO: calculate root and ensure it matches the expected root
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

    pub fn get_root_by_hash(&self, hash: &Hash) -> Option<&Root> {
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

pub fn flatten_leaf_updates(
    leaf_updates: BTreeMap<Root, LeafUpdates>,
) -> eyre::Result<Vec<(u32, Hash)>> {
    let mut flattened_updates = HashMap::new();

    // Iterate in reverse over the sub-tree to ensure the latest updates are applied first
    for (_, leaves) in leaf_updates.into_iter().rev() {
        let updates: NodeUpdates = leaves.into();

        for (index, hash) in updates.into_iter() {
            flattened_updates.entry(index).or_insert(hash);
        }
    }

    let mut updates = flattened_updates.into_iter().collect::<Vec<_>>();
    updates.sort_by_key(|(idx, _)| *idx);

    Ok(updates)
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
