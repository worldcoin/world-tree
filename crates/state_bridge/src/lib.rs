//! crate level docs

pub mod bridge;
pub mod error;
pub mod root;

use std::sync::Arc;

use bridge::StateBridge;
use error::StateBridgeError;
use ethers::{providers::Middleware, types::H160};
use root::{IWorldIdIdentityManager, WorldTreeRoot};

use tokio::task::JoinHandle;

pub struct StateBridgeService<M: Middleware + 'static> {
    pub canonical_root: WorldTreeRoot<M>,
    pub state_bridges: Vec<StateBridge<M>>,
    pub handles: Vec<JoinHandle<Result<(), StateBridgeError<M>>>>,
}

impl<M> StateBridgeService<M>
where
    M: Middleware,
{
    pub async fn new(
        world_tree: IWorldIdIdentityManager<M>,
    ) -> Result<Self, StateBridgeError<M>> {
        Ok(Self {
            canonical_root: WorldTreeRoot::new(world_tree).await?,
            state_bridges: vec![],
            handles: vec![],
        })
    }

    pub async fn new_from_parts(
        world_tree_address: H160,
        middleware: Arc<M>,
    ) -> Result<Self, StateBridgeError<M>> {
        let world_tree =
            IWorldIdIdentityManager::new(world_tree_address, middleware);

        Ok(Self {
            canonical_root: WorldTreeRoot::new(world_tree).await?,
            state_bridges: vec![],
            handles: vec![],
        })
    }

    pub fn add_state_bridge(&mut self, state_bridge: StateBridge<M>) {
        self.state_bridges.push(state_bridge);
    }

    pub async fn spawn(&mut self) -> Result<(), StateBridgeError<M>> {
        //TODO: maybe check that the bridges vec is not empty otherwise, return an error

        //TODO: add a comment why we spawn this first
        for bridge in self.state_bridges.iter() {
            self.handles.push(
                bridge.spawn(self.canonical_root.root_tx.subscribe()).await,
            );
        }

        self.handles.push(self.canonical_root.spawn().await);

        Ok(())
    }
}
