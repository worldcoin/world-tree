pub mod block_scanner;
pub mod config;
pub mod error;
pub mod identity_tree;
pub mod service;
pub mod tree_manager;

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ethers::providers::Middleware;
use ethers::types::U256;
use ruint::Uint;
use semaphore::dynamic_merkle_tree::DynamicMerkleTree;
use semaphore::lazy_merkle_tree::LazyMerkleTree;
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::PoseidonHash;
use serde::{Deserialize, Serialize};
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

pub struct WorldTree<M: Middleware + 'static> {
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

        handles.push(self.handle_canonical_updates(leaf_updates_rx));
        handles.push(self.handle_bridged_updates(bridged_root_rx));

        Ok(handles)
    }

    fn handle_canonical_updates(
        &self,
        mut leaf_updates_rx: Receiver<(Root, LeafUpdates)>,
    ) -> JoinHandle<()> {
        let canonical_chain_id = self.canonical_tree_manager.chain_id;
        let identity_tree = self.identity_tree.clone();
        let chain_state = self.chain_state.clone();

        tokio::spawn(async move {
            if let Some((root, leaf_updates)) = leaf_updates_rx.recv().await {
                let mut identity_tree = identity_tree.write().await;
                identity_tree.append_updates(root, leaf_updates);

                // Update the root for the canonical chain
                chain_state.write().await.insert(canonical_chain_id, root);
            }
        })
    }

    fn handle_bridged_updates(
        &self,
        mut bridged_root_rx: Receiver<(u64, Hash)>,
    ) -> JoinHandle<()> {
        let identity_tree = self.identity_tree.clone();
        let chain_state = self.chain_state.clone();

        tokio::spawn(async move {
            loop {
                if let Some((chain_id, bridged_root)) =
                    bridged_root_rx.recv().await
                {
                    // Get the oldest root across all chains
                    let mut chain_state = chain_state.write().await;
                    let oldest_root: (&u64, &Root) = chain_state
                        .iter()
                        .min_by_key(|&(_, v)| v)
                        .expect("No roots in chain state");

                    let mut identity_tree = identity_tree.write().await;
                    // We can use expect here because the root will always be in tree updates before the root is bridged to other chains
                    let root_nonce = identity_tree
                        .roots
                        .get(&bridged_root)
                        .expect("Could not get root update");
                    let new_root = Root {
                        hash: bridged_root,
                        nonce: *root_nonce,
                    };

                    // If the update is for the chain with the oldest root, apply the updates to the tree
                    if chain_id == *oldest_root.0 {
                        identity_tree.apply_updates_to_root(oldest_root.1);
                    }

                    // Update chain state with the new root
                    chain_state.insert(chain_id, new_root);
                }
            }
        })
    }

    async fn get_latest_roots(&self) -> eyre::Result<HashMap<u64, Hash>> {
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
            .collect::<HashMap<u64, Hash>>();

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

        // Get all logs from the canonical tree from the last synced block to the chain tip
        let logs = self.canonical_tree_manager.block_scanner.next().await?;

        if logs.is_empty() {
            return Ok(());
        }

        // Create a roots hashset to easily index roots by hash
        let mut onchain_roots = HashMap::new();
        for (chain_id, root) in self.get_latest_roots().await? {
            let chain_ids = onchain_roots.entry(root).or_insert_with(Vec::new);
            chain_ids.push(chain_id);
        }

        // Iterate through all of the canonical logs until a root on one of the chains is reached. This is the oldest root across all chains
        let mut pivot = 0;
        for log in logs.iter() {
            let root = Hash::from_be_bytes(log.topics[3].0);
            pivot += 1;

            if onchain_roots.contains_key(&root) {
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

            // Initialize the chain_state with the canonical root corresponding to each chain
            let mut chain_state = self.chain_state.write().await;
            for (root, _) in leaf_updates.iter() {
                if let Some(chain_ids) = onchain_roots.get(&root.hash) {
                    for chain_id in chain_ids {
                        chain_state.insert(*chain_id, *root);
                    }
                    identity_tree.roots.insert(root.hash, root.nonce);
                }
            }

            // Flatten the leaf updates and build the canonical tree
            let flattened_leaves = flatten_leaf_updates(leaf_updates)?;
            let leaves = flattened_leaves
                .iter()
                .map(|(idx, hash)| {
                    if hash != &Hash::ZERO {
                        identity_tree.leaves.insert(*hash, idx.into());
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

            let mut chain_state = self.chain_state.write().await;

            // Update the chain state with the last canonical root
            let (last_canonical_root, _) = canonical_updates
                .last_key_value()
                .expect("No canonical updates");

            identity_tree
                .roots
                .insert(last_canonical_root.hash, last_canonical_root.nonce);
            chain_state.insert(
                self.canonical_tree_manager.chain_id,
                *last_canonical_root,
            );

            // Flatten the leaves and build the canonical tree
            let flattened_leaves = flatten_leaf_updates(canonical_updates)?;
            let canonical_leaves = flattened_leaves
                .iter()
                .map(|(idx, hash)| {
                    if hash != &Hash::ZERO {
                        identity_tree.leaves.insert(*hash, idx.into());
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

                // Initialize the chain_state with the canonical root corresponding to each chain
                if let Some(chain_ids) = onchain_roots.get(&root.hash) {
                    for chain_id in chain_ids {
                        chain_state.insert(*chain_id, root);
                    }
                    identity_tree.roots.insert(root.hash, root.nonce);
                }
            }
        }

        Ok(())
    }

    pub async fn inclusion_proof(
        &self,
        identity_commitment: Hash,
        chain_id: Option<ChainId>,
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

macro_rules! primitive_newtype {
    (pub struct $outer:ident($tname:ty)) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            Serialize,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Deserialize,
        )]
        pub struct $outer(pub $tname);

        impl std::fmt::Display for $outer {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl Deref for $outer {
            type Target = $tname;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl DerefMut for $outer {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
        impl From<$tname> for $outer {
            fn from(value: $tname) -> Self {
                $outer(value)
            }
        }

        impl From<$outer> for $tname {
            fn from(value: $outer) -> Self {
                value.0
            }
        }

        impl From<&$outer> for $tname {
            fn from(value: &$outer) -> Self {
                value.0
            }
        }
    };
}

primitive_newtype!(pub struct ChainId(u64));
// Node index to hash, 0 indexed from the root
primitive_newtype!(pub struct NodeIndex(u32));
// Leaf index to hash, 0 indexed from the initial leaf
primitive_newtype!(pub struct LeafIndex(u32));
