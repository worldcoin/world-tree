use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

use rayon::iter::{Either, IntoParallelIterator, ParallelIterator};
use semaphore::cascading_merkle_tree::CascadingMerkleTree;
use semaphore::generic_storage::{GenericStorage, MmapVec};
use semaphore::merkle_tree::{Branch, Hasher};
use semaphore::poseidon_tree::{PoseidonHash, Proof};
use semaphore::Field;
use serde::Serialize;

use super::error::IdentityTreeError;
use super::{Hash, LeafIndex, NodeIndex};

// Leaf index to hash, 0 indexed from the initial leaf
pub type Leaves = HashMap<LeafIndex, Hash>;
// Node index to hash, 0 indexed from the root
pub type StorageUpdates = HashMap<NodeIndex, Hash>;

pub struct IdentityTree<S> {
    pub tree: CascadingMerkleTree<PoseidonHash, S>,
    pub tree_updates: BTreeMap<Root, StorageUpdates>,
    // Hashmap of root hash to nonce
    pub roots: HashMap<Hash, usize>,
    pub leaves: HashMap<Hash, u32>,
}

impl IdentityTree<Vec<Hash>> {
    pub fn new(depth: usize) -> Self {
        let tree = CascadingMerkleTree::new(vec![], depth, &Hash::ZERO);

        Self {
            tree,
            tree_updates: BTreeMap::new(),
            roots: HashMap::new(),
            leaves: HashMap::new(),
        }
    }
}

impl IdentityTree<MmapVec<Hash>> {
    pub fn new_with_cache(
        depth: usize,
        file_path: PathBuf,
    ) -> Result<Self, IdentityTreeError> {
        let mmap_vec: MmapVec<Hash> =
            match unsafe { MmapVec::restore(&file_path) } {
                Ok(mmap_vec) => mmap_vec,

                Err(_e) => unsafe {
                    tracing::info!("Cache not found, creating new cache file");
                    MmapVec::open_create(file_path)?
                },
            };

        let tree = if mmap_vec.is_empty() {
            CascadingMerkleTree::<PoseidonHash, _>::new(
                mmap_vec,
                depth,
                &Hash::ZERO,
            )
        } else {
            let now = Instant::now();
            tracing::info!("Restoring tree from cache");
            let tree = CascadingMerkleTree::<PoseidonHash, _>::restore(
                mmap_vec,
                depth,
                &Hash::ZERO,
            )?;

            tracing::info!("Restored tree from cache in {:?}", now.elapsed());
            tree
        };

        // If the tree has leaves, restore the leaves hashmap
        let leaves = tree
            .leaves()
            .enumerate()
            .filter_map(|(idx, leaf)| {
                if leaf != Hash::ZERO {
                    Some((leaf, idx as u32))
                } else {
                    None
                }
            })
            .collect::<HashMap<Hash, u32>>();

        Ok(Self {
            tree,
            leaves,
            tree_updates: BTreeMap::new(),
            roots: HashMap::new(),
        })
    }
}

