use std::sync::Arc;

use ethers::providers::{Middleware, StreamExt};
use ethers::types::H160;
use ruint::Uint;
use tokio::task::JoinHandle;
use tracing::instrument;

use super::error::StateBridgeError;
use super::StateBridge;
use crate::abi::{IWorldIDIdentityManager, TreeChangedFilter};
use crate::tree::Hash;

/// Monitors the world tree root for changes and propagates new roots to target Layer 2s
pub struct StateBridgeService<
    L1M: Middleware + 'static,
    L2M: Middleware + 'static,
> {
    /// Monitors `TreeChanged` events from `WorldIDIdentityManager` and broadcasts new roots to through the `root_tx`.
    pub world_id_identity_manager: IWorldIDIdentityManager<L1M>,
    /// Vec of `StateBridge`, responsible for root propagation to target Layer 2s.
    pub state_bridges: Vec<StateBridge<L1M, L2M>>,
}

impl<L1M, L2M> StateBridgeService<L1M, L2M>
where
    L1M: Middleware,
    L2M: Middleware,
{
    pub async fn new(
        world_tree_address: H160,
        middleware: Arc<L1M>,
    ) -> Result<Self, StateBridgeError<L1M, L2M>> {
        let world_id_identity_manager = IWorldIDIdentityManager::new(
            world_tree_address,
            middleware.clone(),
        );
        Ok(Self {
            world_id_identity_manager,
            state_bridges: vec![],
        })
    }

    /// Adds a `StateBridge` to orchestrate root propagation to a target Layer 2.
    pub fn add_state_bridge(&mut self, state_bridge: StateBridge<L1M, L2M>) {
        self.state_bridges.push(state_bridge);
    }

    /// Spawns the `WorldTreeRoot` task which will listen to changes to the `WorldIDIdentityManager`
    /// [merkle tree root](https://github.com/worldcoin/world-id-contracts/blob/852790da8f348d6a2dbb58d1e29123a644f4aece/src/WorldIDIdentityManagerImplV1.sol#L63).
    #[instrument(skip(self))]
    pub fn listen_for_new_roots(
        &self,
        root_tx: tokio::sync::broadcast::Sender<Hash>,
    ) -> JoinHandle<Result<(), StateBridgeError<L1M, L2M>>> {
        let world_id_identity_manager = self.world_id_identity_manager.clone();

        let world_id_identity_manager_address =
            world_id_identity_manager.address();
        tracing::info!(?world_id_identity_manager_address, "Spawning root");

        let root_tx_clone = root_tx.clone();
        tokio::spawn(async move {
            //TODO: add some comments as to what this is doing
            let latest_root = world_id_identity_manager
                .latest_root()
                .call()
                .await
                .map_err(StateBridgeError::L1ContractError)?;

            root_tx_clone.send(Uint::from_limbs(latest_root.0))?;

            // Event emitted when insertions or deletions are made to the tree
            let filter = world_id_identity_manager.event::<TreeChangedFilter>();

            let mut event_stream = filter
                .stream()
                .await
                .map_err(StateBridgeError::L1ContractError)?
                .with_meta();

            // Listen to a stream of events, when a new event is received, send the new root through the channel
            while let Some(Ok((event, _))) = event_stream.next().await {
                let new_root = event.post_root.0;
                tracing::info!(?new_root, "New root from chain");
                root_tx_clone.send(Uint::from_limbs(new_root))?;
            }

            Ok(())
        })
    }

    /// Spawns the `StateBridgeService`.
    pub fn spawn(
        &mut self,
    ) -> Result<
        Vec<JoinHandle<Result<(), StateBridgeError<L1M, L2M>>>>,
        StateBridgeError<L1M, L2M>,
    > {
        if self.state_bridges.is_empty() {
            return Err(StateBridgeError::BridgesNotInitialized);
        }

        let (root_tx, _) = tokio::sync::broadcast::channel::<Hash>(1000);

        let mut handles = vec![];
        // Bridges are spawned before the root so that the broadcast channel has active subscribers before the sender is spawned to avoid a SendError
        for bridge in self.state_bridges.iter() {
            handles.push(bridge.spawn(root_tx.subscribe()));
        }

        handles.push(self.listen_for_new_roots(root_tx));

        Ok(handles)
    }
}
