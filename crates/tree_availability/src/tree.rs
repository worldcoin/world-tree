use std::collections::VecDeque;
use std::ops::DerefMut;
use std::sync::Arc;
use std::time::Duration;

use axum::middleware;
use ethers::abi::AbiDecode;
use ethers::contract::EthEvent;
use ethers::providers::{FilterWatcher, Middleware, StreamExt};
use ethers::types::{BlockNumber, Filter, Log, Transaction, H160, U256};
use futures::stream::FuturesUnordered;
use semaphore::lazy_merkle_tree::{
    Canonical, Derived, LazyMerkleTree, VersionMarker,
};
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::{PoseidonHash, Proof};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::abi::{
    DeleteIdentitiesCall, RegisterIdentitiesCall, TreeChangedFilter,
};
use crate::error::TreeAvailabilityError;
use crate::server::InclusionProof;

pub type PoseidonTree<Version> = LazyMerkleTree<PoseidonHash, Version>;
pub type Hash = <PoseidonHash as Hasher>::Hash;

const STREAM_INTERVAL: Duration = Duration::from_secs(5);

/// An abstraction over a tree with a history of changes
///
/// In our data model the `tree` is the oldest available tree.
/// The entires in `tree_history` represent new additions to the tree.
pub struct WorldTree<M: Middleware> {
    pub tree_history_size: usize,
    pub tree: RwLock<PoseidonTree<Canonical>>,
    // TODO: This is an inefficient representation
    //       we should keep a list of structs where each struct has an associated root
    //       that is equal to the root of the last update
    //       and contains a list of updates
    //       that way we can remove from the history entires associated with actual on-chain roots
    pub tree_history: RwLock<VecDeque<(TreeUpdate, PoseidonTree<Derived>)>>,
    pub address: H160,
    pub latest_synced_block: u64, //TODO: revisit if we want to make this atomic
    pub middleware: Arc<M>,
}

pub struct TreeUpdate {
    pub index: usize,
    pub value: Hash,
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
            tree_history_size,
            tree: RwLock::new(tree),
            tree_history: RwLock::new(VecDeque::new()),
            address,
            latest_synced_block: creation_block,
            middleware,
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

    pub fn listen_for_updates(
        &self,
    ) -> (
        tokio::sync::mpsc::Receiver<Log>,
        JoinHandle<Result<(), TreeAvailabilityError<M>>>,
    ) {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Log>(100);

        let filter = Filter::new()
            .address(self.address)
            .topic0(TreeChangedFilter::signature());
        let middleware = self.middleware.clone();

        let handle = tokio::spawn(async move {
            let mut stream = middleware
                .watch(&filter)
                .await
                .map_err(TreeAvailabilityError::MiddlewareError)?
                .interval(STREAM_INTERVAL)
                .stream();

            while let Some(log) = stream.next().await {
                tx.send(log).await?;
            }

            Ok(())
        });

        (rx, handle)
    }

    // Sync the state of the tree to to the chain head
    pub async fn sync_to_head(&self) -> Result<(), TreeAvailabilityError<M>> {
        //TODO: FIXME: we will likely need to step through this and not query such a big block range
        let filter = Filter::new()
            .address(self.address)
            .topic0(TreeChangedFilter::signature())
            .from_block(self.latest_synced_block);

        dbg!("Fetching logs");
        let logs = self
            .middleware
            .get_logs(&filter)
            .await
            .map_err(TreeAvailabilityError::MiddlewareError)?;

        let mut futures = FuturesUnordered::new();
        //TODO: update this to use a throttle that can be set by the user
        for logs in logs.chunks(20) {
            for log in logs {
                futures.push(self.middleware.get_transaction(
                    log.transaction_hash.ok_or(
                        TreeAvailabilityError::TransactionHashNotFound,
                    )?,
                ));
            }

            while let Some(transaction) = futures.next().await {
                let transaction = transaction
                    .map_err(TreeAvailabilityError::MiddlewareError)?
                    .ok_or(TreeAvailabilityError::TransactionNotFound)?;

                dbg!("syncing from tx");
                self.sync_from_transaction(transaction).await?;
                dbg!("finished from tx");
            }

            //TODO: use a better throttle
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        Ok(())
    }

    pub async fn sync_from_log(
        &self,
        log: Log,
    ) -> Result<(), TreeAvailabilityError<M>> {
        let tx_hash = log
            .transaction_hash
            .ok_or(TreeAvailabilityError::TransactionHashNotFound)?;

        let transaction = self
            .middleware
            .get_transaction(tx_hash)
            .await
            .map_err(TreeAvailabilityError::MiddlewareError)?
            .ok_or(TreeAvailabilityError::MissingTransaction)?;

        self.sync_from_transaction(transaction).await?;

        Ok(())
    }

    pub async fn sync_from_transaction(
        &self,
        transaction: Transaction,
    ) -> Result<(), TreeAvailabilityError<M>> {
        // //TODO: FIXME: This check will ensure that we don't process the same log twice, however if there were ever to be two TreeChanged
        // // events in the same block, this logic would ignore the second log. This should be updated.
        // if log_block_number < self.latest_synced_block {
        //     return Ok(());
        // } else {
        //     //TODO: only update if the block number is greater than the latest synced block, this still needs to be fixed though
        //     //TODO: update this to use an atomic
        //     // self.latest_synced_block = log_block_number;
        // }

        dbg!(transaction.block_number.unwrap());

        if let Ok(delete_identities_call) =
            DeleteIdentitiesCall::decode(transaction.input.as_ref())
        {
            let indices = unpack_indices(
                delete_identities_call.packed_deletion_indices.as_ref(),
            );
            let indices: Vec<_> =
                indices.into_iter().map(|x| x as usize).collect();

            self.delete_many(&indices).await;
        } else if let Ok(register_identities_call) =
            RegisterIdentitiesCall::decode(transaction.input.as_ref())
        {
            let start_index = register_identities_call.start_index;
            let identities = register_identities_call.identity_commitments;
            let identities: Vec<_> = identities
                .into_iter()
                .map(|u256: U256| Hash::from_limbs(u256.0))
                .collect();

            self.insert_many_at(start_index as usize, &identities).await;
        } else {
            return Err(TreeAvailabilityError::UnrecognizedTransaction);
        }

        Ok(())
    }
}

pub fn pack_indices(indices: &[u32]) -> Vec<u8> {
    let mut packed = Vec::with_capacity(indices.len() * 4);

    for index in indices {
        packed.extend_from_slice(&index.to_be_bytes());
    }

    packed
}

pub fn unpack_indices(packed: &[u8]) -> Vec<u32> {
    let mut indices = Vec::with_capacity(packed.len() / 4);

    for packed_index in packed.chunks_exact(4) {
        let index = u32::from_be_bytes(
            packed_index.try_into().expect("Invalid index length"),
        );

        indices.push(index);
    }

    indices
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
