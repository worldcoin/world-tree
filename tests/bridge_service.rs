use std::str::FromStr;
pub use std::time::Duration;

use common::test_utilities::abi::RootAddedFilter;
use common::test_utilities::chain_mock::{spawn_mock_chain, MockChain};
pub use ethers::abi::{AbiEncode, Address};
pub use ethers::core::abi::Abi;
pub use ethers::core::k256::ecdsa::SigningKey;
pub use ethers::core::rand;
pub use ethers::prelude::{
    ContractFactory, Http, LocalWallet, NonceManagerMiddleware, Provider,
    Signer, SignerMiddleware, Wallet,
};
pub use ethers::providers::{Middleware, StreamExt};
pub use ethers::types::{Bytes, H256, U256};
pub use ethers::utils::{Anvil, AnvilInstance};
pub use serde::{Deserialize, Serialize};
pub use serde_json::json;
pub use tokio::spawn;
pub use tokio::task::JoinHandle;
pub use tracing::{error, info, instrument};
use world_tree::abi::{IBridgedWorldID, IStateBridge};
use world_tree::state_bridge::error::StateBridgeError;
use world_tree::state_bridge::service::StateBridgeService;
use world_tree::state_bridge::StateBridge;

// test that spawns a mock anvil chain, deploys world id contracts, instantiates a `StateBridgeService`
// and propagates a root in order to see if the `StateBridgeService` works as intended.
#[tokio::test]
pub async fn test_relay_root() -> eyre::Result<()> {
    // we need anvil to be in scope in order for the middleware provider to not be dropped
    #[allow(unused_variables)]
    let MockChain {
        mock_state_bridge,
        mock_bridged_world_id,
        mock_world_id,
        middleware,
        anvil,
    } = spawn_mock_chain().await?;

    let relaying_period = std::time::Duration::from_secs(5);

    mock_state_bridge.propagate_root().send().await?.await?;

    let state_bridge_address = mock_state_bridge.address();

    let bridged_world_id_address = mock_bridged_world_id.address();

    let mut state_bridge_service =
        StateBridgeService::new(mock_world_id.address(), middleware.clone())
            .await
            .expect("couldn't create StateBridgeService");

    let state_bridge =
        IStateBridge::new(state_bridge_address, middleware.clone());

    let bridged_world_id =
        IBridgedWorldID::new(bridged_world_id_address, middleware.clone());

    let block_confirmations: usize = 6usize;

    let state_bridge = StateBridge::new(
        state_bridge,
        bridged_world_id,
        relaying_period,
        block_confirmations,
    )
    .unwrap();

    state_bridge_service.add_state_bridge(state_bridge);

    state_bridge_service
        .spawn()
        .expect("failed to spawn a state bridge service");

    let latest_root =
        U256::from_str("0x12312321321").expect("couldn't parse hexstring");

    mock_world_id.insert_root(latest_root).send().await?.await?;

    let await_matching_root = tokio::spawn(async move {
        let filter = mock_bridged_world_id.event::<RootAddedFilter>();

        let mut event_stream = filter.stream().await?.with_meta();

        // Listen to a stream of events, when a new event is received, update the root and block number
        while let Some(Ok((event, _))) = event_stream.next().await {
            let latest_bridged_root = event.root;

            if latest_bridged_root == latest_root {
                return Ok(latest_bridged_root);
            }
        }

        Err(eyre::eyre!(
            "The root in the event stream did not match `latest_root`"
        ))
    });

    let bridged_world_id_root = await_matching_root.await??;

    assert_eq!(latest_root, bridged_world_id_root);

    Ok(())
}

#[tokio::test]
pub async fn test_no_state_bridge_relay_fails() -> eyre::Result<()> {
    // we need anvil to be in scope in order for the middleware provider to not be dropped
    #[allow(unused_variables)]
    let MockChain {
        mock_world_id,
        middleware,
        anvil,
        ..
    } = spawn_mock_chain().await?;

    let mut state_bridge_service =
        StateBridgeService::new(mock_world_id.address(), middleware.clone())
            .await
            .expect("couldn't create StateBridgeService");

    let error = state_bridge_service.spawn().unwrap_err();

    assert!(
        matches!(error, StateBridgeError::BridgesNotInitialized),
        "Didn't error out as expected"
    );

    Ok(())
}
