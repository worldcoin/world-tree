use std::sync::Arc;

use ethers::middleware::contract::abigen;
use ethers::providers::Middleware;
use ethers::types::H160;
use ruint::Uint;
use tokio::task::JoinHandle;
use tokio::time::Duration;

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

            loop {
                // Process all of the updates and get the latest root
                //TODO: explain why we use try_recv and drain the channel from updates rather than just using recv
                //TODO: we can maybe just notify across threads when the value is updated instead of using a channel,
                //TODO: allowing us to always have the most recent value and avoiding the need to drain the channel
                while let Ok(root) = root_rx.try_recv() {
                    //TODO: Handle reorgs
                    latest_root = root;
                }

                // Check if the latest root is different than on L2 and if so, update the root
                latest_bridged_root = Uint::from_limbs(
                    bridged_world_id.latest_root().call().await?.0,
                );

                if latest_root != latest_bridged_root {
                    // Propagate root and ensure tx inclusion
                    state_bridge
                        .propagate_root()
                        .send()
                        .await?
                        .confirmations(6usize) //TODO: make this a cli arg or default
                        .await?;

                    // Sleep for the specified time interval
                    tokio::time::sleep(relaying_period).await;
                }
            }
        })
    }
}
