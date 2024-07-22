use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;
use std::time::Instant;

use rayon::iter::{Either, IntoParallelIterator, ParallelIterator};
use semaphore::cascading_merkle_tree::CascadingMerkleTree;
use semaphore::generic_storage::{GenericStorage, MmapVec};
use semaphore::merkle_tree::{Branch, Hasher};
use semaphore::poseidon_tree::{PoseidonHash, Proof};
use semaphore::Field;
use serde::{Deserialize, Serialize};

use super::error::IdentityTreeError;
use super::{Hash, LeafIndex, NodeIndex};

// Leaf index to hash, 0 indexed from the initial leaf
pub type Leaves = HashMap<LeafIndex, Hash>;
// Node index to hash, 0 indexed from the root
pub type StorageUpdates = HashMap<NodeIndex, Hash>;

pub struct IdentityTree<S> {
    /// Densely allocated (and in some cases cached on disk) merkle tree
    pub tree: CascadingMerkleTree<PoseidonHash, S>,

    /// Temporary storage of tree updates for a given root
    /// the updates are stored here until they are applied to all observed chains
    /// at which point they will be removed from this map and applied to the canonical tree
    pub tree_updates: Vec<(Hash, StorageUpdates)>,

    /// Mapping of leaf hash to leaf index
    pub leaves: HashMap<Hash, u32>,
}

impl IdentityTree<Vec<Hash>> {
    pub fn new(depth: usize) -> Self {
        let tree = CascadingMerkleTree::new(vec![], depth, &Hash::ZERO);

        Self {
            tree,
            tree_updates: Vec::new(),
            leaves: HashMap::new(),
        }
    }
}

impl IdentityTree<MmapVec<Hash>> {
    /// Returnes a new IdentityTree from a chached file.
    /// The underlying Merkle Tree is unverified
    pub fn new_with_cache_unchecked(
        depth: usize,
        file_path: &Path,
    ) -> Result<Self, IdentityTreeError> {
        let mmap_vec: MmapVec<Hash> =
            match unsafe { MmapVec::restore_from_path(file_path) } {
                Ok(mmap_vec) => mmap_vec,

                Err(e) => unsafe {
                    tracing::error!("Cache restore error: {:?}", e);
                    MmapVec::create_from_path(file_path)?
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

            let tree =
                match CascadingMerkleTree::<PoseidonHash, _>::restore_unchecked(
                    mmap_vec,
                    depth,
                    &Hash::ZERO,
                ) {
                    Ok(tree) => tree,
                    Err(e) => {
                        tracing::error!(
                        "Error to restoring tree from cache {e:?}, purging cache and creating new tree"
                    );

                        // Remove the existing cache and create a new cache file
                        fs::remove_file(file_path)?;
                        let mmap_vec =
                            unsafe { MmapVec::create_from_path(file_path)? };

                        CascadingMerkleTree::<PoseidonHash, _>::new(
                            mmap_vec,
                            depth,
                            &Hash::ZERO,
                        )
                    }
                };

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
            tree_updates: Vec::new(),
        })
    }
}

