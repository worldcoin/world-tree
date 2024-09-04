use std::sync::Arc;
use std::time::Duration;

use futures::stream::FuturesUnordered;
use futures::StreamExt;

use crate::db::DbMethods;
use crate::tree::error::WorldTreeResult;
use crate::tree::identity_tree::{LeafUpdates, Leaves};
use crate::tree::{ChainId, LeafIndex, WorldTree};

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
        .expect("Missing cache for chain id");

    loop {
        let latest_root = world_tree.db.root_by_chain(chain_id.0).await?;

        // No updates for this chain id yet, no need to update
        let Some(latest_root) = latest_root else {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };

        let mut tree_lock = tree.write().await;

        let next_updates = world_tree
            .db
            .fetch_updates_between(tree_lock.root(), latest_root)
            .await?;

        // No new batches, no need to update
        if next_updates.is_empty() {
            drop(tree_lock);
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        // TODO: Optimize to use extend_from_slice
        for (leaf_idx, update) in next_updates {
            tree_lock.set_leaf(leaf_idx as usize, update);
        }
    }
}

/// Periodically fetches the latest common root from the database and realigns the identity tree
pub async fn reallign(world_tree: Arc<WorldTree>) -> WorldTreeResult<()> {
    loop {
        let latest_cached_canonical_root =
            world_tree.cache.canonical.read().await.root();

        let common = world_tree.db.fetch_latest_common_root().await?;

        // No common root yet, no need to reallign
        let Some(latest_common_root) = common else {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };

        let mut canonical_lock = world_tree.cache.canonical.write().await;

        let updates = world_tree
            .db
            .fetch_updates_between(
                latest_cached_canonical_root,
                latest_common_root,
            )
            .await?;

        // No new batches, no need to reallign
        if updates.is_empty() {
            drop(canonical_lock);
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        for (leaf_idx, update) in updates {
            canonical_lock.set_leaf(leaf_idx as usize, update);
        }
    }
}
