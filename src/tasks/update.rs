use std::sync::Arc;
use std::time::Duration;

use crate::db::DbMethods;
use crate::tree::error::WorldTreeResult;
use crate::tree::identity_tree::{LeafUpdates, Leaves};
use crate::tree::{LeafIndex, WorldTree};

/// Periodically fetches the next batch of updates from the database and appends them to the identity tree
pub async fn append_updates(world_tree: Arc<WorldTree>) -> WorldTreeResult<()> {
    loop {
        let mut identity_tree = world_tree.identity_tree.write().await;
        let latest_root = identity_tree.latest_root();

        let next_updates =
            world_tree.db.fetch_next_updates(latest_root).await?;

        if next_updates.is_empty() {
            // TODO: Configureable & maybe listen/notify?
            drop(identity_tree);
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        tracing::info!(
            n = next_updates.len(),
            ?latest_root,
            "Fetched next batch of updates"
        );

        let is_deletion_batch =
            next_updates.iter().all(|(_, value)| value.is_zero());

        let leaves: Leaves = next_updates
            .into_iter()
            .map(|(leaf_idx, value)| (LeafIndex::from(leaf_idx as u32), value))
            .collect();

        let updates = if is_deletion_batch {
            LeafUpdates::Delete(leaves)
        } else {
            LeafUpdates::Insert(leaves)
        };

        let post_root = identity_tree.append_updates(latest_root, updates)?;

        tracing::info!(?post_root, "Appended updates to identity tree");
    }
}

/// Periodically fetches the latest common root from the database and realigns the identity tree
pub async fn reallign(world_tree: Arc<WorldTree>) -> WorldTreeResult<()> {
    loop {
        let latest_root = world_tree.identity_tree.read().await.latest_root();

        let latest_root_id = world_tree.db.fetch_root_id(latest_root).await?;

        let Some((latest_common_root_id, mut latest_common_root)) =
            world_tree.db.fetch_latest_common_root().await?
        else {
            tracing::debug!("No latest common root found");
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };

        if let Some(latest_root_id) = latest_root_id {
            if latest_common_root_id > latest_root_id {
                latest_common_root = latest_root;
            }
        }

        let latest_alligned_root =
            world_tree.identity_tree.read().await.tree.root();

        if latest_alligned_root == latest_common_root {
            // TODO: Configureable & maybe listen/notify?
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        tracing::info!(
            ?latest_alligned_root,
            ?latest_common_root,
            "Realigning identity tree"
        );

        let mut identity_tree = world_tree.identity_tree.write().await;

        let now = std::time::Instant::now();
        let num_updates_before = identity_tree.tree_updates.len();

        identity_tree.apply_updates_to_root(&latest_common_root);

        let elapsed = now.elapsed();
        let elapsed_ms = elapsed.as_millis();
        let num_updates_after = identity_tree.tree_updates.len();

        tracing::info!(
            ?latest_common_root,
            ?elapsed,
            elapsed_ms,
            num_updates_before,
            num_updates_after,
            "Identity tree realigned"
        );
    }
}
