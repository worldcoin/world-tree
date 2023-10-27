use std::sync::Arc;

use ethers::middleware::contract::abigen;
use ethers::providers::Middleware;
use ethers::types::H160;
use ruint::Uint;
use tokio::select;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant};

use crate::error::StateBridgeError;
use crate::root::Hash;

abigen!(
    IStateBridge,
    r#"[
        function propagateRoot() external
    ]"#;
);

abigen!(
    IBridgedWorldID,
    r#"[
        event TreeChanged(uint256 indexed preRoot, uint8 indexed kind, uint256 indexed postRoot)
        event RootAdded(uint256 root, uint128 timestamp)
        function latestRoot() public view virtual returns (uint256)
        function receiveRoot(uint256 newRoot) external
    ]"#,
    event_derives(serde::Deserialize, serde::Serialize)
);

/// The `StateBridge` takes care of listening to roots propagated from the `WorldRoot` and
/// propagates them periodically every time `relaying_period` elapses and the root was updated.
pub struct StateBridge<M: Middleware + 'static> {
    /// Interface for the `StateBridge` contract
    pub state_bridge: IStateBridge<M>,
    /// Interface for the `BridgedWorldID` contract
    pub bridged_world_id: IBridgedWorldID<M>,
    /// Time in between `propagateRoot()` calls
    pub relaying_period: Duration,
    /// The number of blocks before a `propagateRoot()` call is considered finalized
    pub block_confirmations: usize,
}

impl<M: Middleware> StateBridge<M> {
    /// Constructor \
    /// `state_bridge`: `StateBridge` contract interface \
    /// `bridged_world_id`: `BridgedWorldID` contract interface \
    /// `relaying_period`: Time in between `propagateRoot()` calls \
    /// `block_confirmations: The number of blocks before a `propagateRoot()` call is considered finalized
    pub fn new(
        state_bridge: IStateBridge<M>,
        bridged_world_id: IBridgedWorldID<M>,
        relaying_period: Duration,
        block_confirmations: usize,
    ) -> Result<Self, StateBridgeError<M>> {
        Ok(Self {
            state_bridge,
            bridged_world_id,
            relaying_period,
            block_confirmations,
        })
    }

    /// Constructor with address and middleware \
    /// `bridge_address`: `StateBridge` contract address \
    /// `canonical_middleware`: middleware for the chain where the `StateBridge` is deployed \
    /// `bridged_world_id_address`: `BridgedWorldID` contract address \
    /// `derived_middleware`: middleware for the chain where the `BridgedWorldID` is deployed \
    /// `relaying_period`: Time in between `propagateRoot()` calls \
    /// `block_confirmations: The number of blocks before a `propagateRoot()` call is considered finalized \
    pub fn new_from_parts(
        bridge_address: H160,
        canonical_middleware: Arc<M>,
        bridged_world_id_address: H160,
        derived_middleware: Arc<M>,
        relaying_period: Duration,
        block_confirmations: usize,
    ) -> Result<Self, StateBridgeError<M>> {
        let state_bridge =
            IStateBridge::new(bridge_address, canonical_middleware);

        let bridged_world_id = IBridgedWorldID::new(
            bridged_world_id_address,
            derived_middleware.clone(),
        );

        Ok(Self {
            state_bridge,
            bridged_world_id,
            relaying_period,
            block_confirmations,
        })
    }

    /// Spawns the `StateBridge` which listens to the `WorldRoot` `TreeChanged` events for new roots to propagate. \
    /// `root_rx`: The root receiver that listens to the `WorldRoot` root sender
    pub async fn spawn(
        &self,
        mut root_rx: tokio::sync::broadcast::Receiver<Hash>,
    ) -> JoinHandle<Result<(), StateBridgeError<M>>> {
        let bridged_world_id = self.bridged_world_id.clone();
        let state_bridge = self.state_bridge.clone();
        let relaying_period = self.relaying_period;
        let block_confirmations = self.block_confirmations;

        tokio::spawn(async move {
            let mut latest_root = Hash::ZERO;
            let mut last_propagation = Instant::now();

            loop {
                // will either be positive or zero if difference is negative
                let sleep_time = relaying_period
                    .saturating_sub(Instant::now() - last_propagation);

                select! {
                    root = root_rx.recv() => {

                        latest_root = root?;
                    }

                    _ = tokio::time::sleep(sleep_time) => {}
                }

                let time_since_last_propagation =
                    Instant::now() - last_propagation;

                if time_since_last_propagation > relaying_period {
                    let latest_bridged_root = Uint::from_limbs(
                        bridged_world_id.latest_root().call().await?.0,
                    );

                    if latest_root != latest_bridged_root {
                        state_bridge
                            .propagate_root()
                            .send()
                            .await?
                            .confirmations(block_confirmations)
                            .await?;

                        last_propagation = Instant::now();
                    }
                }
            }
        })
    }
}
