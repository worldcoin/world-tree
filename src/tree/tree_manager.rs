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
use super::error::WorldTreeError;
use super::identity_tree::LeafUpdates;
use super::{Hash, LeafIndex};
use crate::abi::{
    DeleteIdentitiesCall, RegisterIdentitiesCall, RootAddedFilter,
    TreeChangedFilter,
};
use crate::error::{ok, Log as _};

pub const BLOCK_SCANNER_SLEEP_TIME: u64 = 5;

pub trait TreeVersion: Default {
    type ChannelData;

    fn spawn<M: Middleware + 'static>(
        tx: Sender<Self::ChannelData>,
        block_scanner: Arc<BlockScanner<M>>,
    ) -> JoinHandle<Result<(), WorldTreeError<M>>>;

    fn tree_changed_signature() -> H256;
}

pub struct TreeManager<M: Middleware + 'static, T: TreeVersion> {
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
    ) -> Result<Self, WorldTreeError<M>> {
        let chain_id = middleware
            .get_chainid()
            .await
            .map_err(WorldTreeError::MiddlewareError)?
            .as_u64();

        let filter = Filter::new()
            .address(address)
            .topic0(ValueOrArray::Value(T::tree_changed_signature()));

        let block_scanner = Arc::new(
            BlockScanner::new(
                middleware,
                window_size,
                last_synced_block,
                filter,
            )
            .await
            .map_err(WorldTreeError::MiddlewareError)?,
        );

        Ok(Self {
            address,
            block_scanner,
            chain_id,
            _tree_version: PhantomData,
        })
    }

    pub fn spawn(
        &self,
        tx: Sender<T::ChannelData>,
    ) -> JoinHandle<Result<(), WorldTreeError<M>>> {
        T::spawn(tx, self.block_scanner.clone())
    }
}

#[derive(Default)]
pub struct CanonicalTree;

#[derive(Debug, Clone)]
pub struct CanonicalChainUpdate {
    pub pre_root: Hash,
    pub post_root: Hash,
    pub leaf_updates: LeafUpdates,
}

impl TreeVersion for CanonicalTree {
    type ChannelData = CanonicalChainUpdate;

    fn spawn<M: Middleware + 'static>(
        tx: Sender<Self::ChannelData>,
        block_scanner: Arc<BlockScanner<M>>,
    ) -> JoinHandle<Result<(), WorldTreeError<M>>> {
        tokio::spawn(async move {
            let chain_id = block_scanner
                .middleware
                .get_chainid()
                .await
                .map_err(WorldTreeError::MiddlewareError)?
                .as_u64();

            tracing::info!(?chain_id, "Starting canonical tree manager");

            loop {
                async {
                    let (num_blocks, logs) = block_scanner.next().await?;

                    if num_blocks == 0 {
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
                        let new_root = &update.post_root;
                        tracing::info!(?chain_id, ?new_root, "Root updated");
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
    ) -> JoinHandle<Result<(), WorldTreeError<M>>> {
        tokio::spawn(async move {
            let chain_id = block_scanner
                .middleware
                .get_chainid()
                .await
                .map_err(WorldTreeError::MiddlewareError)?
                .as_u64();

            tracing::info!(?chain_id, "Starting bridged tree manager");

            loop {
                async {
                    let (num_blocks, logs) = block_scanner.next().await?;
                    if num_blocks == 0 {
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

/// Extract identity updates from logs emitted by the `WorldIdIdentityManager`.
pub async fn extract_identity_updates<M: Middleware + 'static>(
    logs: &[Log],
    middleware: Arc<M>,
) -> Result<Vec<CanonicalChainUpdate>, WorldTreeError<M>> {
    let mut tree_updates = Vec::new();

    let mut tasks = FuturesUnordered::new();

    // Fetch the transactions for each log concurrently
    for log in logs {
        let tx_hash = log
            .transaction_hash
            .ok_or(WorldTreeError::TransactionHashNotFound)?;

        tracing::debug!(?tx_hash, "Getting transaction");
        tasks.push(middleware.get_transaction(tx_hash));
    }

    let mut sorted_transactions = BTreeMap::new();

    // Sort the transactions by nonce. These should be in order due to the block scanner, but we sort them for redundancy in the case of out-of-order logs.
    while let Some(transaction) = tasks.next().await {
        let transaction = transaction
            .map_err(WorldTreeError::MiddlewareError)?
            .ok_or(WorldTreeError::TransactionNotFound)?;

        let tx_hash = transaction.hash;
        tracing::debug!(?tx_hash, "Transaction received");
        sorted_transactions.insert(transaction.nonce, transaction);
    }

    // Process each transaction, constructing identity updates for each root
    for (_nonce, transaction) in sorted_transactions {
        let calldata = &transaction.input;

        let mut identity_updates: HashMap<LeafIndex, Hash> = HashMap::new();

        let function_selector = Selector::try_from(&calldata[0..4])
            .map_err(|_| WorldTreeError::MissingFunctionSelector)?;

        if function_selector == RegisterIdentitiesCall::selector() {
            tracing::debug!("Decoding registerIdentities calldata");

            let register_identities_call =
                RegisterIdentitiesCall::decode(calldata.as_ref())?;

            let start_index = register_identities_call.start_index;
            let identities = register_identities_call.identity_commitments;

            for (i, identity) in identities
                .into_iter()
                .take_while(|x| *x != U256::zero())
                .enumerate()
            {
                identity_updates.insert(
                    (start_index + i as u32).into(),
                    Hash::from_limbs(identity.0),
                );
            }

            let pre_root =
                Hash::from_limbs(register_identities_call.pre_root.0);
            let post_root =
                Hash::from_limbs(register_identities_call.post_root.0);

            tracing::debug!(?pre_root, ?post_root, "Canonical tree updated");
            tree_updates.push(CanonicalChainUpdate {
                pre_root,
                post_root,
                leaf_updates: LeafUpdates::Insert(identity_updates),
            });
        } else if function_selector == DeleteIdentitiesCall::selector() {
            tracing::debug!("Decoding deleteIdentities calldata");

            let delete_identities_call =
                DeleteIdentitiesCall::decode(calldata.as_ref())?;

            let indices = unpack_indices(
                delete_identities_call.packed_deletion_indices.as_ref(),
            );

            // Note that we use 2**30 as padding for deletions in order to fill the deletion batch size
            for i in indices.into_iter().take_while(|x| *x < 2_u32.pow(30)) {
                identity_updates.insert(i.into(), Hash::ZERO);
            }

            let pre_root = Hash::from_limbs(delete_identities_call.pre_root.0);
            let post_root =
                Hash::from_limbs(delete_identities_call.post_root.0);

            tracing::debug!(?pre_root, ?post_root, "Canonical tree updated");
            tree_updates.push(CanonicalChainUpdate {
                pre_root,
                post_root,
                leaf_updates: LeafUpdates::Delete(identity_updates),
            });
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
