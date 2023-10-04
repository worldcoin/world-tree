pub mod bridge;
pub mod root;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bridge::RootStateBridge;
use ethers::{
    providers::{Middleware, PubsubClient},
    types::{H160, U256},
};
use root::WorldTreeRoot;
use semaphore::{
    merkle_tree::Hasher,
    poseidon_tree::{PoseidonHash, Proof},
};

pub struct StateBridgeService<M: Middleware + PubsubClient> {
    pub canonical_root: WorldTreeRoot<M>,
    pub root_state_bridges: HashSet<RootStateBridge<M>>,
    //TODO: add a field for join handles
}

impl<M: Middleware + PubsubClient> StateBridgeService<M> {
    // pub fn new(world_tree_address: H160, middleware: Arc<M>) {
    //     Self {
    //         WorldTreeRoot::new(world_tree_address, middleware, root_tx),
    //         HashSet::new(),
    //     }
    // }

    pub fn spawn() -> Self {
        todo!()
    }
}
