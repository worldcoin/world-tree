pub mod block_scanner;
pub mod config;
pub mod error;
pub mod identity_tree;
pub mod service;
pub mod tree_manager;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::Context;
use ethers::providers::{Middleware, MiddlewareError};
use ethers::types::{Log, Selector, H160, U256};
use metrics::GaugeFn;
use ruint::Uint;
use semaphore::dynamic_merkle_tree::DynamicMerkleTree;
use semaphore::lazy_merkle_tree::LazyMerkleTree;
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::{PoseidonHash, Proof};
use semaphore::Field;
use serde::{Deserialize, Deserializer, Serialize};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::instrument;

use self::identity_tree::{
    flatten_updates, IdentityTree, InclusionProof, LeafUpdates, Root,
};
use self::tree_manager::{
    extract_identity_updates, BridgedTree, CanonicalTree, TreeManager,
};
use crate::abi::IBridgedWorldID;

pub type PoseidonTree<Version> = LazyMerkleTree<PoseidonHash, Version>;
pub type Hash = <PoseidonHash as Hasher>::Hash;

pub struct WorldTree<M: Middleware> {
    pub identity_tree: Arc<RwLock<IdentityTree>>,
    pub canonical_tree_manager: TreeManager<M, CanonicalTree>,
    pub bridged_tree_manager: Vec<TreeManager<M, BridgedTree>>,
    pub chain_state: Arc<RwLock<HashMap<u64, Root>>>,
}

