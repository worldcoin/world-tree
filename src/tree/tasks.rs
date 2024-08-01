use std::sync::Arc;

use semaphore::generic_storage::MmapVec;
use tokio::sync::mpsc::Receiver;
use tokio::sync::RwLock;

use crate::tree::ChainId;

use super::error::WorldTreeResult;
use super::identity_tree::IdentityTree;
use super::tree_manager::CanonicalChainUpdate;
use super::Hash;

pub async fn handle_canonical_updates(
    canonical_chain_id: u64,
    identity_tree: Arc<RwLock<IdentityTree<MmapVec<Hash>>>>,
    mut leaf_updates_rx: Receiver<CanonicalChainUpdate>,
) -> WorldTreeResult<()> {
    loop {
        if let Some(update) = leaf_updates_rx.recv().await {
            tracing::info!(
                new_root = ?update.post_root,
                "Leaf updates received, appending tree updates"
            );
            let mut identity_tree = identity_tree.write().await;

            identity_tree.append_updates(
                update.pre_root,
                update.post_root,
                update.leaf_updates,
            )?;
            identity_tree
                .update_chain(canonical_chain_id.into(), update.post_root)?;
            identity_tree.update_trees()?;

            let canonical_tree = identity_tree.trees.get(&ChainId(canonical_chain_id)).unwrap();
            if canonical_tree.root() != update.post_root {
                tracing::error!(
                    "Canonical tree root mismatch: expected {}, got {}",
                    canonical_tree.root(),
                    update.post_root
                );

                panic!("Canonical tree root mismatch");
            }

            identity_tree.realign_trees()?;
        }
    }
}

pub async fn handle_bridged_updates(
    identity_tree: Arc<RwLock<IdentityTree<MmapVec<Hash>>>>,
    mut bridged_root_rx: Receiver<(u64, Hash)>,
) -> WorldTreeResult<()> {
    loop {
        if let Some((chain_id, bridged_root)) = bridged_root_rx.recv().await {
            tracing::info!(?chain_id, root = ?bridged_root, "Bridged root received");

            let mut identity_tree = identity_tree.write().await;

            identity_tree.update_chain(chain_id.into(), bridged_root)?;
            identity_tree.update_trees()?;
            identity_tree.realign_trees()?;
        }
    }
}
