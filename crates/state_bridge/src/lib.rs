pub mod bridge;
pub mod error;
pub mod root;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bridge::StateBridge;
use error::StateBridgeError;
use ethers::{
    providers::{Middleware, PubsubClient},
    types::{spoof::State, H160, U256},
};
use root::WorldTreeRoot;
use semaphore::{
    merkle_tree::Hasher,
    poseidon_tree::{PoseidonHash, Proof},
};
use tokio::task::JoinHandle;

pub struct StateBridgeService<M: Middleware + PubsubClient + 'static> {
    pub canonical_root: WorldTreeRoot<M>,
    pub state_bridges: HashMap<usize, StateBridge<M>>,
    pub handles: Vec<JoinHandle<Result<(), StateBridgeError<M>>>>,
}

impl<M: Middleware + PubsubClient> StateBridgeService<M> {
    pub async fn new(
        world_tree_address: H160,
        middleware: Arc<M>,
    ) -> Result<Self, StateBridgeError<M>> {
        Ok(Self {
            canonical_root: WorldTreeRoot::new(world_tree_address, middleware).await?,
            state_bridges: HashMap::new(),
            handles: vec![],
        })
    }

    pub fn add_state_bridge(&mut self, chain_id: usize, state_bridge: StateBridge<M>) {
        self.state_bridges.insert(chain_id, state_bridge);
    }

    pub async fn spawn(&mut self) -> Result<(), StateBridgeError<M>> {
        self.handles.push(self.canonical_root.spawn().await);

        for bridge in self.state_bridges.values() {
            self.handles
                .push(bridge.spawn(self.canonical_root.root_tx.subscribe()).await);
        }

        Ok(())
    }
}
