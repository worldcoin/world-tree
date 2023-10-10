use std::fs::File;
use std::io::BufReader;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use ethers::abi::{AbiDecode, AbiEncode};
use ethers::contract::Contract;
use ethers::core::k256::ecdsa::SigningKey;
use ethers::prelude::{
    ContractFactory, Http, LocalWallet, NonceManagerMiddleware, Provider, Signer, SignerMiddleware,
    Wallet,
};
use ethers::providers::Middleware;
use ethers::types::{H256, U256};
use ethers::utils::{Anvil, AnvilInstance};
use tracing::{info, instrument};

use super::{abi as ContractAbi, CompiledContract};

pub type SpecializedContract = Contract<SpecializedClient>;

pub struct MockChain {
    pub anvil: AnvilInstance,
    pub private_key: H256,
    pub state_bridge: SpecializedContract,
}

pub async fn spawn_mock_chain() -> anyhow::Result<MockChain> {
    let chain = Anvil::new().block_time(2u64).spawn();

    let private_key = H256::from_slice(&chain.keys()[0].to_bytes());

    let provider = Provider::<Http>::try_from(chain.endpoint())
        .expect("Failed to initialize chain endpoint")
        .interval(Duration::from_millis(500u64));

    let chain_id = provider.get_chainid().await?.as_u64();

    let wallet = LocalWallet::from(chain.keys()[0].clone()).with_chain_id(chain_id);

    let root = H256::from_str("0x111").expect("couldn't read from str");

    let client = SignerMiddleware::new(provider, wallet.clone());
    let client = NonceManagerMiddleware::new(client, wallet.address());
    let client = Arc::new(client);

    let state_bridge_factory = load_and_build_contract("./sol/MockStateBridge.json", client)?;

    let state_bridge = state_bridge_factory
        .deploy(())?
        .confirmations(6usize)
        .send()
        .await?;

    Ok(MockChain {
        anvil: chain,
        private_key,
        state_bridge,
    })
}

type SpecializedClient =
    NonceManagerMiddleware<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>>;
type SharableClient = Arc<SpecializedClient>;
type SpecializedFactory = ContractFactory<SpecializedClient>;

fn load_and_build_contract(
    path: impl Into<String>,
    client: SharableClient,
) -> anyhow::Result<SpecializedFactory> {
    let path_string = path.into();
    let contract_file = File::open(&path_string)
        .unwrap_or_else(|_| panic!("Failed to open `{pth}`", pth = &path_string));

    let contract_json: CompiledContract = serde_json::from_reader(BufReader::new(contract_file))
        .unwrap_or_else(|_| {
            panic!(
                "Could not parse the compiled contract at {pth}",
                pth = &path_string
            )
        });
    let contract_bytecode = contract_json.bytecode.object.as_bytes().unwrap_or_else(|| {
        panic!(
            "Could not parse the bytecode for the contract at {pth}",
            pth = &path_string
        )
    });
    let contract_factory =
        ContractFactory::new(contract_json.abi, contract_bytecode.clone(), client);
    Ok(contract_factory)
}
