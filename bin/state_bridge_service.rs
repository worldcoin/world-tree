use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use common::tracing::{init_datadog_subscriber, init_subscriber};
use ethers::abi::Address;
use ethers::prelude::{
    Http, LocalWallet, NonceManagerMiddleware, Provider, Signer,
    SignerMiddleware, H160,
};
use ethers::providers::Middleware;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use opentelemetry::global::shutdown_tracer_provider;
use serde::{Deserialize, Serialize};
use tracing::Level;
use world_tree::abi::{IBridgedWorldID, IStateBridge};
use world_tree::state_bridge::service::StateBridgeService;
use world_tree::state_bridge::StateBridge;

#[derive(Parser, Debug)]
#[clap(
    name = "State Bridge Service",
    about = "The state bridge service listens to root changes from the `WorldIdIdentityManager` and propagates them to each of the corresponding Layer 2s specified in the configuration file."
)]
struct Opts {
    #[clap(
        short,
        long,
        help = "Path to the TOML state bridge service config file"
    )]
    config: PathBuf,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct StateBridgeConfig {
    name: String, //TODO: make this optional
    l1_state_bridge: String,
    l2_world_id_address: String,
    l2_rpc_endpoint: String,
    relaying_period_seconds: Duration,
}

#[derive(Deserialize, Serialize, Debug)]
struct Config {
    rpc_endpoint: String,
    world_id_address: String,
    datadog: bool,
    block_confirmations: usize,
    state_bridge: Vec<StateBridgeConfig>,
}

const SERVICE_NAME: &str = "state-bridge-service";

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let opts = Opts::parse();
    let contents = fs::read_to_string(&opts.config)?;
    let config: Config = toml::from_str(&contents)?;

    if opts.datadog {
        init_datadog_subscriber(SERVICE_NAME, Level::INFO);
    } else {
        init_subscriber(Level::INFO);
    }

    spawn_state_bridge_service(
        config.rpc_endpoint,
        config.private_key,
        config.world_id_address,
        config.state_bridges,
    )
    .await?;

    shutdown_tracer_provider();

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

    let mut state_bridge_service =
        StateBridgeService::new(world_id_address, middleware).await?;

    let wallet = private_key
        .parse::<LocalWallet>()
        .expect("couldn't instantiate wallet from private key");

    for bridge_config in bridge_configs {
        let BridgeConfig {
            state_bridge_address,
            bridged_world_id_address,
            bridged_rpc_url,
            ..
        } = bridge_config;

        let l2_middleware =
            initialize_l2_middleware(&bridged_rpc_url, wallet.clone()).await?;

        let state_bridge_interface =
            IStateBridge::new(state_bridge_address, l2_middleware.clone());

        let bridged_world_id_interface = IBridgedWorldID::new(
            bridged_world_id_address,
            l2_middleware.clone(),
        );

        let state_bridge = StateBridge::new(
            state_bridge_interface,
            bridged_world_id_interface,
            relaying_period,
            block_confirmations,
        )?;

        state_bridge_service.add_state_bridge(state_bridge);
    }

    tracing::info!("Spawning state bridge service");
    let handles = state_bridge_service.spawn()?;

    let mut handles = handles.into_iter().collect::<FuturesUnordered<_>>();
    while let Some(result) = handles.next().await {
        tracing::error!("StateBridgeError: {:?}", result);

        result??;
    }

    Ok(())
}

pub async fn initialize_l1_middleware() {
    todo!()
}

pub async fn initialize_l2_middleware(
    l2_rpc_endpoint: &str,
    wallet: LocalWallet,
) -> eyre::Result<
    Arc<NonceManagerMiddleware<SignerMiddleware<Provider<Http>, LocalWallet>>>,
> {
    let l2_provider = Provider::<Http>::try_from(l2_rpc_endpoint)?;
    let chain_id = l2_provider.get_chainid().await?.as_u64();

    let wallet = wallet.with_chain_id(chain_id);
    let wallet_address = wallet.address();

    let signer_middleware = SignerMiddleware::new(l2_provider, wallet);

    let nonce_manager_middleware =
        NonceManagerMiddleware::new(signer_middleware, wallet_address);

    let l2_middleware = Arc::new(nonce_manager_middleware);

    Ok(l2_middleware)
}
