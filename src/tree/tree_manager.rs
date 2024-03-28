use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::DerefMut;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;
use std::vec;

use common::test_utilities::abi::{
    DeleteIdentitiesCall, RegisterIdentitiesCall,
};
use ethers::abi::AbiDecode;
use ethers::contract::{ContractError, EthCall, EthEvent};
use ethers::providers::{Middleware, MiddlewareError};
use ethers::types::{
    Filter, Log, Selector, Transaction, ValueOrArray, H160, U256,
};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task::JoinHandle;

use super::block_scanner::{self, BlockScanner};
use super::identity_tree::{LeafUpdates, Root};
use super::Hash;
use crate::abi::{
    DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall,
    TreeChangedFilter,
};

pub const BLOCK_SCANNER_SLEEP_TIME: u64 = 5;

pub trait TreeVersion: Default {
    type ChannelData;

    fn spawn<M: Middleware + 'static>(
        tx: Sender<Self::ChannelData>,
        block_scanner: Arc<BlockScanner<M>>,
    ) -> JoinHandle<eyre::Result<()>>;
}

pub struct TreeManager<M: Middleware, T: TreeVersion> {
    pub address: H160,
    pub block_scanner: Arc<BlockScanner<M>>,
    pub chain_id: u64,
    tree_version: T,
}

impl<M, T> TreeManager<M, T>
where
    M: Middleware + 'static,
    T: TreeVersion,
{
    pub async fn new(
        address: H160,
        window_size: u64,
        last_synced_block: u64,
        middleware: Arc<M>,
    ) -> eyre::Result<Self> {
        let chain_id = middleware.get_chainid().await?.as_u64();

        //FIXME: Update to use RootAdded filter for bridged tree
        let filter = Filter::new()
            .address(address)
            .topic0(ValueOrArray::Value(TreeChangedFilter::signature()));

        let block_scanner = Arc::new(BlockScanner::new(
            middleware,
            window_size,
            last_synced_block,
            filter,
        ));

        Ok(Self {
            address,
            block_scanner,
            chain_id,
            tree_version: T::default(),
        })
    }

    pub fn spawn(
        &self,
        tx: Sender<T::ChannelData>,
    ) -> JoinHandle<eyre::Result<()>> {
        T::spawn(tx, self.block_scanner.clone())
    }
}

#[derive(Default)]
pub struct CanonicalTree;
impl TreeVersion for CanonicalTree {
    type ChannelData = (Root, LeafUpdates);

    fn spawn<M: Middleware + 'static>(
        tx: Sender<Self::ChannelData>,
        block_scanner: Arc<BlockScanner<M>>,
    ) -> JoinHandle<eyre::Result<()>> {
        tokio::spawn(async move {
            loop {
                let logs = block_scanner.next().await.expect("TODO:");

                if logs.is_empty() {
                    tokio::time::sleep(Duration::from_secs(
                        BLOCK_SCANNER_SLEEP_TIME,
                    ))
                    .await;

                    continue;
                }

                let identity_updates = extract_identity_updates(
                    &logs,
                    block_scanner.middleware.clone(),
                )
                .await?;

                for update in identity_updates {
                    tx.send(update).await?;
                }
            }
        })
    }
}

#[derive(Default)]
pub struct BridgedTree;
impl TreeVersion for BridgedTree {
    type ChannelData = (u64, Root);
    fn spawn<M: Middleware + 'static>(
        tx: Sender<Self::ChannelData>,
        block_scanner: Arc<BlockScanner<M>>,
    ) -> JoinHandle<eyre::Result<()>> {
        tokio::spawn(async move {
            let chain_id =
                block_scanner.middleware.get_chainid().await?.as_u64();

            loop {
                let logs = block_scanner.next().await.expect("TODO:");

                if logs.is_empty() {
                    tokio::time::sleep(Duration::from_secs(
                        BLOCK_SCANNER_SLEEP_TIME,
                    ))
                    .await;

                    continue;
                }

                for log in logs {
                    let block_number = log
                        .block_number
                        .expect("TODO: handle this case")
                        .as_u64();

                    //FIXME: It is possible for a newer root to have a smaller block number
                    //FIXME: we also can not use the timestamp in the root added log since it is when the root is received
                    //We need to be able to get the block number or timestamp of when the root was added to the canonical tree
                    let root = Root {
                        hash: Hash::from_le_bytes(log.topics[1].0),
                        block_number,
                    };

                    tx.send((chain_id, root)).await?;
                }
            }
        })
    }
}

pub async fn extract_identity_updates<M: Middleware + 'static>(
    logs: &[Log],
    middleware: Arc<M>,
) -> eyre::Result<BTreeMap<Root, LeafUpdates>> {
    let mut tree_updates = BTreeMap::new();

    //TODO: you can do this concurrently
    for log in logs {
        let block_number =
            log.block_number.expect("TODO: handle this case").as_u64();

        let root = Root {
            hash: Hash::from_le_bytes(log.topics[3].0),
            block_number,
        };

        let mut identity_updates = HashMap::new();

        let tx_hash = log.transaction_hash.expect("TODO: handle this case");

        tracing::info!(?tx_hash, "Getting transaction");

        //TODO: you can get all transactions concurrently
        let transaction = middleware
            .get_transaction(tx_hash)
            .await?
            .expect("TODO: Handle this case");

        let calldata = &transaction.input;

        let function_selector = Selector::try_from(&calldata[0..4])
            .expect("Transaction data does not contain a function selector");

        if function_selector == RegisterIdentitiesCall::selector() {
            tracing::info!("Decoding registerIdentities calldata");

            let register_identities_call =
                RegisterIdentitiesCall::decode(calldata.as_ref())?;

            let start_index = register_identities_call.start_index;
            let identities = register_identities_call.identity_commitments;

            for (i, identity) in identities.into_iter().take_while(|x| *x != U256::zero()).enumerate(){
                identity_updates.insert(start_index +  i as u32, Hash::from_limbs(identity.0));
            }


        } else if function_selector == DeleteIdentitiesCall::selector() {
            tracing::info!("Decoding deleteIdentities calldata");

            let delete_identities_call =
                DeleteIdentitiesCall::decode(calldata.as_ref())?;

            let indices= unpack_indices(
                delete_identities_call.packed_deletion_indices.as_ref(),
            );


            // Note that we use 2**30 as padding for deletions in order to fill the deletion batch size
                for i in indices.into_iter().take_while(|x| *x < 2_u32.pow(30)){
                    identity_updates.insert(  i , Hash::ZERO);
                }
        } else if function_selector == DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall::selector() {

            tracing::info!("Decoding deleteIdentities calldata");

            // @dev This is a type that is generated by abigen!() since there is a function defined with a conflicting function name but different params
            let delete_identities_call =
            DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall::decode(calldata.as_ref())?;
            let indices= unpack_indices(
                delete_identities_call.packed_deletion_indices.as_ref(),
            );


            // Note that we use 2**30 as padding for deletions in order to fill the deletion batch size
            for i in indices.into_iter().take_while(|x| *x < 2_u32.pow(30)){
            identity_updates.insert(  i , Hash::ZERO);
        }
    }

        tree_updates.insert(root, identity_updates);
    }

    Ok(tree_updates)
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
