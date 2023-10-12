use std::sync::Arc;

use ethers::contract::EthEvent;
use ethers::providers::Middleware;
use ethers::types::H160;

use crate::abi::TreeChangedFilter;
use crate::block_scanner::BlockScanner;
use crate::error::TreeAvailabilityError;

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
    pub async fn sync_to_head(&self) -> Result<(), TreeAvailabilityError<M>> {
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
