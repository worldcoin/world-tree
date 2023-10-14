use std::fs::File;
use std::io::BufReader;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use ethers::abi::{AbiDecode, AbiEncode, Uint};
use ethers::contract::Contract;
use ethers::core::{k256::ecdsa::SigningKey, types::Bytes};
use ethers::prelude::{
    ContractFactory, Http, LocalWallet, NonceManagerMiddleware, Provider, Signer, SignerMiddleware,
    Wallet,
};
use ethers::providers::Middleware;
use ethers::types::{Uint8, H256, U256};
use ethers::utils::{Anvil, AnvilInstance};
use state_bridge::bridge::{self, bridged_world_id};
use state_bridge::{
    bridge::{BridgedWorldID, IStateBridge},
    root::IWorldIdIdentityManager,
};
use tracing::{info, instrument};

use super::abi::{MockStateBridge, MockWorldID};

pub type SpecializedContract = Contract<SpecializedClient>;
type TestMiddleware = NonceManagerMiddleware<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>>;

pub struct MockChain<M: Middleware> {
    pub anvil: AnvilInstance,
    pub private_key: H256,
    pub state_bridge: MockStateBridge<M>,
    pub mock_world_id: MockWorldID<M>,
    pub mock_bridged_world_id: BridgedWorldID<M>,
    pub middleware: Arc<TestMiddleware>,
}

pub async fn spawn_mock_chain() -> eyre::Result<MockChain<TestMiddleware>> {
    let chain = Anvil::new().block_time(2u64).spawn();

    let private_key = H256::from_slice(&chain.keys()[0].to_bytes());

    let provider = Provider::<Http>::try_from(chain.endpoint())
        .expect("Failed to initialize chain endpoint")
        .interval(Duration::from_millis(500u64));

    let chain_id = provider.get_chainid().await?.as_u64();

    let wallet = LocalWallet::from(chain.keys()[0].clone()).with_chain_id(chain_id);

    let client = SignerMiddleware::new(provider, wallet.clone());
    let client = NonceManagerMiddleware::new(client, wallet.address());
    let client = Arc::new(client);

    let initial_root = U256::from_str("0x111").expect("couln't convert hex string to u256");

    let world_id_factory =
        load_and_build_contract("./sol/WorldIDIdentityManagerMock.json", client.clone())?;

    let world_id = world_id_factory
        .deploy(initial_root)?
        .confirmations(6usize)
        .send()
        .await?;

    let world_id_mock_address = world_id.address();

    let bridged_world_id_factory =
        load_and_build_contract("./sol/MockBridgedWorldID.json", client.clone())?;

    let tree_depth = Uint8::from(30);

    let bridged_world_id = bridged_world_id_factory
        .deploy(tree_depth)?
        .confirmations(6usize)
        .send()
        .await?;

    let bridged_world_id_address = bridged_world_id.address();

    let state_bridge_factory =
        load_and_build_contract("./sol/MockStateBridge.json", client.clone())?;

    let state_bridge = state_bridge_factory
        .deploy((world_id_mock_address, bridged_world_id_address))?
        .confirmations(6usize)
        .send()
        .await?;

    let state_bridge = MockStateBridge::new(state_bridge.address(), client.clone());

    let mock_bridged_world_id = BridgedWorldID::new(bridged_world_id_address, client.clone());

    let mock_world_id = MockWorldID::new(world_id_mock_address, client.clone());

    Ok(MockChain {
        anvil: chain,
        private_key,
        state_bridge,
        mock_bridged_world_id,
        mock_world_id,
        middleware: client,
    })
}

type SpecializedClient =
    NonceManagerMiddleware<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>>;
type SharableClient = Arc<SpecializedClient>;
type SpecializedFactory = ContractFactory<SpecializedClient>;

fn load_and_build_contract(
    path: impl Into<String>,
    client: SharableClient,
) -> eyre::Result<SpecializedFactory> {
    let path_string = path.into();
    let contract_file = File::open(&path_string)
        .unwrap_or_else(|_| panic!("Failed to open `{pth}`", pth = &path_string));

    let contract_artifact: serde_json::Value =
        serde_json::from_reader(BufReader::new(contract_file)).unwrap_or_else(|_| {
            panic!(
                "Could not parse the compiled contract at {pth}",
                pth = &path_string
            )
        });

    let bytecode_hex_encoded = contract_artifact["bytecode"].as_object().unwrap()["object"]
        .as_str()
        .unwrap();

    let bytecode_hex_encoded = bytecode_hex_encoded.trim_start_matches("0x");

    let bytes = hex::decode(bytecode_hex_encoded).unwrap();
    let bytecode = Bytes::from(bytes);

    let contract_factory = ContractFactory::new(Default::default(), bytecode, client);

    Ok(contract_factory)
}
