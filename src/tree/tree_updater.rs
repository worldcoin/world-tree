use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use ethers::abi::AbiDecode;
use ethers::contract::{EthCall, EthEvent};
use ethers::providers::{Middleware, StreamExt};
use ethers::types::{Filter, Selector, Transaction, ValueOrArray, H160, U256};
use futures::stream::FuturesOrdered;
use tokio::sync::RwLock;
use tracing::instrument;

use super::block_scanner::BlockScanner;
use super::error::TreeAvailabilityError;
use super::tree_data::TreeData;
use crate::abi::{
    DeleteIdentitiesCall,
    DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall,
    RegisterIdentitiesCall, TreeChangedFilter,
};
use crate::tree::Hash;

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
    pub fn new(
        address: H160,
        creation_block: u64,
        window_size: u64,
        middleware: Arc<M>,
    ) -> Self {
        let filter = Filter::new()
            .address(address)
            .topic0(ValueOrArray::Value(TreeChangedFilter::signature()));

        Self {
            address,
            latest_synced_block: AtomicU64::new(creation_block),
            block_scanner: BlockScanner::new(
                middleware.clone(),
                window_size,
                creation_block,
                filter,
            ),
            middleware,
        }
    }

    /// Updates the in-memory tree to reflect the latest state of the onchain tree.
    ///
    /// # Arguments
    ///
    /// * `tree_data` - Instance of `TreeData` maintaining the current state of the tree and tree history.
    #[instrument(skip(self, tree_data))]
    pub async fn sync_to_head(
        &self,
        tree_data: &RwLock<TreeData>,
    ) -> Result<(), TreeAvailabilityError<M>> {
        tracing::info!("Syncing tree to chain head");

        let logs = self
            .block_scanner
            .next()
            .await
            .map_err(TreeAvailabilityError::MiddlewareError)?;

        if logs.is_empty() {
            tracing::info!("No `TreeChanged` events found within block range");
            return Ok(());
        }

        let mut tree_data = tree_data.write().await;
        let mut futures = FuturesOrdered::new();

        for log in logs {
            let tx_hash = log
                .transaction_hash
                .ok_or(TreeAvailabilityError::TransactionHashNotFound)?;

            tracing::info!(?tx_hash, "Getting transaction");

            futures.push_back(self.middleware.get_transaction(tx_hash));
        }

        while let Some(transaction) = futures.next().await {
            let transaction = transaction
                .map_err(TreeAvailabilityError::MiddlewareError)?
                .ok_or(TreeAvailabilityError::TransactionNotFound)?;

            self.sync_from_transaction(&mut tree_data, &transaction)
                .await?;
        }

        Ok(())
    }

    /// Updates the in-memory tree based transaction calldata.
    ///
    /// # Arguments
    ///
    /// * `tree_data` - Instance of `TreeData` maintaining the current state of the tree and tree history.
    /// * `transaction` - Transaction containing the calldata necessary to update the local tree.
    #[instrument(skip(self, tree_data, transaction))]
    pub async fn sync_from_transaction(
        &self,
        tree_data: &mut TreeData,
        transaction: &Transaction,
    ) -> Result<(), TreeAvailabilityError<M>> {
        let tx_hash = transaction.hash;
        tracing::info!(?tx_hash, "Syncing from transaction");

        let calldata = &transaction.input;

        let function_selector = Selector::try_from(&calldata[0..4])
            .expect("Transaction data does not contain a function selector");

        if function_selector == RegisterIdentitiesCall::selector() {
            tracing::info!("Decoding registerIdentities calldata");

            let register_identities_call =
                RegisterIdentitiesCall::decode(calldata.as_ref())?;

            let start_index = register_identities_call.start_index;
            let identities = register_identities_call.identity_commitments;

            let identities: Vec<Hash> = identities
                .into_iter().take_while(|x| *x != U256::zero())
                .map(|u256: U256| Hash::from_limbs(u256.0))
                .collect();

            metrics::increment_counter!(
                "tree_availability.tree_updater.insertion"
            );

            tree_data
                .insert_many_at(start_index as usize, &identities);
        } else if function_selector == DeleteIdentitiesCall::selector() {
            tracing::info!("Decoding deleteIdentities calldata");

            let delete_identities_call =
                DeleteIdentitiesCall::decode(calldata.as_ref())?;

            let indices= unpack_indices(
                delete_identities_call.packed_deletion_indices.as_ref(),
            );

            let indices: Vec<usize> = indices
                .into_iter().take_while(|x| *x != 2_u32.pow(tree_data.depth as u32))
                .map(|x| x as usize)
                .collect();

            metrics::increment_counter!(
                "tree_availability.tree_updater.deletion"
            );
            tree_data.delete_many(&indices);

        } else if function_selector == DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall::selector() {

            tracing::info!("Decoding deleteIdentities calldata");

            // @dev This is a type that is generated by abigen!() since there is a function defined with a conflicting function name but different params
            let delete_identities_call =
            DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall::decode(calldata.as_ref())?;
            let indices= unpack_indices(
                delete_identities_call.packed_deletion_indices.as_ref(),
            );

            metrics::increment_counter!(
                "tree_availability.tree_updater.deletion"
            );

            let indices: Vec<usize> = indices
                .into_iter().take_while(|x| *x != 2_u32.pow(tree_data.depth as u32))
                .map(|x| x as usize)
                .collect();
            tree_data.delete_many(&indices);


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
