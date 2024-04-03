use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap, VecDeque};

use eyre::OptionExt;
use semaphore::dynamic_merkle_tree::DynamicMerkleTree;
use semaphore::merkle_tree::{Branch, Hasher};
use semaphore::poseidon_tree::{PoseidonHash, Proof};
use semaphore::Field;
use serde::{Deserialize, Deserializer, Serialize};

use super::Hash;

pub enum LeafUpdates {
    Insert(Leaves),
    Delete(Leaves),
}

impl Into<Leaves> for LeafUpdates {
    fn into(self) -> Leaves {
        match self {
            LeafUpdates::Insert(leaves) => leaves,
            LeafUpdates::Delete(leaves) => leaves,
        }
    }
}

// Node index to hash, 0 indexed from the root
pub type StorageUpdates = HashMap<u32, Hash>;

// Leaf index to hash, 0 indexed from the initial leaf
pub type Leaves = HashMap<u32, Hash>;

pub fn leaf_to_storage_idx(leaf_idx: u32, tree_depth: usize) -> u32 {
    let leaf_0 = (1 << tree_depth) - 1;
    leaf_0 + leaf_idx
}

pub fn storage_to_leaf_idx(storage_idx: u32, tree_depth: usize) -> u32 {
    let leaf_0 = (1 << tree_depth) - 1;
    storage_idx - leaf_0
}

pub fn storage_idx_to_coords(index: usize) -> (usize, usize) {
    let depth = (index + 1).ilog2();
    let offset = index - (2usize.pow(depth) - 1);
    (depth as usize, offset)
}

#[derive(PartialEq, PartialOrd, Eq, Clone, Copy, Debug)]
pub struct Root {
    pub hash: Hash,
    //NOTE: note that this assumes that there is only one wallet that sequences transactions,
    // in the future, we should emit a root nonce with the tree changes or a similar mechanism so that it is
    // easy to sync with multiple sequencers
    pub nonce: usize,
}

impl Ord for Root {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.nonce.cmp(&other.nonce)
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
        let leaf_idx = self.leaves.get(&leaf).ok_or_eyre("Leaf not found")?;

