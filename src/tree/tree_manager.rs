use std::collections::{BTreeMap, HashMap};
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use ethers::abi::{AbiDecode, RawLog};
use ethers::contract::{EthCall, EthEvent};
use ethers::providers::Middleware;
use ethers::types::{Filter, Log, Selector, ValueOrArray, H160, H256, U256};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use super::block_scanner::BlockScanner;
use super::identity_tree::{LeafUpdates, Root};
use super::Hash;
use crate::abi::{
    DeleteIdentitiesCall,
    DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall,
    RegisterIdentitiesCall, RootAddedFilter, TreeChangedFilter,
};
use crate::error::{ok, Log as _};

pub const BLOCK_SCANNER_SLEEP_TIME: u64 = 5;

pub trait TreeVersion: Default {
    type ChannelData;

    fn spawn<M: Middleware + 'static>(
        tx: Sender<Self::ChannelData>,
        block_scanner: Arc<BlockScanner<M>>,
    ) -> JoinHandle<()>;

    fn tree_changed_signature() -> H256;
}

pub struct TreeManager<M: Middleware, T: TreeVersion> {
    pub address: H160,
    pub block_scanner: Arc<BlockScanner<M>>,
    pub chain_id: u64,
    _tree_version: PhantomData<T>,
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

        let filter = Filter::new()
            .address(address)
            .topic0(ValueOrArray::Value(T::tree_changed_signature()));

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
            _tree_version: PhantomData,
        })
    }

    pub fn spawn(&self, tx: Sender<T::ChannelData>) -> JoinHandle<()> {
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
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                async {
                    let logs = block_scanner.next().await?;

                    if logs.is_empty() {
                        tokio::time::sleep(Duration::from_secs(
                            BLOCK_SCANNER_SLEEP_TIME,
                        ))
                        .await;

                        return ok(());
                    }

                    let identity_updates = extract_identity_updates(
                        &logs,
                        block_scanner.middleware.clone(),
                    )
                    .await?;

                    for update in identity_updates {
                        tx.send(update).await?;
                    }
                    ok(())
                }
                .await
                .log();
            }
        })
    }

    fn tree_changed_signature() -> H256 {
        TreeChangedFilter::signature()
    }
}

#[derive(Default)]
pub struct BridgedTree;
impl TreeVersion for BridgedTree {
    type ChannelData = (u64, Hash);
    fn spawn<M: Middleware + 'static>(
        tx: Sender<Self::ChannelData>,
        block_scanner: Arc<BlockScanner<M>>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let chain_id = block_scanner
                .middleware
                .get_chainid()
                .await
                .expect("Failed to get chain_id")
                .as_u64();

            loop {
                async {
                    let logs = block_scanner.next().await?;
                    if logs.is_empty() {
                        tokio::time::sleep(Duration::from_secs(
                            BLOCK_SCANNER_SLEEP_TIME,
                        ))
                        .await;

                        return ok(());
                    }

                    for log in logs {
                        // Extract the root from the RootAdded log
                        let data =
                            RootAddedFilter::decode_log(&RawLog::from(log))?;
                        let new_root = Hash::from_limbs(data.root.0);

                        tracing::info!(?chain_id, ?new_root, "Root updated");
                        tx.send((chain_id, new_root)).await?;
                    }
                    ok(())
                }
                .await
                .log();
            }
        })
    }

    fn tree_changed_signature() -> H256 {
        RootAddedFilter::signature()
    }
}

pub async fn extract_identity_updates<M: Middleware + 'static>(
    logs: &[Log],
    middleware: Arc<M>,
) -> eyre::Result<BTreeMap<Root, LeafUpdates>> {
    let mut tree_updates = BTreeMap::new();

    let mut tasks = FuturesUnordered::new();

    for log in logs {
        let tx_hash = log.transaction_hash.expect("TODO: handle this case");

        tracing::debug!(?tx_hash, "Getting transaction");

        tasks.push(middleware.get_transaction(tx_hash));
    }

    let mut sorted_transactions = BTreeMap::new();

    while let Some(transaction) = tasks.next().await {
        let transaction = transaction?.expect("TODO: handle this case");

        let tx_hash = transaction.hash;
        tracing::debug!(?tx_hash, "Transaction received");
        sorted_transactions.insert(transaction.nonce, transaction);
    }

    for (nonce, transaction) in sorted_transactions {
        let calldata = &transaction.input;

        let mut identity_updates = HashMap::new();

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

            let root = Root { hash: Hash::from_limbs(register_identities_call.post_root.0), nonce: nonce.as_u64() as usize };
         
            tracing::info!(?root, "Canonical tree updated");
            tree_updates.insert(root, LeafUpdates::Insert(identity_updates));

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

            let root = Root { hash: Hash::from_limbs(delete_identities_call.post_root.0), nonce: nonce.as_u64() as usize };
            
            tracing::info!(?root, "Canonical tree updated");
            tree_updates.insert(root, LeafUpdates::Delete(identity_updates));
        } else if function_selector == DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall::selector() {

            tracing::info!("Decoding deleteIdentities calldata");

            // @dev This is a type that is generated by abigen!() since there is a function defined with a conflicting function name but different params
            let delete_identities_call =
            DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall::decode(calldata.as_ref())?;
            let indices= unpack_indices(
                delete_identities_call.packed_deletion_indices.as_ref(),
            );


            // Note that we use 2**30 as padding for deletions in order to fill the deletion batch size
            for i in indices.into_iter().take_while(|x| *x < 2_u32.pow(30)) {
                identity_updates.insert(  i , Hash::ZERO);
            }

            let root = Root { hash: Hash::from_limbs(delete_identities_call.post_root.0), nonce: nonce.as_u64() as usize };
            
            tracing::info!(?root, "Canonical tree updated");
            tree_updates.insert(root, LeafUpdates::Delete(identity_updates));
        }
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
