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
    pub block_confirmations: usize,
}

impl<M: Middleware> StateBridge<M> {
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

    pub async fn spawn(
        &self,
        mut root_rx: tokio::sync::broadcast::Receiver<Hash>,
    ) -> JoinHandle<Result<(), StateBridgeError<M>>> {
        let bridged_world_id = self.bridged_world_id.clone();
        let state_bridge = self.state_bridge.clone();
        let relaying_period = self.relaying_period;
        let block_confirmations = self.block_confirmations;

        tokio::spawn(async move {
            #[allow(unused_assignments)]
            let mut latest_bridged_root: Uint<256, 4> = Hash::ZERO;
            let mut latest_root: Uint<256, 4> = Hash::ZERO;

            let mut last_propagation: Instant = Instant::now();
            #[allow(unused_assignments)]
            let mut time_since_last_propagation: Duration = relaying_period;

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

                time_since_last_propagation = Instant::now() - last_propagation;

                if time_since_last_propagation > relaying_period {
                    latest_bridged_root = Uint::from_limbs(
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
