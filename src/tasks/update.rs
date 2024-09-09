use std::cmp::Ordering;
use std::sync::Arc;
use std::time::Duration;

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use semaphore::cascading_merkle_tree::CascadingMerkleTree;
use semaphore::generic_storage::GenericStorage;
use semaphore::poseidon_tree::PoseidonHash;

use crate::db::DbMethods;
use crate::tree::error::WorldTreeResult;
use crate::tree::{ChainId, Hash, WorldTree};

pub async fn append_updates(world_tree: Arc<WorldTree>) -> WorldTreeResult<()> {
    let mut handles = FuturesUnordered::new();

    for chain_id in world_tree.chain_ids.iter() {
        let world_tree = world_tree.clone();
        let chain_id = *chain_id;

        handles.push(tokio::spawn(async move {
            append_chain_updates(world_tree, chain_id).await
        }));
    }

    while let Some(result) = handles.next().await {
        result??;
    }

    Ok(())
}

async fn append_chain_updates(
    world_tree: Arc<WorldTree>,
    chain_id: ChainId,
) -> WorldTreeResult<()> {
    let tree = world_tree
        .cache
        .trees
        .get(&chain_id)
        .expect("Missing cache for chain id")
        .clone();

    loop {
        let latest_root = world_tree.db.root_by_chain(chain_id.0).await?;

        // No updates for this chain id yet, no need to update
        let Some(latest_root) = latest_root else {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };

        let mut tree_lock = tree.clone().write_owned().await;

        let current_root = tree_lock.root();

        let updates = world_tree
            .db
            .fetch_updates_between(current_root, latest_root)
            .await?;

        // No new batches, no need to update
        if updates.is_empty() {
            drop(tree_lock);
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        let num_updates = updates.len();
        tracing::info!(%chain_id, num_updates, "Updating tree");

        let start = std::time::Instant::now();

        tokio::task::spawn_blocking(move || {
            apply_updates_to_tree(&mut tree_lock, &updates)?;

            WorldTreeResult::Ok(())
        })
        .await??;

        let elapsed = start.elapsed();
        let elapsed_ms = elapsed.as_millis();

        tracing::info!(%chain_id, ?elapsed, elapsed_ms, prev_root = ?current_root, root = ?latest_root, "Tree updated");
    }
}

/// Periodically fetches the latest common root from the database and realigns the identity tree
pub async fn reallign(world_tree: Arc<WorldTree>) -> WorldTreeResult<()> {
    loop {
        let common = world_tree.db.fetch_latest_common_root().await?;

        // No common root yet, no need to reallign
        let Some(latest_common_root) = common else {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };

        let mut canonical_lock =
            world_tree.cache.canonical.clone().write_owned().await;

        let canonical_tree_root = canonical_lock.root();

        let updates = world_tree
            .db
            .fetch_updates_between(canonical_tree_root, latest_common_root)
            .await?;

        // No new batches, no need to reallign
        if updates.is_empty() {
            drop(canonical_lock);
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        let num_updates = updates.len();
        tracing::info!(num_updates, "Updating canonical tree");

        let start = std::time::Instant::now();

        tokio::task::spawn_blocking(move || {
            apply_updates_to_tree(&mut canonical_lock, &updates)?;

            WorldTreeResult::Ok(())
        })
        .await??;

        let elapsed = start.elapsed();
        let elapsed_ms = elapsed.as_millis();

        tracing::info!(
            ?elapsed,
            elapsed_ms,
            prev_root = ?canonical_tree_root,
            root = ?latest_common_root,
            "Canonical tree updated"
        );
    }
}

fn apply_updates_to_tree<S: GenericStorage<Hash>>(
    tree: &mut CascadingMerkleTree<PoseidonHash, S>,
    updates: &[(u64, Hash)],
) -> WorldTreeResult<()> {
    for (leaf_idx, leaf) in updates {
        let leaf_idx = *leaf_idx as usize;

        match leaf_idx.cmp(&tree.num_leaves()) {
            Ordering::Less => {
                tree.set_leaf(leaf_idx, *leaf);
            }
            Ordering::Equal => {
                tree.push(*leaf)?;
            }
            Ordering::Greater => {
                return Err(
                    eyre::format_err!("Leaf index out of bounds").into()
                );
            }
        }
    }

    Ok(())
}
