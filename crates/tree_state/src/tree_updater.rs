use std::sync::Arc;

use ethers::contract::EthEvent;
use ethers::providers::Middleware;
use ethers::types::{Filter, H160};

use crate::abi::TreeChangedFilter;
use crate::error::TreeAvailabilityError;

pub struct TreeUpdater<M: Middleware> {
    pub middleware: Arc<M>,
    pub last_synced_block: u64,
    pub address: H160,
}

impl<M: Middleware> TreeUpdater<M> {
    pub fn new(middleware: Arc<M>, last_synced_block: u64, address: H160) -> Self {
        Self {
            middleware,
            last_synced_block,
            address,
        }
    }

    // Sync the state of the tree to to the chain head
    pub async fn sync_to_head(&self) -> Result<(), TreeAvailabilityError<M>> {
        let current_block = self
            .middleware
            .get_block_number()
            .await
            .map_err(TreeAvailabilityError::MiddlewareError)?;

        let topic = TreeChangedFilter::signature();

        // Initialize a new filter to get all of the tree changed events
        let filter = Filter::new()
            .topic0(topic)
            .address(self.address)
            .from_block(self.last_synced_block)
            .to_block(current_block.as_u64());

        let logs = self
            .middleware
            .get_logs(&filter)
            .await
            .map_err(TreeAvailabilityError::MiddlewareError)?;

        for log in logs {
            if let Some(tx_hash) = log.transaction_hash {
                let Some(transaction) = self
                    .middleware
                    .get_transaction(tx_hash)
                    .await
                    .map_err(TreeAvailabilityError::MiddlewareError)? else{

                        todo!("Return an error here")
                    };

                //TODO: decode the tx data depending on if it is an insertion or deletion, we can use the same functionality from the sequencer

                //TODO: for each batch of changes, add the changes to the tree history and update the tree state
            }
        }

        Ok(())
    }
}
