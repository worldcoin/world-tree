use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
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
    pub fn new_with_cache(
        depth: usize,
        file_path: PathBuf,
    ) -> Result<Self, IdentityTreeError> {
        let mmap_vec: MmapVec<Hash> =
            match unsafe { MmapVec::restore(&file_path) } {
                Ok(mmap_vec) => mmap_vec,

                Err(_e) => unsafe {
                    tracing::info!("Cache not found, creating new cache file");
                    MmapVec::open_create(&file_path)?
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

            let tree = match CascadingMerkleTree::<PoseidonHash, _>::restore(
                mmap_vec,
                depth,
                &Hash::ZERO,
            ) {
                Ok(tree) => tree,
                Err(_) => {
                    tracing::error!(
                        "Failed to restore tree from cache, purging cache and creating new tree"
                    );

                    // Remove the existing cache and create a new cache file
                    fs::remove_file(&file_path)?;
                    let mmap_vec = unsafe { MmapVec::open_create(file_path)? };

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

    /// Extends the tree with new leaves and updates the leaves hashmap
    pub fn extend_from_slice(&mut self, leaves: &[(u32, Hash)]) {
        // Update the leaves hashmap and collect the new leaf values
        let leaves = leaves
            .iter()
            .map(|(idx, hash)| {
                self.leaves.insert(*hash, *idx);
                *hash
            })
            .collect::<Vec<_>>();

        // Insert the new leaves into the tree
        self.tree.extend_from_slice(&leaves);
    }

    /// Removes a leaf from the tree and updates the leaves hashmap
    pub fn remove(&mut self, index: usize) {
        let leaf = self.tree.get_leaf(index);
        self.leaves.remove(&leaf);
        self.tree.set_leaf(index, Hash::ZERO);
    }

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

        self.update_leaf_index_mapping(&leaf_updates);

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
    use semaphore::generic_storage::MmapVec;
    use semaphore::merkle_tree::Branch;
    use semaphore::poseidon_tree::PoseidonHash;
    use tempfile::NamedTempFile;

    use super::{leaf_to_storage_idx, IdentityTree, LeafUpdates};
    use crate::tree::identity_tree::{node_to_leaf_idx, storage_idx_to_coords};
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

    #[test]
    fn test_auto_purge_cache() -> eyre::Result<()> {
        let temp_file = NamedTempFile::new()?;
        let path = temp_file.path().to_path_buf();

        let mut identity_tree =
            IdentityTree::new_with_cache(TREE_DEPTH, path.clone()).unwrap();

        let leaves = generate_all_leaves();

        for leaf in leaves.iter() {
            identity_tree.tree.push(*leaf).unwrap();
        }

        let mut cache: MmapVec<ruint::Uint<256, 4>> =
            unsafe { MmapVec::<Hash>::restore(&path)? };
        cache[0] = Hash::ZERO;

        let restored_tree =
            IdentityTree::new_with_cache(TREE_DEPTH, path).unwrap();

        assert!(restored_tree.tree.num_leaves() == 0);
        assert!(restored_tree.leaves.is_empty());

        Ok(())
    }
}
