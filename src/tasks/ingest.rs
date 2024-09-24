//! Batch ingestion logic
//! this module defines the task responsible for fetching
//! batch events from the canonical chain and
//! ingesting them into the database

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;

use alloy::primitives::{TxHash, U256};
use alloy::providers::Provider;
use alloy::rpc::types::{Filter, Log, Transaction};
use alloy::sol_types::{SolCall, SolEvent};
use alloy::transports::Transport;
use eyre::ContextCompat;
use futures::{StreamExt, TryStreamExt};
use tokio::pin;

use crate::abi::IWorldIDIdentityManager::{
    deleteIdentitiesCall, registerIdentitiesCall, TreeChanged,
};
use crate::db::DbMethods;
use crate::tree::block_scanner::BlockScanner;
use crate::tree::error::{WorldTreeError, WorldTreeResult};
use crate::tree::leaf_updates::{LeafUpdates, Leaves};
use crate::tree::{Hash, LeafIndex, WorldTree};

/// Ingests the canonical logs from the mainnet contract
/// Saves the ingested data into the database
pub async fn ingest_canonical(
    world_tree: Arc<WorldTree>,
) -> WorldTreeResult<()> {
    let provider = world_tree.canonical_provider().await?;
    let chain_id = provider.get_chain_id().await?;

    let latest_block_number =
        world_tree.db.fetch_latest_block_number(chain_id).await?;

    let latest_block_number = latest_block_number
        .map(|x| x + 1)
        .unwrap_or(world_tree.config.canonical_tree.creation_block);

    let filter = Filter::new()
        .address(world_tree.config.canonical_tree.address)
        .event_signature(TreeChanged::SIGNATURE_HASH);

    let scanner = BlockScanner::new(
        provider.clone(),
        world_tree.config.canonical_tree.provider.window_size,
        latest_block_number,
        filter,
    )
    .await?;

    tracing::info!(chain_id, latest_block_number, "Starting ingestion");

    // TODO: Make buffer size configurable?
    let stream = scanner.block_stream().buffered(10);
    pin!(stream);

    while let Some(log_batch) = stream.next().await {
        let logs: Vec<Log> = log_batch?;

        let updates = extract_identity_updates(logs, provider.clone()).await?;

        for update in updates {
            tracing::info!(
                ?update.pre_root,
                ?update.post_root,
                "Processing canonical update"
            );

            let mut tx = world_tree.db.begin().await?;

            // 1. Insert tx metadata
            let tx_id = tx
                .insert_tx(chain_id, update.block_number, update.tx_hash)
                .await?;

            tracing::debug!("Inserted tx metadata");

            // 2. Insert canonical updates data (pre & post root)
            let update_id = tx
                .insert_update(update.pre_root, update.post_root, tx_id)
                .await?;

            tracing::debug!("Inserted update data");

            // 3. Insert leaf updates
            // Fetch the last update id
            let start_update_id = if let Some(last_leaf_update_id) =
                tx.get_last_leaf_update_id().await?
            {
                last_leaf_update_id + 1
            } else {
                0
            };

            tracing::debug!(?start_update_id, "Inserting leaf updates");

            let leaf_updates: Leaves = update.leaf_updates.into();
            let mut leaf_updates: Vec<_> = leaf_updates.into_iter().collect();
            leaf_updates.sort_by_key(|x| x.0);
            let leaf_updates: Vec<_> = leaf_updates
                .into_iter()
                .map(|(leaf_idx, value)| (leaf_idx.0 as u64, value))
                .collect();

            tx.insert_leaf_updates(start_update_id, &leaf_updates)
                .await?;

            tracing::debug!("Inserted leaf updates");

            // 4. Insert leaf batch
            tx.insert_leaf_batch(
                update_id,
                start_update_id,
                start_update_id + leaf_updates.len() as u64 - 1,
            )
            .await?;

            // 5. Insert root
            tx.insert_root(update.post_root, tx_id).await?;

            tx.commit().await?;

            tracing::info!(chain_id, tx_hash = ?update.tx_hash, root = ?update.post_root, "Inserted root");
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct CanonicalChainUpdate {
    pub pre_root: Hash,
    pub post_root: Hash,
    pub leaf_updates: LeafUpdates,
    pub tx_hash: TxHash,
    pub block_number: u64,
}

/// Extract identity updates from logs emitted by the `WorldIdIdentityManager`.
async fn extract_identity_updates<T, P>(
    logs: Vec<Log>,
    provider: Arc<P>,
) -> WorldTreeResult<Vec<CanonicalChainUpdate>>
where
    T: Transport + Clone,
    P: Provider<T>,
{
    let mut tree_updates = Vec::new();

    // Fetch the transactions for each log concurrently
    // and sort them by block number and transaction index
    let mut txs: Vec<_> = futures::stream::iter(logs)
        .map(|log| {
            let provider = provider.clone();
            async move {
                let tx_hash = log
                    .transaction_hash
                    .ok_or(WorldTreeError::TransactionHashNotFound)?;

                tracing::debug!(?tx_hash, "Getting transaction");

                provider
                    .get_transaction_by_hash(tx_hash)
                    .await
                    .map_err(|e| {
                        WorldTreeError::TransactionSearchError(e.to_string())
                    })?
                    .ok_or(WorldTreeError::TransactionNotFound)
            }
        })
        .buffer_unordered(100)
        .try_collect()
        .await?;

    txs.sort_unstable_by_key(|tx| (tx.block_number, tx.transaction_index));

    // Process each transaction, constructing identity updates for each root
    for transaction in txs {
        if let Some(update) = extract_identity_update(&transaction)? {
            tree_updates.push(update);
        }
    }

    Ok(tree_updates)
}

fn extract_identity_update(
    transaction: &Transaction,
) -> WorldTreeResult<Option<CanonicalChainUpdate>> {
    let calldata = &transaction.input;

    let tx_hash = transaction.hash;
    let block_number =
        transaction.block_number.context("Missing block number")?;

    let mut identity_updates: HashMap<LeafIndex, Hash> = HashMap::new();

    let function_selector: [u8; 4] = calldata
        .len()
        .gt(&3)
        .then(|| {
            let mut selector = [0; 4];
            selector.copy_from_slice(&calldata[0..4]);
            selector
        })
        .ok_or(WorldTreeError::MissingFunctionSelector)?;

    if function_selector == registerIdentitiesCall::SELECTOR {
        tracing::debug!("Decoding registerIdentities calldata");

        let register_identities_call =
            registerIdentitiesCall::abi_decode(calldata.as_ref(), true)?;

        let start_index = register_identities_call.startIndex;
        let identities = register_identities_call.identityCommitments;

        for (i, identity) in identities
            .into_iter()
            .take_while(|x| *x != U256::ZERO)
            .enumerate()
        {
            identity_updates.insert(
                (start_index + i as u32).into(),
                Hash::from_limbs(*identity.as_limbs()),
            );
        }

        let pre_root =
            Hash::from_limbs(*register_identities_call.preRoot.as_limbs());
        let post_root =
            Hash::from_limbs(*register_identities_call.postRoot.as_limbs());

        tracing::debug!(?pre_root, ?post_root, "Canonical tree updated");

        Ok(Some(CanonicalChainUpdate {
            pre_root,
            post_root,
            leaf_updates: LeafUpdates::Insert(identity_updates),
            tx_hash,
            block_number,
        }))
    } else if function_selector == deleteIdentitiesCall::SELECTOR {
        tracing::debug!("Decoding deleteIdentities calldata");

        let delete_identities_call =
            deleteIdentitiesCall::abi_decode(calldata.as_ref(), true)?;

        let indices = unpack_indices(
            delete_identities_call.packedDeletionIndices.as_ref(),
        );

        // Note that we use 2**30 as padding for deletions in order to fill the deletion batch size
        for i in indices.into_iter().take_while(|x| *x < 2_u32.pow(30)) {
            identity_updates.insert(i.into(), Hash::ZERO);
        }

        let pre_root =
            Hash::from_limbs(*delete_identities_call.preRoot.as_limbs());
        let post_root =
            Hash::from_limbs(*delete_identities_call.postRoot.as_limbs());

        tracing::debug!(?pre_root, ?post_root, "Canonical tree updated");

        Ok(Some(CanonicalChainUpdate {
            pre_root,
            post_root,
            leaf_updates: LeafUpdates::Delete(identity_updates),
            tx_hash,
            block_number,
        }))
    } else {
        Ok(None)
    }
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
#[cfg(test)]
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
