use std::sync::Arc;

use ruint::Uint;
use tokio::time::{Duration, Instant};

use ethers::{
    middleware::contract::abigen, providers::Middleware, types::H160,
};
use tokio::{select, task::JoinHandle};

use crate::{error::StateBridgeError, root::Hash};

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

pub struct StateBridge<M: Middleware + 'static> {
    pub state_bridge: IStateBridge<M>,
    pub bridged_world_id: IBridgedWorldID<M>,
    pub relaying_period: Duration,
}

impl<M: Middleware> StateBridge<M> {
    pub fn new(
        state_bridge: IStateBridge<M>,
        bridged_world_id: IBridgedWorldID<M>,
        relaying_period: Duration,
    ) -> Result<Self, StateBridgeError<M>> {
        Ok(Self {
            state_bridge,
            bridged_world_id,
            relaying_period,
        })
    }

    pub fn new_from_parts(
        bridge_address: H160,
        canonical_middleware: Arc<M>,
        bridged_world_id_address: H160,
        derived_middleware: Arc<M>,
        relaying_period: Duration,
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
        })
    }

    pub async fn spawn(
        &self,
        mut root_rx: tokio::sync::broadcast::Receiver<Hash>,
    ) -> JoinHandle<Result<(), StateBridgeError<M>>> {
        let bridged_world_id = self.bridged_world_id.clone();
        let state_bridge = self.state_bridge.clone();
        let relaying_period = self.relaying_period;

        tokio::spawn(async move {
            let mut latest_bridged_root;
            let mut latest_root = Hash::ZERO;

            let mut last_propagation: Instant = Instant::now();

            loop {
                let sleep_time = relaying_period
                    .saturating_sub(Instant::now() - last_propagation);

                select! {
                    root = root_rx.recv() => {
                        match root {
                            Ok(root) => {
                                latest_root = root;
                            }
                            Err(_) => {
                                break;
                            }
                        }
                    }
                    // will either be positive or zero if difference is negative
                    _ = tokio::time::sleep(sleep_time) => {}
                }

                latest_bridged_root = Uint::from_limbs(
                    bridged_world_id.latest_root().call().await?.0,
                );

                let time_since_last_propagation =
                    Instant::now() - last_propagation;

                if latest_root != latest_bridged_root
                    && time_since_last_propagation > relaying_period
                {
                    state_bridge
                        .propagate_root()
                        .send()
                        .await?
                        .confirmations(6usize) //TODO: make this a cli arg or default
                        .await?;

                    last_propagation = Instant::now();
                }
            }
            Ok(())
        })
    }
}
