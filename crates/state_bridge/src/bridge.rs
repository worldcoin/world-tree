use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use tokio::sync::Mutex;

use ethers::{
    middleware::contract::abigen,
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

abigen!(
    IStateBridge,
    r#"[
    function propagateRoot() external
    ]"#;
);

#[derive(Eq, Hash, PartialEq)]
pub struct StateBridge<M: Middleware + PubsubClient + 'static> {
    //TODO: replace this with IStateBridge
    pub canonical_middleware: Arc<M>,

    //TODO: replace this with IBridgedWorldId that represents the receiver contract on the l2
    pub derived_middleware: Arc<M>,
}

impl<M: Middleware + PubsubClient> StateBridge<M> {
    //TODO: pass in the state bridge contract and bridge world id contract
    pub fn new(canonical_middleware: Arc<M>, derived_middleware: Arc<M>) -> Self {
        Self {
            canonical_middleware,
            derived_middleware,
        }
    }

    pub async fn spawn(
        &self,
        mut root_rx: tokio::sync::broadcast::Receiver<Hash>,
    ) -> JoinHandle<Result<(), StateBridgeError<M>>> {
        tokio::spawn(async move {
            let mut latest_root = Hash::ZERO;
            loop {
                // Process all of the updates and get the latest root
                while let Ok(root) = root_rx.try_recv() {
                    latest_root = root;
                }

                //TODO: Check if the latest root is different than on L2 and if so, update the root

                //TODO: Sleep for the specified time interval, this still need to be added
            }
        })
    }
}
