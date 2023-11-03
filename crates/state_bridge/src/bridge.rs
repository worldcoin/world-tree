use std::sync::Arc;

use ethers::providers::Middleware;
use ethers::types::H160;
use ruint::Uint;
use tokio::select;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant};

use crate::abi::{IBridgedWorldID, IStateBridge};
use crate::error::StateBridgeError;
use crate::root::Hash;

/// The `StateBridge` is responsible for monitoring root changes from the `WorldRoot`, propagating the root to the corresponding Layer 2.
pub struct StateBridge<M: Middleware + 'static> {
    /// Interface for the `StateBridge` contract
    pub state_bridge: IStateBridge<M>,
    /// Interface for the `BridgedWorldID` contract
    pub bridged_world_id: IBridgedWorldID<M>,
    /// Time delay between `propagateRoot()` transactions
    pub relaying_period: Duration,
    /// The number of block confirmations before a `propagateRoot()` transaction is considered finalized
    pub block_confirmations: usize,
}

impl<M: Middleware> StateBridge<M> {
    /// # Arguments
    ///
    /// * state_bridge - Interface to the StateBridge smart contract.
    /// * bridged_world_id - Interface to the BridgedWorldID smart contract.
    /// * relaying_period - Duration between successive propagateRoot() invocations.
    /// * block_confirmations - Number of block confirmations required to consider a propagateRoot() transaction as finalized.
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

    /// # Arguments
    ///
    /// * `bridge_address` - Address of the StateBridge contract.
    /// * `canonical_middleware` - Middleware for interacting with the chain where StateBridge is deployed.
    /// * `bridged_world_id_address` - Address of the BridgedWorldID contract.
    /// * `derived_middleware` - Middleware for interacting with the chain where BridgedWorldID is deployed.
    /// * `relaying_period` - Duration between `propagateRoot()` transactions.
    /// * `block_confirmations` - Number of block confirmations before a`propagateRoot()` transaction is considered finalized.
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

    /// Spawns a `StateBridge` task to listen for `TreeChanged` events from `WorldRoot` and propagate new roots.
    ///
    /// # Arguments
    ///
    /// * `root_rx` - Receiver channel for roots from `WorldRoot`.
    pub async fn spawn(
        &self,
        mut root_rx: tokio::sync::broadcast::Receiver<Hash>,
    ) -> JoinHandle<Result<(), StateBridgeError<M>>> {
        let bridged_world_id = self.bridged_world_id.clone();
        let state_bridge = self.state_bridge.clone();
        let relaying_period = self.relaying_period;
        let block_confirmations = self.block_confirmations;

        let bridged_world_id_address = bridged_world_id.address();
        let state_bridge_address = state_bridge.address();
        tracing::info!(
            ?bridged_world_id_address,
            ?state_bridge_address,
            ?relaying_period,
            ?block_confirmations,
            "Spawning bridge"
        );

        tokio::spawn(async move {
            let mut latest_root = Hash::ZERO;
            let mut last_propagation = Instant::now();

            loop {
                // will either be positive or zero if difference is negative
                let sleep_time = relaying_period
                    .saturating_sub(Instant::now() - last_propagation);

                select! {
                    root = root_rx.recv() => {
                        tracing::info!(?root, "Root received from rx");
                        latest_root = root?;
                    }

                    _ = tokio::time::sleep(sleep_time) => {
                        tracing::info!("Sleep time elapsed");
                    }
                }

                let time_since_last_propagation =
                    Instant::now() - last_propagation;

                if time_since_last_propagation > relaying_period {
                    tracing::info!("Relaying period elapsed");

                    let latest_bridged_root = Uint::from_limbs(
                        bridged_world_id.latest_root().call().await?.0,
                    );

                    if latest_root != latest_bridged_root {
                        tracing::info!(
                            ?latest_root,
                            ?latest_bridged_root,
                            "Propagating root"
                        );

                        state_bridge
                            .propagate_root()
                            .send()
                            .await?
                            .confirmations(block_confirmations)
                            .await?;

                        last_propagation = Instant::now();
                    } else {
                        tracing::info!("Root already propagated");
                    }
                }
            }
        })
    }
}
