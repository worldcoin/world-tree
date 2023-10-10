use ethers::{
    providers::Middleware,
    types::{Filter, H256, U256},
};
use semaphore::{
    lazy_merkle_tree::{self, Canonical},
    poseidon_tree::Proof,
};
use tokio::task::JoinHandle;
use tracing::info;

use super::{Hash, TreeData, TreeItem, TreeReader, TreeVersion, TreeWriter, WorldTree};

use crate::abi::{IWorldIdIdentityManager, TreeChangedFilter};
use crate::{abi::TREE_CHANGE_EVENT_SIGNATURE, error::TreeAvailabilityError};

impl<M: Middleware + 'static> WorldTree<TreeData<Canonical>, M> {
    pub async fn sync(&self) -> Result<(), TreeAvailabilityError<M>> {
        let to_block = self
            .middleware
            .get_block_number()
            .await
            .map_err(TreeAvailabilityError::MiddlewareError)?;

        // Initialize a new filter to get all of the tree changed events
        let filter = Filter::new()
            .topic0(TREE_CHANGE_EVENT_SIGNATURE)
            .address(self.address)
            .from_block(self.last_synced_block)
            .to_block(to_block);

        let logs = self
            .middleware
            .get_logs(&filter)
            .await
            .map_err(TreeAvailabilityError::MiddlewareError)?;

        for log in logs {
            if let Some(tx_hash) = log.transaction_hash {
                let Some(transaction) = self
                    .middleware
                    .get_transaction(tx_hash)
                    .await
                    .map_err(TreeAvailabilityError::MiddlewareError)? else{

                        todo!("Return an error here")
                    };

                //TODO: decode the tx data

                //TODO: for each batch of changes, add the changes to the tree history and update the tree
            }
        }

        Ok(())
    }

    pub async fn spawn(
        &self,
        //TODO: simplify this error
    ) -> Result<JoinHandle<Result<(), TreeAvailabilityError<M>>>, TreeAvailabilityError<M>> {
        // Sync the tree to the current block
        self.sync().await?;

        let middleware = self.middleware.clone();
        let world_tree_address = self.address.clone();
        let mut last_synced_block = self.last_synced_block;

        let thread = tokio::spawn(async move { Ok(()) });

        Ok(thread)
    }
}

impl TreeVersion for Canonical {}

impl TreeWriter for TreeData<Canonical> {
    fn update(&mut self, item: TreeItem) -> Hash {
        // Figure out if this will work or not
        take_mut::take(&mut self.tree, |tree| {
            tree.update_with_mutation(item.leaf_index, &item.element)
        });

        if item.element != Hash::ZERO {
            self.next_leaf = item.leaf_index + 1;
        }

        self.tree.root()
    }
}

impl TreeData<Canonical> {
    //TODO: FIXME: will probably need to update these
    /// Appends many identities to the tree, returns a list with the root, proof
    /// of inclusion and leaf index
    #[must_use]
    fn append_many(&self, identities: &[Hash]) -> Vec<(Hash, Proof, usize)> {
        let next_leaf = self.next_leaf;
        let mut output = Vec::with_capacity(identities.len());

        for (idx, identity) in identities.iter().enumerate() {
            let leaf_index = next_leaf + idx;
            //TODO: FIXME: update this to mutate the tree
            self.tree.update(leaf_index, identity);
            let proof = self.tree.proof(leaf_index);

            output.push((self.tree.root(), proof, leaf_index));
        }

        output
    }

    /// Deletes many identities from the tree, returns a list with the root
    /// and proof of inclusion
    #[must_use]
    fn delete_many(&self, leaf_indices: &[usize]) -> Vec<(Hash, Proof)> {
        let mut output = Vec::with_capacity(leaf_indices.len());

        for leaf_index in leaf_indices {
            //TODO: FIXME: update this to mutate the tree
            self.tree.update(*leaf_index, &Hash::ZERO);
            let proof = self.tree.proof(*leaf_index);
            output.push((self.tree.root(), proof));
        }

        output
    }
}
