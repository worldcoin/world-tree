use std::collections::VecDeque;

use semaphore::lazy_merkle_tree::{Canonical, Derived, VersionMarker};
use semaphore::poseidon_tree::Proof;
use tokio::sync::RwLock;

use super::{Hash, PoseidonTree};
use crate::server::InclusionProof;

/// The `TreeData` structure represents the in-memory representation of the tree as well as its historic
/// versions up to `tree_history_size`.
pub struct TreeData {
    /// Tree holding all leaves
    pub tree: RwLock<PoseidonTree<Derived>>,
    /// Number of most recent historical roots `WorldTree` keeps to be able to serve proofs for
    pub tree_history_size: usize,
    /// Array of derived trees for each historical root
    pub tree_history: RwLock<VecDeque<PoseidonTree<Derived>>>, //TODO: make a note that the latest is at the front
}
/// Defines how the tree changed
pub struct TreeUpdate {
    pub index: usize,
    /// New hash of the tree
    pub value: Hash,
}

impl TreeData {
    /// Constructor
    ///
    /// `tree`: Canonical in-memory tree
    ///
    /// `tree_history_size`: Number of most recent historical roots that the `WorldTree` can serve proofs for
    pub fn new(
        tree: PoseidonTree<Canonical>,
        tree_history_size: usize,
    ) -> Self {
        Self {
            tree_history_size,
            tree: RwLock::new(tree.derived()),
            tree_history: RwLock::new(VecDeque::new()),
        }
    }

    /// Inserts identities at a given starting index.  
    ///
    /// `start_index`: Index at which to start inserting identity commitments
    ///
    /// `identities`: Identity commitments
    pub async fn insert_many_at(
        &self,
        start_index: usize,
        identities: &[Hash],
    ) {
        self.cache_tree_history().await;

        let mut tree = self.tree.write().await;
        for (i, identity) in identities.iter().enumerate() {
            *tree = tree.update(start_index + i, identity);
        }
    }

    /// Deletes identity commitments in the `WorldTree` at the specified indices.
    ///
    /// `delete_indices`: positions of the identity commitments in the tree to be deleted
    pub async fn delete_many(&self, delete_indices: &[usize]) {
        self.cache_tree_history().await;

        let mut tree = self.tree.write().await;

        for idx in delete_indices.iter() {
            *tree = tree.update(*idx, &Hash::ZERO);
        }
    }

    /// Caches the historical tree representations in memory in order for the `WorldTree` to be able to serve
    /// proofs against their roots
    pub async fn cache_tree_history(&self) {
        if self.tree_history_size != 0 {
            let mut tree_history = self.tree_history.write().await;

            if tree_history.len() == self.tree_history_size {
                tree_history.pop_back();
            }

            tree_history.push_front(self.tree.read().await.clone());
        }
    }

    /// Fetches the inclusion proof of the provided identity at the given root hash
    ///
    /// `identity`: Identity commitment to fetch the inclusion proof for
    ///
    /// `root`: The root against which the `WorldTree` will serve the inclusion proof against
    ///
    /// Returns None if the provided root hash is not in the latest one or is not present in tree history
    /// or if the identity is not present in the tree.
    pub async fn get_inclusion_proof(
        &self,
        identity: Hash,
        root: Option<Hash>,
    ) -> Option<InclusionProof> {
        let tree = self.tree.read().await;

        // If the root is not specified, use the latest root
        if root.is_none() {
            Some(InclusionProof::new(
                tree.root(),
                Self::proof(&tree, identity)?,
                None,
            ))
        } else {
            let root = root.unwrap();

            // If the root is the latest root, use the current version of the tree
            if root == tree.root() {
                return Some(InclusionProof::new(
                    root,
                    Self::proof(&tree, identity)?,
                    None,
                ));
            } else {
                let tree_history = self.tree_history.read().await;
                // // Otherwise, search the tree history for the root and use the corresponding tree
                for prev_tree in tree_history.iter() {
                    if prev_tree.root() == root {
                        return Some(InclusionProof::new(
                            root,
                            Self::proof(prev_tree, identity)?,
                            None,
                        ));
                    }
                }
            }

            None
        }
    }

