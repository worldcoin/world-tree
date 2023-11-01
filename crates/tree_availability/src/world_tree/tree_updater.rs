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
use crate::world_tree::abi::DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall;
use crate::world_tree::{abi, Hash};

pub struct TreeUpdater<M: Middleware> {
    pub address: H160,
    pub latest_synced_block: AtomicU64,
    pub synced: AtomicBool,
    block_scanner: BlockScanner<Arc<M>>,
    pub middleware: Arc<M>,
}

impl<M: Middleware> TreeUpdater<M> {
    pub fn new(address: H160, creation_block: u64, window_size: u64, middleware: Arc<M>) -> Self {
        Self {
            address,
            latest_synced_block: AtomicU64::new(creation_block),
            synced: AtomicBool::new(false),
            block_scanner: BlockScanner::new(
                middleware.clone(),
                window_size,
                creation_block,
            ),
            middleware,
        }
    }

    // Sync the state of the tree to to the chain head
    pub async fn sync_to_head(
        &self,
        tree_data: &TreeData,
    ) -> Result<(), TreeAvailabilityError<M>> {
        tracing::info!("Syncing tree to chain head");

        

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
            tracing::info!("No `TreeChanged` events found within block range");
            return Ok(());
        }

        let mut futures = FuturesOrdered::new();

        //TODO: update this to use a throttle that can be set by the user, however this is likely only going to result in one log per query
        for logs in logs.chunks(20) {
            for log in logs {
                let tx_hash = log
                    .transaction_hash
                    .ok_or(TreeAvailabilityError::TransactionHashNotFound)?;

                tracing::info!("Getting transaction for {tx_hash}");

                futures.push_back(self.middleware.get_transaction(tx_hash));
            }

            while let Some(transaction) = futures.next().await {
                let transaction = transaction
                    .map_err(TreeAvailabilityError::MiddlewareError)?
                    .ok_or(TreeAvailabilityError::TransactionNotFound)?;

                self.sync_from_transaction(tree_data, &transaction).await?;
            }

            //TODO: use a better throttle
            // tokio::time::sleep(Duration::from_secs(1)).await;
        }

        Ok(())
    }

    pub async fn sync_from_transaction(
        &self,
        tree_data: &TreeData,
        transaction: &Transaction,
    ) -> Result<(), TreeAvailabilityError<M>> {
        tracing::info!("Syncing from transaction {:?}", transaction.hash);

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

            tree_data
                .insert_many_at(start_index as usize, &identities)
                .await;
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
            
            tree_data.delete_many(&indices).await;

        } else if function_selector == DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall::selector() {

            tracing::info!("Decoding deleteIdentities calldata");

            // @dev This is a type that is generated by abigen!() since there is a function defined with a conflicting function name but different params
            let delete_identities_call =
            DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall::decode(calldata.as_ref())?;

            let indices= unpack_indices(
                delete_identities_call.packed_deletion_indices.as_ref(),
            );

            let indices: Vec<usize> = indices
                .into_iter().take_while(|x| *x != 2_u32.pow(tree_data.depth as u32))
                .map(|x| x as usize)
                .collect();
        
            tree_data.delete_many(&indices).await;


        } else {
            return Err(TreeAvailabilityError::UnrecognizedFunctionSelector);
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
