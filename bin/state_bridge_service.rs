use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use ethers::abi::Address;
use ethers::prelude::{
    Http, LocalWallet, NonceManagerMiddleware, Provider, Signer,
    SignerMiddleware, H160,
};
use ethers::providers::Middleware;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use state_bridge::abi::{
    IBridgedWorldID, IStateBridge, IWorldIDIdentityManager,
};
use state_bridge::bridge::StateBridge;
use state_bridge::StateBridgeService;
use tracing::info;

#[derive(Parser, Debug)]
#[clap(
    name = "State Bridge Service",
    about = "The state bridge service listens to root changes from the WorldIdIdentityManager and propagates them to each of the corresponding Layer 2s specified in the configuration file."
)]
struct Opts {
    #[clap(
        short,
        long,
        help = "Path to the TOML state bridge service config file"
    )]
    config: PathBuf,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
struct BridgeConfig {
    name: String,
    state_bridge_address: Address,
    bridged_world_id_address: Address,
    bridged_rpc_url: String,
}

//TODO: lets update this to be a yaml file and then we can do something like the following:
// rpc_url: ""
// private_key: ""
// world_id_address: ""
// block_confirmations: ""
// state_bridges:
//   optimism:
//      state_bridge_address: ""
//      bridged_world_id_address: ""
//      bridged_rpc_url: ""
//      relaying_period_seconds: 5
//
//   arbitrum:
//      state_bridge_address: ""
//      bridged_world_id_address: ""
//      bridged_rpc_url: ""
//      relaying_period_seconds: 5

#[derive(Deserialize, Serialize, Debug, Clone)]
struct Config {
    // RPC URL for the HTTP provider (World ID IdentityManager)
    rpc_url: String,
    // Private key to use for the middleware signer
    private_key: String,
    // `WorldIDIdentityManager` contract address
    world_id_address: H160,
    // List of `StateBridge` and `BridgedWorldID` pair addresses
    bridge_configs: Vec<BridgeConfig>,
    // `propagateRoot()` call period time in seconds
    relaying_period_seconds: Duration,
    // Number of block confirmations required for the `propagateRoot` call on the `StateBridge`
    // contract
    block_confirmations: Option<usize>,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let opts = Opts::parse();
    let contents = fs::read_to_string(&opts.config)?;
    let config: Config = toml::from_str(&contents)?;

    spawn_state_bridge_service(
        config.rpc_url,
        config.private_key,
        config.world_id_address,
        config.bridge_configs,
        config.relaying_period_seconds,
        config.block_confirmations.unwrap_or(0),
    )
    .await?;

    Ok(())
}

async fn spawn_state_bridge_service(
    rpc_url: String,
    private_key: String,
    world_id_address: H160,
    bridge_configs: Vec<BridgeConfig>,
    relaying_period: Duration,
    block_confirmations: usize,
) -> eyre::Result<()> {
    let provider = Provider::<Http>::try_from(rpc_url)
        .expect("failed to initialize Http provider");

    let chain_id = provider.get_chainid().await?.as_u64();

    let wallet = private_key.parse::<LocalWallet>()?.with_chain_id(chain_id);
    let wallet_address = wallet.address();

    let signer_middleware = SignerMiddleware::new(provider, wallet);
    let nonce_manager_middleware =
        NonceManagerMiddleware::new(signer_middleware, wallet_address);
    let middleware = Arc::new(nonce_manager_middleware);

    let world_id_interface =
        IWorldIDIdentityManager::new(world_id_address, middleware.clone());

    let mut state_bridge_service =
        StateBridgeService::new(world_id_interface).await?;

    for bridge_config in bridge_configs {
        let BridgeConfig {
            name,
            state_bridge_address,
            bridged_world_id_address,
            bridged_rpc_url,
        } = bridge_config;

        let bridged_provider = Provider::<Http>::try_from(bridged_rpc_url)
            .expect("failed to initialize Http provider");

        let chain_id = bridged_provider.get_chainid().await?.as_u64();

        let wallet = private_key
            .parse::<LocalWallet>()
            .expect("couldn't instantiate wallet from private key")
            .with_chain_id(chain_id);
        let wallet_address = wallet.address();

        let bridged_middleware =
            SignerMiddleware::new(bridged_provider, wallet);
        let bridged_middleware =
            NonceManagerMiddleware::new(bridged_middleware, wallet_address);
        let bridged_middleware = Arc::new(bridged_middleware);

        let state_bridge_interface =
            IStateBridge::new(state_bridge_address, middleware.clone());

        let bridged_world_id_interface = IBridgedWorldID::new(
            bridged_world_id_address,
            bridged_middleware.clone(),
        );

        let state_bridge = StateBridge::new(
            state_bridge_interface,
            bridged_world_id_interface,
            relaying_period,
            block_confirmations,
        )?;

        state_bridge_service.add_state_bridge(state_bridge);
        info!("Added a bridge to {} to the state-bridge-service", name);
    }

    let handles = state_bridge_service.spawn().await?;

    let mut handles = handles.into_iter().collect::<FuturesUnordered<_>>();
    while let Some(result) = handles.next().await {
        result??;
    }

    Ok(())
}
