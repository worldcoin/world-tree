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
use tokio::task::JoinHandle;

use crate::{
    error::StateBridgeError,
    root::{self, Hash},
};

pub struct StateBridge<M: Middleware + PubsubClient + 'static> {
    pub latest_root: Hash,
    pub root_rx: Option<tokio::sync::broadcast::Receiver<Hash>>,
    //TODO: document this, it is using the same naming conventions as the tree_state crate.
    //TODO: Canonical is mainnet, derived is any chain that we are bridging to that has a derived state from the canonical tree.
    //TODO: We might want to update this naming convention in the state bridge
    pub canonical_middleware: Arc<M>,
    pub derived_middleware: Arc<M>,
}

impl<M: Middleware + PubsubClient> StateBridge<M> {
    pub fn new(
        canonical_middleware: Arc<M>,
        derived_middleware: Arc<M>,
        root_rx: tokio::sync::broadcast::Receiver<Hash>,
    ) -> Self {
        Self {
            latest_root: Hash::ZERO,
            root_rx: Some(root_rx),
            canonical_middleware,
            derived_middleware,
        }
    }

    pub async fn spawn(&mut self) -> JoinHandle<Result<(), StateBridgeError<M>>> {
        let mut root_rx = self
            .root_rx
            .take()
            .expect("TODO: Propagate error indicating bridge arleady spawned");

        tokio::spawn(async move {
            let mut latest_root = Hash::ZERO;
            loop {
                // Drain the updates and get the latest
                while let Ok(root) = root_rx.try_recv() {
                    latest_root = root;
                }

                //TODO: Check if the latest root is different than on L2 and if so, update the root

                //TODO: Sleep for the specified time interval, this still need to be added
            }
        })
    }
}
