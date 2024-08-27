use std::collections::HashMap;
use std::sync::Arc;

use semaphore::generic_storage::MmapVec;
use tokio::sync::mpsc::Receiver;
use tokio::sync::RwLock;

use super::error::WorldTreeResult;
use super::identity_tree::IdentityTree;
use super::Hash;

pub mod update;
pub mod ingest;
pub mod observe;

// pub async fn handle_canonical_updates(
//     canonical_chain_id: u64,
//     identity_tree: Arc<RwLock<IdentityTree<MmapVec<Hash>>>>,
//     chain_state: Arc<RwLock<HashMap<u64, Hash>>>,
//     mut leaf_updates_rx: Receiver<CanonicalChainUpdate>,
// ) -> WorldTreeResult<()> {
//     loop {
//         if let Some(update) = leaf_updates_rx.recv().await {
//             tracing::info!(
//                 new_root = ?update.post_root,
//                 "Leaf updates received, appending tree updates"
//             );
//             let mut chain_state = chain_state.write().await;
//             let mut identity_tree = identity_tree.write().await;

//             identity_tree.append_updates(
//                 update.pre_root,
//                 update.post_root,
//                 update.leaf_updates,
//             )?;

//             // Update the root for the canonical chain
//             chain_state.insert(canonical_chain_id, update.post_root);

//             // NOTE: In practice reallignment should only happen when we receive events on one
//             //       of the bridged networks. However we don't have 100% guarantee that the canonical network
//             //       events will always arrive before the bridged network events.
//             //       So to maintain liveliness we reallign on every update.
//             realign_trees(&mut identity_tree, &mut chain_state).await;
//         }
//     }
// }

// pub async fn handle_bridged_updates(
//     identity_tree: Arc<RwLock<IdentityTree<MmapVec<Hash>>>>,
//     chain_state: Arc<RwLock<HashMap<u64, Hash>>>,
//     mut bridged_root_rx: Receiver<BridgeTreeUpdate>,
// ) -> WorldTreeResult<()> {
//     loop {
//         if let Some(BridgeTreeUpdate {
//             chain_id,
//             root: bridged_root,
//             ..
//         }) = bridged_root_rx.recv().await
//         {
//             tracing::info!(?chain_id, root = ?bridged_root, "Bridged root received");

//             let mut chain_state = chain_state.write().await;
//             let mut identity_tree = identity_tree.write().await;

//             // Update chain state with the new root
//             chain_state.insert(chain_id, bridged_root);

//             realign_trees(&mut identity_tree, &mut chain_state).await;
//         }
//     }
// }

/// Realligns all the observed chains.
///
/// This function figures out the root that has been seen across all observed networks
/// And applies all the updates up to that root to the canonical tree.
async fn realign_trees(
    identity_tree: &mut IdentityTree<MmapVec<Hash>>,
    chain_state: &mut HashMap<u64, Hash>,
) {
    // let latest_common_root = identity_tree.roots[latest_common_root_idx];
    let latest_common_root = todo!();

    // Apply updates up to the greatest common root
    identity_tree.apply_updates_to_root(&latest_common_root);

    tracing::info!(?latest_common_root, "Trees realligned");
}
