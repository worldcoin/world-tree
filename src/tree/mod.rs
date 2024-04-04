pub mod block_scanner;
pub mod config;
pub mod error;
pub mod identity_tree;
pub mod service;
pub mod tree_manager;

use crate::error::Log as _;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ethers::providers::Middleware;
use ethers::types::U256;
use ruint::Uint;
use semaphore::dynamic_merkle_tree::DynamicMerkleTree;
use semaphore::lazy_merkle_tree::LazyMerkleTree;
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::sync::mpsc::Receiver;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::instrument;

use self::identity_tree::{IdentityTree, InclusionProof, LeafUpdates, Root};
use self::tree_manager::{
    extract_identity_updates, BridgedTree, CanonicalTree, TreeManager,
};
use crate::abi::IBridgedWorldID;
use crate::tree::identity_tree::flatten_leaf_updates;

pub type PoseidonTree<Version> = LazyMerkleTree<PoseidonHash, Version>;
pub type Hash = <PoseidonHash as Hasher>::Hash;

pub struct WorldTree<M: Middleware> {
    pub identity_tree: Arc<RwLock<IdentityTree>>,
    pub canonical_tree_manager: TreeManager<M, CanonicalTree>,
    pub bridged_tree_manager: Vec<TreeManager<M, BridgedTree>>,
    pub chain_state: Arc<RwLock<HashMap<u64, Root>>>,
    pub synced: AtomicBool,
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
            synced: AtomicBool::new(false),
        }
    }

    pub async fn spawn(&self) -> eyre::Result<Vec<JoinHandle<()>>> {
        let start_time = Instant::now();

        tracing::info!("Syncing to head");
        self.sync_to_head().await?;
        tracing::info!("Synced to head in {:?} seconds", start_time.elapsed());

        let (leaf_updates_tx, leaf_updates_rx) =
            tokio::sync::mpsc::channel(100);
        let (bridged_root_tx, bridged_root_rx) =
            tokio::sync::mpsc::channel(100);

        // Spawn the tree managers for the canonical and bridged trees
        let mut handles = vec![];
        handles.push(self.canonical_tree_manager.spawn(leaf_updates_tx));

        //TODO: handle if there are no bridged roots to spawn, otherwise the channel will close
        for bridged_tree in self.bridged_tree_manager.iter() {
            handles.push(bridged_tree.spawn(bridged_root_tx.clone()));
        }

        handles
            .push(self.handle_tree_updates(leaf_updates_rx, bridged_root_rx));

        Ok(handles)
    }

    fn handle_tree_updates(
        &self,
        mut leaf_updates_rx: Receiver<(Root, LeafUpdates)>,
        mut bridged_root_rx: Receiver<(u64, Hash)>,
    ) -> JoinHandle<()> {
        let identity_tree = self.identity_tree.clone();
        let chain_state = self.chain_state.clone();
        let canonical_chain_id = self.canonical_tree_manager.chain_id;

        tokio::spawn(async move {
            loop {
                async {
                    tokio::select! {
                        leaf_updates = leaf_updates_rx.recv() => {
                            if let Some((root, leaf_updates)) = leaf_updates{
                                let mut identity_tree = identity_tree.write().await;

                                identity_tree.append_updates(root, leaf_updates);

                                // Update the root for the canonical chain
                                chain_state.write().await.insert(canonical_chain_id, root);
                            }
                        }

                        bridged_root = bridged_root_rx.recv() => {
                            if let Some((chain_id, bridged_root)) = bridged_root{
                                // Get the oldest root across all chains
                                let mut chain_state = chain_state.write().await;
                                let oldest_root: (&u64, &Root) = chain_state
                                    .iter()
                                    .min_by_key(|&(_, v)| v)
                                    .expect("No roots in chain state");

                                // If the update is for the chain with the oldest root, apply the updates to the tree
                                if chain_id == *oldest_root.0 {
                                    let mut identity_tree = identity_tree.write().await;
                                    identity_tree.apply_updates_to_root(oldest_root.1)?;
                                }

                                let  identity_tree = identity_tree.read().await;
                                // We can use expect here because the root will always be in tree updates before the root is bridged to other chains
                                let new_root = identity_tree.get_root_by_hash(&bridged_root).expect("Could not get root update");

                                // Update chain state with the new root
                                chain_state.insert(chain_id, *new_root);
                            }
                        }
                    }
                    Ok(())
                }.await.log();
            }
        })
    }

    async fn get_latest_roots(&self) -> eyre::Result<HashMap<u64, Root>> {
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
                tracing::info!(?chain_id, ?hash, "Latest root");

                let root = Root {
                    hash,
                    // We can set the nonce to 0 here
                    nonce: 0,
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
            .map(|(_, root)| *root)
            .collect::<HashSet<_>>();

        // Get all logs from the canonical tree from the last synced block to the chain tip
        let logs = self.canonical_tree_manager.block_scanner.next().await?;

        if logs.is_empty() {
            return Ok(());
        }

        // Iterate through all of the canonical logs until a root on one of the chains is reached. This is the oldest root across all chains
        let mut pivot = 0;
        for log in logs.iter() {
            // We can set post root start index to 0 since the Hash implementation of Root only evaluates the `root` field
            let post_root = Root {
                hash: Hash::from_le_bytes(log.topics[3].0),
                nonce: 0,
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
        tracing::info!("Extracting identity updates from logs");
        if pivot == logs.len() {
            let leaf_updates =
                extract_identity_updates(&logs, canonical_middleware).await?;

            let flattened_leaves = flatten_leaf_updates(leaf_updates)?;
            let leaves = flattened_leaves
                .iter()
                .map(|(idx, hash)| {
                    if hash != &Hash::ZERO {
                        identity_tree.leaves.insert(*hash, *idx);
                    } else {
                        identity_tree.leaves.remove(hash);
                    }

                    *hash
                })
                .collect::<Vec<_>>();

            tracing::info!(num_leaves = ?leaves.len(), "Building the canonical tree");
            let tree = DynamicMerkleTree::new_with_leaves(
                (),
                identity_tree.tree.depth(),
                &Hash::ZERO,
                &leaves,
            );

            identity_tree.tree = tree;
        } else {
            // Split the logs into canonical and pending logs
            let (canonical_logs, pending_logs) = logs.split_at(pivot);
            let canonical_updates = extract_identity_updates(
                canonical_logs,
                canonical_middleware.clone(),
            )
            .await?;

            let flattened_leaves = flatten_leaf_updates(canonical_updates)?;
            let canonical_leaves = flattened_leaves
                .iter()
                .map(|(idx, hash)| {
                    if hash != &Hash::ZERO {
                        identity_tree.leaves.insert(*hash, *idx);
                    } else {
                        identity_tree.leaves.remove(hash);
                    }

                    *hash
                })
                .collect::<Vec<_>>();

            tracing::info!(num_leaves = ?canonical_leaves.len(), "Building the canonical tree");
            let tree = DynamicMerkleTree::new_with_leaves(
                (),
                identity_tree.tree.depth(),
                &Hash::ZERO,
                &canonical_leaves,
            );

            identity_tree.tree = tree;

            tracing::info!("Extracting pending identity updates from logs");
            let pending_updates =
                extract_identity_updates(pending_logs, canonical_middleware)
                    .await?;

            for (root, leaves) in pending_updates {
                identity_tree.append_updates(root, leaves);
            }
        }

        Ok(())
    }

    pub async fn inclusion_proof(
        &self,
        identity_commitment: Hash,
        chain_id: Option<u64>,
    ) -> eyre::Result<Option<InclusionProof>> {
        let chain_state = self.chain_state.read().await;

        let root = if let Some(chain_id) = chain_id {
            chain_state.get(&chain_id)
        } else {
            None
        };

        let inclusion_proof = self
            .identity_tree
            .read()
            .await
            .inclusion_proof(identity_commitment, root)?;

        Ok(inclusion_proof)
    }
}
