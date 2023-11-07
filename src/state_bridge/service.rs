use std::sync::Arc;

use ethers::providers::Middleware;
use ethers::types::H160;
use tokio::task::JoinHandle;

use super::error::StateBridgeError;
use super::StateBridge;
use crate::abi::IWorldIDIdentityManager;

//TODO: we dont need this root field and we can spawn the root logic so that this doesnt need to be here, we can simplify this service
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

    /// Spawns the `WorldTreeRoot` task which will listen to changes to the `WorldIDIdentityManager`
    /// [merkle tree root](https://github.com/worldcoin/world-id-contracts/blob/852790da8f348d6a2dbb58d1e29123a644f4aece/src/WorldIDIdentityManagerImplV1.sol#L63).
    #[allow(clippy::async_yields_async)]
    #[instrument(skip(self))]
    pub async fn spawn_listener(
        &self,
    ) -> JoinHandle<Result<(), StateBridgeError<M>>> {
        let root_tx = self.root_tx.clone();
        let world_id_identity_manager = self.world_id_identity_manager.clone();

        let world_id_identity_manager_address =
            world_id_identity_manager.address();
        tracing::info!(?world_id_identity_manager_address, "Spawning root");

        tokio::spawn(async move {
            // Event emitted when insertions or deletions are made to the tree
            let filter = world_id_identity_manager.event::<TreeChangedFilter>();

            let mut event_stream = filter.stream().await?.with_meta();

            // Listen to a stream of events, when a new event is received, update the root and block number
            while let Some(Ok((event, _))) = event_stream.next().await {
                let new_root = event.post_root.0;
                tracing::info!(?new_root, "New root from chain");
                root_tx.send(Uint::from_limbs(event.post_root.0))?;
            }

            Ok(())
        })
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
