use std::sync::Arc;

use ruint::Uint;
use tokio::time::Duration;

use ethers::{
    middleware::contract::abigen, providers::Middleware, types::H160,
};
use tokio::task::JoinHandle;

use crate::{
    error::StateBridgeError,
    root::{self, Hash},
};

abigen!(
    IStateBridge,
    r#"[
    function propagateRoot() external
    ]"#;
);

abigen!(
    BridgedWorldID,
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
    pub bridged_world_id: BridgedWorldID<M>,
    pub relaying_period: Duration,
}

impl<M: Middleware> StateBridge<M> {
    pub fn new(
        state_bridge: IStateBridge<M>,
        bridged_world_id: BridgedWorldID<M>,
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

        let bridged_world_id = BridgedWorldID::new(
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
            let mut latest_root = Hash::ZERO;
            loop {
                // Process all of the updates and get the latest root
                while let Ok(root) = root_rx.recv().await {
                    // Check if the latest root is different than on L2 and if so, update the root
                    latest_root = Uint::from_limbs(
                        bridged_world_id.latest_root().call().await?.0,
                    );

                    if latest_root != root {
                        // Propagate root and ensure tx inclusion
                        state_bridge
                            .propagate_root()
                            .send()
                            .await?
                            .confirmations(6usize)
                            .await?;

                        //TODO: Handle reorgs
                        latest_root = root;
                    }

                    // Sleep for the specified time interval, this still need to be added
                    tokio::time::sleep(relaying_period).await;
                }

                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        })
    }
}