    /// Returns an inclusion proof for a given identity commitment in the specified tree
    ///
    /// `tree`: The `WorldTree` (canonical or derived) to fetch the inclusion proof against
    ///
    /// `identity`: Identity commitment to create the inclusion proof for
    fn proof<V: VersionMarker>(
        tree: &PoseidonTree<V>,
        identity: Hash,
    ) -> Option<Proof> {
        let idx = tree.leaves().position(|leaf| leaf == identity)?;

        Some(tree.proof(idx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE_DEPTH: usize = 10;
    const NUM_IDENTITIES: usize = 10;
    const TREE_HISTORY_SIZE: usize = 5;

    fn initialize_tree_data(
        tree_depth: usize,
        tree_history_size: usize,
        num_identities: usize,
    ) -> (TreeData, PoseidonTree<Canonical>, Vec<Hash>) {
        let poseidon_tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
            tree_depth,
            tree_depth,
            &Hash::ZERO,
        );
        let ref_tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
            tree_depth,
            tree_depth,
            &Hash::ZERO,
        );

        let identities: Vec<_> = (0..num_identities).map(Hash::from).collect();

        let tree: TreeData = TreeData::new(poseidon_tree, tree_history_size);

        (tree, ref_tree, identities)
    }

    #[tokio::test]
    async fn test_get_inclusion_proof() {
        let (tree_data, mut ref_tree, identities) =
            initialize_tree_data(TREE_DEPTH, TREE_HISTORY_SIZE, NUM_IDENTITIES);

        tree_data.insert_many_at(0, &identities).await;

        for (idx, identity) in identities.iter().enumerate() {
            ref_tree = ref_tree.update_with_mutation(idx, identity);
        }

        assert_eq!(
            tree_data.tree_history.read().await.len(),
            1,
            "We should have 1 entry in tree history"
        );

        let root = ref_tree.root();

        for (i, identity) in identities.iter().enumerate().take(NUM_IDENTITIES)
        {
            let proof_from_world_tree = tree_data
                .get_inclusion_proof(*identity, Some(root))
                .await
                .unwrap();

            assert_eq!(ref_tree.proof(i), proof_from_world_tree.proof);
        }
    }

    #[tokio::test]
    async fn test_get_inclusion_proof_for_intermediate_root() {
        let (tree_data, mut ref_tree, identities) =
            initialize_tree_data(TREE_DEPTH, TREE_HISTORY_SIZE, NUM_IDENTITIES);

        for (idx, identity) in identities.iter().enumerate().take(5) {
            ref_tree = ref_tree.update_with_mutation(idx, identity);
        }

        let root = ref_tree.root();

        // Since the tree state is cached to tree history before a sequence of updates, we need to apply the first 5 updates to
        // ensure that the intermediate root is in the tree history
        tree_data.insert_many_at(0, &identities[0..5]).await;

        // Then you can apply the remaining updates
        tree_data.insert_many_at(5, &identities[5..]).await;

        for (i, _identity) in identities.iter().enumerate().take(5) {
            let proof_from_world_tree = tree_data
                .get_inclusion_proof(identities[i], Some(root))
                .await
                .unwrap();

            assert_eq!(ref_tree.proof(i), proof_from_world_tree.proof);
        }
    }

    #[tokio::test]
    async fn test_tree_history_capacity() {
        let (tree_data, _, identities) =
            initialize_tree_data(TREE_DEPTH, TREE_HISTORY_SIZE, NUM_IDENTITIES);

        // Apply an update to the tree one identity at a time to apply all changes to the tree history cache
        for (idx, identity) in identities.into_iter().enumerate() {
            tree_data.insert_many_at(idx, &[identity]).await;
        }

        // The tree history should not be larger than the tree history size
        assert_eq!(
            tree_data.tree_history.read().await.len(),
            tree_data.tree_history_size,
        );
    }

    #[tokio::test]
    async fn test_get_inclusion_proof_after_deletions() {
        let (tree_data, mut ref_tree, identities) =
            initialize_tree_data(TREE_DEPTH, TREE_HISTORY_SIZE, NUM_IDENTITIES);

        // Apply all identity updates to the ref tree and test tree
        for (idx, identity) in identities.iter().enumerate() {
            ref_tree = ref_tree.update_with_mutation(idx, identity);
        }

        tree_data.insert_many_at(0, &identities).await;

        // Initialize a vector of indices to delete
        let deleted_identity_idxs = &[3, 7];
        let non_deleted_identity_idxs: Vec<_> = (0..NUM_IDENTITIES)
            .filter(|idx| !deleted_identity_idxs.contains(idx))
            .collect();

        // Delete the identities at the specified indices for the ref tree and test tree
        for idx in deleted_identity_idxs {
            ref_tree = ref_tree.update_with_mutation(*idx, &Hash::ZERO);
        }
        tree_data.delete_many(deleted_identity_idxs).await;

        let root = ref_tree.root();

        // Ensure that an inclusion proof can be generated for all identities that were not deleted
        for i in non_deleted_identity_idxs {
            let proof_from_world_tree = tree_data
                .get_inclusion_proof(identities[i], Some(root))
                .await
                .unwrap();

            assert_eq!(ref_tree.proof(i), proof_from_world_tree.proof);
        }

        // Ensure that an inclusion proof cannot be generated for deleted identities
        for i in deleted_identity_idxs {
            let proof_from_world_tree = tree_data
                .get_inclusion_proof(identities[*i], Some(root))
                .await;

            assert!(proof_from_world_tree.is_none());
        }
    }
}
