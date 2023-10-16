use std::fs::File;
use std::io::BufReader;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use ethers::abi::{Abi, AbiDecode, AbiEncode, Uint};
use ethers::contract::{abigen, Contract};
use ethers::core::{k256::ecdsa::SigningKey, types::Bytes};
use ethers::prelude::{
    ContractFactory, Http, LocalWallet, NonceManagerMiddleware, Provider, Signer, SignerMiddleware,
    Wallet,
};
use ethers::providers::{Middleware, Ws};
use ethers::types::{Uint8, H256, U256};
use ethers::utils::{Anvil, AnvilInstance};
use state_bridge::bridge::{self, bridged_world_id};
use tracing::{info, instrument};

use super::abi::{MockBridgedWorldID, MockStateBridge, MockWorldID};

type TestMiddleware = NonceManagerMiddleware<SignerMiddleware<Provider<Ws>, Wallet<SigningKey>>>;

pub struct MockChain<M: Middleware> {
    pub anvil: AnvilInstance,
    pub private_key: H256,
    pub state_bridge: MockStateBridge<M>,
    pub mock_world_id: MockWorldID<M>,
    pub mock_bridged_world_id: MockBridgedWorldID<M>,
    pub middleware: Arc<TestMiddleware>,
}

pub async fn spawn_mock_chain() -> eyre::Result<MockChain<TestMiddleware>> {
    let chain = Anvil::new().block_time(2u64).spawn();

    let private_key = H256::from_slice(&chain.keys()[0].to_bytes());

    let provider = Provider::<Ws>::connect(chain.ws_endpoint())
        .await
        .expect("Failed to initialize chain endpoint")
        .interval(Duration::from_millis(500u64));

    let chain_id = provider.get_chainid().await?.as_u64();

    let wallet = LocalWallet::from(chain.keys()[0].clone()).with_chain_id(chain_id);

    let client = SignerMiddleware::new(provider, wallet.clone());
    let client = NonceManagerMiddleware::new(client, wallet.address());
    let client = Arc::new(client);

    let initial_root = U256::from_str("0x111").expect("couln't convert hex string to u256");

    let mock_world_id = MockWorldID::deploy(client.clone(), initial_root)
        .unwrap()
        .send()
        .await
        .unwrap();

    let world_id_mock_address = mock_world_id.address();

    let tree_depth = Uint8::from(30);

    let mock_bridged_world_id = MockBridgedWorldID::deploy(client.clone(), tree_depth)
        .unwrap()
        .send()
        .await
        .unwrap();

    let bridged_world_id_address = mock_bridged_world_id.address();

    let state_bridge = MockStateBridge::deploy(
        client.clone(),
        (world_id_mock_address, bridged_world_id_address),
    )
    .unwrap()
    .send()
    .await
    .unwrap();

    Ok(MockChain {
        anvil: chain,
        private_key,
        state_bridge,
        mock_bridged_world_id,
        mock_world_id,
        middleware: client,
    })
}
