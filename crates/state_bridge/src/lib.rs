pub mod bridge;
pub mod error;
pub mod root;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bridge::RootStateBridge;
use error::StateBridgeError;
use ethers::{
    providers::{Middleware, PubsubClient},
    types::{H160, U256},
};
use root::WorldTreeRoot;
use semaphore::{
    merkle_tree::Hasher,
    poseidon_tree::{PoseidonHash, Proof},
};

pub struct StateBridgeService<M: Middleware + PubsubClient + 'static> {
    pub canonical_root: WorldTreeRoot<M>,
    pub state_bridges: HashMap<usize, RootStateBridge<M>>,
    //TODO: add a field for join handles
}

impl<M: Middleware + PubsubClient> StateBridgeService<M> {
    pub async fn new(
        world_tree_address: H160,
        middleware: Arc<M>,
    ) -> Result<Self, StateBridgeError<M>> {
        Ok(Self {
            canonical_root: WorldTreeRoot::new(world_tree_address, middleware).await?,
            state_bridges: HashMap::new(),
        })
    }

    pub fn add_state_bridge(&mut self, chain_id: usize, state_bridge: RootStateBridge<M>) {
        self.state_bridges.insert(chain_id, state_bridge);
    }

    pub async fn spawn(&self) -> Result<(), StateBridgeError<M>> {
        let mut join_handles = vec![];

        join_handles.push(self.canonical_root.spawn().await?);

        for bridge in self.state_bridges.values() {
            join_handles.push(bridge.spawn().await?);
        }

        Ok(())
    }
}
