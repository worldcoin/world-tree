use std::collections::{BTreeMap, HashMap, HashSet};

use common::test_utilities::abi::{
    DeleteIdentitiesCall, RegisterIdentitiesCall,
};
use ethers::abi::AbiDecode;
use ethers::contract::ContractError;
use ethers::providers::{Middleware, MiddlewareError};
use ethers::types::{Log, Selector, H160, U256};
use ruint::Uint;
use semaphore::dynamic_merkle_tree::DynamicMerkleTree;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::sync::mpsc::Sender;
use tracing::instrument;

use super::block_scanner::BlockScanner;
use super::tree_manager::{IdentityUpdates, Root, TreeManager};
use super::Hash;
use crate::abi::IBridgedWorldID;
use crate::tree::tree_manager::extract_identity_updates;

pub struct IdentityTree<M: Middleware> {
    // NOTE: update comments describing the canonical tree, it is the greatest common root across all trees
    pub canonical_tree: DynamicMerkleTree<PoseidonHash>,
    //NOTE: update comment to describe the tree updates to be applied to the canonical tree
    pub tree_updates: BTreeMap<Root, IdentityUpdates>,
    /// NOTE: update comment about updating the tree updates and canonical tree
    pub tree_manager: TreeManager<M>,
    //NOTE: update to describe this tracks the state of the tree on different chains for querying inclusion proofs.
    pub chain_state: HashMap<u64, Hash>,

    pub leaves: HashSet<Hash>, //NOTE: quick access to leaves to have a fast response for inclusion proofs in case the leaf is not there
                               //NOTE: if the leaf is deleted from the canonical tree, then you will not be able to get a proof for it even if the new root is not bridged yet, due to this hashset
                               // NOTE: also nice that we can return an error message that is specific when the leaf is present but not yet bridged to the chain!
}

impl<M> IdentityTree<M>
where
    M: Middleware + 'static,
{
    fn new() {}

    //TODO: going to need mutex instead of mut
    async fn spawn(&mut self) -> eyre::Result<()> {
        //TODO: sync tree from cache

        self.sync_to_head().await?;

        //TODO: spawn the tree manager to poll for updates
        let (mut identity_rx, mut root_rx, tree_handles) =
            self.tree_manager.spawn();

        loop {
            tokio::select! {

                //TODO: this should be a group of identity updates not a single one
                            identity_update = identity_rx.recv() => {
                                if let Some((root, updates)) = identity_update {

                                    for (leaf_index, val) in updates {

                                        if val == Hash::ZERO{
                                            let leaf = self.canonical_tree.get_leaf(leaf_index as usize);
                                            self.leaves.remove(&val);
                                        }else{

                                            //TODO: handle insertions

                                        }

                                    }



                            }

                        }

                            bridged_root = root_rx.recv() => {
                                if let Some((chain_id, root)) = bridged_root {
                                //TODO: check if updates need to be applied to the tree

                            }



                }
            }

            // some loop and tokio select! where we handle the bridged tree messages and the canonical tree messages
            // the canonical tree messages will append to the tree state
            // the bridged tree messages will update the root for the chain, as well as check the lcd across all of the roots and if there is a new common root,
            // the logic will advance the state of the canonical tree by consuming the next tree state into the canonical tree.
        }

        Ok(())
    }

    pub async fn get_latest_roots(
        &self,
    ) -> eyre::Result<HashMap<u64, Uint<256, 4>>> {
        let mut tree_data = vec![];

        for bridged_tree in self.tree_manager.bridged_tree_manager.iter() {
            let bridged_world_id = IBridgedWorldID::new(
                bridged_tree.address,
                bridged_tree.block_scanner.middleware.clone(),
            );

            tree_data.push((bridged_tree.chain_id, bridged_world_id));
        }

        tree_data.push((
            self.tree_manager.canonical_tree_manager.chain_id,
            IBridgedWorldID::new(
                self.tree_manager.canonical_tree_manager.address,
                self.tree_manager
                    .canonical_tree_manager
                    .block_scanner
                    .middleware
                    .clone(),
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
            .map(|(chain_id, root)| (chain_id, root))
            .collect::<HashMap<u64, Uint<256, 4>>>();

        Ok(roots)
    }

    #[instrument(skip(self))]
    pub async fn sync_to_head(&mut self) -> eyre::Result<()> {
        self.chain_state = self.get_latest_roots().await?;

        let roots = self
            .chain_state
            .iter()
            .map(|(_, root)| *root)
            .collect::<HashSet<_>>();

        // Get all logs from the canonical tree
        let logs = self
            .tree_manager
            .canonical_tree_manager
            .block_scanner
            .next()
            .await?;

        if logs.is_empty() {
            return Ok(());
        }

        //TODO: double check this logic
        // Split logs into groups where the root has already been bridged to all chains, and all other roots
        let mut pivot = 0;
        for log in logs.iter() {
            //TODO: check if le bytes or not
            let post_root = Hash::from_le_bytes(log.topics[3].0);
            pivot += 1;

            if roots.contains(&post_root) {
                break;
            }
        }

        let middleware = self
            .tree_manager
            .canonical_tree_manager
            .block_scanner
            .middleware
            .clone();

        // If all logs are already bridged to all chains, then sync the canonical tree
        if pivot == logs.len() {
            let identity_updates =
                extract_identity_updates(&logs, middleware).await?;
            let flattened_updates = flatten_updates(&identity_updates, None);
            for (leaf_index, value) in flattened_updates.into_iter() {
                self.canonical_tree.set_leaf(leaf_index as usize, *value);
            }
        } else {
            let (canonical_logs, pending_logs) = logs.split_at(pivot);
            let canonical_updates =
                extract_identity_updates(&canonical_logs, middleware.clone())
                    .await?;
            let flattened_updates = flatten_updates(&canonical_updates, None);

            for (leaf_index, value) in flattened_updates.into_iter() {
                self.canonical_tree.set_leaf(leaf_index as usize, *value);
            }
            let pending_updates =
                extract_identity_updates(&pending_logs, middleware).await?;

            self.tree_updates.extend(pending_updates);
        }

        Ok(())
    }
}

fn flatten_updates(
    identity_updates: &BTreeMap<Root, HashMap<u32, Hash>>,
    root: Option<Hash>,
) -> HashMap<u32, &Hash> {
    let mut flattened_updates = HashMap::new();

    let bound = if let Some(root) = root {
        std::ops::Bound::Included(root)
    } else {
        std::ops::Bound::Unbounded
    };

    // Create a range up to and including `specific_root`
    let sub_tree = identity_updates.range((std::ops::Bound::Unbounded, bound));

    // Iterate in reverse over the sub-tree to ensure the latest updates are applied first
    for (_, updates) in sub_tree.rev() {
        for (index, hash) in updates.iter() {
            flattened_updates.entry(*index).or_insert(hash);
        }
    }

    Ok(flattened_updates)
}
