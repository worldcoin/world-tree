use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use semaphore::cascading_merkle_tree::CascadingMerkleTree;
use semaphore::generic_storage::{GenericStorage, MmapVec};
use semaphore::merkle_tree::{Branch, Hasher};
use semaphore::poseidon_tree::{PoseidonHash, Proof};
use semaphore::Field;
use serde::{Deserialize, Serialize};

use super::error::{IdentityTreeError, WorldTreeResult};
use super::{ChainId, Hash, LeafIndex, NodeIndex};

// Leaf index to hash, 0 indexed from the initial leaf
pub type Leaves = HashMap<LeafIndex, Hash>;
// Node index to hash, 0 indexed from the root
pub type StorageUpdates = HashMap<NodeIndex, Hash>;

pub struct IdentityTree<S> {
    /// The canonical tree - the tree that is synced to all other trees
    pub canonical_tree: CascadingMerkleTree<PoseidonHash, S>,

    /// Dense merkle trees for each chain
    pub trees: HashMap<ChainId, CascadingMerkleTree<PoseidonHash, S>>,

    /// Mapping that tracks the latest observed root on each chain id
    pub chain_state: HashMap<ChainId, Hash>,

    /// Updates to the tree that have not yet been applied to all trees
    pub updates: VecDeque<TreeUpdate>,
}

#[derive(Debug, Clone)]
pub struct TreeUpdate {
    pub pre_root: Hash,
    pub update: LeafUpdates,
    pub post_root: Hash,
}

impl IdentityTree<Vec<Hash>> {
    pub fn new(depth: usize, chain_ids: &[ChainId]) -> Self {
        let trees = chain_ids
            .iter()
            .map(|chain_id| {
                (
                    *chain_id,
                    CascadingMerkleTree::new(vec![], depth, &Hash::ZERO),
                )
            })
            .collect();

        let canonical_tree =
            CascadingMerkleTree::new(vec![], depth, &Hash::ZERO);

        let initial_root = canonical_tree.root();

        let chain_state = chain_ids
            .iter()
            .map(|chain_id| (*chain_id, initial_root))
            .collect();

        Self {
            canonical_tree,
            trees,
            chain_state,
            updates: VecDeque::new(),
        }
    }
}

impl IdentityTree<MmapVec<Hash>> {
    /// Returnes a new IdentityTree from a chached file.
    /// The underlying Merkle Tree is unverified
    pub fn new_with_cache_unchecked(
        depth: usize,
        cache_dir: &Path,
        chain_ids: &[ChainId],
    ) -> WorldTreeResult<Self> {
        std::fs::create_dir_all(cache_dir)?;

        let mmap_vecs: HashMap<ChainId, MmapVec<Hash>> = chain_ids
            .iter()
            .map(|chain_id| {
                let path = Self::tree_cache_path(cache_dir, *chain_id);
                let storage = Self::init_storage(path)?;
                WorldTreeResult::Ok((*chain_id, storage))
            })
            .collect::<Result<_, _>>()?;

        let trees: HashMap<
            ChainId,
            CascadingMerkleTree<PoseidonHash, MmapVec<Hash>>,
        > = mmap_vecs
            .into_iter()
            .map(|(chain_id, mmap_vec)| {
                let path = Self::tree_cache_path(cache_dir, chain_id);
                let tree = Self::init_tree(depth, path, mmap_vec)?;
                WorldTreeResult::Ok((chain_id, tree))
            })
            .collect::<Result<_, _>>()?;

        let canonical_cache_path = cache_dir.join("canonical.cache");
        let canonical_storage = Self::init_storage(&canonical_cache_path)?;
        let canonical_tree =
            Self::init_tree(depth, canonical_cache_path, canonical_storage)?;

        let chain_state = chain_ids
            .iter()
            .map(|chain_id| (*chain_id, canonical_tree.root()))
            .collect();

        Ok(Self {
            canonical_tree,
            trees,
            chain_state,
            updates: VecDeque::new(),
        })
    }

