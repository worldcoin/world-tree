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
use super::tree_manager::TreeManager;
use super::Hash;
use crate::abi::IBridgedWorldID;
use crate::tree::tree_manager::extract_identity_updates;

pub type IdentityUpdates = HashMap<u32, Hash>;

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
    //TODO: going to need mutex instead of mut
    async fn spawn(&mut self) -> eyre::Result<()> {
        // let (identity_update_tx, identity_update_rx) =
        //     tokio::sync::mpsc::channel(100);

        // let (bridged_root_tx, bridged_root_rx) =
        //     tokio::sync::mpsc::channel(100);
        //TODO: sync tree from cache

        self.sync_to_head().await?;

        //TODO: spawn the tree manager to poll for updates
        self.tree_manager.spawn();

        // which calls poll for upates on the canonical tree manager and the bridged tree manager

        // some loop and tokio select! where we handle the bridged tree messages and the canonical tree messages
        // the canonical tree messages will append to the tree state
        // the bridged tree messages will update the root for the chain, as well as check the lcd across all of the roots and if there is a new common root,
        // the logic will advance the state of the canonical tree by consuming the next tree state into the canonical tree.

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
            //TODO: flatten all updates so there are no dups and then apply all updates to the tree
        } else {
            let (canonical_logs, pending_logs) = logs.split_at(pivot);
            let canonical_updates =
                extract_identity_updates(&logs, middleware.clone()).await?;
            //TODO: flatten all updates so there are no dups and then apply all updates to the tree

            //TODO: you also need the root here
            let pending_updates =
                extract_identity_updates(&logs, middleware).await?;
        }

        Ok(())
    }
}

#[derive(PartialEq, PartialOrd, Eq)]
pub struct Root {
    pub root: Hash,
    pub block_number: u64,
}

//TODO: ord the root by block timestamp so that they can have order in tree state

impl Ord for Root {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.block_number.cmp(&other.block_number)
    }
}
