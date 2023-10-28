use std::sync::Arc;

use ethers::middleware::Middleware;
use ethers::providers::StreamExt;
use ethers::types::H160;
use ruint::Uint;
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::PoseidonHash;

pub type Hash = <PoseidonHash as Hasher>::Hash;
use ethers::prelude::abigen;
use tokio::task::JoinHandle;

use crate::error::StateBridgeError;

// Creates the ABI for the WorldIDIdentityManager interface
abigen!(
    IWorldIDIdentityManager,
    r#"[
        function latestRoot() external returns (uint256)
        event TreeChanged(uint256 indexed preRoot, uint8 indexed kind, uint256 indexed postRoot)
    ]"#;
);

/// `WorldTreeRoot` is the struct that has a `WorldIDIdentityManager` interface
/// and sends the latest roots to a channel so that the `StateBridgeService` can
/// consume them. It listens to `TreeChanged` events on `WorldIDIdentityManager` in
/// order to update its internal state.
pub struct WorldTreeRoot<M: Middleware + 'static> {
    /// `WorldIDIdentityManager` contract interface
    pub world_id_identity_manager: IWorldIDIdentityManager<M>,
    /// channel to send the latest roots to the `StateBridge`
    pub root_tx: tokio::sync::broadcast::Sender<Hash>,
}

impl<M> WorldTreeRoot<M>
where
    M: Middleware,
{
    /// `WorldTreeRoot` constructor
    /// `world_id_identity_manager`:`IWorldIDIdentityManager<M>` - `WorldIDIdentityManager` interface
    pub async fn new(
        world_id_identity_manager: IWorldIDIdentityManager<M>,
    ) -> Result<Self, StateBridgeError<M>> {
        let (root_tx, _) = tokio::sync::broadcast::channel::<Hash>(1000);

        Ok(Self {
            world_id_identity_manager,
            root_tx,
        })
    }

    /// `WorldTreeRoot` constructor from address and middleware
    /// `world_id_identity_manager`:`IWorldIDIdentityManager<M>` - `WorldIDIdentityManager` interface
    /// `world_tree_address`: `H160` - `WorldIDIdentityManager` contract address
    /// `middleware`: `Arc<M>` - Middleware provider (ethers)
    pub async fn new_from_parts(
        world_tree_address: H160,
        middleware: Arc<M>,
    ) -> Result<Self, StateBridgeError<M>> {
        let (root_tx, _) = tokio::sync::broadcast::channel::<Hash>(1000);

        let world_id_identity_manager = IWorldIDIdentityManager::new(
            world_tree_address,
            middleware.clone(),
        );

        Ok(Self {
            world_id_identity_manager,
            root_tx,
        })
    }

    /// Spawns the `WorldTreeRoot` task which will fetch the latest root in the `WorldIDIdentityManager`
    /// [merkle tree root](https://github.com/worldcoin/world-id-contracts/blob/852790da8f348d6a2dbb58d1e29123a644f4aece/src/WorldIDIdentityManagerImplV1.sol#L63) every single time the
    /// tree is updated by a batch insertion or batch deletion operation.
    pub async fn spawn(&self) -> JoinHandle<Result<(), StateBridgeError<M>>> {
        let root_tx = self.root_tx.clone();
        let world_id_identity_manager = self.world_id_identity_manager.clone();

        tokio::spawn(async move {
            // event emitted when insertions or deletions on the merkle tree have taken place
            let filter = world_id_identity_manager.event::<TreeChangedFilter>();

            let mut event_stream = filter.stream().await?.with_meta();

            // Listen to a stream of events, when a new event is received, update the root and block number
            while let Some(Ok((event, _))) = event_stream.next().await {
                // Send it through the tx, you can convert ethers U256 to ruint with Uint::from_limbs()
                root_tx.send(Uint::from_limbs(event.post_root.0))?;
            }

            Ok(())
        })
    }
}

#[cfg(test)]

mod tests {
    use std::str::FromStr;

    use common::test_utilities::chain_mock::{spawn_mock_chain, MockChain};
    use ethers::types::U256;
    use tokio::time::Duration;

    use super::*;

    #[tokio::test]
    async fn listen_and_propagate_root() -> eyre::Result<()> {
        // we need anvil to be in scope for the provider to not be dropped
        #[allow(unused_variables)]
        let MockChain {
            mock_world_id,
            middleware,
            anvil,
            ..
        } = spawn_mock_chain().await?;

        let world_id = IWorldIDIdentityManager::new(
            mock_world_id.address(),
            middleware.clone(),
        );

        let tree_root = WorldTreeRoot::new(world_id).await?;

        tree_root.spawn().await;

        let test_root = U256::from_str("0x222").unwrap();

        mock_world_id.insert_root(test_root).send().await?.await?;

        let mut root_rx = tree_root.root_tx.subscribe();

        let relaying_period = Duration::from_secs(5);

        tokio::spawn(async move {
            loop {
                // Process all of the updates and get the latest root
                while let Ok(root) = root_rx.recv().await {
                    if root == Uint::from_limbs(test_root.0) {
                        break;
                    }
                    // Check if the latest root is different than on L2 and if so, update the root
                    tokio::time::sleep(relaying_period).await;
                }

                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });

        Ok(())
    }
}
