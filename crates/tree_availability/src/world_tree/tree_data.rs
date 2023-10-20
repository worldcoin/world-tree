use std::collections::VecDeque;
use std::ops::DerefMut;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use semaphore::lazy_merkle_tree::{
    Canonical, Derived, LazyMerkleTree, VersionMarker,
};
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::{PoseidonHash, Proof};
use tokio::sync::RwLock;

use super::{Hash, PoseidonTree};
use crate::server::InclusionProof;

pub struct TreeData {
    pub tree: RwLock<PoseidonTree<Canonical>>,
    pub tree_history_size: usize,
    // TODO: This is an inefficient representation
    //       we should keep a list of structs where each struct has an associated root
    //       that is equal to the root of the last update
    //       and contains a list of updates
    //       that way we can remove from the history entires associated with actual on-chain roots
    pub tree_history: RwLock<VecDeque<(TreeUpdate, PoseidonTree<Derived>)>>,
}
pub struct TreeUpdate {
    pub index: usize,
    pub value: Hash,
}

impl TreeData {
    pub fn new(
        tree: PoseidonTree<Canonical>,
        tree_history_size: usize,
    ) -> Self {
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
        let mut history = self.tree_history.write().await;

        let Some(first_identity) = identities.get(0) else {
            return;
        };

        let mut next = if history.is_empty() {
            let tree = self.tree.read().await;
            tree.update(start_index, first_identity)
        } else {
            let (_, last_history_entry) = history.back().unwrap();

            last_history_entry.update(start_index, first_identity)
        };

        let first_update = TreeUpdate {
            index: start_index,
            value: first_identity.clone(),
        };
        history.push_back((first_update, next.clone()));

        for (i, identity) in identities.iter().enumerate().skip(1) {
            let update = TreeUpdate {
                index: start_index + i,
                value: identity.clone(),
            };

            next = next.update(start_index + i, identity);
            history.push_back((update, next.clone()));
        }
    }

    pub async fn delete_many(&self, delete_indices: &[usize]) {
        let mut history = self.tree_history.write().await;

        let Some(first_idx) = delete_indices.get(0) else {
            return;
        };

        let mut next = if history.is_empty() {
            let tree: tokio::sync::RwLockReadGuard<
                '_,
                LazyMerkleTree<PoseidonHash, Canonical>,
            > = self.tree.read().await;
            tree.update(*first_idx, &Hash::ZERO)
        } else {
            let (_, last_history_entry) = history.back().unwrap();

            last_history_entry.update(*first_idx, &Hash::ZERO)
        };

        let first_update = TreeUpdate {
            index: *first_idx,
            value: Hash::ZERO,
        };
        history.push_back((first_update, next.clone()));

        for idx in delete_indices.iter().skip(1) {
            let update = TreeUpdate {
                index: *idx,
                value: Hash::ZERO,
            };

            next = next.update(*idx, &Hash::ZERO);
            history.push_back((update, next.clone()));
        }
    }

    /// Garbage collects the tree history
    ///
    /// Leaves up to `self.tree_history_size` entries in `self.tree_history`
    /// The deleted entries are applied to the canonical tree
    ///
    /// This method also recalculates the updates on top of the canonical tree
    pub async fn gc(&self) {
        let mut tree_history = self.tree_history.write().await;
        let mut tree = self.tree.write().await;

        while tree_history.len() > self.tree_history_size {
            let (update, _updated_tree) = tree_history.pop_front().unwrap();

            take_mut::take(tree.deref_mut(), |tree| {
                tree.update_with_mutation(update.index, &update.value)
            });
        }

        let mut history_drain = tree_history.drain(..);
        let (first_update, _) = history_drain.next().unwrap();

        let mut next = tree.update(first_update.index, &first_update.value);

        let mut new_history = VecDeque::new();
        new_history.push_back((first_update, next.clone()));

        for (update, _) in history_drain {
            next = next.update(update.index, &update.value);
            new_history.push_back((update, next.clone()));
        }

        *tree_history = new_history;
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
            return Some(InclusionProof::new(
                tree.root(),
                Self::proof(&tree, identity)?,
                None,
            ));
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
                // Otherwise, search the tree history for the root and use the corresponding tree
                for (_, prev_tree) in tree_history.iter() {
                    if prev_tree.root() == root {
                        return Some(InclusionProof::new(
                            root,
                            Self::proof(&prev_tree, identity)?,
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

// #[test]
// fn test_pack_indices() {
//     let indices = vec![1, 2, 3, 4, 5, 6, 7, 8];

//     let packed = pack_indices(&indices);

//     assert_eq!(packed.len(), 32);

//     let unpacked = unpack_indices(&packed);

//     assert_eq!(unpacked, indices);
// }

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

//         let world_tree = WorldTree::new(poseidon_tree, TREE_HISTORY_SIZE);

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

//         let world_tree = WorldTree::new(poseidon_tree, TREE_HISTORY_SIZE);

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

//         let world_tree = WorldTree::new(poseidon_tree, TREE_HISTORY_SIZE);

//         for (idx, identity) in identities.iter().enumerate() {
//             ref_tree = ref_tree.update_with_mutation(idx, identity);
//         }

//         world_tree.insert_many_at(0, &identities).await;

//         let deleted_identity_idxs = &[3, 7];
//         let non_deleted_identity_idxs: Vec<_> = (0..NUM_IDENTITIES)
//             .filter(|idx| !deleted_identity_idxs.contains(idx))
//             .collect();

//         for idx in deleted_identity_idxs {
//             ref_tree = ref_tree.update_with_mutation(*idx, &Hash::ZERO);
//         }

//         world_tree.delete_many(deleted_identity_idxs).await;

//         let root = ref_tree.root();

//         for i in non_deleted_identity_idxs {
//             let proof_from_world_tree = world_tree
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
//         let world_tree = WorldTree::new(poseidon_tree, 5);

//         for (idx, identity) in identities.iter().enumerate() {
//             ref_tree = ref_tree.update_with_mutation(idx, identity);
//         }

//         world_tree.insert_many_at(0, &identities).await;

//         assert_eq!(
//             world_tree.tree_history.read().await.len(),
//             NUM_IDENTITIES,
//             "We should have {NUM_IDENTITIES} before GC"
//         );

//         world_tree.gc().await;

//         assert_eq!(
//             world_tree.tree_history.read().await.len(),
//             5,
//             "We should have 5 entries in tree history after GC"
//         );

//         let root = ref_tree.root();

//         for i in 0..NUM_IDENTITIES {
//             let proof_from_world_tree = world_tree
//                 .get_inclusion_proof(identities[i], Some(root))
//                 .await
//                 .unwrap();

//             assert_eq!(ref_tree.proof(i), proof_from_world_tree.proof);
//         }
//     }
// }
