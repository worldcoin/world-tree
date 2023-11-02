//! # State Bridge Service
//!
//! ### Description
//!
//! The state bridge service for the World ID protocol takes care of periodically relaying the latest roots from the World ID Identity Manager onto L2 networks or sidechains that implement native bridge on Ethereum or have an integration with third party messaging protocol. The state bridge service requires a deployment of the [`world-id-state-bridge`](github.com/worldcoin/world-id-state-bridge/) contracts which in turn also have to be connected to a valid [`world-id-contracts`](https://github.com/worldcoin/world-id-contracts/) deployment.

pub mod abi;
pub mod bridge;
pub mod error;
pub mod root;

use std::sync::Arc;

use abi::IWorldIDIdentityManager;
use bridge::StateBridge;
use error::StateBridgeError;
use ethers::providers::Middleware;
use ethers::types::H160;
use root::WorldTreeRoot;
use tokio::task::JoinHandle;

/// Monitors the world tree root for changes and propagates new roots to target Layer 2s
pub struct StateBridgeService<M: Middleware + 'static> {
    /// Local state of the world tree root, responsible for listening to TreeChanged events from the`WorldIDIdentityManager`.
    pub canonical_root: WorldTreeRoot<M>,
    /// Vec of `StateBridge`, responsible for root propagation to target Layer 2s.
    pub state_bridges: Vec<StateBridge<M>>,
}

impl<M> StateBridgeService<M>
where
    M: Middleware,
{
    pub async fn new(
        world_tree: IWorldIDIdentityManager<M>,
    ) -> Result<Self, StateBridgeError<M>> {
        Ok(Self {
            canonical_root: WorldTreeRoot::new(world_tree).await?,
            state_bridges: vec![],
        })
    }

    pub async fn new_from_parts(
        world_tree_address: H160,
        middleware: Arc<M>,
    ) -> Result<Self, StateBridgeError<M>> {
        let world_tree =
            IWorldIDIdentityManager::new(world_tree_address, middleware);

        Ok(Self {
            canonical_root: WorldTreeRoot::new(world_tree).await?,
            state_bridges: vec![],
        })
    }

    /// Adds a `StateBridge` to orchestrate root propagation to a target Layer 2.
    pub fn add_state_bridge(&mut self, state_bridge: StateBridge<M>) {
        self.state_bridges.push(state_bridge);
    }

    /// Spawns the `StateBridgeService`.
    pub async fn spawn(
        &mut self,
    ) -> Result<
        Vec<JoinHandle<Result<(), StateBridgeError<M>>>>,
        StateBridgeError<M>,
    > {
        if self.state_bridges.is_empty() {
            return Err(StateBridgeError::BridgesNotInitialized);
        }

        let mut handles = vec![];

        // Bridges are spawned before the root so that the broadcast channel has active subscribers before the sender is spawned to avoid a SendError
        for bridge in self.state_bridges.iter() {
            handles.push(
                bridge.spawn(self.canonical_root.root_tx.subscribe()).await,
            );
        }

        handles.push(self.canonical_root.spawn().await);

        Ok(handles)
    }
}