impl<S> IdentityTree<S>
where
    S: GenericStorage<Hash>,
{
    /// Inserts a new leaf into the tree and updates the leaves hashmap
    /// Returns an error if the leaf already exists
    pub fn insert(
        &mut self,
        index: u32,
        leaf: Hash,
    ) -> Result<(), IdentityTreeError> {
        // Check if the leaf already exists
        if self.leaves.contains_key(&leaf) {
            return Err(IdentityTreeError::LeafAlreadyExists);
        }
        self.leaves.insert(leaf, index);

        // We can expect here because the `reallocate` implementation for Vec<H::Hash> as DynamicTreeStorage does not fail
        self.tree.push(leaf).expect("Failed to insert into tree");

        Ok(())
    }

    /// Removes a leaf from the tree and updates the leaves hashmap
    pub fn remove(&mut self, index: usize) {
        let leaf = self.tree.get_leaf(index);
        self.leaves.remove(&leaf);
        self.tree.set_leaf(index, Hash::ZERO);
    }

    // Appends new leaf updates to the `leaves` hashmap and adds newly calculated storage nodes to `tree_updates`
    pub fn append_updates(&mut self, root: Root, leaf_updates: LeafUpdates) {
        self.update_leaves(&leaf_updates);

        let updates = self.construct_storage_updates(leaf_updates);
        self.tree_updates.insert(root, updates);
        self.roots.insert(root.hash, root.nonce);
    }

    fn update_leaves(&mut self, leaf_updates: &LeafUpdates) {
        match &leaf_updates {
            LeafUpdates::Insert(updates) => {
                for (idx, val) in updates.iter() {
                    self.leaves.insert(*val, idx.into());
                }
            }
            LeafUpdates::Delete(updates) => {
                for (_, val) in updates.iter() {
                    self.leaves.remove(val);
                }
            }
        }
    }

    /// Constructs storage updates from leaf updates
    /// The identity tree maintains a sequence of `tree_updates` which consists of BTreeMap<Root, StorageUpdates>,
    /// representing the updated nodes within the tree for a given root. Each update flattens the previous update and overwrites any nodes that change as a result
    /// from the newly added leaf updates. Storing the node updates for a given root allows for efficient construction
    /// of inclusion proofs for a given root without needing to recalculate nodes upon each request.
    fn construct_storage_updates(
        &self,
        leaf_updates: LeafUpdates,
    ) -> StorageUpdates {
        let mut updates = HashMap::new();
        let mut node_queue = VecDeque::new();

        // Convert leaf indices into storage indices and insert into updates
        let leaves: Leaves = leaf_updates.into();
        for (leaf_idx, hash) in leaves.into_iter() {
            let storage_idx = leaf_to_storage_idx(*leaf_idx, self.tree.depth());
            updates.insert(storage_idx.into(), hash);

            // Queue the parent index
            let parent_idx = (storage_idx - 1) / 2;
            node_queue.push_front(parent_idx);
        }

        // Get the previous update to flatten existing storage nodes into the newly updated nodes
        let prev_update = if let Some(update) = self.tree_updates.iter().last()
        {
            update.1.clone()
        } else {
            HashMap::new()
        };

        while let Some(node_idx) = node_queue.pop_back() {
            // Check if the parent is already in the updates hashmap, indicating it has already been calculated
            let parent_idx = if node_idx == 0 {
                continue;
            } else {
                (node_idx - 1) / 2
            };
            if updates.contains_key(&parent_idx.into()) {
                continue;
            }

            let left_child_idx = node_idx * 2 + 1;
            let right_child_idx = node_idx * 2 + 2;

            // Get the left child, with precedence given to the updates
            let left = updates
                .get(&left_child_idx.into())
                .copied()
                // If the left child is not in the updates, check the previous update
                .or_else(|| prev_update.get(&left_child_idx.into()).copied())
                // Otherwise, get the node from the tree
                .or_else(|| {
                    let (depth, offset) =
                        storage_idx_to_coords(left_child_idx as usize);
                    Some(self.tree.get_node(depth, offset))
                })
                .expect("Could not find node in tree");

            // Get the right child, with precedence given to the updates
            let right = updates
                .get(&right_child_idx.into())
                .copied()
                // If the right child is not in the updates, check the previous update
                .or_else(|| prev_update.get(&right_child_idx.into()).copied())
                // Otherwise, get the node from the tree
                .or_else(|| {
                    let (depth, offset) =
                        storage_idx_to_coords(right_child_idx as usize);
                    Some(self.tree.get_node(depth, offset))
                })
                .expect("Could not find node in tree");

            let hash = PoseidonHash::hash_node(&left, &right);

            updates.insert(node_idx.into(), hash);

            // Queue the parent index if not the root
            if node_idx != 0 {
                node_queue.push_front(parent_idx);
            }
        }

        // Flatten any remaining updates from the previous update
        for (node_idx, hash) in prev_update {
            updates.entry(node_idx).or_insert(hash);
        }

        updates
    }

    // Applies updates up to the specified root, inclusive
    pub fn apply_updates_to_root(&mut self, root: &Root) {
        // Get the update at the specified root and apply to the tree
        if let Some(update) = self.tree_updates.remove(root) {
            self.roots.remove(&root.hash);

            // Filter out updates that are not leaves
            let mut leaf_updates = update
                .into_iter()
                .filter_map(|(idx, value)| {
                    if *idx >= 1 << self.tree.depth() {
                        let leaf_idx =
                            storage_to_leaf_idx(*idx, self.tree.depth());
                        Some((leaf_idx, value))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            leaf_updates.sort_by_key(|(idx, _)| *idx);

            // Partition the leaf updates into insertions and deletions
            let (insertions, deletions): (Vec<Hash>, Vec<usize>) = leaf_updates
                .into_par_iter()
                .partition_map(|(leaf_idx, value)| {
                    if value != Hash::ZERO {
                        Either::Left(value)
                    } else {
                        Either::Right(leaf_idx as usize)
                    }
                });

            // Insert/delete leaves in the canonical tree
            // Note that the leaves are inserted/removed from the leaves hashmap when the updates are first applied to tree_updates
            self.tree.extend_from_slice(&insertions);

            for leaf_idx in deletions {
                self.tree.set_leaf(leaf_idx, Hash::ZERO);
            }
        }

        // Split off tree updates at the new root
        // Since the root was already removed from the updates, we can use split_off to separate the updates non inclusive of the root
        let current_tree_updates = self.tree_updates.split_off(root);

        // Clean up any roots that are no longer needed
        for root in self.tree_updates.keys() {
            self.roots.remove(&root.hash);
        }

        self.tree_updates = current_tree_updates;
    }

    /// Construct an inclusion proof for a given leaf
    /// If a root is provided, the proof is constructed from the specified root
    /// Otherwise, the proof is constructed from the current canonical tree
    pub fn inclusion_proof(
        &self,
        leaf: Hash,
        root: Option<&Root>,
    ) -> Result<Option<InclusionProof>, IdentityTreeError> {
        let leaf_idx = match self.leaves.get(&leaf) {
            Some(idx) => idx,
            None => return Ok(None),
        };

        if let Some(root) = root {
            if root.hash == self.tree.root() {
                let proof = self.tree.proof(*leaf_idx as usize);
                Ok(Some(InclusionProof::new(self.tree.root(), proof)))
            } else {
                let proof = self.construct_proof_from_root(*leaf_idx, root)?;
                Ok(Some(InclusionProof::new(root.hash, proof)))
            }
        } else {
            if *leaf_idx as usize > self.tree.num_leaves() {
                return Ok(None);
            }

            let proof = self.tree.proof(*leaf_idx as usize);
            Ok(Some(InclusionProof::new(self.tree.root(), proof)))
        }
    }

    /// Construct an inclusion proof for a given leaf at a specified root
    pub fn construct_proof_from_root(
        &self,
        leaf_idx: u32,
        root: &Root,
    ) -> Result<Proof, IdentityTreeError> {
        // Get the updates at the specified root
        let updates = self
            .tree_updates
            .get(root)
            .ok_or(IdentityTreeError::RootNotFound)?;

        // Convert the leaf index to a storage index for easier indexing
        let mut node_idx = leaf_to_storage_idx(leaf_idx, self.tree.depth());

        let mut proof: Vec<Branch<Hash>> = vec![];

        // Traverse the tree from the leaf to the root, constructing the proof along the way with precedence for the updated node values
        while node_idx > 0 {
            let sibling_idx = if node_idx % 2 == 0 {
                node_idx - 1
            } else {
                node_idx + 1
            };

            // Check if the sibling is in the updates, otherwise get the node from the tree
            let sibling = updates
                .get(&sibling_idx.into())
                .copied()
                .or_else(|| {
                    let (depth, offset) =
                        storage_idx_to_coords(sibling_idx as usize);
                    Some(self.tree.get_node(depth, offset))
                })
                .expect("Could not find node in tree");

            // Add the sibling to the proof and adjust the node index
            proof.push(if node_idx % 2 == 0 {
                Branch::Right(sibling)
            } else {
                Branch::Left(sibling)
            });

            node_idx = (node_idx - 1) / 2;
        }

        Ok(semaphore::merkle_tree::Proof(proof))
    }
}

/// Flattens leaf updates into a single vector of leaf indices and hashes with precedence given to the latest updates
pub fn flatten_leaf_updates(
    leaf_updates: BTreeMap<Root, LeafUpdates>,
) -> Vec<(LeafIndex, Hash)> {
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

    updates
}

pub enum LeafUpdates {
    Insert(Leaves),
    Delete(Leaves),
}

impl From<LeafUpdates> for Leaves {
    fn from(val: LeafUpdates) -> Self {
        match val {
            LeafUpdates::Insert(leaves) => leaves,
            LeafUpdates::Delete(leaves) => leaves,
        }
    }
}

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

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct Root {
    pub hash: Hash,
    //NOTE: note that this assumes that there is only one wallet that sequences transactions
    // we should update to a syncing mechanism that can account for multiple sequencers
    pub nonce: usize,
}

impl Ord for Root {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.nonce.cmp(&other.nonce)
    }
}

impl PartialOrd for Root {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InclusionProof {
    pub root: Field,
    pub proof: Proof,
}

impl InclusionProof {
    pub fn new(root: Field, proof: Proof) -> InclusionProof {
        Self { root, proof }
    }

    pub fn verify(&self, leaf: Field) -> bool {
        let mut hash = leaf;

        for branch in self.proof.0.iter() {
            match branch {
                Branch::Left(sibling) => {
                    hash = PoseidonHash::hash_node(&hash, sibling);
                }
                Branch::Right(sibling) => {
                    hash = PoseidonHash::hash_node(sibling, &hash);
                }
            }
        }

        hash == self.root
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use eyre::{eyre, ContextCompat};
    use rand::{Rng, SeedableRng};
    use semaphore::cascading_merkle_tree::CascadingMerkleTree;
    use semaphore::merkle_tree::Branch;
    use semaphore::poseidon_tree::PoseidonHash;

    use super::{leaf_to_storage_idx, IdentityTree, LeafUpdates, Root};
    use crate::tree::identity_tree::{
        storage_idx_to_coords, storage_to_leaf_idx,
    };
    use crate::tree::{Hash, LeafIndex};

    const TREE_DEPTH: usize = 2;
    const NUM_LEAVES: usize = 1 << TREE_DEPTH;

    fn infinite_leaves() -> impl Iterator<Item = Hash> {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(42);

        std::iter::from_fn(move || {
            let mut limbs: [u64; 4] = rng.gen();
            limbs[3] = 0; // nullify most significant limb to keep the values in the Field

            Some(Hash::from_limbs(limbs))
        })
    }

    fn generate_all_leaves() -> Vec<Hash> {
        infinite_leaves().take(NUM_LEAVES).collect()
    }

    #[test]
    fn test_ord_root() {
        let root_1 = Root {
            hash: Hash::from(1),
            nonce: 1,
        };

        let root_2 = Root {
            hash: Hash::from(2),
            nonce: 2,
        };

        let root_3 = Root {
            hash: Hash::from(3),
            nonce: 1,
        };

        assert!(root_1 < root_2);
        assert!(root_2 > root_3);
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
    fn test_insert() -> eyre::Result<()> {
        let mut identity_tree = IdentityTree::new(TREE_DEPTH);

        // Generate new leaves and insert into the tree
        let leaves = generate_all_leaves();
        for (idx, leaf) in leaves.iter().enumerate() {
            identity_tree
                .insert(idx as u32, *leaf)
                .expect("Could not insert leaf");
        }

        // Initialize an expected tree with the same leaves
        let expected_tree: CascadingMerkleTree<PoseidonHash> =
            CascadingMerkleTree::new_with_leaves(
                vec![],
                TREE_DEPTH,
                &Hash::ZERO,
                &leaves,
            );

        // Ensure the tree roots are equal
        assert_eq!(identity_tree.tree.root(), expected_tree.root());

        // Assert that each of the leaves are in the leaves hashmap
        for (leaf_idx, leaf) in leaves.iter().enumerate() {
            assert_eq!(
                identity_tree.leaves.get(leaf),
                Some(&(leaf_idx as u32))
            );
        }

        Ok(())
    }

    #[test]
    fn test_remove() -> eyre::Result<()> {
        let mut identity_tree = IdentityTree::new(TREE_DEPTH);

        // Generate new leaves and insert into the tree
        let leaves = generate_all_leaves();
        for (idx, leaf) in leaves.iter().enumerate() {
            identity_tree
                .insert(idx as u32, *leaf)
                .expect("Could not insert leaf");
        }

        // Remove each leaf from the tree
        for i in 0..1 << TREE_DEPTH {
            identity_tree.remove(i as usize);
        }

        // Initialize an expected tree with all leaves set to 0x00
        let expected_tree: CascadingMerkleTree<PoseidonHash> =
            CascadingMerkleTree::new_with_leaves(
                vec![],
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

        Ok(())
    }

    #[test]
    fn test_append_updates() -> eyre::Result<()> {
        let mut identity_tree = IdentityTree::new(TREE_DEPTH);

        // Generate the first half of the leaves and insert into the tree
        let leaves = generate_all_leaves();
        for (idx, leaf) in leaves[0..NUM_LEAVES / 2].iter().enumerate() {
            identity_tree.insert(idx as u32, *leaf)?;
        }

        let expected_root = identity_tree.tree.root();

        // Generate the updated tree with all of the leaves
        let updated_tree: CascadingMerkleTree<PoseidonHash> =
            CascadingMerkleTree::new_with_leaves(
                vec![],
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
            .map(|(idx, value)| ((idx as u32).into(), *value))
            .collect::<HashMap<LeafIndex, Hash>>();

        identity_tree
            .append_updates(new_root, LeafUpdates::Insert(leaf_updates));

        // Ensure that the root is correct
        assert_eq!(identity_tree.tree.root(), expected_root);
        assert_eq!(identity_tree.tree_updates.len(), 1);

        //TODO: assert expected updates

        Ok(())
    }

    #[test]
    fn test_apply_updates_to_root() -> eyre::Result<()> {
        let mut identity_tree = IdentityTree::new(TREE_DEPTH);

        // Generate the first half of the leaves and insert into the tree
        let leaves = generate_all_leaves();

        for (idx, leaf) in leaves[0..NUM_LEAVES / 2].iter().enumerate() {
            identity_tree.insert(idx as u32, *leaf)?;
        }

        // Generate the updated tree with all of the leaves
        let expected_tree: CascadingMerkleTree<PoseidonHash> =
            CascadingMerkleTree::new_with_leaves(
                vec![],
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
            .map(|(idx, value)| {
                (LeafIndex((NUM_LEAVES / 2 + idx) as u32), *value)
            })
            .collect::<HashMap<LeafIndex, Hash>>();

        identity_tree
            .append_updates(new_root, LeafUpdates::Insert(leaf_updates));

        // Apply updates to the tree
        identity_tree.apply_updates_to_root(&new_root);

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
    fn test_inclusion_proof() -> eyre::Result<()> {
        let mut identity_tree = IdentityTree::new(TREE_DEPTH);

        let leaves: Vec<_> = infinite_leaves().take(4).collect();

        println!("leaves: {:?}", leaves);

        // We insert only the first leaf
        identity_tree.insert(0, leaves[0])?;

        // Simulate and create updates
        let (root_012, updates) = {
            let mut tree = CascadingMerkleTree::<PoseidonHash>::new(
                vec![],
                TREE_DEPTH,
                &Hash::ZERO,
            );

            tree.push(leaves[0])?;
            tree.push(leaves[1])?;
            tree.push(leaves[2])?;

            let updates = LeafUpdates::Insert(
                vec![(1.into(), leaves[1]), (2.into(), leaves[2])]
                    .into_iter()
                    .collect::<HashMap<LeafIndex, Hash>>(),
            );

            let root = Root {
                hash: tree.root(),
                nonce: 1,
            };

            (root, updates)
        };

        identity_tree.append_updates(root_012, updates);

        // Simulate and create updates
        let (root_0123, updates) = {
            let mut tree = CascadingMerkleTree::<PoseidonHash>::new(
                vec![],
                TREE_DEPTH,
                &Hash::ZERO,
            );

            tree.push(leaves[0])?;
            tree.push(leaves[1])?;
            tree.push(leaves[2])?;
            tree.push(leaves[3])?;

            let updates = LeafUpdates::Insert(
                vec![(3.into(), leaves[3])]
                    .into_iter()
                    .collect::<HashMap<LeafIndex, Hash>>(),
            );

            let root = Root {
                hash: tree.root(),
                nonce: 2,
            };

            (root, updates)
        };

        identity_tree.append_updates(root_0123, updates);

        let proof = identity_tree
            .inclusion_proof(leaves[3], Some(&root_0123))?
            .context("Missing proof")?;

        assert_eq!(
            proof.proof.0[0],
            Branch::Right(leaves[2]),
            "The first sibling of leaf 3 must be leaf 2"
        );

        let proof = identity_tree
            .inclusion_proof(leaves[2], Some(&root_012))?
            .context("Missing proof")?;

        assert_eq!(
            proof.proof.0[0],
            Branch::Left(Hash::ZERO),
            "The first sibling of leaf 2 must be zero hash"
        );

        let proof = identity_tree.inclusion_proof(leaves[2], None)?;

        assert!(
            proof.is_none(),
            "The canonical tree does not contain this update yet"
        );

        Ok(())
    }

    #[test]
    fn test_construct_proof_from_root() {}

    #[test]
    fn test_mmap_cache() -> eyre::Result<()> {
        let path = PathBuf::from("tree_cache");

        let mut identity_tree =
            IdentityTree::new_with_cache(TREE_DEPTH, path.clone())?;

        let leaves = generate_all_leaves();

        for leaf in leaves.iter() {
            identity_tree.tree.push(*leaf)?;
        }

        let restored_tree = IdentityTree::new_with_cache(TREE_DEPTH, path)?;

        assert_eq!(identity_tree.tree.root(), restored_tree.tree.root());

        for leaf in leaves.iter() {
            let proof = restored_tree
                .inclusion_proof(*leaf, None)?
                .expect("Could not get proof");

            assert!(proof.verify(*leaf));
        }

        Ok(())
    }
}
