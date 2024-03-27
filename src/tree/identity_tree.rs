use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use anyhow::Context;
use ethers::providers::{Middleware, MiddlewareError};
use ethers::types::{Log, Selector, H160, U256};
use ruint::Uint;
use semaphore::dynamic_merkle_tree::DynamicMerkleTree;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::sync::mpsc::Sender;
use tracing::instrument;

use super::block_scanner::BlockScanner;
use super::tree_manager::{
    BridgedTree, CanonicalTree, TreeManager, TreeVersion,
};
use super::Hash;
use crate::abi::IBridgedWorldID;
use crate::tree::tree_manager::extract_identity_updates;

pub type IdentityUpdates = HashMap<u32, Hash>;

#[derive(PartialEq, PartialOrd, Eq, Clone, Copy)]
pub struct Root {
    pub root: Hash,
    pub block_number: u64,
}

impl Ord for Root {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.block_number.cmp(&other.block_number)
    }
}

impl std::hash::Hash for Root {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.root.hash(state);
    }
}

pub struct IdentityTree<M: Middleware> {
    pub canonical_tree: DynamicMerkleTree<PoseidonHash>,
    pub tree_updates: BTreeMap<Root, IdentityUpdates>,

    pub canonical_tree_manager: TreeManager<M, CanonicalTree>,
    pub bridged_tree_manager: Vec<TreeManager<M, BridgedTree>>,

    pub chain_state: HashMap<u64, Root>,
    pub leaves: HashSet<Hash>,
}

