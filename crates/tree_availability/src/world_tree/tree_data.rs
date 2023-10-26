use std::collections::VecDeque;
use std::fmt::format;
use std::ops::DerefMut;

use semaphore::lazy_merkle_tree::{
    Canonical, Derived, LazyMerkleTree, VersionMarker,
};
use semaphore::poseidon_tree::{PoseidonHash, Proof};
use tokio::sync::RwLock;

use super::{Hash, PoseidonTree};
use crate::server::InclusionProof;

pub struct TreeData {
    pub tree: RwLock<PoseidonTree<Derived>>,
    pub tree_history_size: usize,
    pub tree_history: RwLock<VecDeque<PoseidonTree<Derived>>>, //TODO: make a note that the latest is at the front
}
pub struct TreeUpdate {
    pub index: usize,
    pub value: Hash,
}

impl TreeData {
    pub fn new(tree: PoseidonTree<Derived>, tree_history_size: usize) -> Self {
        Self {
            tree_history_size,
            tree: RwLock::new(tree),
            tree_history: RwLock::new(VecDeque::new()),
        }
    }

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

    pub async fn delete_many(&self, delete_indices: &[usize]) {
        self.cache_tree_history().await;
        let mut tree = self.tree.write().await;

        for idx in delete_indices.iter() {
            *tree = tree.update(*idx, &Hash::ZERO);
        }
    }

    pub async fn cache_tree_history(&self) {
        let mut tree_history = self.tree_history.write().await;

        if tree_history.len() == self.tree_history_size {
            tree_history.pop_back();
        }

        tree_history.push_front(self.tree.read().await.clone());
    }

