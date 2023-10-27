//! # State Bridge Service
//!
//! ### Description
//!
//! The state bridge service for the World ID protocol takes care of periodically relaying the latest roots from the World ID Identity Manager onto L2 networks or sidechains that implement native bridge on Ethereum or have an integration with third party messaging protocol. The state bridge service requires a deployment of the [`world-id-state-bridge`](github.com/worldcoin/world-id-state-bridge/) contracts which in turn also have to be connected to a valid [`world-id-contracts`](https://github.com/worldcoin/world-id-contracts/) deployment.
//!
//! ### Usage
//!
//! #### CLI
//!
//! Create a state_bridge_service.toml file which will hold the configuration parameters for the state bridge
//! service. You can use the example in the test as a template:
//!
//! ```toml
//! rpc_url = "127.0.0.1:8545"
//! private_key = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318"
//! world_id_address = "0x3f0BF744bb79A0b919f7DED73724ec20c43572B9"
//! bridge_configs = [
//!     [
//!         "Optimism",
//!         # StateBridge Address
//!         "0x3f0BF744bb79A0b919f7DED73724ec20c43572B9",
//!         # BridgedWorldID Address
//!         "0x4f0BF744bb79A0b919f7DED73724ec20c43572B9",
//!         "127.0.0.1:8545",
//!     ]
//! ]
//! relaying_period_seconds = 5
//! ```
//!
//! ```bash
//! cargo build --bin state-bridge-service --release
//! ./target/release/state-bridge-service --config <CONFIG>
//! ```
//!
//! #### Library
//! In order to launch a `StateBridgeService` as a library you can use the following example from the [`bridge_service.rs`](https://github.com/worldcoin/identity-sequencer/blob/359f0fe3ec62b18d6f569d8ad31967c048401fa1/crates/state_bridge/tests/bridge_service.rs#L37) test file as a guide.
//! ```
//! use state_bridge::bridge::{IBridgedWorldID, IStateBridge, StateBridge};
//! use state_bridge::root::IWorldIDIdentityManager;
//! use state_bridge::StateBridgeService;
//! use common::test_utilities::chain_mock::{spawn_mock_chain, MockChain};
//! // If you deploy your own state bridge and run your own Ethereum RPC
//! // (or use a third party service like Alchemy)
//! // you can instantiate your own variables by providing the right addresses
//! // and a middleware (implements ethers::middleware::Middleware).
//! #[tokio::test]
//! async fn doc_example() -> eyre::Result<()> {
//! let MockChain {
//!        mock_state_bridge,
//!        mock_bridged_world_id,
//!        mock_world_id,
//!        middleware,
//!        anvil,
//!        ..
//!    } = spawn_mock_chain().await?;
//!
//!    let relaying_period = std::time::Duration::from_secs(5);
//!
//!    let world_id = IWorldIDIdentityManager::new(
//!        mock_world_id.address(),
//!        middleware.clone(),
//!    );
//!
//!    mock_state_bridge.propagate_root().send().await?.await?;
//!
//!    let state_bridge_address = mock_state_bridge.address();
//!
//!    let bridged_world_id_address = mock_bridged_world_id.address();
//!    
//!    let block_confirmations = 6;
//!
//!    let mut state_bridge_service = StateBridgeService::new(world_id)
//!        .await
//!        .expect("couldn't create StateBridgeService");
//!
//!    let state_bridge =
//!        IStateBridge::new(state_bridge_address, middleware.clone());
//!
//!    let bridged_world_id =
//!        IBridgedWorldID::new(bridged_world_id_address, middleware.clone());
//!
//!    let state_bridge =
//!        StateBridge::new(state_bridge, bridged_world_id, relaying_period, block_confirmations)
//!            .unwrap();
//!
//!    state_bridge_service.add_state_bridge(state_bridge);
//!
//!    state_bridge_service
//!        .spawn()
//!        .await
//!        .expect("failed to spawn a state bridge service");
//! }
//! ```
//!
pub mod bridge;
pub mod error;
pub mod root;

use std::sync::Arc;

use bridge::StateBridge;
use error::StateBridgeError;
use ethers::providers::Middleware;
use ethers::types::H160;
use root::{IWorldIDIdentityManager, WorldTreeRoot};
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
    /// ### Constructor for the `StateBridgeService` \
    /// `world_tree`:`IWorldID ` - interface to the `WorldIDIdentityManager` \
    pub async fn new(
        world_tree: IWorldIDIdentityManager<M>,
    ) -> Result<Self, StateBridgeError<M>> {
        Ok(Self {
            canonical_root: WorldTreeRoot::new(world_tree).await?,
            state_bridges: vec![],
            handles: vec![],
        })
    }

    /// Constructor for the `StateBridgeService` \
    /// `world_tree_address`:`H160` - interface to the `WorldIDIdentityManager` \
    /// `middleware`:`Arc\<M\>` - Middleware provider \
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
    /// to propagate roots on chain to their destination chains. \
    /// `state_bridge`: `StateBridge<M>` - state bridge contract interface with provider\
    /// ### Notes
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