impl<M> IdentityTree<M>
where
    M: Middleware + 'static,
{
    pub fn new(
        tree_depth: usize,
        canonical_tree_manager: TreeManager<M, CanonicalTree>,
        bridged_tree_manager: Vec<TreeManager<M, BridgedTree>>,
    ) -> Self {
        let canonical_tree =
            DynamicMerkleTree::new((), tree_depth, &Hash::ZERO);

        Self {
            canonical_tree,
            canonical_tree_manager,
            bridged_tree_manager,
            tree_updates: BTreeMap::new(),
            chain_state: HashMap::new(),
            leaves: HashSet::new(),
        }
    }

    pub async fn spawn(&mut self) -> eyre::Result<()> {
        //TODO: sync tree from cache
        self.sync_to_head().await?;

        let (identity_tx, mut identity_rx) = tokio::sync::mpsc::channel(100);
        let (root_tx, mut root_rx) = tokio::sync::mpsc::channel(100);

        // Spawn the tree managers for the canonical and bridged trees
        let mut handles = vec![];
        handles.push(self.canonical_tree_manager.spawn(identity_tx));

        for bridged_tree in self.bridged_tree_manager.iter() {
            handles.push(bridged_tree.spawn(root_tx.clone()));
        }

        //TODO: abstract into a function and return join handles
        loop {
            tokio::select! {
                identity_update = identity_rx.recv() => {
                    let (root, updates) = identity_update.expect("TODO: handle this case");
                    let first_update = updates.iter().take(1).next().expect("TODO: handle this case");
                        if *first_update.1 == Hash::ZERO {
                            for (leaf_index, _) in updates.iter() {
                                let leaf = self.canonical_tree.get_leaf(*leaf_index as usize);
                                self.leaves.remove(&leaf);
                            }
                        }else{
                            for (_, val) in updates.iter() {
                                self.leaves.insert(val.clone());
                            }
                        }

                        self.tree_updates.insert(root, updates);

                        //TODO: we also need to account for if mainnet
                    }


                bridged_root = root_rx.recv() => {
                    let (chain_id, new_root) = bridged_root.expect("TODO: handle this case");

                    let oldest_root: (&u64, &Root) = self.chain_state
                        .iter()
                        .min_by_key(|&(_, v)| v)
                        .expect("TODO: handle case ");


                    // Check if the new root is updating the oldest root, if so apply the updates to the canonical tree up to the oldest root
                    if chain_id == *oldest_root.0 {
                        let updates = self.tree_updates
                            .range((std::ops::Bound::Unbounded, std::ops::Bound::Included(oldest_root.1)))
                            .map(|(_, updates)| updates.clone())
                            .collect::<Vec<IdentityUpdates>>();

                        for update in updates.iter() {
                            let first_update = update.iter().take(1).next().expect("TODO: handle this case");
                                if *first_update.1 == Hash::ZERO {
                                    for (leaf_index, _) in update.iter() {
                                        self.canonical_tree.set_leaf(*leaf_index as usize, Hash::ZERO);
                                    }
                                }else{
                                    for (_, val) in update.iter() {
                                        self.canonical_tree.push(*val)?;
                                    }
                                }
                        }


                        // Insert the new root and recalculate the oldest root
                        self.chain_state.insert(chain_id, new_root);

                        let oldest_root: (&u64, &Root) = self.chain_state
                            .iter()
                            .min_by_key(|&(_, v)| v)
                            .expect("TODO: handle case ");

                        // split off the tree updates up to the new oldest root
                        self.tree_updates = self.tree_updates.split_off(oldest_root.1);
                    }else{
                        self.chain_state.insert(chain_id, new_root);

                    }
                }
            }
        }
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
            .map(|(chain_id, root)| {
                let root = Root {
                    root,
                    block_number: 0, //TODO: ensure its ok to set 0 here initially
                };

                (chain_id, root)
            })
            .collect::<HashMap<u64, Root>>();

        Ok(roots)
    }

    #[instrument(skip(self))]
    pub async fn sync_to_head(&mut self) -> eyre::Result<()> {
        self.chain_state = self.get_latest_roots().await?;

        let roots = self
            .chain_state
            .iter()
            .map(|(_, root)| root.clone())
            .collect::<HashSet<_>>();

        // Get all logs from the canonical tree
        let logs = self.canonical_tree_manager.block_scanner.next().await?;

        if logs.is_empty() {
            return Ok(());
        }

        //TODO: double check this logic
        // Split logs into groups where the root has already been bridged to all chains, and all other roots
        let mut pivot = 0;
        for log in logs.iter() {
            //TODO: check if le bytes or not

            // We can set post root block number to 0 since the Hash implementation of Root only evaluates the `root` field
            let post_root = Root {
                root: Hash::from_le_bytes(log.topics[3].0),
                block_number: 0,
            };

            pivot += 1;

            if roots.contains(&post_root) {
                break;
            }
        }

        let canonical_middleware =
            self.canonical_tree_manager.block_scanner.middleware.clone();

        // If all logs are already bridged to all chains, then sync the canonical tree
        if pivot == logs.len() {
            let identity_updates =
                extract_identity_updates(&logs, canonical_middleware).await?;
            let flattened_updates = flatten_updates(&identity_updates, None)?;
            for (leaf_index, value) in flattened_updates.into_iter() {
                self.canonical_tree.set_leaf(leaf_index as usize, *value);
            }
        } else {
            let (canonical_logs, pending_logs) = logs.split_at(pivot);
            let canonical_updates = extract_identity_updates(
                &canonical_logs,
                canonical_middleware.clone(),
            )
            .await?;
            let flattened_updates = flatten_updates(&canonical_updates, None)?;

            for (leaf_index, value) in flattened_updates.into_iter() {
                self.canonical_tree.set_leaf(leaf_index as usize, *value);
            }
            let pending_updates =
                extract_identity_updates(&pending_logs, canonical_middleware)
                    .await?;

            self.tree_updates.extend(pending_updates);
        }

        Ok(())
    }
}

fn flatten_updates(
    identity_updates: &BTreeMap<Root, HashMap<u32, Hash>>,
    root: Option<Root>,
) -> eyre::Result<HashMap<u32, &Hash>> {
    let mut flattened_updates = HashMap::new();

    let bound = if let Some(root) = root {
        std::ops::Bound::Included(root)
    } else {
        std::ops::Bound::Unbounded
    };

    let sub_tree = identity_updates.range((std::ops::Bound::Unbounded, bound));

    // Iterate in reverse over the sub-tree to ensure the latest updates are applied first
    for (_, updates) in sub_tree.rev() {
        for (index, hash) in updates.iter() {
            flattened_updates.entry(*index).or_insert(hash);
        }
    }

    Ok(flattened_updates)
}
