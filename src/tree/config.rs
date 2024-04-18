use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use ethers::types::Address;
use serde::{Deserialize, Serialize};
use url::Url;

pub const CONFIG_PREFIX: &str = "WLD";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceConfig {
    pub tree_depth: usize,
    /// Configuration for the canonical tree on mainnet
    pub canonical_tree: TreeConfig,
    /// Configuration for tree cache
    pub cache: CacheConfig,
    /// Configuration for bridged trees
    pub bridged_trees: Option<Vec<TreeConfig>>,
    /// Socket at which to serve the service
    #[serde(default = "default::socket_address")]
    pub socket_address: SocketAddr,
    #[serde(default)]
    pub telemetry: Option<TelemetryConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CacheConfig {
    /// Path to mmap file responsible for caching the state of the canonical tree
    pub cache_file: PathBuf,
    #[serde(default)]
    pub purge_cache: bool,
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
pub struct TreeConfig {
    pub address: Address,
    #[serde(default = "default::window_size")]
    pub window_size: u64,
    pub creation_block: u64,
    pub provider: ProviderConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    /// Ethereum RPC endpoint
    #[serde(with = "crate::serde_utils::url")]
    pub rpc_endpoint: Url,
    #[serde(default = "default::provider_throttle")]
    pub throttle: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    // Service name - used for logging, metrics and tracing
    pub service_name: String,
    // Traces
    pub traces_endpoint: Option<String>,
    // Metrics
    pub metrics: Option<MetricsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    pub host: String,
    pub port: u16,
    pub queue_size: usize,
    pub buffer_size: usize,
    pub prefix: String,
}

mod default {
    use super::*;

    pub fn socket_address() -> SocketAddr {
        ([127, 0, 0, 1], 8080).into()
    }

    pub fn window_size() -> u64 {
        1000
    }

    pub fn provider_throttle() -> u32 {
        0
    }
}
