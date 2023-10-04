use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

use ethers::{providers::Middleware, types::H160};
use semaphore::{
    lazy_merkle_tree::{Canonical, Derived},
    Field,
};

use super::{
    canonical::CanonicalMetadata, derived::DerivedMetadata, Hash, PoseidonTree, TreeData, TreeItem,
    TreeReader, TreeWriter, WorldTree,
};

/// A helper for building the first tree version. Exposes a type-safe API over
/// building a sequence of tree versions efficiently.
pub struct WorldTreeBuilder<M: Middleware> {
    canonical_tree: WorldTree<TreeData<Canonical>, M>,
    derived_trees: HashMap<usize, WorldTree<TreeData<Derived>, M>>,
}
impl<M: Middleware> WorldTreeBuilder<M> {
    pub fn new(
        tree_depth: usize,
        dense_prefix_depth: usize,
        tree_address: H160,
        middleware: Arc<M>,
    ) -> Self {
        let tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
            tree_depth,
            dense_prefix_depth,
            &Hash::ZERO,
        );

        let (identity_tx, _) = tokio::sync::broadcast::channel(1000);

        Self {
            canonical_tree: WorldTree::new(
                tree_address,
                Arc::new(RwLock::new(TreeData::new(
                    tree,
                    0,
                    CanonicalMetadata {
                        identity_tx,
                        last_synced_block: 0,
                    },
                ))),
                middleware,
            ),
            derived_trees: HashMap::new(),
        }
    }

    pub async fn add_derived_tree(
        &mut self,
        chain_id: usize,
        tree_address: H160,
        middleware: Arc<M>,
    ) {
        let canonical_tree_data = self.canonical_tree.tree_data.read().await;
        let derived_tree = canonical_tree_data.tree.derived();
        let identity_rx = canonical_tree_data.metadata.identity_tx.subscribe();

        self.derived_trees.insert(
            chain_id,
            WorldTree::new(
                tree_address,
                Arc::new(RwLock::new(TreeData::new(
                    derived_tree,
                    0,
                    DerivedMetadata { identity_rx },
                ))),
                middleware,
            ),
        );
    }

    //TODO: this function should get the current root, build both of the trees and then when the tree availabilty service starts, it will sync the canonical tree and then send the updates through the tx
    pub async fn build(
        &mut self,
    ) -> (
        WorldTree<TreeData<Canonical>, M>,
        HashMap<usize, WorldTree<TreeData<Derived>, M>>,
    ) {
        //TODO: in the future this should check the db and build from there first

        //TODO: get the most recent block and root at that block for the canonical tree

        //TODO: get the most recent block and the root at that block for the derived trees

        //TODO: sync the canonical tree from the last synced block to the current block, update the current block, updating the derived trees as the roots are the same

        todo!()
    }
}
