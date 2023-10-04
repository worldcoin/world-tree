use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use ethers::{
    providers::{Middleware, PubsubClient},
    types::{H160, U256},
};
use semaphore::{
    merkle_tree::Hasher,
    poseidon_tree::{PoseidonHash, Proof},
};

use crate::root::Hash;

pub struct RootStateBridge<M: Middleware + PubsubClient> {
    pub latest_root: Hash,
    pub block_number: u64, //TODO: update to use a different type
    pub canonical_tree_address: H160,
    pub root_rx: tokio::sync::broadcast::Receiver<Hash>,
    //TODO: document this, it is using the same naming conventions as the tree_state crate.
    //TODO: Canonical is mainnet, derived is any chain that we are bridging to that has a derived state from the canonical tree.
    //TODO: We might want to update this naming convention in the state bridge
    pub canonical_middleware: Arc<M>,
    pub derived_middleware: Arc<M>,
}

impl<M: Middleware + PubsubClient> RootStateBridge<M> {
    //TODO: new func

    pub fn spawn() -> Self {
        //TODO: the idea is that we will spawn a task that runs a loop to check if there is either a new root from the root tx or if the timer has elapsed to bridge a new root
        //TODO: if either condition is met and the root is different than the latest root on the l2, then bridge a new root.
        //TODO: we will probably also need to keep track of historical roots bridged in the case that the sleep time or new root from the tx is shorter than the time to propagate the root to the l2
        //TODO: as is the case for Polygon if the sleep time is < 40 min ish
        todo!()
    }
}
