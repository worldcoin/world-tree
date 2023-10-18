use std::sync::Arc;

use ethers::abi::AbiDecode;
use ethers::contract::EthEvent;
use ethers::providers::Middleware;
use ethers::types::{H160, U256};

use crate::abi::{
    DeleteIdentitiesCall, RegisterIdentitiesCall, TreeChangedFilter,
};
use crate::block_scanner::BlockScanner;
use crate::error::TreeAvailabilityError;
use crate::index_packing::unpack_indices;
use crate::tree::{Hash, WorldTree};

// TODO: Change to a configurable parameter
const SCANNING_WINDOW_SIZE: u64 = 100;

pub struct TreeUpdater<M: Middleware> {
    pub middleware: Arc<M>,
    pub last_synced_block: u64,
    pub address: H160,
    block_scanner: BlockScanner<Arc<M>>,
}

impl<M: Middleware> TreeUpdater<M> {
    pub fn new(
        middleware: Arc<M>,
        last_synced_block: u64,
        address: H160,
    ) -> Self {
        Self {
            block_scanner: BlockScanner::new(
                middleware.clone(),
                SCANNING_WINDOW_SIZE,
                last_synced_block,
            ),
            middleware,
            last_synced_block,
            address,
        }
    }

    // Sync the state of the tree to to the chain head
    pub async fn sync_to_head(
        &self,
        world_tree: &WorldTree,
    ) -> Result<(), TreeAvailabilityError<M>> {
        let topic = TreeChangedFilter::signature();

        let logs = self
            .block_scanner
            .next(
                Some(self.address.into()),
                [Some(topic.into()), None, None, None],
            )
            .await
            .map_err(TreeAvailabilityError::MiddlewareError)?;

        for log in logs {
            if let Some(tx_hash) = log.transaction_hash {
                let transaction = self
                    .middleware
                    .get_transaction(tx_hash)
                    .await
                    .map_err(TreeAvailabilityError::MiddlewareError)?
                    .ok_or(TreeAvailabilityError::MissingTransaction)?;

                if let Ok(delete_identities_call) =
                    DeleteIdentitiesCall::decode(transaction.input.as_ref())
                {
                    let indices = unpack_indices(
                        delete_identities_call.packed_deletion_indices.as_ref(),
                    );
                    let indices: Vec<_> =
                        indices.into_iter().map(|x| x as usize).collect();

                    world_tree.delete_many(&indices).await;
                } else if let Ok(register_identities_call) =
                    RegisterIdentitiesCall::decode(transaction.input.as_ref())
                {
                    let start_index = register_identities_call.start_index;
                    let identities =
                        register_identities_call.identity_commitments;
                    let identities: Vec<_> = identities
                        .into_iter()
                        .map(|u256: U256| Hash::from_limbs(u256.0))
                        .collect();

                    world_tree
                        .insert_many_at(start_index as usize, &identities)
                        .await;
                } else {
                    return Err(TreeAvailabilityError::UnrecognizedTransaction);
                }
            }
        }

        Ok(())
    }
}