    fn tree_cache_path(
        cache_dir: impl AsRef<Path>,
        chain_id: ChainId,
    ) -> PathBuf {
        cache_dir.as_ref().join(format!("{}.cache", chain_id))
    }

    fn init_storage(path: impl AsRef<Path>) -> WorldTreeResult<MmapVec<Hash>> {
        let path = path.as_ref();

        let mmap_vec: MmapVec<Hash> =
            match unsafe { MmapVec::restore_from_path(path) } {
                Ok(mmap_vec) => mmap_vec,

                Err(e) => unsafe {
                    tracing::error!("Cache restore error: {:?}", e);
                    MmapVec::create_from_path(path)?
                },
            };

        WorldTreeResult::Ok(mmap_vec)
    }

    fn init_tree(
        depth: usize,
        path: impl AsRef<Path>,
        mmap_vec: MmapVec<Hash>,
    ) -> WorldTreeResult<CascadingMerkleTree<PoseidonHash, MmapVec<Hash>>> {
        let path = path.as_ref();

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
                    fs::remove_file(path)?;
                    let mmap_vec = unsafe { MmapVec::create_from_path(path)? };

                    CascadingMerkleTree::<PoseidonHash, _>::new(
                        mmap_vec,
                        depth,
                        &Hash::ZERO,
                    )
                }
            };
        Ok(tree)
    }
}

