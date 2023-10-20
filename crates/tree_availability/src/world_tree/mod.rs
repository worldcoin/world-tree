pub mod block_scanner;
pub mod tree_data;
pub mod tree_updater;

use std::collections::VecDeque;
use std::ops::DerefMut;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ethers::providers::{FilterWatcher, Middleware, StreamExt};
use ethers::types::{BlockNumber, Filter, Log, Transaction, H160, U256};
use semaphore::lazy_merkle_tree::{
    Canonical, Derived, LazyMerkleTree, VersionMarker,
};
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::task::JoinHandle;

use self::tree_data::TreeData;
use self::tree_updater::TreeUpdater;
use crate::error::TreeAvailabilityError;

pub type PoseidonTree<Version> = LazyMerkleTree<PoseidonHash, Version>;
pub type Hash = <PoseidonHash as Hasher>::Hash;

const STREAM_INTERVAL: Duration = Duration::from_secs(5);

/// An abstraction over a tree with a history of changes
///
/// In our data model the `tree` is the oldest available tree.
/// The entires in `tree_history` represent new additions to the tree.
pub struct WorldTree<M: Middleware> {
    pub tree_data: Arc<TreeData>,
    pub tree_updater: Arc<TreeUpdater<M>>,
}

impl<M: Middleware> WorldTree<M> {
    pub fn new(
        tree: PoseidonTree<Canonical>,
        tree_history_size: usize,
        address: H160,
        creation_block: u64,
        middleware: Arc<M>,
    ) -> Self {
        Self {
            tree_data: Arc::new(TreeData::new(tree, tree_history_size)),
            tree_updater: Arc::new(TreeUpdater::new(
                address,
                creation_block,
                middleware,
            )),
        }
    }

    pub async fn spawn(
        &self,
    ) -> JoinHandle<Result<(), TreeAvailabilityError<M>>> {
        let tree_data = self.tree_data.clone();
        let tree_updater = self.tree_updater.clone();
        tokio::spawn(async move {
            tree_updater.sync_to_head(&tree_data).await?;
            tree_updater.synced.store(true, Ordering::Relaxed);

            loop {
                tree_updater.sync_to_head(&tree_data).await?;

                // Sleep a little to unblock the executor
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        })
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;

//     const TREE_DEPTH: usize = 10;
//     const NUM_IDENTITIES: usize = 10;

//     const TREE_HISTORY_SIZE: usize = 10;

//     #[test]
//     fn test_pack_indices() {
//         let indices = vec![1, 2, 3, 4, 5, 6, 7, 8];

//         let packed = pack_indices(&indices);

//         assert_eq!(packed.len(), 32);

//         let unpacked = unpack_indices(&packed);

//         assert_eq!(unpacked, indices);
//     }

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
