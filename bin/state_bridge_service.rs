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
use serde::{Deserialize, Serialize};
use state_bridge::bridge::{IBridgedWorldID, IStateBridge, StateBridge};
use state_bridge::root::IWorldIDIdentityManager;
use state_bridge::StateBridgeService;
use tracing::info;

/// The state bridge service propagates roots according to the specified relaying_period by
/// calling the propagateRoot() method on each specified World ID StateBridge. The state bridge
/// service will also make sure that it doesn't propagate roots that have already been propagated
/// and have finalized on the BridgedWorldID side.
#[derive(Parser, Debug)]
#[clap(
    name = "State Bridge Service",
    about = "The state bridge service propagates roots according to the specified relaying_period by calling the propagateRoot() method on each specified World ID StateBridge. The state bridge service will also make sure that it doesn't propagate roots that have already been propagated and have finalized on the BridgedWorldID side."
)]
struct Options {
    #[clap(long, help = "Path to the TOML state bridge service config file")]
    config: PathBuf,
}

/// Converts a u64 into a Duration using Duration::from_secs
mod duration_seconds {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(duration: &Duration, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(d: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(d)?;
        Ok(Duration::from_secs(secs))
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
struct BridgeConfig {
    name: String,
    state_bridge_address: Address,
    bridged_world_id_address: Address,
    bridged_rpc_url: String,
}

/// The config TOML file defines all the necessary parameters to spawn a state bridge service. \
/// rpc_url - HTTP rpc url for an Ethereum node (string) \
/// private_key - pk to an address that will call the propagateRoot() method on the StateBridge contract (string) \
/// world_id_address - WorldIDIdentityManager contract address (string) \
/// bridge_pair_addresses - List of StateBridge and BridgedWorldID contract address pairs (strings) \
/// bridged_world_id_addresses - List of BridgedWorldID contract addresses (strings) \
/// relaying_period:  propagateRoot() call period time in seconds (u64) \
/// block_confirmations - Number of block confirmations required for the propagateRoot call on the StateBridge contract (optional number)
#[derive(Deserialize, Serialize, Debug, Clone)]
struct Config {
    /// RPC URL for the HTTP provider (World ID IdentityManager)
    rpc_url: String,
    /// Private key to use for the middleware signer
    private_key: String,
    /// `WorldIDIdentityManager` contract address
    world_id_address: H160,
    /// List of `StateBridge` and `BridgedWorldID` pair addresses
    bridge_configs: Vec<BridgeConfig>,
    /// `propagateRoot()` call period time in seconds
    #[serde(with = "duration_seconds")]
    relaying_period_seconds: Duration,
    /// Number of block confirmations required for the `propagateRoot` call on the `StateBridge`
    /// contract
    #[serde(default = "default_block_confirmations")]
    block_confirmations: usize,
}

fn default_block_confirmations() -> usize {
    1usize
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let options = Options::parse();

    let contents = fs::read_to_string(&options.config)?;

    let config: Config = toml::from_str(&contents)?;

    let rpc_url = config.rpc_url;

    let block_confirmations = config.block_confirmations;

    spawn_state_bridge_service(
        rpc_url,
        config.private_key,
        config.world_id_address,
        config.bridge_configs,
        config.relaying_period_seconds,
        block_confirmations,
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

    let wallet = private_key
        .parse::<LocalWallet>()
        .expect("couldn't instantiate wallet from private key")
        .with_chain_id(chain_id);
    let wallet_address = wallet.address();

    let middleware = SignerMiddleware::new(provider, wallet);
    let middleware = NonceManagerMiddleware::new(middleware, wallet_address);
    let middleware = Arc::new(middleware);

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

    state_bridge_service.spawn().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::time::Duration;

    use super::{Config, H160};
    use crate::BridgeConfig;

    #[tokio::test]
    async fn test_deserialize_toml() -> eyre::Result<()> {
        let config: Config = toml::from_str(
            r#"
            rpc_url = "127.0.0.1:8545"
            private_key = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318"
            world_id_address = "0x3f0BF744bb79A0b919f7DED73724ec20c43572B9"
            bridge_configs = [
                [
                    "Optimism",
                    "0x3f0BF744bb79A0b919f7DED73724ec20c43572B9",
                    "0x4f0BF744bb79A0b919f7DED73724ec20c43572B9",
                    "127.0.0.1:8545",
                ]
            ]
            relaying_period_seconds = 5
            "#)
        .expect("couldn't deserialize toml-encoded string");

        assert_eq!(
            config.rpc_url,
            String::from("127.0.0.1:8545"),
            "RPC didn't match"
        );
        assert_eq!(config.private_key, String::from("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318"), "private key didn't match");
        assert_eq!(
            config.world_id_address,
            H160::from_str("0x3f0BF744bb79A0b919f7DED73724ec20c43572B9")
                .unwrap(),
            "World ID address didn't match"
        );
        let bridged_configs: Vec<BridgeConfig> = vec![BridgeConfig {
            name: "Optimism".to_string(),
            state_bridge_address: H160::from_str(
                "0x3f0BF744bb79A0b919f7DED73724ec20c43572B9",
            )
            .unwrap(),
            bridged_world_id_address: H160::from_str(
                "0x4f0BF744bb79A0b919f7DED73724ec20c43572B9",
            )
            .unwrap(),
            bridged_rpc_url: "127.0.0.1:8545".to_string(),
        }];

        assert_eq!(config.bridge_configs, bridged_configs);
        // assert it uses serde default 1 block confirmation
        assert_eq!(
            config.block_confirmations, 1usize,
            "block confirmations didn't match"
        );
        assert_eq!(
            config.relaying_period_seconds,
            Duration::from_secs(5u64),
            "relaying period didn't match"
        );

        Ok(())
    }
}
