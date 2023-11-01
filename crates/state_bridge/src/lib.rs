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

/// `StateBridgeService` has handles to `StateBridge` contracts, periodically
/// calls the `propagateRoot` method on them and ensures that the transaction
/// finalizes on Ethereum mainnet. It also monitors `_latestRoot` changes
/// on the `WorldIDIdentityManager` contract and calls `propagateRoot` only if
/// the root has changed and a specific relay period amount of time has elapsed.
pub struct StateBridgeService<M: Middleware + 'static> {
    /// `WorldIDIdentityManager` contract interface
    pub canonical_root: WorldTreeRoot<M>,
    /// List of `StateBridge` contract interfaces
    pub state_bridges: Vec<StateBridge<M>>,
    /// `StateBridge` Tokio task handles
    pub handles: Vec<JoinHandle<Result<(), StateBridgeError<M>>>>,
}

impl<M> StateBridgeService<M>
where
    M: Middleware,
{
    /// Initializes the `StateBridgeService`
    ///
    /// `world_tree`:`IWorldID ` - interface to the `WorldIDIdentityManager`
    pub async fn new(
        world_tree: IWorldIDIdentityManager<M>,
    ) -> Result<Self, StateBridgeError<M>> {
        Ok(Self {
            canonical_root: WorldTreeRoot::new(world_tree).await?,
            state_bridges: vec![],
            handles: vec![],
        })
    }

    /// Initializes the `StateBridgeService`
    ///
    /// `world_tree_address`:`H160` - interface to the `WorldIDIdentityManager`
    ///
    /// `middleware`:`Arc<M>` - Middleware provider
    pub async fn new_from_parts(
        world_tree_address: H160,
        middleware: Arc<M>,
    ) -> Result<Self, StateBridgeError<M>> {
        let world_tree =
            IWorldIDIdentityManager::new(world_tree_address, middleware);

        Ok(Self {
            canonical_root: WorldTreeRoot::new(world_tree).await?,
            state_bridges: vec![],
            handles: vec![],
        })
    }

    /// Adds a state bridge to the list of state bridges the service will use
    /// to propagate roots on chain to their destination chains.
    ///
    /// `state_bridge`: `StateBridge<M>` - state bridge contract interface with provider
    ///
    /// ### Notes
    ///
    /// Needs to be called before the spawn function so that the `StateBridgeService`
    /// knows where to propagate roots to.
    pub fn add_state_bridge(&mut self, state_bridge: StateBridge<M>) {
        self.state_bridges.push(state_bridge);
    }

    /// Spawns the `StateBridgeService`.
    pub async fn spawn(&mut self) -> Result<(), StateBridgeError<M>> {
        // if no state bridge initialized then there is no point in spawning
        // the state bridge service as there'd be no receivers for new roots.
        if self.state_bridges.is_empty() {
            return Err(StateBridgeError::BridgesNotInitialized);
        }

        // We first instantiate the receivers on the state bridges
        // so that the root sender doesn't yield an error when pushing roots
        // through the channel.
        for bridge in self.state_bridges.iter() {
            self.handles.push(
                bridge.spawn(self.canonical_root.root_tx.subscribe()).await,
            );
        }

        // creates a sender to the channel which will fetch new roots
        // and pass it to the `StateBridge` through the channel
        self.handles.push(self.canonical_root.spawn().await);

        Ok(())
    }
}
