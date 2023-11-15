use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use common::tracing::{init_datadog_subscriber, init_subscriber};
use ethers::middleware::gas_escalator::{
    Frequency, GasEscalatorMiddleware, GeometricGasPrice,
};
use ethers::prelude::{
    Http, LocalWallet, NonceManagerMiddleware, Provider, Signer, H160,
};
use ethers::providers::Middleware;
use ethers::types::U256;
use ethers_throttle::ThrottledProvider;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use governor::Jitter;
use opentelemetry::global::shutdown_tracer_provider;
use serde::{de, Deserialize, Deserializer, Serialize};
use tracing::Level;
use url::Url;
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
    #[clap(
        short,
        long,
        help = "Private key for account used to send `propagateRoot()` txs"
    )]
    private_key: String,
    #[clap(short, long, help = "Enable datadog backend for instrumentation")]
    datadog: bool,
}

#[derive(Deserialize, Serialize, Debug)]
struct Config {
    l1_rpc_endpoint: String,
    #[serde(deserialize_with = "deserialize_h160")]
    l1_world_id: H160,
    block_confirmations: usize,
    state_bridge: Vec<StateBridgeConfig>,
    throttle: u32,
    #[serde(deserialize_with = "deserialize_opt_u256", default)]
    max_gas_price: Option<U256>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct StateBridgeConfig {
    #[serde(deserialize_with = "deserialize_h160")]
    l1_state_bridge: H160,
    #[serde(deserialize_with = "deserialize_h160")]
    l2_world_id: H160,
    l2_rpc_endpoint: String,
    #[serde(deserialize_with = "deserialize_duration_from_seconds")]
    relaying_period_seconds: Duration,
    throttle: u32,
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

    let mut wallet = opts.private_key.parse::<LocalWallet>()?;
    let l1_middleware = initialize_l1_middleware(
        &config.l1_rpc_endpoint,
        config.throttle,
        wallet.address(),
        config.max_gas_price,
    )
    .await?;

    let chain_id = l1_middleware.get_chainid().await?.as_u64();
    wallet = wallet.with_chain_id(chain_id);

    let mut state_bridge_service =
        StateBridgeService::new(config.l1_world_id, l1_middleware.clone())
            .await?;

    for bridge_config in config.state_bridge {
        let l2_middleware = initialize_l2_middleware(
            &bridge_config.l2_rpc_endpoint,
            bridge_config.throttle,
        )
        .await?;

        let state_bridge = StateBridge::new_from_parts(
            bridge_config.l1_state_bridge,
            wallet.clone(),
            l1_middleware.clone(),
            bridge_config.l2_world_id,
            l2_middleware,
            bridge_config.relaying_period_seconds,
            config.block_confirmations,
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

    shutdown_tracer_provider();

    Ok(())
}

pub async fn initialize_l1_middleware(
    rpc_endpoint: &str,
    throttle: u32,
    wallet_address: H160,
    max_gas_price: Option<U256>,
) -> eyre::Result<
    Arc<
        GasEscalatorMiddleware<
            NonceManagerMiddleware<Provider<ThrottledProvider<Http>>>,
        >,
    >,
> {
    let provider = initialize_throttled_provider(rpc_endpoint, throttle)?;
    let nonce_manager_middleware =
        NonceManagerMiddleware::new(provider, wallet_address);

    let geometric_escalator =
        GeometricGasPrice::new(1.125, 60_u64, max_gas_price);
    let gas_escalator_middleware = GasEscalatorMiddleware::new(
        nonce_manager_middleware,
        geometric_escalator,
        Frequency::PerBlock,
    );

    Ok(Arc::new(gas_escalator_middleware))
}

pub async fn initialize_l2_middleware(
    l2_rpc_endpoint: &str,
    throttle: u32,
) -> eyre::Result<Arc<Provider<ThrottledProvider<Http>>>> {
    Ok(Arc::new(initialize_throttled_provider(
        l2_rpc_endpoint,
        throttle,
    )?))
}

pub fn initialize_throttled_provider(
    rpc_endpoint: &str,
    throttle: u32,
) -> eyre::Result<Provider<ThrottledProvider<Http>>> {
    let http_provider = Http::new(Url::parse(rpc_endpoint)?);
    let throttled_http_provider = ThrottledProvider::new(
        http_provider,
        throttle,
        Some(Jitter::new(
            Duration::from_millis(10),
            Duration::from_millis(100),
        )),
    );

    Ok(Provider::new(throttled_http_provider))
}

fn deserialize_h160<'de, D>(deserializer: D) -> Result<H160, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    H160::from_str(&s).map_err(de::Error::custom)
}

fn deserialize_duration_from_seconds<'de, D>(
    deserializer: D,
) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let secs = u64::deserialize(deserializer)?;
    Ok(Duration::from_secs(secs))
}

fn deserialize_opt_u256<'de, D>(
    deserializer: D,
) -> Result<Option<U256>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = Option::<String>::deserialize(deserializer)?;
    match s {
        Some(value) => U256::from_dec_str(&value)
            .map(Some)
            .map_err(serde::de::Error::custom),
        None => Ok(None),
    }
}