impl<S> IdentityTree<S>
where
    S: GenericStorage<Hash>,
{
    // Appends new leaf updates to the `leaves` hashmap and adds newly calculated storage nodes to `tree_updates`
    pub fn append_updates(
        &mut self,
        pre_root: Hash,
        post_root: Hash,
        leaf_updates: LeafUpdates,
    ) -> Result<(), IdentityTreeError> {
        let latest_root =
            if let Some((last_root, _updates)) = self.tree_updates.last() {
                // We already have pending updates
                *last_root
            } else {
                // New root to an empty tree or all chains are synced
                self.tree.root()
            };

        self.update_leaf_index_mapping(&leaf_updates);

        if pre_root != latest_root {
            // This can occur if the tree has been restored from cache, but we're replaying chain events
            tracing::warn!(
                ?latest_root,
                ?pre_root,
                ?post_root,
                "Attempted to insert root out of order"
            );
            return Ok(());
        }

        let updates = self.construct_storage_updates(leaf_updates, None)?;

        self.tree_updates.push((post_root, updates));

        Ok(())
    }

    fn update_leaf_index_mapping(&mut self, leaf_updates: &LeafUpdates) {
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
    ///
    /// # Arguments
    ///
    /// * `leaf_updates` - The new leaf values used to construct the storage updates.
    ///
    ///  * `root` - Optional root to construct updates from, otherwise the most recent update is used.
    ///
    /// # Returns
    ///
    /// `StorageUpdates` which is a hashmap of node indices to their updated values.
    ///
    /// # Errors
    ///
    /// Returns an error if the specified root is not found in tree updates.
    fn construct_storage_updates(
        &self,
        leaf_updates: LeafUpdates,
        root: Option<&Hash>,
    ) -> Result<StorageUpdates, IdentityTreeError> {
        // Get the previous update to flatten existing storage nodes into the newly updated nodes
        // If a specific root is specified, get the update at that root
        let mut updates = if let Some(root) = root {
            if let Some(update) = self
                .tree_updates
                .iter()
                .find(|(update_root, _updates)| update_root == root)
            {
                update.1.clone()
            } else {
                return Err(IdentityTreeError::RootNotFound);
            }
        } else {
            // Otherwise, get the most recent update
            if let Some(update) = self.tree_updates.iter().last() {
                update.1.clone()
            } else {
                HashMap::new()
            }
        };

        let mut node_queue = VecDeque::new();
        let mut nodes_updated = HashSet::new();

        // Convert leaf indices into storage indices and insert into updates
        let leaves: Leaves = leaf_updates.into();
        for (leaf_idx, hash) in leaves.into_iter() {
            let storage_idx = leaf_to_storage_idx(*leaf_idx, self.tree.depth());
            updates.insert(storage_idx.into(), hash);

            // Queue the parent index
            node_queue.push_front(parent_of(storage_idx));
        }

        // Reads a node from the list of updates or from the tree
        let read_node = |node_idx: u32, updates: &StorageUpdates| {
            if let Some(node) = updates.get(&node_idx.into()) {
                *node
            } else {
                let (depth, offset) = storage_idx_to_coords(node_idx as usize);
                self.tree.get_node(depth, offset)
            }
        };

        while let Some(node_idx) = node_queue.pop_back() {
            // Check if the node has already been updated
            // this can happen if e.g. we updated 2 leaves that share the same parent
            if nodes_updated.contains(&node_idx) {
                continue;
            } else {
                nodes_updated.insert(node_idx);
            }

            let (left_child_idx, right_child_idx) = children_of(node_idx);

            let left = read_node(left_child_idx, &updates);
            let right = read_node(right_child_idx, &updates);

            let hash = PoseidonHash::hash_node(&left, &right);

            updates.insert(node_idx.into(), hash);

            // Queue the parent index if not the root
            if node_idx == 0 {
                break;
            } else {
                node_queue.push_front(parent_of(node_idx));
            };
        }

        Ok(updates)
    }

    // Applies updates up to the specified root, inclusive
    pub fn apply_updates_to_root(&mut self, root: &Hash) {
        let idx_of_root = self
            .tree_updates
            .iter()
            .position(|(update_root, _update)| update_root == root)
            .expect("Tried applying updates to a non-existent root");

        // Drain the updates up to and including the root
        let drained = self.tree_updates.drain(..=idx_of_root);
        let (_root, update) = drained.last().unwrap();

        // Filter out updates that are not leaves
        let mut leaf_updates = update
            .into_iter()
            .filter_map(|(idx, value)| {
                let leaf_idx = node_to_leaf_idx(idx.0, self.tree.depth())?;

                Some((leaf_idx, value))
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

        // TODO: Implement bulk deletions
        for leaf_idx in deletions {
            self.tree.set_leaf(leaf_idx, Hash::ZERO);
        }
    }

    /// Construct an inclusion proof for a given leaf
    /// If a root is provided, the proof is constructed from the specified root
    /// Otherwise, the proof is constructed from the current canonical tree
    pub fn inclusion_proof(
        &self,
        leaf: Hash,
        root: Option<&Hash>,
    ) -> Result<Option<InclusionProof>, IdentityTreeError> {
        let leaf_idx = match self.leaves.get(&leaf) {
            Some(idx) => idx,
            None => return Err(IdentityTreeError::LeafNotFound),
        };

        match root {
            // TODO: This doesn't work for old roots which have been flattened into the cascading tree
            Some(root) if *root != self.tree.root() => {
                let proof = self.construct_proof_from_root(*leaf_idx, root)?;
                Ok(Some(InclusionProof::new(*root, proof)))
            }
            _ => {
                if *leaf_idx as usize >= self.tree.num_leaves() {
                    return Ok(None);
                }

                let proof = self.tree.proof(*leaf_idx as usize);
                Ok(Some(InclusionProof::new(self.tree.root(), proof)))
            }
        }
    }

    /// Construct an inclusion proof for a given leaf at a specified root
    /// using the pending updates
    pub fn construct_proof_from_root(
        &self,
        leaf_idx: u32,
        root: &Hash,
    ) -> Result<Proof, IdentityTreeError> {
        // Get the updates at the specified root
        let (_update_root, updates) = self
            .tree_updates
            .iter()
            .find(|(update_root, _update)| update_root == root)
            .ok_or(IdentityTreeError::RootNotFound)?;

        // Convert the leaf index to a storage index for easier indexing
        let mut node_idx = leaf_to_storage_idx(leaf_idx, self.tree.depth());

        let mut proof: Vec<Branch<Hash>> = vec![];

        // Traverse the tree from the leaf to the root, constructing the proof along the way with precedence for the updated node values
        while node_idx > 0 {
            let sibling_idx = sibling_of(node_idx);

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

            node_idx = parent_of(node_idx);
        }

        Ok(semaphore::merkle_tree::Proof(proof))
    }

    // Computes the updated root hash from a list of new leaves
    pub fn compute_root(
        &self,
        leaves: &[Hash],
        root: Option<&Hash>,
    ) -> Result<Hash, IdentityTreeError> {
        let next_leaf_index = self.tree.num_leaves();

        let leaf_updates = leaves
            .iter()
            .enumerate()
            .map(|(idx, value)| {
                (LeafIndex((next_leaf_index + idx) as u32), *value)
            })
            .collect::<HashMap<LeafIndex, Hash>>();

        let mut storage_updates = self.construct_storage_updates(
            LeafUpdates::Insert(leaf_updates),
            root,
        )?;

        let updated_root = storage_updates
            .remove(&NodeIndex(0))
            .ok_or(IdentityTreeError::RootNotFound)?;

        Ok(updated_root)
    }
}

#[derive(Debug, Clone)]
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

pub fn parent_of(node_idx: u32) -> u32 {
    (node_idx - 1) / 2
}

pub fn sibling_of(node_idx: u32) -> u32 {
    if node_idx % 2 == 0 {
        node_idx - 1
    } else {
        node_idx + 1
    }
}

pub fn children_of(node_idx: u32) -> (u32, u32) {
    (node_idx * 2 + 1, node_idx * 2 + 2)
}

pub fn node_to_leaf_idx(node_idx: u32, tree_depth: usize) -> Option<u32> {
    let leaf_0 = (1 << tree_depth) - 1;

    if node_idx < leaf_0 {
        None
    } else {
        Some(node_idx - leaf_0)
    }
}

pub fn storage_idx_to_coords(index: usize) -> (usize, usize) {
    let depth = (index + 1).ilog2();
    let offset = index - (2usize.pow(depth) - 1);
    (depth as usize, offset)
}

#[derive(Debug, Serialize, Deserialize)]
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

    use eyre::{eyre, ContextCompat};
    use rand::{Rng, SeedableRng};
    use semaphore::cascading_merkle_tree::CascadingMerkleTree;
    use semaphore::generic_storage::{GenericStorage, MmapVec};
    use semaphore::merkle_tree::Branch;
    use semaphore::poseidon_tree::PoseidonHash;
    use tempfile::NamedTempFile;

    use super::*;
    use crate::tree::error::IdentityTreeError;
    use crate::tree::identity_tree::{
        node_to_leaf_idx, storage_idx_to_coords, Leaves,
    };
    use crate::tree::{Hash, LeafIndex};

    const TREE_DEPTH: usize = 2;
    const NUM_LEAVES: usize = 1 << TREE_DEPTH;

    // Extra methods for testing
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
    }

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
            let leaf_idx = node_to_leaf_idx(storage_idx, TREE_DEPTH).unwrap();
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
        tracing_subscriber::fmt::init();

        let mut identity_tree = IdentityTree::new(TREE_DEPTH);
        let mut tree: CascadingMerkleTree<PoseidonHash> =
            CascadingMerkleTree::new(vec![], TREE_DEPTH, &Hash::ZERO);

        // Generate the first half of the leaves and insert into the tree
        let leaves = generate_all_leaves();
        let first_half = leaves[0..NUM_LEAVES / 2].to_vec();
        let second_half = leaves[NUM_LEAVES / 2..].to_vec();

        for (idx, leaf) in first_half.iter().enumerate() {
            identity_tree.insert(idx as u32, *leaf)?;
        }

        tree.extend_from_slice(&first_half);
        let pre_root = tree.root();

        tree.extend_from_slice(&second_half);
        let post_root = tree.root();

        // Collect the second half of the leaves
        let offset = NUM_LEAVES / 2;
        let leaf_updates = second_half
            .iter()
            .enumerate()
            .map(|(idx, value)| (((idx + offset) as u32).into(), *value))
            .collect::<HashMap<LeafIndex, Hash>>();

        // Cache the expected root as the tree root should not change from the appended updates
        let expected_root = identity_tree.tree.root();

        identity_tree.append_updates(
            pre_root,
            post_root,
            LeafUpdates::Insert(leaf_updates),
        )?;

        // Ensure that the root is correct and the updates are stored
        assert_eq!(identity_tree.tree.root(), expected_root);
        assert_eq!(identity_tree.tree_updates.len(), 1);

        //TODO: assert expected updates

        identity_tree.apply_updates_to_root(&post_root);

        assert_eq!(identity_tree.tree.root(), tree.root());
        assert_eq!(identity_tree.tree_updates.len(), 0);

        Ok(())
    }

    #[test]
    fn test_apply_updates_to_root() -> eyre::Result<()> {
        let mut identity_tree = IdentityTree::new(TREE_DEPTH);
        let mut tree: CascadingMerkleTree<PoseidonHash> =
            CascadingMerkleTree::new(vec![], TREE_DEPTH, &Hash::ZERO);

        // Generate the first half of the leaves and insert into the tree
        let leaves = generate_all_leaves();
        let first_half = leaves[0..NUM_LEAVES / 2].to_vec();
        let second_half = leaves[NUM_LEAVES / 2..].to_vec();

        for (idx, leaf) in first_half.iter().enumerate() {
            identity_tree.insert(idx as u32, *leaf)?;
            assert_eq!(tree.num_leaves(), idx);
            tree.push(*leaf)?;
        }

        // Generate the updated tree with all of the leaves
        let pre_root = tree.root();
        tree.extend_from_slice(&second_half);
        let post_root = tree.root();

        // Collect the second half of the leaves
        let leaf_updates = second_half
            .iter()
            .enumerate()
            .map(|(idx, value)| {
                (LeafIndex((NUM_LEAVES / 2 + idx) as u32), *value)
            })
            .collect::<HashMap<LeafIndex, Hash>>();

        identity_tree.append_updates(
            pre_root,
            post_root,
            LeafUpdates::Insert(leaf_updates),
        )?;

        // Apply updates to the tree
        identity_tree.apply_updates_to_root(&post_root);

        assert_eq!(identity_tree.tree.root(), post_root);
        assert_eq!(identity_tree.tree_updates.len(), 0);

        for (leaf_idx, leaf) in leaves.iter().enumerate() {
            let proof = identity_tree
                .inclusion_proof(*leaf, None)?
                .ok_or(eyre!("Proof not found"))?;

            assert_eq!(proof.root, post_root);
            assert_eq!(proof.proof, tree.proof(leaf_idx));
        }

        Ok(())
    }

    #[test]
    fn test_compute_root() -> eyre::Result<()> {
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

        // Collect the second half of the leaves
        let leaf_updates = leaves[(NUM_LEAVES / 2)..].to_vec();

        let updated_root = identity_tree.compute_root(&leaf_updates, None)?;
        let expected_root = expected_tree.root();

        assert_eq!(updated_root, expected_root);

        Ok(())
    }

    #[test]
    fn test_inclusion_proof() -> eyre::Result<()> {
        let mut identity_tree = IdentityTree::new(TREE_DEPTH);

        let leaves: Vec<_> = infinite_leaves().take(4).collect();

        println!("leaves: {:?}", leaves);

        // We insert only the first leaf
        identity_tree.insert(0, leaves[0])?;

        let initial_root = {
            let mut tree = CascadingMerkleTree::<PoseidonHash>::new(
                vec![],
                TREE_DEPTH,
                &Hash::ZERO,
            );

            tree.push(leaves[0])?;
            tree.root()
        };

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

            (tree.root(), updates)
        };

        identity_tree.append_updates(initial_root, root_012, updates)?;

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

            (tree.root(), updates)
        };

        identity_tree.append_updates(root_012, root_0123, updates)?;

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
    fn test_mmap_cache() -> eyre::Result<()> {
        let temp_file = NamedTempFile::new()?;
        let path = temp_file.path().to_path_buf();

        let mut identity_tree =
            IdentityTree::new_with_cache_unchecked(TREE_DEPTH, &path)?;

        let leaves = generate_all_leaves();

        for leaf in leaves.iter() {
            identity_tree.tree.push(*leaf)?;
        }

        let restored_tree =
            IdentityTree::new_with_cache_unchecked(TREE_DEPTH, &path)?;

        assert_eq!(identity_tree.tree.root(), restored_tree.tree.root());

        for leaf in leaves.iter() {
            let proof = restored_tree
                .inclusion_proof(*leaf, None)?
                .expect("Could not get proof");

            assert!(proof.verify(*leaf));
        }

        Ok(())
    }

    #[test]
    fn test_auto_purge_cache() -> eyre::Result<()> {
        let temp_file = NamedTempFile::new()?;
        let path = temp_file.path().to_path_buf();

        let mut identity_tree =
            IdentityTree::new_with_cache_unchecked(TREE_DEPTH, &path).unwrap();

        let leaves = generate_all_leaves();

        for leaf in leaves.iter() {
            identity_tree.tree.push(*leaf).unwrap();
        }

        let mut cache: MmapVec<ruint::Uint<256, 4>> =
            unsafe { MmapVec::<Hash>::restore_from_path(&path)? };
        cache[0] = Hash::ZERO;

        let restored_tree =
            IdentityTree::new_with_cache_unchecked(TREE_DEPTH, &path).unwrap();

        assert!(restored_tree.tree.num_leaves() == 0);
        assert!(restored_tree.leaves.is_empty());

        Ok(())
    }

    const IDENTITIES: &[&str] = &[
        "359857356768713916212181562811263788201694105810826576342",
        "1027206428976790202121229502334373676838089253551281397295",
        "1374063583558983561838411199440084262124275451366195139528",
        "1822894452105942680860750388184642879081708532880279519285",
        "4716256158125244291137304903311790851205755293889708452484",
        "712310186779827345504969336483111504852127702684360705595",
        "6059749032223047454033829313859035305255764511461518936970",
        "730267538988922751416402391051692469360157292034850742548",
        "1196157778301940548321891902831920546841562583366274094669",
        "4956430068512231635297449050964747166516341770556951577218",
        "3818108848211057969385067925724169496120671256306468292989",
        "2184996638399825730812926259950123318452695043688971474871",
        "562094918943778622630543506851427604948509114303005359021",
        "2287519341125439847274003186294896446042438015917837582391",
        "3788301047562836099306943757120647382287165509127985995452",
        "2564239418650970177006188158720735977714166947086419030212",
        "1745692727535931258807640745912818437219408912030724773630",
        "2730035476828727800689861444027144166025489634360946239110",
        "1873292368606244222435270309185073182895584530786042462650",
        "4494079969320276838476879810051719428137659377147727409493",
        "5989532692880004969905508864094913291859218914531210732370",
        "2836675449268739024150796609835893922580567005926109979017",
        "1433660760435008873165383860601517216864807504900469311362",
        "3979528979846679829031459873835630164275750310986036124923",
        "4239067348830790401288670353475403389710734961359207010408",
        "2987801229816360997084929796571257734673672869789366138483",
        "6047669321778550380773220606019477806406349629933756356655",
        "2113051538857651600548687597152152810985706837944964877143",
        "2228061846149934360999687256068288506115620129827501209039",
        "2018330052545307676968194438064003649605303748854995815065",
        "6083137810052925086634048938068100420198290087505036317721",
        "2813512827565882206603806413236013153978071580046213364494",
        "3578111974976889599128487097078067678183440384122142033935",
        "3717983358980935069400225511438250547440648870535895291307",
        "1757503210549780408147011479537977899926069200701896615399",
        "1771451997302226686810359543616219652777442900240174733180",
        "84292351971163804239217471965389056167502298165896232289",
        "384326640570110354196546927495110889051836542804455133673",
        "3505550865080162266630776315886607509102054639980920028688",
        "1594296511663968386964421427846985273361822437553169715217",
        "2997064958353445881098441987021270163619907163836462183703",
        "2279952261128128018966749664957209870645262335915608925657",
        "1462940665889789844787587628810181095635668771426589500991",
        "2455135061387561306717238493452647686694303207884260535655",
        "5309250970617697594826516329456798925254476410759290500406",
        "4840200299098405668103270276167746821901477650753806167803",
        "2320462812519129945460710656666088203306488881079460982937",
        "1354643610534533830388092367971323443620170790757051292842",
        "5175676000236795479168354388835882305794576453193347173363",
        "5710736822522996239249650393464995197968031843122040443614",
        "3517575805584058646831625834951308852777213588554367038634",
        "1233248350797200740283656177158244491811713107097129745206",
        "3484942913276376208825515639981184590488399275532042231050",
        "2276749923033735741852881724911879740323290632618047944008",
        "903914524937942883713650012383204668194605266365403596147",
        "5725437973160547189929078787873162037376052785729415758950",
        "3785157364938787171619350126889100006839753496973185444563",
        "4713733047626101620166120070452044765427919206101639417018",
        "214503571798360936431589358503660567998650085277569073496",
        "4063286904712909723187503368932335338284532583671850979562",
        "6126737123941544914211094457521178319164593202840177132781",
        "2411435831506486500979496457564850213066231929064878232171",
        "5340552680935068277575624371867557161738122042809608414725",
        "1771376165139264624465066779160979815970310914436064315968",
        "1102328356080959084278443296678076249099181532181453273676",
        "1316420910178140124443929517191395371822196051013063425542",
        "2784996543348323107345637123307051268955127442168701413774",
        "5432307338511904452690272373447262559937842451360359045728",
        "1839406644843914349311486587261941989622113945538389897772",
        "2635386622365584451027883307182335898737005133959058312561",
        "3307150281799188732525246160495760717032180964345130225512",
        "5194802593591700214231881753008935585414422036701413165391",
        "1721309905656314434721899509395958585862119996017276407586",
        "1398002149732945164533301556060718558656121406781264580979",
        "4576775462193107675693982192082875659516921923384821196696",
        "823176145648554650719381225094795328341292373215758023903",
        "2229737105119567206309894224053639320911677330439855039740",
        "2657381422287369223160759070427261284381288004078559902026",
        "1206744558843415082552823168795404881164974518316457982564",
        "2732437423056191716754103329620141455597862754347388995898",
        "4501101233483861812479281771026884471761833554137885526862",
        "257268310567171324860161562511876169930529451829797057386",
        "3009648068039762242220609826097325694300233664566246192770",
        "3342928520847291637748714358449344602288999310067620173943",
        "1939596089776191909369756565850404889396364328305468926635",
        "708964898738978972713514184817689077859674445024334386460",
        "5545418497481296271626131986802897365822038059253161517998",
        "370504811220506522579027451649306940315995909373110361580",
        "2387519777512582630597008111444649555630389410964973012067",
        "6034734810739394177555274573274217454378685308453090779892",
        "1393124284270924052137449196363556568842003546586996600768",
        "2238744494535227364420509320000869452449495952627081484456",
        "1545499141950055903428444096691898103389986942028886952310",
        "5549715436147084184890571475728666807834588469509183075845",
        "309634882633531864015969707324386598836026960874382275899",
        "1963454278677531910350176475978301004671196341434676838215",
        "584199869927591193691611430187675711186816571589282245722",
        "2975014084888859404059439081797373723787094871614546911289",
        "3107434650604113175098445974524339965947370721768838704371",
        "5742158346108896964995771508241591291171942233530200105881",
        "3103184557588598110621309737016127411403623632292605900209",
        "3415725858512732684415038641578538809981125131162330715932",
        "745019059764496283289214461016887415404806725255761818426",
        "1006559568437550541771739791523231255089329004045501950994",
        "4801069001874008437621454093016498421072131097242320271913",
        "5668271549286602656863954697262454665935712459991168711631",
        "1825145001345059723162820956186159102383986121138023853035",
        "4666830577695949012366883243822682072218173114267507678916",
        "3704686002755402740697095183700863973258025014141265557248",
        "5186188780686195288921460804006198137449499290342585633677",
        "1336927971044727864706589880968532374946167332897755236489",
        "1627022324851456007934884834032032781682680931355797599998",
        "2184450396384108762111149134144499690949166161267960374199",
        "652171738174445793610892192103145009505387813769629370094",
        "2392100085214900242480523085413134811227740598812122606769",
        "5439021745178643527326930729157430052398112996550174752983",
        "5837936193303738379298916835875534189891578662534477931617",
        "4979007663745640932577188762111214378244925129758727072473",
        "2044517043798292834763487715443398633410257149136733260026",
        "5731644866639418289547189428822634302712618771120872614516",
        "3473054017063813508728642494662973595534931327651416324065",
        "4907619223514793453121493028269096431811546088474359691430",
        "2991831559645769449704045898661774266552648233086479463376",
        "4945182354998003008076069908296848886215775788869391592927",
        "4535110905116175794713797526835535793910325550504185463260",
        "1628212580674187673287838717477389742417778111205145202022",
        "5309665010596186833630694361216800301932029122886812620057",
        "4284975398947085282423957533968794912696144182482315419210",
        "1478667449773573850888849219203471812781568493600285332910",
        "5808190518030056917700915449128276058727068972165497575712",
        "3634265953163040225501593433982055205426233468068952458148",
        "5322047325435599306238183597298173833312224666707823591789",
        "3008842459200925553006560165842425171006493450310571712304",
        "2576595547484980926896784981263816035681720296823878710349",
        "3412019386412655729925793467055458929470500673060053903125",
        "852370765799908319737001029745601717102313401892448247565",
        "6140542267936171506814462455829427354806320092880889616018",
        "2668716553984158365234079379700515888048195986706256903922",
        "228605987316159810505928955117436023690904800958182736978",
        "6125353094162737411205522899059994916362420896458490044911",
        "4692075463054407443375238727488850500624997895390932119046",
        "2334655754799628500845813485383144637322947614570728658713",
        "2982192860904175702709827253346350713622631620650987005079",
        "183263179803825713359374565702704063137711852780561264740",
        "1890511266906642073329576340303876690421432489572140725981",
        "538095941727829466402744283527540113378604961057534390711",
        "2217795293725673408764037274537191958595468480553666997517",
        "2574729613379143900672898440756574660369220524185902181930",
        "4773558654727507475964164908689527022211078930940532601321",
        "2019177526182050976522055328727445547888296992765634848512",
        "5551430495827425768013776517685383814903148696996192366268",
        "603993823021137073625065105490017449370461994997182936901",
        "3410701984208774078241137631989268684368664296480666788267",
        "424961560933127440688891766280256394003749738000383068331",
        "4844478869815478625805519011039826911349547254516591946972",
        "4477983685347164946546581735479685069413250928368890613792",
        "720173891963032988948751703666414557514338535213315309752",
        "1905283397757806417623838134999980905807035024032971107373",
        "2129590950973913876428733537850960998986828023124906139357",
        "1208055623119279784783974708934049307018892566921322845328",
        "4288963892995365733013846435641977788387626959915876809746",
        "4755218121347483418942385867782311918670481791449197454421",
        "1824120188526276661215134242095942828228349978166906213305",
        "950405596707016718909925296992860223323585159944804475173",
        "3085523670139421860585070864156345042480545347235734056008",
        "1263805513521893187757304578006977780453032494231124700751",
        "1300293500379712863455805532984378260854076721372331655575",
        "4364460318497530156891609464016086897773306076966478538147",
        "5848380107388483320465446791851648677426942130343138835394",
        "5772162671664441194527652658102258041155976988904777471955",
        "341914865304176877834821916030657742228865254316996867677",
        "188656022235419435126681916919584372402527185994157821830",
        "4028817605842698374403705821239372774648797563946308498708",
        "6180620330835145399783530767882551496126427708824440754236",
        "3076718117450131365423090922215758370515343959285362158549",
        "1131147807424769243813822471091477758725484586170201041176",
        "1872425011620095827202752554711664625119086864161943553741",
        "4018163653541589060406631345681174036469770584002459429332",
        "596237250564134936614152593890062136929210129772685450697",
        "5221337829926663947884088665183743429543304341554992628000",
        "4829648751918577099654563872244719228827101275705890232973",
        "5324923124310948279364632437866943470040359529123283678722",
        "3864854664368425345324141498527269589221721649703972382434",
        "2529014443524164274146368917486913744032185664051994490406",
        "4502450813965095227907030338472916837373249349238110935303",
        "3717479844577206034739796085432732606724040384448844151716",
        "5619686020691274864332009044940900197327171313642904195173",
        "2552918973480868843756251398747093594711473985043186579647",
        "2120217263098662091671292712489175657671943784488909160942",
        "1968276460413320484976800491511773616642960473260793063060",
        "1516782786864680720245601022891565590567517615613647681551",
        "4710551184335159781973057924586376631926501842282879918972",
        "4095248204218497668471371810761449931060282881810717123043",
        "857544005866333754399162733795196257411520602009278419639",
        "351132589866143765766384374849260030942044289480148045350",
        "3787931937260204365315004035932887064170734407478266611609",
        "2904996490972096536356103622328562222746133802101369678617",
        "4043540013077920634589871361640147761063807745323370159940",
        "2329357659994652278846708733149193582831852459155639500093",
        "1393885935399840571714631578204137607771499072718331687350",
        "2195288607650456692471649832266744625412490395051688511252",
        "3354310107859309702887693137336052604239572152552653544807",
        "4041472803942390360735920819046902761387212768183260369632",
        "4207965250494018090070889552630154454534913206301059397463",
        "4883340658801564342487577554595053339222370704013313048578",
        "2082723901267610964565891339167628121240412118673711538758",
        "3162018784648077410894689342189615458606488064282599274217",
        "168254231586890903586670815045055155900317541034886600022",
        "1114197532323282079982552636493511549848668915132104982218",
        "2735826473370617840840592894244328535535455540809570519640",
        "1968563887649178263408565642223980874876366311537967772626",
        "3259019994483735187099609386968946497452495221836994915179",
        "5892434909674434912703350808645371497600691284621193962187",
        "4422691622295786078749474809050120351008426415013168255621",
        "3747783229390907445150448175147333845325626451647214486727",
        "4885913943453902641342086806773238134381223015293728109465",
        "3584017090915031412029515590630803076988969699235677907317",
        "3013114878189046952699837717955990777039914009085807484373",
        "2264989499096424281839007291000418217891462787073258593850",
        "1941934737096863271002804578524897321382418474848356460610",
        "3611029396290359543162727791774396951684064344280809343732",
        "612737588354278791986670482601921056083024497950530596068",
        "2039057832090035191702110931326030656070184229338298975795",
        "4737693084401634951098734616953107376280974146783916228401",
        "510968367936262105235627944473783594668754304266285984099",
        "2556172632136155793084034527762885826292789227055380675212",
        "5096172940369694061110379665100008181925428675998784480485",
        "3871490285366947433970635766473402574478378604770384783991",
        "3713559222782603672251477691794784568632790992139622196422",
        "4174829947441819570451513306929886022718315740922879330209",
        "813253474858710415941353579620091096114155061279478644228",
        "3266592131979906386752390717375264585785362091380330079241",
        "1500698891542878287964618504987151131464159261980712376081",
        "3127981321175223203756419598589878323663145369350070321297",
        "1020334793794203252277920103751801709332291624962682614938",
        "778382333632611417895127093639521467105383178715753322344",
        "3394321512658703525448866523622470299267488337070322777885",
        "5928548989361132225019800076322411806826245734072929216806",
        "3377109985552364617125667651540020699265292634624540057961",
        "720176750106897921492356683704961354509095449865839431162",
        "2303389830289525096129321854836104405216448696087404243073",
        "402153010499809431872246887822448491616519850799561070696",
        "2577585320775277043754968040323508432595701249622078571594",
        "4464754431263391332145938134689538241808862871727522442596",
        "3333260410766078891772529536039089307087122620801602811936",
        "1475221997596549547593858369813970233712361359152044438612",
        "1265553617380172222757828147109790187256656598156676606200",
        "3790018608016560424033861895548583325387625961542000382625",
        "5647347755439949389132176149290141257207038154567838992115",
        "1785340302156934557835428185207979980077417474544087238046",
        "233733913090882705005969902812876366763824704817262792037",
        "4938529891442542526541822836284433119772152810484061590144",
        "2547613963505945981843836197752483867139920946920906683194",
        "1623667123942538660519316196436432327865859666066144036131",
        "4614338406935167340369686928369859246470416182783661324846",
        "2674484653233276748831744918517928999651467376490783739052",
        "2426992565444007516955236563768324387315304098641740599554",
        "3778299409361565655486438078662932135845527883885336949789",
        "3019520547089998071138146156540084624504884820556786020975",
        "4354679784631705690748226316490380878033660194537500478862",
        "5040316778219385466648059784797465196637214279916507458282",
        "1707544487865627951747325870788997148537263466160185875158",
        "783990298548544786178798589329670788364860544587575455046",
        "4622327238085713065259204355157840619101707526519969256683",
        "2235783737915810140148049257823714345789757149046921315310",
        "642318625819389839594007877905684077021835297283595754409",
        "6212720680552762358025232767628990234038344254411513295275",
        "3747017405786115259193239909940403013988608163284938848749",
        "2296668519990248249670731940253383285216016516989670891277",
        "1852508991927650566452173195941131588739134019268742240119",
        "2646985468396391009806240534231160644568660130812352761417",
        "4866218334333799432093646309793848164288628549206196044295",
        "4427144937358863313359838256895701948639758103981171693112",
        "5454386591049011303416499822648479962602501375142782868052",
        "3879594571148467145727329181451759144708179934401732970410",
        "1021465970145922489451871123784755798067867318120135361918",
        "98465311719054934773458416236542622885008685336758734369",
        "4181062513970947181215301822850615595855215565167553800652",
        "2056442040610261760703574217264299705543609465083715504500",
        "3702268133814904855655363644008921828587754182549341387709",
        "2565322052548121485385554631133885134970140191193501653771",
        "1253384495178939486305074109644466177660612019961560793907",
        "940115646844686831881746677957860557667218059837036096466",
        "4580131851266142319572689539124933749918085169072273116461",
        "904318822164321858979649597173697620032801751315250489082",
        "411102968345491327449265367009345871611470667164004956681",
        "5070331607777024793925741811809837709621632535784393286518",
        "2828749370955665334124376283101298254349123430624554771334",
        "2762927342719541310458902788587240249318315634962003525264",
        "1675166434333383209917304881091480688093372932580208437041",
        "4603575760427777312578024187861691487878520694748972475288",
        "1002610695730860774039540321148186174804587461488810000526",
        "5140410682315116119656949242006552065850090906575639203907",
        "4992382342957341435244626969158566720684493976490411772447",
        "1329229230252035151004910295303399021976380676272322856715",
        "6003067373325022782981371269263515839851355120367677087128",
        "2897343071801113504240334615992607800054287344881833721139",
        "1775512802539788724347747042158116459712933585396355132585",
        "1295102045495928064303303903899572075609417216748350507838",
        "3249300334602424378910584752142610618643662336127049324784",
        "5979294915741841177518336096764474916995128435446424755344",
        "838697867863032599517430593431727950271833598685048238690",
        "878897281727442929166612922104422046026882285071112027389",
        "5635999485007183347177373366930356408296255600645612438608",
        "5677842363608334680607592460542827134432605659908518888397",
        "3681078401021484325264701286580543849589782069080678088271",
        "4155194750185176840551668579864079279822664745262734721563",
        "4648465397990626889949692506957363909333655689935279636459",
        "1800531041339321372581271933136570084266784910576914897938",
        "438310906994709748818732788165519970450219718414153976886",
        "3942560673889202984716005224063651570898904210172747151336",
        "4292420682350913922098787502100876624994131029052886134025",
        "604011427861790632998672547822189613517861437628458069995",
        "402947181064674456641438718629500252501241702247659616916",
        "3080713500016944937966455837371412772169687302479865655639",
        "204169734639276721165065568482422867317838023532067713357",
        "5317242325084285904215830559489005773214302252011453622785",
        "3991204552624626670443700428905804079812521287451913879815",
        "5355597437301936961171960679432924414002967182607430509885",
        "762214884454496302206607832830509986224674021313366892647",
        "2656693378318517945145850469620119598474799013182259490738",
        "184265166636733660259052997454350317586065844621788621207",
        "309821812671929837977692496296344414256293420898714392975",
        "610376890456351668935566604542190055617016657436272547981",
        "2898425455902961527715418633884653058324461081003371370585",
        "2351556285812145558805198190442988806854580538853834325302",
        "5393874993910426837461850729345486869998755989182011219090",
        "756762234301960313847430333728852611958056759271069889827",
        "4708028672239901676850472563125602111296011068021530110475",
        "380847056483877859518665097740244515811389895547696517448",
        "2824103900296912916474253210867726965773287603815582058985",
        "3942910202643439135402586201776800869228653549310823048378",
        "126982740907384914105222615276241326955141056325174868226",
        "447712696079921125783414030695306946438780889172199065041",
        "152049551291940332483103665994153514180151984216138685090",
        "1517144581135759282483516242621287072189515866206008349229",
        "3307539762956941961859371717984650168911936447131653789440",
        "1685348783447011242031838940787000164396242848181099768960",
        "4615397187877976361740279770838777014223603749717892620612",
        "2314986318901420341505562936746189728026676090220687537574",
        "2719570265164435245404567928512432031829341446188698651264",
        "845968091401822731487157747100368766813928950751838070016",
        "4560306132244933590646979325756361282908331374176239055979",
        "3472673230249627103244844581264218414913129614379151876719",
        "5967279600391159777465356398055275077430772676044283597518",
        "5460767466961170635623213281486308426583556486769783442872",
        "1118867307891220781785316266814693936968260914588136367994",
        "661419893708363567487669813088878831556635710294828193806",
        "5586289306935858614803631933599471554823695232399226000980",
        "1330684383020451991528262134959257465608958008206982362455",
        "3698656282897635598999830248260265289649878168304103090055",
        "324652159824201454342935288837430648824406193447525495361",
        "397739824580016512647185518662221984629498863751782075015",
        "3656455902079600245725044082486840415566332169372311147324",
        "4010341849989185068700630727489187699988799833739398993708",
        "446265897543183667008144512461518604733788289323974018559",
        "1733377119579271949067384750363674837330739157702765455088",
        "1957777712361843714263445490174738494682941718293340934628",
        "3588168019950423629928721060295510894703636795273098849015",
        "1676593940428787133592049119029375412656927740649141011340",
        "362918521256872238903350605373617681726269516159264097950",
        "6241839670134181848837672908631067933130587946473174509654",
        "278781274585963592015067789153654790360649517549342553507",
        "700360687340868954274597253380873171677184972442444432030",
        "4607361001293359019330891864802959345637985361073956041142",
        "2014562319751135548534138357045247530800447347878981377328",
        "4284531940074381216269551550065159429521431511049184428703",
        "949714733465010124815566259755031264075285726934968393155",
        "5852382199248260391372749483056466273716935201795149294734",
        "3049211709757740070662580209062578277173741540974858773164",
        "3684048848624514528285926144836618783647659138115071229452",
        "6190119455040466925225529623770007976601651339338711066988",
        "73469032811483968279991934697213945008246580928966273073",
        "4502304500434797826768303126565814788950710518464182342784",
        "5955203210446744459669785060031948904282033216331757877848",
        "4661565402401843364275198850237039638505902145644072770608",
        "5421076576427291270704280988898667796907245873310907185863",
        "6034132091371926251186437305351610551043244795064939793542",
        "3870326841819184222290016777921783023848456551930022202150",
        "4391047261534391318289192592572845032506435272813533769640",
        "6058843706469094798371721472678837157480645435003699895898",
        "6055419935680233221495118218969560132667352881012216976758",
        "4481992047474989750227049263774234772096050122919133401700",
        "4294549666531945673995977336004818657985998536782611191063",
        "2051100727201882860086398766941389218708990116853221664424",
        "519505043532395739297145735237579852185804618360940322937",
        "3376989923127162308104944944184527120187211899264795061170",
        "2942719407674944298952704366320816091840556248700173605311",
        "3877000678136072131577283932518596339601077603761719565860",
        "2901561257694989795297903830569099453385317814389216493184",
    ];

    #[test]
    fn crazy_test() -> eyre::Result<()> {
        let identities: Vec<Hash> =
            IDENTITIES.iter().map(|x| x.parse().unwrap()).collect();

        println!("identities[0] = {:?}", identities[0]);

        let mut ref_tree: CascadingMerkleTree<PoseidonHash, Vec<_>> =
            CascadingMerkleTree::new(vec![], 30, &Hash::ZERO);

        let mut identity_tree = IdentityTree::new(30);

        // let initial_root = tree.root();

        for batch in identities.chunks(10) {
            let start_index = ref_tree.num_leaves();
            let pre_root = ref_tree.root();
            ref_tree.extend_from_slice(batch);
            let post_root = ref_tree.root();

            let leaf_updates: Leaves = batch
                .iter()
                .enumerate()
                .map(|(idx, leaf)| {
                    (LeafIndex((idx + start_index) as u32), leaf.clone())
                })
                .collect();

            let leaf_updates = LeafUpdates::Insert(leaf_updates);

            identity_tree.append_updates(pre_root, post_root, leaf_updates)?;
        }

        // let last_root

        // tree.extend_from_slice(&identities);

        // let root = tree.root();

        let ref_tree_root = ref_tree.root();
        identity_tree.apply_updates_to_root(&ref_tree_root);

        let id_tree_root = identity_tree.tree.root();

        println!("ref_tree_root = {:?}", ref_tree_root);
        println!("id_tree_root = {:?}", id_tree_root);

        panic!();
        Ok(())
    }
}
