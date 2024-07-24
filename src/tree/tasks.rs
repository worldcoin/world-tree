use std::collections::HashMap;
use std::sync::Arc;

use ethers::providers::Middleware;
use semaphore::generic_storage::MmapVec;
use tokio::sync::mpsc::Receiver;
use tokio::sync::RwLock;

use super::error::WorldTreeError;
use super::identity_tree::IdentityTree;
use super::tree_manager::CanonicalChainUpdate;
use super::Hash;

pub async fn handle_canonical_updates<M>(
    canonical_chain_id: u64,
    identity_tree: Arc<RwLock<IdentityTree<MmapVec<Hash>>>>,
    chain_state: Arc<RwLock<HashMap<u64, Hash>>>,
    mut leaf_updates_rx: Receiver<CanonicalChainUpdate>,
) -> Result<(), WorldTreeError<M>>
where
    M: Middleware + 'static,
{
    loop {
        if let Some(update) = leaf_updates_rx.recv().await {
            tracing::info!(
                new_root = ?update.post_root,
                "Leaf updates received, appending tree updates"
            );
            let mut chain_state = chain_state.write().await;
            let mut identity_tree = identity_tree.write().await;

            identity_tree.append_updates(
                update.pre_root,
                update.post_root,
                update.leaf_updates,
            )?;

            // Update the root for the canonical chain
            chain_state.insert(canonical_chain_id, update.post_root);

            // NOTE: In practice reallignment should only happen when we receive events on one
            //       of the bridged networks. However we don't have 100% guarantee that the canonical network
            //       events will always arrive before the bridged network events.
            //       So to maintain liveliness we reallign on every update.
            realign_trees(&mut identity_tree, &mut chain_state).await;
        }
    }
}

pub async fn handle_bridged_updates<M>(
    identity_tree: Arc<RwLock<IdentityTree<MmapVec<Hash>>>>,
    chain_state: Arc<RwLock<HashMap<u64, Hash>>>,
    mut bridged_root_rx: Receiver<(u64, Hash)>,
) -> Result<(), WorldTreeError<M>>
where
    M: Middleware + 'static,
{
    loop {
        if let Some((chain_id, bridged_root)) = bridged_root_rx.recv().await {
            tracing::info!(?chain_id, root = ?bridged_root, "Bridged root received");

            let mut chain_state = chain_state.write().await;
            let mut identity_tree = identity_tree.write().await;

            // Update chain state with the new root
            chain_state.insert(chain_id, bridged_root);

            realign_trees(&mut identity_tree, &mut chain_state).await;
        }
    }
}

/// Realligns all the observed chains.
///
/// This function figures out the root that has been seen across all observed networks
/// And applies all the updates up to that root to the canonical tree.
async fn realign_trees(
    identity_tree: &mut IdentityTree<MmapVec<Hash>>,
    chain_state: &mut HashMap<u64, Hash>,
) {
    let mut chain_state_idxs = vec![];

    for (chain_id, root) in chain_state.iter() {
        let idx = identity_tree.root_map.get(root).copied();

        if let Some(idx) = idx {
            chain_state_idxs.push(idx);
        } else {
            // This can happen if a bridged event arrives before the canonical event
            // this means however that the bridged chain is ahead of the canonical chain
            // and we can realign to the canonical chain root (or root of other bridged networks)
            tracing::warn!(?chain_id, ?root, "Root not found in identity tree");
        }
    }

    let Some(latest_common_root_idx) = chain_state_idxs.iter().min().copied()
    else {
        // If we don't find any common roots then there's nothing to reallign
        return;
    };

    let latest_common_root = identity_tree.roots[latest_common_root_idx];

    // Apply updates up to the greatest common root
    identity_tree.apply_updates_to_root(&latest_common_root);

    tracing::info!(?latest_common_root, "Trees realligned");
}
