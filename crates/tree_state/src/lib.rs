use std::collections::HashMap;

use ethers::{
    providers::{Middleware, PubsubClient},
    types::H160,
};
use semaphore::lazy_merkle_tree::{Canonical, Derived};
use tree::{builder::WorldTreeBuilder, Hash, TreeData, WorldTree};
pub mod tree;

//TODO: change to inherit both middleware and pubsub? or maybe just pubsub since we will be listening to streams?
pub struct TreeAvailabilityService<M: Middleware + PubsubClient> {
    pub canonical_tree: WorldTree<TreeData<Canonical>, M>,
    pub derived_trees: HashMap<usize, WorldTree<TreeData<Derived>, M>>,
    //TODO: add a field for join handles
}

impl<M: Middleware + PubsubClient> TreeAvailabilityService<M> {
    pub fn spawn() {
        //         let canonical_tree_builder = CanonicalTreeBuilder::new(tree_depth, dense_prefix_depth);

        //         //TODO: get the current block height and root for the canonical tree

        //         //TODO: get the current block height and root for the derived trees

        //         //TODO: check database and start sync from there

        //         //TODO: get each block, get the identity updates and then update the tree accordingly
        //         //TODO: after each update to the canonical tree, check if the derived tree's root is the same and if so cut a new derived tree, removing from the temp hashmap

        //         //TODO: after all trees are synced o the current block and initialize, spawn a task for each tree to listen for new blocks. The canonical tree will send updates through
        //         //TODO: a broadcast channel and when a new root is received from the derived chain, it will process updates until it hits the new root.
    }
}