fn initial_root(depth: usize) -> Hash {
    CascadingMerkleTree::<PoseidonHash, _>::new(vec![], depth, &Hash::ZERO)
        .root()
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
        let last_root = self
            .updates
            .back()
            .map(|update| update.post_root)
            .unwrap_or_else(|| initial_root(self.canonical_tree.depth()));

        if pre_root != last_root {
            tracing::error!(?pre_root, ?last_root, "Root mismatch",);

            return Err(IdentityTreeError::RootNotFound);
        }

        self.updates.push_back(TreeUpdate {
            pre_root,
            update: leaf_updates,
            post_root,
        });

        Ok(())
    }

    pub fn update_chain(
        &mut self,
        chain_id: ChainId,
        post_root: Hash,
    ) -> Result<(), IdentityTreeError> {
        let chain_root = self
            .chain_state
            .get_mut(&chain_id)
            .ok_or(IdentityTreeError::TreeNotFound)?;

        *chain_root = post_root;

        Ok(())
    }

    pub fn update_trees(&mut self) -> Result<(), IdentityTreeError> {
        for (chain_id, post_root) in self.chain_state.clone().into_iter() {
            self.update_tree(chain_id, post_root)?;
        }

        Ok(())
    }

    pub fn update_tree(
        &mut self,
        chain_id: ChainId,
        post_root: Hash,
    ) -> Result<(), IdentityTreeError> {
        let tree = self
            .trees
            .get(&chain_id)
            .ok_or(IdentityTreeError::TreeNotFound)?;

        let last_root = tree.root();

        let updates = self.find_updates(last_root, post_root)?;
        let tree = self.trees.get_mut(&chain_id).expect("Tree not found"); // this must success as the previous fetch succeded

        let pre_root = tree.root();
        for update in updates {
            Self::apply_update(tree, update.update);
        }

        let actual_post_root = tree.root();

        tracing::info!(
            ?chain_id,
            ?pre_root,
            ?post_root,
            ?actual_post_root,
            "Updated tree"
        );

        Ok(())
    }

    pub fn realign_trees(&mut self) -> Result<(), IdentityTreeError> {
        let maybe_tree_root_indexes: Vec<_> = self
            .trees
            .values()
            .map(|tree| tree.root())
            .map(|root| {
                let position = self
                    .updates
                    .iter()
                    .position(|update| update.post_root == root);

                position
            })
            .collect();

        let mut tree_root_indexes = vec![];
        for maybe_idx in maybe_tree_root_indexes {
            if let Some(idx) = maybe_idx {
                tree_root_indexes.push(idx);
            } else {
                // None means a given tree is at initial root
                // we can't update
                return Ok(());
            }
        }

        let common_root_idx = *tree_root_indexes.iter().min().unwrap();

        let common_root = self.updates[common_root_idx].post_root;
        let canonical_root = self.canonical_tree.root();

        let updates = self.find_updates(canonical_root, common_root)?;

        for update in updates {
            Self::apply_update(&mut self.canonical_tree, update.update);
        }

        tracing::info!(?common_root, "Realigned trees");

        Ok(())
    }

    /// Finds updates between two roots
    /// If post_root is not present in the update list, will return all the updates after last_post_root
    fn find_updates(
        &self,
        last_post_root: Hash,
        post_root: Hash,
    ) -> Result<Vec<TreeUpdate>, IdentityTreeError> {
        if last_post_root == post_root {
            tracing::debug!(
                ?last_post_root,
                ?post_root,
                "No updates - same root"
            );
            return Ok(vec![]);
        }

        // TODO: Searching in reverse will likely be much faster
        let start = self
            .updates
            .iter()
            .position(|update| update.post_root == last_post_root);

        let start = if let Some(start) = start {
            // We want the next udpate after the last post root
            start + 1
        } else {
            // Initial root
            0
        };

        let mut updates = vec![];

        for i in start..self.updates.len() {
            let update = self.updates[i].clone();
            let update_post_root = update.post_root;
            updates.push(update);

            if update_post_root == post_root {
                break;
            }
        }

        Ok(updates)
    }

    /// Construct an inclusion proof for a given leaf
    /// If a root is provided, the proof is constructed from the specified root
    /// Otherwise, the proof is constructed from the current canonical tree
    pub fn inclusion_proof(
        &self,
        leaf: Hash,
        chain_id: Option<ChainId>,
    ) -> Result<Option<InclusionProof>, IdentityTreeError> {
        let tree = if let Some(chain_id) = chain_id {
            self.trees
                .get(&chain_id)
                .ok_or(IdentityTreeError::TreeNotFound)?
        } else {
            &self.canonical_tree
        };

        let root = tree.root();

        // TODO: Maybe optimize by caching leaf to index mapping
        let Some(proof) = tree.proof_from_hash(leaf) else {
            return Ok(None);
        };

        Ok(Some(InclusionProof { root, proof }))
    }

    fn apply_update(
        tree: &mut CascadingMerkleTree<PoseidonHash, S>,
        update: LeafUpdates,
    ) {
        match update {
            LeafUpdates::Insert(insertions) => {
                let mut leaves: Vec<_> = insertions.into_iter().collect();
                leaves.sort_unstable_by_key(|(idx, _value)| *idx);
                let leaves: Vec<_> =
                    leaves.into_iter().map(|(_idx, value)| value).collect();

                tree.extend_from_slice(&leaves);
            }
            LeafUpdates::Delete(deletions) => {
                for (leaf_index, value) in deletions {
                    tree.set_leaf(leaf_index.0 as usize, value);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum LeafUpdates {
    Insert(Leaves),
    Delete(Leaves),
}

impl LeafUpdates {
    pub fn into_leaves(self) -> Leaves {
        match self {
            LeafUpdates::Insert(leaves) => leaves,
            LeafUpdates::Delete(leaves) => leaves,
        }
    }
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
    // use std::collections::HashMap;

    // use eyre::{eyre, ContextCompat};
    // use rand::{Rng, SeedableRng};
    // use semaphore::cascading_merkle_tree::CascadingMerkleTree;
    // use semaphore::generic_storage::{GenericStorage, MmapVec};
    // use semaphore::merkle_tree::Branch;
    // use semaphore::poseidon_tree::PoseidonHash;
    // use tempfile::NamedTempFile;

    // use super::*;
    // use crate::tree::{Hash, LeafIndex};

    // fn leaves() -> impl Iterator<Item = Hash> {
    //     let mut rng = rand::rngs::SmallRng::seed_from_u64(42);

    //     std::iter::from_fn(move || {
    //         let mut limbs: [u64; 4] = rng.gen();
    //         limbs[3] = 0; // nullify most significant limb to keep the values in the Field

    //         Some(Hash::from_limbs(limbs))
    //     })
    // }

    // fn init_tracing() {
    //     let _ = tracing_subscriber::fmt::try_init();
    // }

    // #[test]
    // fn test_append_updates() -> eyre::Result<()> {
    //     init_tracing();

    //     let tree_depth = 30;
    //     let num_leaves = 1 << 12;

    //     let mut identity_tree = IdentityTree::new(tree_depth);
    //     let mut tree: CascadingMerkleTree<PoseidonHash> =
    //         CascadingMerkleTree::new(vec![], tree_depth, &Hash::ZERO);

    //     // Generate the first half of the leaves and insert into the tree
    //     let leaves = leaves().take(num_leaves).collect::<Vec<_>>();
    //     let first_half = leaves[0..num_leaves / 2].to_vec();
    //     let second_half = leaves[num_leaves / 2..].to_vec();

    //     for (idx, leaf) in first_half.iter().enumerate() {
    //         identity_tree.insert(idx as u32, *leaf)?;
    //     }

    //     tree.extend_from_slice(&first_half);
    //     let pre_root = tree.root();
    //     tree.extend_from_slice(&second_half);
    //     let post_root = tree.root();

    //     // Need to update the internal root tracking
    //     identity_tree.roots.push(pre_root);
    //     identity_tree.root_map.insert(pre_root, 1);

    //     // Collect the second half of the leaves
    //     let offset = NUM_LEAVES / 2;
    //     let leaf_updates = second_half
    //         .iter()
    //         .enumerate()
    //         .map(|(idx, value)| (((idx + offset) as u32).into(), *value))
    //         .collect::<HashMap<LeafIndex, Hash>>();

    //     // Cache the expected root as the tree root should not change from the appended updates
    //     let expected_root = identity_tree.tree.root();

    //     identity_tree.append_updates(
    //         pre_root,
    //         post_root,
    //         LeafUpdates::Insert(leaf_updates),
    //     )?;

    //     // Ensure that the root is correct and the updates are stored
    //     assert_eq!(identity_tree.tree.root(), expected_root);
    //     assert_eq!(identity_tree.tree_updates.len(), 1);

    //     //TODO: assert expected updates

    //     identity_tree.apply_updates_to_root(&post_root);

    //     assert_eq!(identity_tree.tree.root(), tree.root());
    //     assert_eq!(identity_tree.tree_updates.len(), 0);

    //     Ok(())
    // }

    // #[test]
    // fn test_apply_updates_to_root() -> eyre::Result<()> {
    //     let mut identity_tree = IdentityTree::new(TREE_DEPTH);
    //     let mut tree: CascadingMerkleTree<PoseidonHash> =
    //         CascadingMerkleTree::new(vec![], TREE_DEPTH, &Hash::ZERO);

    //     // Generate the first half of the leaves and insert into the tree
    //     let leaves = generate_all_leaves();
    //     let first_half = leaves[0..NUM_LEAVES / 2].to_vec();
    //     let second_half = leaves[NUM_LEAVES / 2..].to_vec();

    //     for (idx, leaf) in first_half.iter().enumerate() {
    //         identity_tree.insert(idx as u32, *leaf)?;
    //         assert_eq!(tree.num_leaves(), idx);
    //         tree.push(*leaf)?;
    //     }

    //     // Generate the updated tree with all of the leaves
    //     let pre_root = tree.root();

    //     identity_tree.roots.push(pre_root);
    //     identity_tree.root_map.insert(pre_root, 1);

    //     tree.extend_from_slice(&second_half);
    //     let post_root = tree.root();

    //     // Collect the second half of the leaves
    //     let leaf_updates = second_half
    //         .iter()
    //         .enumerate()
    //         .map(|(idx, value)| {
    //             (LeafIndex((NUM_LEAVES / 2 + idx) as u32), *value)
    //         })
    //         .collect::<HashMap<LeafIndex, Hash>>();

    //     identity_tree.append_updates(
    //         pre_root,
    //         post_root,
    //         LeafUpdates::Insert(leaf_updates),
    //     )?;

    //     // Apply updates to the tree
    //     identity_tree.apply_updates_to_root(&post_root);

    //     assert_eq!(identity_tree.tree.root(), post_root);
    //     assert_eq!(identity_tree.tree_updates.len(), 0);

    //     for (leaf_idx, leaf) in leaves.iter().enumerate() {
    //         let proof = identity_tree
    //             .inclusion_proof(*leaf, None)?
    //             .ok_or(eyre!("Proof not found"))?;

    //         assert_eq!(proof.root, post_root);
    //         assert_eq!(proof.proof, tree.proof(leaf_idx));

    //         assert!(proof.verify(*leaf));
    //     }

    //     Ok(())
    // }

    // #[test]
    // fn test_compute_root() -> eyre::Result<()> {
    //     let mut identity_tree = IdentityTree::new(TREE_DEPTH);

    //     // Generate the first half of the leaves and insert into the tree
    //     let leaves = generate_all_leaves();

    //     for (idx, leaf) in leaves[0..NUM_LEAVES / 2].iter().enumerate() {
    //         identity_tree.insert(idx as u32, *leaf)?;
    //     }

    //     // Generate the updated tree with all of the leaves
    //     let expected_tree: CascadingMerkleTree<PoseidonHash> =
    //         CascadingMerkleTree::new_with_leaves(
    //             vec![],
    //             TREE_DEPTH,
    //             &Hash::ZERO,
    //             &leaves,
    //         );

    //     // Collect the second half of the leaves
    //     let leaf_updates = leaves[(NUM_LEAVES / 2)..].to_vec();

    //     let updated_root = identity_tree.compute_root(&leaf_updates, None)?;
    //     let expected_root = expected_tree.root();

    //     assert_eq!(updated_root, expected_root);

    //     Ok(())
    // }

    // #[test]
    // fn test_inclusion_proof() -> eyre::Result<()> {
    //     let mut identity_tree = IdentityTree::new(TREE_DEPTH);

    //     let leaves: Vec<_> = leaves().take(4).collect();

    //     println!("leaves: {:?}", leaves);

    //     // We insert only the first leaf
    //     identity_tree.insert(0, leaves[0])?;
    //     identity_tree.roots.push(identity_tree.tree.root());
    //     identity_tree.root_map.insert(identity_tree.tree.root(), 1);

    //     let initial_root = {
    //         let mut tree = CascadingMerkleTree::<PoseidonHash>::new(
    //             vec![],
    //             TREE_DEPTH,
    //             &Hash::ZERO,
    //         );

    //         tree.push(leaves[0])?;
    //         tree.root()
    //     };

    //     // Simulate and create updates
    //     let (root_012, updates) = {
    //         let mut tree = CascadingMerkleTree::<PoseidonHash>::new(
    //             vec![],
    //             TREE_DEPTH,
    //             &Hash::ZERO,
    //         );

    //         tree.push(leaves[0])?;
    //         tree.push(leaves[1])?;
    //         tree.push(leaves[2])?;

    //         let updates = LeafUpdates::Insert(
    //             vec![(1.into(), leaves[1]), (2.into(), leaves[2])]
    //                 .into_iter()
    //                 .collect::<HashMap<LeafIndex, Hash>>(),
    //         );

    //         (tree.root(), updates)
    //     };

    //     identity_tree.append_updates(initial_root, root_012, updates)?;

    //     // Simulate and create updates
    //     let (root_0123, updates) = {
    //         let mut tree = CascadingMerkleTree::<PoseidonHash>::new(
    //             vec![],
    //             TREE_DEPTH,
    //             &Hash::ZERO,
    //         );

    //         tree.push(leaves[0])?;
    //         tree.push(leaves[1])?;
    //         tree.push(leaves[2])?;
    //         tree.push(leaves[3])?;

    //         let updates = LeafUpdates::Insert(
    //             vec![(3.into(), leaves[3])]
    //                 .into_iter()
    //                 .collect::<HashMap<LeafIndex, Hash>>(),
    //         );

    //         (tree.root(), updates)
    //     };

    //     identity_tree.append_updates(root_012, root_0123, updates)?;

    //     let proof = identity_tree
    //         .inclusion_proof(leaves[3], Some(&root_0123))?
    //         .context("Missing proof")?;

    //     assert_eq!(
    //         proof.proof.0[0],
    //         Branch::Right(leaves[2]),
    //         "The first sibling of leaf 3 must be leaf 2"
    //     );

    //     let proof = identity_tree
    //         .inclusion_proof(leaves[2], Some(&root_012))?
    //         .context("Missing proof")?;

    //     assert_eq!(
    //         proof.proof.0[0],
    //         Branch::Left(Hash::ZERO),
    //         "The first sibling of leaf 2 must be zero hash"
    //     );

    //     let proof = identity_tree.inclusion_proof(leaves[2], None)?;

    //     assert!(
    //         proof.is_none(),
    //         "The canonical tree does not contain this update yet"
    //     );

    //     Ok(())
    // }

    // #[test]
    // fn test_mmap_cache() -> WorldTreeResult<()> {
    //     let temp_file = NamedTempFile::new()?;
    //     let path = temp_file.path().to_path_buf();

    //     let mut identity_tree =
    //         IdentityTree::new_with_cache_unchecked(TREE_DEPTH, &path)?;

    //     let leaves = generate_all_leaves();

    //     for leaf in leaves.iter() {
    //         identity_tree.tree.push(*leaf)?;
    //     }

    //     let restored_tree =
    //         IdentityTree::new_with_cache_unchecked(TREE_DEPTH, &path)?;

    //     assert_eq!(identity_tree.tree.root(), restored_tree.tree.root());

    //     for leaf in leaves.iter() {
    //         let proof = restored_tree
    //             .inclusion_proof(*leaf, None)?
    //             .expect("Could not get proof");

    //         assert!(proof.verify(*leaf));
    //     }

    //     Ok(())
    // }

    // #[test]
    // fn test_mmap_append_and_apply() -> WorldTreeResult<()> {
    //     let tree_depth = 30;
    //     let batch_size = 16;
    //     let num_batches = 64;
    //     let num_leaves = num_batches * batch_size;

    //     let temp_file = NamedTempFile::new()?;
    //     let path = temp_file.path().to_path_buf();

    //     let mut identity_tree =
    //         IdentityTree::new_with_cache_unchecked(tree_depth, &path)?;

    //     let mut ref_tree: CascadingMerkleTree<PoseidonHash> =
    //         CascadingMerkleTree::new(vec![], tree_depth, &Hash::ZERO);

    //     let leaves: Vec<_> = leaves().take(num_leaves).collect();
    //     let mut batches = vec![];
    //     for batch in leaves.chunks(batch_size) {
    //         batches.push(batch.to_vec());
    //     }

    //     for (idx, batch) in batches.iter().enumerate() {
    //         let pre_root = ref_tree.root();
    //         ref_tree.extend_from_slice(batch);
    //         let post_root = ref_tree.root();

    //         let start_index = idx * batch_size;
    //         let insertions = batch
    //             .iter()
    //             .enumerate()
    //             .map(|(idx, value)| {
    //                 let leaf_idx = LeafIndex((start_index + idx) as u32);
    //                 (leaf_idx, *value)
    //             })
    //             .collect::<HashMap<LeafIndex, Hash>>();

    //         identity_tree.append_updates(
    //             pre_root,
    //             post_root,
    //             LeafUpdates::Insert(insertions),
    //         )?;
    //     }

    //     let last_root = ref_tree.root();

    //     identity_tree.apply_updates_to_root(&last_root);

    //     drop(identity_tree);

    //     let size = cross_platform_file_size(&path)?;

    //     let meta_size = std::mem::size_of::<usize>();
    //     let expected_size =
    //         num_leaves * 2 * std::mem::size_of::<Hash>() + meta_size;

    //     assert_eq!(
    //         size, expected_size,
    //         "Cache size should be {expected_size} but is {size}"
    //     );

    //     Ok(())
    // }

    // fn cross_platform_file_size(path: impl AsRef<Path>) -> eyre::Result<usize> {
    //     let meta = std::fs::metadata(path.as_ref())?;

    //     #[cfg(unix)]
    //     let size = std::os::unix::fs::MetadataExt::size(&meta) as usize;

    //     #[cfg(windows)]
    //     let size = std::os::windows::fs::MetadataExt::file_size(&meta) as usize;

    //     Ok(size)
    // }

    // #[test]
    // fn test_auto_purge_cache() -> eyre::Result<()> {
    //     let temp_file = NamedTempFile::new()?;
    //     let path = temp_file.path().to_path_buf();

    //     let mut identity_tree =
    //         IdentityTree::new_with_cache_unchecked(TREE_DEPTH, &path).unwrap();

    //     let leaves = generate_all_leaves();

    //     for leaf in leaves.iter() {
    //         identity_tree.tree.push(*leaf).unwrap();
    //     }

    //     let mut cache: MmapVec<Hash> =
    //         unsafe { MmapVec::<Hash>::restore_from_path(&path)? };
    //     cache[0] = Hash::ZERO;

    //     let restored_tree =
    //         IdentityTree::new_with_cache_unchecked(TREE_DEPTH, &path).unwrap();

    //     assert!(restored_tree.tree.num_leaves() == 0);
    //     assert!(restored_tree.leaves.is_empty());

    //     Ok(())
    // }

    // #[test]
    // fn consecutive_staggered_updates() -> eyre::Result<()> {
    //     let tree_depth = 30;
    //     let num_leaves = 1 << 12;
    //     let batch_size = 32;
    //     let num_batches = num_leaves / batch_size;

    //     assert_eq!(num_batches, 128);

    //     let mut identity_tree = IdentityTree::new(tree_depth);
    //     let mut ref_tree: CascadingMerkleTree<PoseidonHash> =
    //         CascadingMerkleTree::new(vec![], tree_depth, &Hash::ZERO);

    //     let leaves: Vec<_> = leaves().take(num_leaves).collect();
    //     let batches: Vec<_> = leaves
    //         .chunks(batch_size)
    //         .map(|batch| batch.to_vec())
    //         .collect();

    //     let mut precalc_batches = vec![];
    //     for batch in &batches {
    //         let pre_root = ref_tree.root();
    //         ref_tree.extend_from_slice(batch);
    //         let post_root = ref_tree.root();

    //         precalc_batches.push((pre_root, post_root));
    //     }

    //     let install_batch = |identity_tree: &mut IdentityTree<Vec<Hash>>,
    //                          batch_idx: usize| {
    //         let batch = &batches[batch_idx];
    //         let leaf_updates = batch
    //             .iter()
    //             .enumerate()
    //             .map(|(idx, value)| {
    //                 (LeafIndex((batch_idx * batch_size + idx) as u32), *value)
    //             })
    //             .collect::<HashMap<LeafIndex, Hash>>();

    //         let pre_root = precalc_batches[batch_idx].0;
    //         let post_root = precalc_batches[batch_idx].1;

    //         identity_tree.append_updates(
    //             pre_root,
    //             post_root,
    //             LeafUpdates::Insert(leaf_updates),
    //         )?;

    //         eyre::Result::<Hash>::Ok(post_root)
    //     };

    //     install_batch(&mut identity_tree, 0)?;
    //     install_batch(&mut identity_tree, 1)?;
    //     install_batch(&mut identity_tree, 2)?;

    //     // Apply updates up to root 1
    //     identity_tree.apply_updates_to_root(&precalc_batches[1].1);

    //     install_batch(&mut identity_tree, 3)?;

    //     // Apply updates up to root 3
    //     identity_tree.apply_updates_to_root(&precalc_batches[3].1);

    //     // Verify all the inserted leaves
    //     for batch_idx in 0..=3 {
    //         for n in 0..batch_size {
    //             let leaf = batches[batch_idx][n];
    //             let inclusion_proof =
    //                 identity_tree.inclusion_proof(leaf, None)?;
    //             let inclusion_proof =
    //                 inclusion_proof.expect("Missing inclusion proof");

    //             assert!(inclusion_proof.verify(leaf), "Proof must be valid for leaf {leaf:?} (batch {batch_idx}, index {n})");
    //         }
    //     }

    //     Ok(())
    // }
}
