use std::collections::HashMap;
use std::sync::Arc;

use ethers::providers::Middleware;
use semaphore::generic_storage::MmapVec;
use tokio::sync::mpsc::Receiver;
use tokio::sync::{broadcast, RwLock};

use super::error::WorldTreeError;
use super::identity_tree::IdentityTree;
use super::tree_manager::CanonicalChainUpdate;
use super::Hash;

pub async fn handle_canonical_updates<M>(
    canonical_chain_id: u64,
    identity_tree: Arc<RwLock<IdentityTree<MmapVec<Hash>>>>,
    chain_state: Arc<RwLock<HashMap<u64, Hash>>>,
    mut leaf_updates_rx: Receiver<CanonicalChainUpdate>,
    mut cancel_rx: broadcast::Receiver<()>,
) -> Result<(), WorldTreeError<M>>
where
    M: Middleware + 'static,
{
    loop {
        let update = tokio::select! {
            res = leaf_updates_rx.recv() => {
                match res {
                    Some(update) => update,
                    None => break,
                }
            }
            _ = cancel_rx.recv() => {
                break
            }
        };

        tracing::info!(
            new_root = ?update.post_root,
            "Leaf updates received, appending tree updates"
        );
        let mut identity_tree = identity_tree.write().await;
        let mut chain_state = chain_state.write().await;

        identity_tree.append_updates(
            update.pre_root,
            update.post_root,
            update.leaf_updates,
        )?;

        // Update the root for the canonical chain
        chain_state.insert(canonical_chain_id, update.post_root);
    }

    Err(WorldTreeError::LeafChannelClosed)
}

pub async fn handle_bridged_updates<M>(
    identity_tree: Arc<RwLock<IdentityTree<MmapVec<Hash>>>>,
    chain_state: Arc<RwLock<HashMap<u64, Hash>>>,
    mut bridged_root_rx: Receiver<(u64, Hash)>,
    mut cancel_rx: broadcast::Receiver<()>,
) -> Result<(), WorldTreeError<M>>
where
    M: Middleware + 'static,
{
    loop {
        let (chain_id, bridged_root) = tokio::select! {
            res = bridged_root_rx.recv() => {
                match res {
                    Some((chain_id, bridged_root)) => (chain_id, bridged_root),
                    None => break,
                }
            }
            _ = cancel_rx.recv() => {
                break
            }
        };

        tracing::info!(?chain_id, root = ?bridged_root, "Bridged root received");

        let mut identity_tree = identity_tree.write().await;
        let mut chain_state = chain_state.write().await;

        // Update chain state with the new root
        chain_state.insert(chain_id, bridged_root);

        let greatest_common_root =
            chain_state.values().min().expect("No roots in chain state");

        // If the current tree root is less than the greatest common root, apply updates up to the common root across all chains
        if identity_tree.tree.root() < *greatest_common_root {
            tracing::info!(
                ?greatest_common_root,
                "Applying updates to the canonical tree"
            );

            // Apply updates up to the common root
            identity_tree.apply_updates_to_root(greatest_common_root);
        }
    }

    Err(WorldTreeError::BridgedRootChannelClosed)
}

/// Realigns the trees across all chains to the greatest common root
async fn reallign_trees(
    identity_tree: &mut IdentityTree<MmapVec<Hash>>,
    chain_state: &mut HashMap<u64, Hash>,
) {
}