        if let Some(root) = root {
            if root.hash == self.tree.root() {
                let proof = self.tree.proof(*leaf_idx as usize);
                return Ok(Some(InclusionProof::new(self.tree.root(), proof)));
            } else {
                let proof = self.construct_proof_from_root(*leaf_idx, root)?;
                return Ok(Some(InclusionProof::new(root.hash, proof)));
            }
        } else {
            let proof = self.tree.proof(*leaf_idx as usize);
            return Ok(Some(InclusionProof::new(self.tree.root(), proof)));
        }
    }

    pub fn construct_proof_from_root(
        &self,
        leaf_idx: u32,
        root: &Root,
    ) -> eyre::Result<Proof> {
        let updates = self
            .tree_updates
            .get(root)
            .ok_or_eyre("Could not find root in tree updates")?;

        let mut node_idx = leaf_to_storage_idx(leaf_idx, self.tree.depth());

        let mut proof: Vec<Branch<Hash>> = vec![];

        while node_idx > 0 {
            let sibling_idx = if node_idx % 2 == 0 {
                node_idx - 1
            } else {
                node_idx + 1
            };

            let sibling = updates
                .get(&sibling_idx)
                .copied()
                .or_else(|| {
                    let (depth, offset) =
                        storage_idx_to_coords(sibling_idx as usize);
                    Some(self.tree.get_node(depth, offset))
                })
                .expect("Could not find node in tree");

            proof.push(if node_idx % 2 == 0 {
                Branch::Right(sibling)
            } else {
                Branch::Left(sibling)
            });

            node_idx = (node_idx - 1) / 2;
        }

        Ok(semaphore::merkle_tree::Proof(proof))
    }

    pub fn insert(&mut self, index: u32, value: Hash) {
        //TODO: do we want to return an error if a non unique leaf is being inserted?
        self.leaves.insert(value, index);
        // We can expect here because the `reallocate` implementation for Vec<H::Hash> as DynamicTreeStorage does not fail
        self.tree.push(value).expect("Failed to insert into tree");
    }

    pub fn remove(&mut self, index: usize) {
        let leaf = self.tree.get_leaf(index);
        self.leaves.remove(&leaf);
        self.tree.set_leaf(index, Hash::ZERO);
    }

    // Appends new leaf updates and newly calculated intermediate nodes to the tree updates
    pub fn append_updates(&mut self, root: Root, leaf_updates: LeafUpdates) {
        // Update leaves
        match leaf_updates {
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

        let mut updates = HashMap::new();
        let mut node_queue = VecDeque::new();

        // Convert leaf indices into storage indices and insert into updates
        let leaves: Leaves = leaf_updates.into();
        for (leaf_idx, hash) in leaves.into_iter() {
            let storage_idx = leaf_to_storage_idx(leaf_idx, self.tree.depth());
            updates.insert(storage_idx, hash);

            // Queue the parent index
            let parent_idx = (storage_idx - 1) / 2;
            node_queue.push_front(parent_idx);
        }

        let prev_update = if let Some(update) = self.tree_updates.iter().last()
        {
            //TODO: Use a more efficient approach than to clone the last update
            update.1.clone()
        } else {
            HashMap::new()
        };

        while let Some(node_idx) = node_queue.pop_back() {
            // Check if the parent is already in the updates hashmap, indicating it has already been calculated
            //TODO: note why we set to 0 if idx is 0
            let parent_idx = if node_idx == 0 { 0 } else { (node_idx - 1) / 2 };
            if updates.contains_key(&parent_idx) {
                continue;
            }

            let left_sibling_idx = node_idx * 2 + 1;
            let right_sibling_idx = node_idx * 2 + 2;

            // Get the left sibling, with precedence given to the updates
            let left = updates
                .get(&left_sibling_idx)
                .copied()
                .or_else(|| prev_update.get(&left_sibling_idx).copied())
                .or_else(|| {
                    let (depth, offset) =
                        storage_idx_to_coords(left_sibling_idx as usize);
                    Some(self.tree.get_node(depth, offset))
                })
                .expect("Could not find node in tree");

            // Get the right sibling, with precedence given to the updates
            let right = updates
                .get(&right_sibling_idx)
                .copied()
                .or_else(|| prev_update.get(&right_sibling_idx).copied())
                .or_else(|| {
                    let (depth, offset) =
                        storage_idx_to_coords(right_sibling_idx as usize);
                    Some(self.tree.get_node(depth, offset))
                })
                .expect("Could not find node in tree");

            let hash = PoseidonHash::hash_node(&left, &right);

            updates.insert(node_idx, hash);

            // Queue the parent index if not the root
            if node_idx != 0 {
                node_queue.push_front(parent_idx);
            }
        }

        // Flatten any remaining updates from the previous update
        for update in prev_update {
            if !updates.contains_key(&update.0) {
                updates.insert(update.0, update.1);
            }
        }

        self.tree_updates.insert(root, updates);
    }

    // Applies updates up to the specified root, inclusive
    pub fn apply_updates_to_root(&mut self, root: &Root) -> eyre::Result<()> {
        // Get the update at the specified root and apply to the tree
        if let Some(update) = self.tree_updates.remove(root) {
            // Filter out updates that are not leaves
            let mut leaf_updates = update
                .into_iter()
                .filter_map(|(idx, value)| {
                    if idx >= 1 << self.tree.depth() {
                        let leaf_idx =
                            storage_to_leaf_idx(idx, self.tree.depth());
                        Some((leaf_idx, value))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            // Sort leaves by leaf idx
            leaf_updates.sort_by_key(|(idx, _)| *idx);

            // Apply all leaf updates to the tree
            for (leaf_idx, val) in leaf_updates {
                // Insert/update leaves in the canonical tree
                // Note that the leaves are inserted/removed from the leaves hashmap when the updates are first applied to tree_updates
                if val == Hash::ZERO {
                    //TODO:FIXME: is it possible that this leaf is not actually in the dynamic tree already?
                    self.tree.set_leaf(leaf_idx as usize, Hash::ZERO);
                } else {
                    self.tree.push(val)?;
                }
            }
        }

        // Split off tree updates at the new root
        // Since the root was already removed from the updates, we can use split_off to separate the updates non inclusive of the root
        self.tree_updates = self.tree_updates.split_off(root);

        Ok(())
    }

    pub fn get_root_by_hash(&self, hash: &Hash) -> Option<&Root> {
        let target_root = Root {
            hash: *hash,
            nonce: 0,
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
        let updates: Leaves = leaves.into();

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

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use std::time::{self, Instant, SystemTime, UNIX_EPOCH};

    use eyre::eyre;
    use semaphore::dynamic_merkle_tree::DynamicMerkleTree;
    use semaphore::merkle_tree::Hasher;
    use semaphore::poseidon_tree::PoseidonHash;

    use super::{leaf_to_storage_idx, IdentityTree, LeafUpdates, Root};
    use crate::tree::identity_tree::{
        storage_idx_to_coords, storage_to_leaf_idx,
    };
    use crate::tree::Hash;

    const TREE_DEPTH: usize = 2;
    const NUM_LEAVES: usize = 1 << TREE_DEPTH;

    fn generate_leaves() -> Vec<Hash> {
        (0..NUM_LEAVES)
            .into_iter()
            .map(Hash::from)
            .collect::<Vec<_>>()
    }

    #[test]
    fn test_leaf_to_storage_idx() {
        for i in 0..1 << TREE_DEPTH {
            let storage_idx = leaf_to_storage_idx(i, TREE_DEPTH);
            let expected_storage_idx = (1 << TREE_DEPTH) + i - 1;
            assert_eq!(storage_idx, expected_storage_idx);
        }
    }

    #[test]
    fn test_storage_to_leaf_idx() {
        for i in 0..1 << TREE_DEPTH {
            let storage_idx = leaf_to_storage_idx(i, TREE_DEPTH);
            let leaf_idx = storage_to_leaf_idx(storage_idx, TREE_DEPTH);
            assert_eq!(leaf_idx, i);
        }
    }

    #[test]
    fn test_storage_idx_to_coords() {
        for i in 0..1 << TREE_DEPTH {
            let storage_idx = leaf_to_storage_idx(i, TREE_DEPTH);

            let (depth, offset) = storage_idx_to_coords(storage_idx as usize);

            let expected_depth = (storage_idx + 1).ilog2();
            let expected_offset = storage_idx - (2_u32.pow(expected_depth) - 1);

            assert_eq!(depth, expected_depth as usize);
            assert_eq!(offset, expected_offset as usize);
        }
    }

    #[test]
    fn test_insert() {
        let mut identity_tree = IdentityTree::new(TREE_DEPTH);

        // Generate new leaves and insert into the tree
        let leaves = generate_leaves();
        for (idx, leaf) in leaves.iter().enumerate() {
            identity_tree.insert(idx as u32, *leaf);
        }

        // Initialize an expected tree with the same leaves
        let expected_tree: DynamicMerkleTree<PoseidonHash> =
            DynamicMerkleTree::new_with_leaves(
                (),
                TREE_DEPTH,
                &Hash::ZERO,
                &leaves,
            );

        // Ensure the tree roots are equal
        assert_eq!(identity_tree.tree.root(), expected_tree.root());

        // Assert that each of the leaves are in the leaves hashmap
        for leaf_idx in 0..1 << TREE_DEPTH {
            let leaf = Hash::from(leaf_idx);
            assert_eq!(identity_tree.leaves.get(&leaf), Some(&leaf_idx));
        }
    }

    #[test]
    fn test_remove() {
        let mut identity_tree = IdentityTree::new(TREE_DEPTH);

        // Generate new leaves and insert into the tree
        let leaves = generate_leaves();
        for (idx, leaf) in leaves.iter().enumerate() {
            identity_tree.insert(idx as u32, *leaf);
        }

        // Remove each leaf from the tree
        for i in 0..1 << TREE_DEPTH {
            identity_tree.remove(i as usize);
        }

        // Initialize an expected tree with all leaves set to 0x00
        let expected_tree: DynamicMerkleTree<PoseidonHash> =
            DynamicMerkleTree::new_with_leaves(
                (),
                TREE_DEPTH,
                &Hash::ZERO,
                &vec![Hash::default(); leaves.len()],
            );

        // Ensure the tree roots are equal
        assert_eq!(identity_tree.tree.root(), expected_tree.root());

        // Assert that each of the leaves are not in the leaves hashmap
        for leaf in 0..1 << TREE_DEPTH {
            let leaf_hash = Hash::from(leaf);
            assert_eq!(identity_tree.leaves.get(&leaf_hash), None);
        }
    }

    #[test]
    fn test_append_updates() {
        let mut identity_tree = IdentityTree::new(TREE_DEPTH);

        // Generate the first half of the leaves and insert into the tree
        let leaves = generate_leaves();
        for (idx, leaf) in leaves[0..NUM_LEAVES / 2].iter().enumerate() {
            identity_tree.insert(idx as u32, *leaf);
        }

        let expected_root = identity_tree.tree.root();

        // Generate the updated tree with all of the leaves
        let updated_tree: DynamicMerkleTree<PoseidonHash> =
            DynamicMerkleTree::new_with_leaves(
                (),
                TREE_DEPTH,
                &Hash::ZERO,
                &leaves,
            );

        // Append the new leaves to the tree
        let new_root = Root {
            hash: updated_tree.root(),
            nonce: 1,
        };

        // Collect the second half of the leaves
        let leaf_updates = leaves[(NUM_LEAVES / 2)..NUM_LEAVES]
            .iter()
            .enumerate()
            .map(|(idx, value)| (idx as u32, *value))
            .collect::<HashMap<u32, Hash>>();

        identity_tree
            .append_updates(new_root, LeafUpdates::Insert(leaf_updates));

        // Ensure that the root is correct
        assert_eq!(identity_tree.tree.root(), expected_root);
        assert_eq!(identity_tree.tree_updates.len(), 1);

        //TODO: create expected updates

        // let updates = identity_tree
        //     .tree_updates
        //     .get(&new_root)
        //     .expect("Could not get updates");

        // assert_eq!(updates, &new_leaves);
    }

    #[test]
    fn test_apply_updates_to_root() -> eyre::Result<()> {
        let mut identity_tree = IdentityTree::new(TREE_DEPTH);

        // Generate the first half of the leaves and insert into the tree
        let leaves = generate_leaves();

        for (idx, leaf) in leaves[0..NUM_LEAVES / 2].iter().enumerate() {
            identity_tree.insert(idx as u32, *leaf);
        }

        // Generate the updated tree with all of the leaves
        let expected_tree: DynamicMerkleTree<PoseidonHash> =
            DynamicMerkleTree::new_with_leaves(
                (),
                TREE_DEPTH,
                &Hash::ZERO,
                &leaves,
            );

        let expected_root = expected_tree.root();

        // Append the new leaves to the tree
        let new_root = Root {
            hash: expected_root,
            nonce: 1,
        };

        // Collect the second half of the leaves
        let leaf_updates = leaves[(NUM_LEAVES / 2)..]
            .iter()
            .enumerate()
            .map(|(idx, value)| ((NUM_LEAVES / 2 + idx) as u32, *value))
            .collect::<HashMap<u32, Hash>>();

        identity_tree
            .append_updates(new_root, LeafUpdates::Insert(leaf_updates));

        // Apply updates to the tree
        identity_tree.apply_updates_to_root(&new_root)?;

        assert_eq!(identity_tree.tree.root(), expected_root);
        assert_eq!(identity_tree.tree_updates.len(), 0);

        for (leaf_idx, leaf) in leaves.iter().enumerate() {
            let proof = identity_tree
                .inclusion_proof(*leaf, None)?
                .ok_or(eyre!("Proof not found"))?;

            assert_eq!(proof.root, expected_root);
            assert_eq!(proof.proof, expected_tree.proof(leaf_idx));
        }

        Ok(())
    }

    #[test]
    fn test_flatten_leaf_updates() {}

    #[test]
    fn test_inclusion_proof() {}

    #[test]
    fn test_construct_proof_from_root() {}

    #[test]
    fn test_get_root_by_hash() {}
}
