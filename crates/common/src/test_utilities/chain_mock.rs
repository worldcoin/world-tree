use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use ethers::core::k256::ecdsa::SigningKey;
use ethers::prelude::{
    Http, LocalWallet, NonceManagerMiddleware, Provider, Signer,
    SignerMiddleware, Wallet,
};
use ethers::providers::Middleware;
use ethers::types::{Uint8, U256};
use ethers::utils::{Anvil, AnvilInstance};

use super::abi::{MockBridgedWorldID, MockStateBridge, MockWorldID};
use super::TREE_DEPTH;

pub type TestMiddleware = NonceManagerMiddleware<
    SignerMiddleware<Provider<Http>, Wallet<SigningKey>>,
>;

/// Creates an `Anvil` node with all the World ID contracts necessary
/// to mock a production World ID on-chain deployment
pub struct MockChain<M: Middleware> {
    /// Anvil node running the EVM locally for testing
    pub anvil: AnvilInstance,
    /// `MockStateBridge` contract
    pub mock_state_bridge: MockStateBridge<M>,
    /// `MockWorldIDIdentityManager` contract
    pub mock_world_id: MockWorldID<M>,
    /// `MockBridgedWorldID` contract
    pub mock_bridged_world_id: MockBridgedWorldID<M>,
    /// Middleware provider
    pub middleware: Arc<TestMiddleware>,
}

/// Spawns an anvil local chain with all World ID contracts deployed on it
pub async fn spawn_mock_chain() -> eyre::Result<MockChain<TestMiddleware>> {
    let chain = Anvil::new().block_time(2u64).spawn();

    let provider = Provider::<Http>::try_from(chain.endpoint())
        .expect("Failed to initialize chain endpoint")
        .interval(Duration::from_millis(500u64));

    let chain_id = provider.get_chainid().await?.as_u64();

    let wallet =
        LocalWallet::from(chain.keys()[0].clone()).with_chain_id(chain_id);
    let wallet_address = wallet.address();

    let client = SignerMiddleware::new(provider, wallet);
    let client = NonceManagerMiddleware::new(client, wallet_address);
    let client = Arc::new(client);

    let initial_root =
        U256::from_str("0x111").expect("couldn't convert hex string to u256");

    let mock_world_id = MockWorldID::deploy(client.clone(), initial_root)
        .expect("Couldn't deploy Mock World ID")
        .send()
        .await
        .expect("The MockWorldID deployment transaction couldn't finalize");

    let world_id_mock_address = mock_world_id.address();

    let tree_depth = Uint8::from(TREE_DEPTH);

    let mock_bridged_world_id = MockBridgedWorldID::deploy(
        client.clone(),
        tree_depth,
    )
    .expect("Couldn't deploy MockBridgedWorldID")
    .send()
    .await
    .expect("The MockBridgedWorldID deployment transaction couldn't finalize");

    let bridged_world_id_address = mock_bridged_world_id.address();

    let mock_state_bridge = MockStateBridge::deploy(
        client.clone(),
        (world_id_mock_address, bridged_world_id_address),
    )
    .expect("Couldn't deploy MockStateBridge")
    .send()
    .await
    .expect("The MockStateBridge deployment transaction couldn't finalize");

    mock_bridged_world_id
        .transfer_ownership(mock_state_bridge.address())
        .send()
        .await?
        .await?;

    Ok(MockChain {
        anvil: chain,
        mock_state_bridge,
        mock_bridged_world_id,
        mock_world_id,
        middleware: client,
    })
}
