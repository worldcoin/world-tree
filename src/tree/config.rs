use std::net::SocketAddr;
use std::path::Path;

use ethers::types::Address;
use serde::{Deserialize, Serialize};
use url::Url;

pub const CONFIG_PREFIX: &str = "WLD";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceConfig {
    pub world_tree: WorldTreeConfig,
    pub provider: ProviderConfig,
}

impl ServiceConfig {
    pub fn load(config_path: Option<&Path>) -> eyre::Result<Self> {
        let mut settings = config::Config::builder();

        if let Some(path) = config_path {
            settings =
                settings.add_source(config::File::from(path).required(true));
        }

        let settings = settings
            .add_source(
                config::Environment::with_prefix(CONFIG_PREFIX)
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?;

        let config = settings.try_deserialize::<Self>()?;

        Ok(config)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorldTreeConfig {
    pub tree_depth: usize,
    /// Configuration for the canonical tree on mainnet
    pub canonical_tree: TreeConfig,
    /// Configuration for bridged trees
    pub bridged_trees: Vec<TreeConfig>,
    /// Maximum window size when scanning blocks for TreeChanged events
    #[serde(default = "default::window_size")]
    pub window_size: u64,
    /// Socket at which to serve the service
    #[serde(default = "default::socket_address")]
    pub socket_address: SocketAddr,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TreeConfig {
    pub address: Address,
    pub window_size: u64,
    pub last_synced_block: u64,
    pub provider: ProviderConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    /// Ethereum RPC endpoint
    #[serde(with = "crate::serde_utils::url")]
    pub rpc_endpoint: Url,
    /// Request per minute limit
    pub throttle: Option<u32>,
}

mod default {
    use super::*;

    pub fn socket_address() -> SocketAddr {
        ([127, 0, 0, 1], 8080).into()
    }

    pub fn window_size() -> u64 {
        1000
    }
}