impl<M> WorldTree<M>
where
    M: Middleware + 'static,
{
    pub fn new(
        tree_depth: usize,
        canonical_tree_manager: TreeManager<M, CanonicalTree>,
        bridged_tree_manager: Vec<TreeManager<M, BridgedTree>>,
    ) -> Self {
        let identity_tree = IdentityTree::new(tree_depth);

        Self {
            identity_tree: Arc::new(RwLock::new(identity_tree)),
            canonical_tree_manager,
            bridged_tree_manager,
            chain_state: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn spawn(
        &self,
    ) -> eyre::Result<Vec<JoinHandle<eyre::Result<()>>>> {
        //TODO: sync tree from cache
        self.sync_to_head().await?;

        let (leaf_updates_tx, leaf_updates_rx) =
            tokio::sync::mpsc::channel(100);
        let (root_tx, root_rx) = tokio::sync::mpsc::channel(100);

        // Spawn the tree managers for the canonical and bridged trees
        let mut handles = vec![];
        handles.push(self.canonical_tree_manager.spawn(leaf_updates_tx));

        for bridged_tree in self.bridged_tree_manager.iter() {
            handles.push(bridged_tree.spawn(root_tx.clone()));
        }

        handles.push(self.handle_tree_updates(leaf_updates_rx, root_rx));

        Ok(handles)
    }

    fn handle_tree_updates(
        &self,
        mut leaf_updates_rx: Receiver<(Root, LeafUpdates)>,
        mut root_rx: Receiver<(u64, Root)>,
    ) -> JoinHandle<eyre::Result<()>> {
        let identity_tree = self.identity_tree.clone();
        let chain_state = self.chain_state.clone();
        let canonical_chain_id = self.canonical_tree_manager.chain_id;

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    leaf_updates = leaf_updates_rx.recv() => {
                        let mut identity_tree = identity_tree.write().await;

                        let (root, updates) = leaf_updates.expect("TODO: handle this case");

                       //FIXME: update to be more succinct
                        let first_update = updates.iter().take(1).next().expect("TODO: handle this case");

                            if *first_update.1 == Hash::ZERO {
                                let leaves = updates.iter().map(|(_, leaf)| *leaf).collect::<Vec<_>>();
                                identity_tree.remove_many_leaves(&leaves);
                            }else{
                                let leaves = updates.iter().map(|(idx, leaf)| (*idx as usize, *leaf)).collect::<Vec<_>>();
                                identity_tree.insert_many_leaves(&leaves);
                            }

                            identity_tree.append_updates(updates);

                            // Update the root for the canonical chain
                            chain_state.write().await.insert(canonical_chain_id, root);
                        }


                    bridged_root = root_rx.recv() => {
                        let (chain_id, new_root) = bridged_root.expect("TODO: handle this case");

                        // Get the oldest root across all chains
                        //FIXME: we need to compare roots by block number or timestamp on the canonical chain, not across chains
                        let mut chain_state = chain_state.write().await;
                        let oldest_root: (&u64, &Root) = chain_state
                            .iter()
                            .min_by_key(|&(_, v)| v)
                            .expect("TODO: handle case ");

                        // If the update is for the chain with the oldest root, apply the updates to the tree
                        if chain_id == *oldest_root.0 {
                            let mut identity_tree = identity_tree.write().await;
                            identity_tree.apply_updates_to_root(oldest_root.1.hash)?;
                        }

                          // Insert the new root and recalculate the oldest root
                          chain_state.insert(chain_id, new_root);
                    }
                }
            }
        })
    }

    pub async fn get_latest_roots(&self) -> eyre::Result<HashMap<u64, Root>> {
        let mut tree_data = vec![];

        for bridged_tree in self.bridged_tree_manager.iter() {
            let bridged_world_id = IBridgedWorldID::new(
                bridged_tree.address,
                bridged_tree.block_scanner.middleware.clone(),
            );

            tree_data.push((bridged_tree.chain_id, bridged_world_id));
        }

        tree_data.push((
            self.canonical_tree_manager.chain_id,
            IBridgedWorldID::new(
                self.canonical_tree_manager.address,
                self.canonical_tree_manager.block_scanner.middleware.clone(),
            ),
        ));

        let futures = tree_data.iter().map(|(chain_id, contract)| async move {
            let root: U256 = contract.latest_root().await?;

            eyre::Result::<_, eyre::Report>::Ok((
                *chain_id,
                Uint::<256, 4>::from_limbs(root.0),
            ))
        });

        let roots = futures::future::try_join_all(futures)
            .await?
            .into_iter()
            .map(|(chain_id, hash)| {
                let root = Root {
                    hash,
                    block_number: 0, //TODO: ensure its ok to set 0 here initially
                };

                (chain_id, root)
            })
            .collect::<HashMap<u64, Root>>();

        Ok(roots)
    }

    #[instrument(skip(self))]
    pub async fn sync_to_head(&self) -> eyre::Result<()> {
        // Update the last synced block for each bridged tree to the current block
        for bridged_tree in self.bridged_tree_manager.iter() {
            let current_block = bridged_tree
                .block_scanner
                .middleware
                .get_block_number()
                .await?
                .as_u64();

            bridged_tree
                .block_scanner
                .last_synced_block
                .store(current_block, Ordering::SeqCst);
        }

        // Update the latest root for all chains
        let mut chain_state = self.chain_state.write().await;
        *chain_state = self.get_latest_roots().await?;

        let roots = chain_state
            .iter()
            .map(|(_, root)| root.clone())
            .collect::<HashSet<_>>();

        // Get all logs from the canonical tree from the last synced block to the chain tip
        let logs = self.canonical_tree_manager.block_scanner.next().await?;

        if logs.is_empty() {
            return Ok(());
        }

        // Iterate through all of the canonical logs until a root on one of the chains is reached. This is the oldest root across all chains
        let mut pivot = 0;
        for log in logs.iter() {
            // We can set post root block number to 0 since the Hash implementation of Root only evaluates the `root` field
            let post_root = Root {
                hash: Hash::from_le_bytes(log.topics[3].0),
                block_number: 0,
            };

            pivot += 1;

            if roots.contains(&post_root) {
                break;
            }
        }

        let canonical_middleware =
            self.canonical_tree_manager.block_scanner.middleware.clone();

        let mut identity_tree = self.identity_tree.write().await;

        // Split the logs into canonical and pending. All canonical logs will be applied directly to the tree, while pending logs will be stored in the tree_updates map
        if pivot == logs.len() {
            let identity_updates =
                extract_identity_updates(&logs, canonical_middleware).await?;

            let flattened_updates = flatten_updates(&identity_updates, None)?
                .into_iter()
                .map(|(idx, hash)| (idx as usize, *hash))
                .collect::<Vec<_>>();

            identity_tree.insert_many(&flattened_updates);
        } else {
            // Split the logs into canonical and pending logs
            let (canonical_logs, pending_logs) = logs.split_at(pivot);
            let canonical_updates = extract_identity_updates(
                &canonical_logs,
                canonical_middleware.clone(),
            )
            .await?;
            let flattened_canonical_updates =
                flatten_updates(&canonical_updates, None)?
                    .into_iter()
                    .map(|(idx, hash)| (idx as usize, *hash))
                    .collect::<Vec<_>>();

            identity_tree.insert_many(&flattened_canonical_updates);

            let pending_updates =
                extract_identity_updates(&pending_logs, canonical_middleware)
                    .await?;

            let flattened_pending_updates =
                flatten_updates(&pending_updates, None)?
                    .into_iter()
                    .map(|(idx, hash)| (idx as usize, *hash))
                    .collect::<Vec<_>>();

            identity_tree.insert_many_leaves(&flattened_pending_updates);

            identity_tree.tree_updates.extend(pending_updates);
        }

        Ok(())
    }

    pub async fn inclusion_proof(
        &self,
        identity_commitment: Hash,
        chain_id: u64,
    ) -> eyre::Result<Option<InclusionProof>> {
        let identity_tree = self.identity_tree.read().await;
        let chain_state = self.chain_state.read().await;

        let target_root =
            chain_state.get(&chain_id).expect("TODO: handle this case");

        identity_tree.inclusion_proof(identity_commitment, Some(target_root))
    }
}
