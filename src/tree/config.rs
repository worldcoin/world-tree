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
    /// Address of the World Tree
    pub world_id_contract_address: Address,
    /// Maximum window size when scanning blocks for TreeChanged events
    #[serde(default = "default::window_size")]
    pub window_size: u64,
    /// Creation block of the World Tree
    pub creation_block: u64,
    /// Quantity of recent tree changes to cache. This allows inclusion proof requests to specify a historical root
    pub tree_history_size: usize,
    /// Depth of the World Tree
    pub tree_depth: usize,
    /// Depth of merkle tree that should be represented as a densely populated prefix. The remainder of the tree will be represented with pointer-based structures.
    pub dense_prefix_depth: usize,
    /// Socket at which to serve the service
    #[serde(default = "default::socket_address")]
    pub socket_address: SocketAddr,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    /// Ethereum RPC endpoint
    #[serde(with = "crate::serde_utils::url")]
    pub rpc_endpoint: Url,
    /// Request per minute limit
    #[serde(default = "default::throttle")]
    pub throttle: u32,
}

mod default {
    use super::*;

    pub fn socket_address() -> SocketAddr {
        ([127, 0, 0, 1], 8080).into()
    }

    pub fn window_size() -> u64 {
        1000
    }

    pub fn throttle() -> u32 {
        0
    }
}