    /// Fetches the inclusion proof of the provided identity at the given root hash
    ///
    /// Returns None if the provided root hash is not in the latest one or is not present in tree history
    /// or if the identity is not present in the tree
    pub async fn get_inclusion_proof(
        &self,
        identity: Hash,
        root: Option<Hash>,
    ) -> Option<InclusionProof> {
        let tree = self.tree.read().await;

        // If the root is not specified, use the latest root
        if root.is_none() {
            dbg!(tree.root());
            for leaf in tree.leaves().into_iter() {
                dbg!(leaf);
            }

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

    fn proof<V: VersionMarker>(
        tree: &PoseidonTree<V>,
        identity: Hash,
    ) -> Option<Proof> {
        let idx = tree.leaves().position(|leaf| leaf == identity)?;

        Some(tree.proof(idx))
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;

//     const TREE_DEPTH: usize = 10;
//     const NUM_IDENTITIES: usize = 10;

//     const TREE_HISTORY_SIZE: usize = 10;

//     #[tokio::test]
//     async fn fetch_proof_for_latest_root() {
//         let poseidon_tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
//             TREE_DEPTH,
//             TREE_DEPTH,
//             &Hash::ZERO,
//         );
//         let mut ref_tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
//             TREE_DEPTH,
//             TREE_DEPTH,
//             &Hash::ZERO,
//         );

//         let identities: Vec<_> = (0..NUM_IDENTITIES).map(Hash::from).collect();

//         let world_tree = TreeData::new(poseidon_tree, TREE_HISTORY_SIZE);

//         for (idx, identity) in identities.iter().enumerate() {
//             ref_tree = ref_tree.update_with_mutation(idx, identity);
//         }

//         world_tree.insert_many_at(0, &identities).await;

//         let root = ref_tree.root();

//         for i in 0..NUM_IDENTITIES {
//             let proof_from_world_tree = world_tree
//                 .get_inclusion_proof(identities[i], Some(root))
//                 .await
//                 .unwrap();

//             assert_eq!(ref_tree.proof(i), proof_from_world_tree.proof);
//         }
//     }

//     #[tokio::test]
//     async fn fetch_proof_for_intermediate_root() {
//         let poseidon_tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
//             TREE_DEPTH,
//             TREE_DEPTH,
//             &Hash::ZERO,
//         );

//         let mut ref_tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
//             TREE_DEPTH,
//             TREE_DEPTH,
//             &Hash::ZERO,
//         );

//         let identities: Vec<_> = (0..NUM_IDENTITIES).map(Hash::from).collect();

//         let world_tree = TreeData::new(poseidon_tree, TREE_HISTORY_SIZE);

//         for (idx, identity) in identities.iter().enumerate().take(5) {
//             ref_tree = ref_tree.update_with_mutation(idx, identity);
//         }

//         let root = ref_tree.root();

//         // No more updates to the reference tree as we need to fetch
//         // the proof from an older version

//         world_tree.insert_many_at(0, &identities).await;

//         for i in 0..5 {
//             let proof_from_world_tree = world_tree
//                 .get_inclusion_proof(identities[i], Some(root))
//                 .await
//                 .unwrap();

//             assert_eq!(ref_tree.proof(i), proof_from_world_tree.proof);
//         }
//     }

//     #[tokio::test]
//     async fn deletion_of_identities() {
//         let poseidon_tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
//             TREE_DEPTH,
//             TREE_DEPTH,
//             &Hash::ZERO,
//         );

//         let mut ref_tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
//             TREE_DEPTH,
//             TREE_DEPTH,
//             &Hash::ZERO,
//         );

//         let identities: Vec<_> = (0..NUM_IDENTITIES).map(Hash::from).collect();

//         let tree = TreeData::new(poseidon_tree, TREE_HISTORY_SIZE);

//         for (idx, identity) in identities.iter().enumerate() {
//             ref_tree = ref_tree.update_with_mutation(idx, identity);
//         }

//         tree.insert_many_at(0, &identities).await;

//         let deleted_identity_idxs = &[3, 7];
//         let non_deleted_identity_idxs: Vec<_> = (0..NUM_IDENTITIES)
//             .filter(|idx| !deleted_identity_idxs.contains(idx))
//             .collect();

//         for idx in deleted_identity_idxs {
//             ref_tree = ref_tree.update_with_mutation(*idx, &Hash::ZERO);
//         }

//         tree.delete_many(deleted_identity_idxs).await;

//         let root = ref_tree.root();

//         for i in non_deleted_identity_idxs {
//             let proof_from_world_tree = tree
//                 .get_inclusion_proof(identities[i], Some(root))
//                 .await
//                 .unwrap();

//             assert_eq!(ref_tree.proof(i), proof_from_world_tree.proof);
//         }
//     }

//     #[tokio::test]
//     async fn fetching_proof_after_gc() {
//         let poseidon_tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
//             TREE_DEPTH,
//             TREE_DEPTH,
//             &Hash::ZERO,
//         );
//         let mut ref_tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
//             TREE_DEPTH,
//             TREE_DEPTH,
//             &Hash::ZERO,
//         );

//         let identities: Vec<_> = (0..NUM_IDENTITIES).map(Hash::from).collect();

//         // NOTE: History size is set to 2
//         let tree = TreeData::new(poseidon_tree, 5);

//         for (idx, identity) in identities.iter().enumerate() {
//             ref_tree = ref_tree.update_with_mutation(idx, identity);
//         }

//         tree.insert_many_at(0, &identities).await;

//         assert_eq!(
//             tree.tree_history.read().await.len(),
//             NUM_IDENTITIES,
//             "We should have {NUM_IDENTITIES} before GC"
//         );

//         tree.gc().await;

//         assert_eq!(
//             tree.tree_history.read().await.len(),
//             5,
//             "We should have 5 entries in tree history after GC"
//         );

//         let root = ref_tree.root();

//         for i in 0..NUM_IDENTITIES {
//             let proof_from_world_tree = tree
//                 .get_inclusion_proof(identities[i], Some(root))
//                 .await
//                 .unwrap();

//             assert_eq!(ref_tree.proof(i), proof_from_world_tree.proof);
//         }
//     }
// }
