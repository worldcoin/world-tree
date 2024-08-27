use std::sync::Arc;
use std::time::Duration;

use crate::db::DbMethods;
use crate::tree::error::WorldTreeResult;
use crate::tree::identity_tree::{LeafUpdates, Leaves};
use crate::tree::{LeafIndex, WorldTree};

/// Periodically fetches the next batch of updates from the database and appends them to the identity tree
pub async fn append_updates(world_tree: Arc<WorldTree>) -> WorldTreeResult<()> {
    loop {
        let latest_root = world_tree.identity_tree.read().await.latest_root();

        let next_updates =
            world_tree.db.fetch_next_updates(latest_root).await?;

        if next_updates.is_empty() {
            // TODO: Configureable & maybe listen/notify?
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }

        let mut identity_tree = world_tree.identity_tree.write().await;

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

pub async fn reallign(world_tree: Arc<WorldTree>) -> WorldTreeResult<()> {
    loop {
        let latest_common_root =
            world_tree.db.fetch_latest_common_root().await?;
        let latest_root = world_tree.identity_tree.read().await.latest_root();

        if latest_root == latest_common_root {
            // TODO: Configureable & maybe listen/notify?
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }

        let mut identity_tree = world_tree.identity_tree.write().await;
        identity_tree.apply_updates_to_root(&latest_common_root);
    }
}
