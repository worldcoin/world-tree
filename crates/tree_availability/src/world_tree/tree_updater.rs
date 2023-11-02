use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use std::time::Duration;

use ethers::abi::AbiDecode;
use ethers::contract::{EthCall, EthEvent};
use ethers::providers::{Middleware, StreamExt};
use ethers::types::{Selector, Transaction, H160, U256};
use futures::stream::FuturesOrdered;

use super::abi::{
    DeleteIdentitiesCall, RegisterIdentitiesCall, TreeChangedFilter,
};
use super::block_scanner::BlockScanner;
use super::tree_data::TreeData;
use crate::error::TreeAvailabilityError;
use crate::world_tree::Hash;

// TODO: Change to a configurable parameter
const SCANNING_WINDOW_SIZE: u64 = 100;

/// Manages the synchronization of the World Tree with it's onchain representation.
pub struct TreeUpdater<M: Middleware> {
    /// Contract address of the `WorldIDIdentityManager`.
    pub address: H160,
    /// Latest block that has been synced.
    pub latest_synced_block: AtomicU64,
    /// Scanner responsible for fetching logs and parsing calldata to decode tree updates.
    block_scanner: BlockScanner<Arc<M>>,
    /// Provider to interact with Ethereum.
    pub middleware: Arc<M>,
}

impl<M: Middleware> TreeUpdater<M> {
    /// Initializes TreeUpdater
    ///
    /// `address`: `WorldIDIdentityManager` contract address
    ///
    /// `creation_block`: The block height of the `WorldIDIdentityManager` contract deployment
    ///
    /// `middleware`: provider
    pub fn new(address: H160, creation_block: u64, middleware: Arc<M>) -> Self {
        Self {
            address,
            latest_synced_block: AtomicU64::new(creation_block),
            block_scanner: BlockScanner::new(
                middleware.clone(),
                SCANNING_WINDOW_SIZE,
                creation_block,
            ),
            middleware,
        }
    }

    /// Updates the in-memory tree to reflect the latest state of the onchain tree.
    ///
    /// # Arguments
    ///
    /// * `tree_data` - Instance of `TreeData` maintaining the current state of the tree and tree history.
    pub async fn sync_to_head(
        &self,
        tree_data: &TreeData,
    ) -> Result<(), TreeAvailabilityError<M>> {
        let logs = self
            .block_scanner
            .next(
                Some(self.address.into()),
                [
                    Some(TreeChangedFilter::signature().into()),
                    None,
                    None,
                    None,
                ],
            )
            .await
            .map_err(TreeAvailabilityError::MiddlewareError)?;

        if logs.is_empty() {
            return Ok(());
        }

        let mut futures = FuturesOrdered::new();

        //TODO: update this to use a throttle that can be set by the user
        for logs in logs.chunks(20) {
            for log in logs {
                futures.push_back(self.middleware.get_transaction(
                    log.transaction_hash.ok_or(
                        TreeAvailabilityError::TransactionHashNotFound,
                    )?,
                ));
            }

            while let Some(transaction) = futures.next().await {
                let transaction = transaction
                    .map_err(TreeAvailabilityError::MiddlewareError)?
                    .ok_or(TreeAvailabilityError::TransactionNotFound)?;

                self.sync_from_transaction(tree_data, &transaction).await?;
            }

            //TODO: use a better throttle
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        Ok(())
    }

    /// Updates the in-memory tree based transaction calldata.
    ///
    /// # Arguments
    ///
    /// * `tree_data` - Instance of `TreeData` maintaining the current state of the tree and tree history.
    /// * `transaction` - Transaction containing the calldata necessary to update the local tree.
    pub async fn sync_from_transaction(
        &self,
        tree_data: &TreeData,
        transaction: &Transaction,
    ) -> Result<(), TreeAvailabilityError<M>> {
        let calldata = &transaction.input;

        let function_selector = Selector::try_from(&calldata[0..4])
            .expect("Transaction data does not contain a function selector");

        if function_selector == RegisterIdentitiesCall::selector() {
            let register_identities_call =
                RegisterIdentitiesCall::decode(calldata.as_ref())?;

            let start_index = register_identities_call.start_index;
            let identities = register_identities_call.identity_commitments;
            let identities: Vec<_> = identities
                .into_iter()
                .map(|u256: U256| Hash::from_limbs(u256.0))
                .collect();

            metrics::increment_counter!(
                "tree_availability.tree_updater.insertion"
            );

            tree_data
                .insert_many_at(start_index as usize, &identities)
                .await;
        } else if function_selector == DeleteIdentitiesCall::selector() {
            let delete_identities_call =
                DeleteIdentitiesCall::decode(calldata.as_ref())?;

            let indices = unpack_indices(
                delete_identities_call.packed_deletion_indices.as_ref(),
            );
            let indices: Vec<_> =
                indices.into_iter().map(|x| x as usize).collect();

            metrics::increment_counter!(
                "tree_availability.tree_updater.deletion"
            );

            tree_data.delete_many(&indices).await;
        } else {
            return Err(TreeAvailabilityError::UnrecognizedFunctionSelector);
        }

        Ok(())
    }
}

/// Packs an array of 32-bit indices into a contiguous byte vector.
///
/// # Arguments
///
/// * `indices` - The array of indices representing positions of identity commitments in the `WorldTree`.
pub fn pack_indices(indices: &[u32]) -> Vec<u8> {
    let mut packed = Vec::with_capacity(indices.len() * 4);

    for index in indices {
        packed.extend_from_slice(&index.to_be_bytes());
    }

    packed
}

/// Unpacks a contiguous byte array into a vector of 32-bit indices.
///
/// # Arguments
///
/// * `packed` - The packed byte array containing positions of identity commitments in the `WorldTree`.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_indices() {
        let indices = vec![1, 2, 3, 4, 5, 6, 7, 8];

        let packed = pack_indices(&indices);

        assert_eq!(packed.len(), 32);

        let unpacked = unpack_indices(&packed);

        assert_eq!(unpacked, indices);
    }
}
