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

use crate::{error::StateBridgeError, root::Hash};

pub struct RootStateBridge<M: Middleware + PubsubClient> {
    pub latest_root: Hash,
    pub root_rx: tokio::sync::broadcast::Receiver<Hash>,
    //TODO: document this, it is using the same naming conventions as the tree_state crate.
    //TODO: Canonical is mainnet, derived is any chain that we are bridging to that has a derived state from the canonical tree.
    //TODO: We might want to update this naming convention in the state bridge
    pub canonical_middleware: Arc<M>,
    pub derived_middleware: Arc<M>,
}

impl<M: Middleware + PubsubClient> RootStateBridge<M> {
    pub fn new(
        canonical_middleware: Arc<M>,
        derived_middleware: Arc<M>,
        root_rx: tokio::sync::broadcast::Receiver<Hash>,
    ) -> Self {
        Self {
            latest_root: Hash::ZERO,
            root_rx,
            canonical_middleware,
            derived_middleware,
        }
    }

    pub async fn spawn(&self) -> Result<(), StateBridgeError<M>> {
        //TODO: spawn a task that continuously listens to the root_rx channel

        todo!()
    }
}
