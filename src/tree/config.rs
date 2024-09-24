use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use alloy::primitives::Address;
use alloy::transports::layers::RetryBackoffLayer;
use serde::{Deserialize, Serialize};
use url::Url;

pub const CONFIG_PREFIX: &str = "WLD";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub tree_depth: usize,
    pub db: DbConfig,
    /// Configuration for the canonical tree on mainnet
    pub canonical_tree: TreeConfig,
    /// Configuration for tree cache
    pub cache: CacheConfig,
    /// Configuration for bridged trees
    #[serde(with = "map_vec", default)]
    pub bridged_trees: Vec<TreeConfig>,
    /// Socket at which to serve the service
    #[serde(default = "default::socket_address")]
    pub socket_address: Option<SocketAddr>,
    #[serde(default)]
    pub telemetry: Option<TelemetryConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DbConfig {
    /// The db connection string
    /// i.e. postgresql://user:password@localhost:5432/dbname
    pub connection_string: String,

    /// Whether to create the database if it does not exist
    #[serde(default = "default::bool_true")]
    pub create: bool,

    /// Whether to run migrations
    #[serde(default = "default::bool_true")]
    pub migrate: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CacheConfig {
    /// Path to the directory containing all cache files
    pub dir: PathBuf,

    #[serde(default)]
    pub purge: bool,
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

        let config = serde_path_to_error::deserialize(settings)?;

        Ok(config)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TreeConfig {
    /// The address of the tree contract
    pub address: Address,
    /// The block number at which the tree was created
    pub creation_block: u64,
    pub provider: ProviderConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    /// Ethereum RPC endpoint
    #[serde(with = "crate::serde_utils::url")]
    pub rpc_endpoint: Url,
    /// The maximum number of retries for rate limit errors
    #[serde(default = "default::max_rate_limit_retries")]
    pub max_rate_limit_retries: u32,
    /// The initial backoff in milliseconds
    #[serde(default = "default::initial_backoff")]
    pub initial_backoff: u64,
    /// The number of compute units per second for this provider
    #[serde(default = "default::compute_units_per_second")]
    pub compute_units_per_second: u64,
    #[serde(default = "default::window_size")]
    pub window_size: u64,
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

    pub fn socket_address() -> Option<SocketAddr> {
        Some(([127, 0, 0, 1], 8080).into())
    }

    pub const fn window_size() -> u64 {
        1000
    }

    pub const fn max_rate_limit_retries() -> u32 {
        1
    }

    pub const fn initial_backoff() -> u64 {
        100
    }

    pub const fn compute_units_per_second() -> u64 {
        10000
    }

    pub const fn bool_true() -> bool {
        true
    }
}

// Utility functions to convert map to vec
mod map_vec {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize};

    pub fn serialize<T, S>(
        values: &[T],
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
        T: Serialize,
    {
        let map: BTreeMap<String, &T> = values
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect();
        map.serialize(serializer)
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        let v: BTreeMap<String, T> = Deserialize::deserialize(deserializer)?;

        Ok(v.into_values().collect())
    }
}

#[cfg(test)]
mod tests {
    use alloy::primitives::address;
    use serde::de;

    use super::*;

    #[test]
    fn basic_serialize() {
        const S: &str = indoc::indoc! {r#"
            tree_depth = 10
            socket_address = "127.0.0.1:8080"

            [db]
            connection_string = "postgresql://user:password@localhost:5432/dbname"
            create = true
            migrate = true

            [canonical_tree]
            address = "0xb3e7771a6e2d7dd8c0666042b7a07c39b938eb7d"
            creation_block = 0

            [canonical_tree.provider]
            rpc_endpoint = "http://localhost:8545/"
            max_rate_limit_retries = 1
            initial_backoff = 100
            compute_units_per_second = 10000
            window_size = 10

            [cache]
            dir = ".world-tree.cache/"
            purge = true

            [bridged_trees.0]
            address = "0xb3e7771a6e2d7dd8c0666042b7a07c39b938eb7d"
            creation_block = 0

            [bridged_trees.0.provider]
            rpc_endpoint = "http://localhost:8546/"
            max_rate_limit_retries = 1
            initial_backoff = 100
            compute_units_per_second = 10000
            window_size = 10
        "#};

        let config = ServiceConfig {
            tree_depth: 10,
            db: DbConfig {
                connection_string:
                    "postgresql://user:password@localhost:5432/dbname"
                        .to_string(),
                migrate: true,
                create: true,
            },
            canonical_tree: TreeConfig {
                address: address!("b3e7771a6e2d7dd8c0666042b7a07c39b938eb7d"),
                creation_block: 0,
                provider: ProviderConfig {
                    rpc_endpoint: "http://localhost:8545".parse().unwrap(),
                    max_rate_limit_retries: 1,
                    initial_backoff: 100,
                    compute_units_per_second: 10000,
                    window_size: 10,
                },
            },
            cache: CacheConfig {
                dir: PathBuf::from(".world-tree.cache/"),
                purge: true,
            },
            bridged_trees: vec![TreeConfig {
                address: address!("b3e7771a6e2d7dd8c0666042b7a07c39b938eb7d"),
                creation_block: 0,
                provider: ProviderConfig {
                    rpc_endpoint: "http://localhost:8546".parse().unwrap(),
                    max_rate_limit_retries: 1,
                    initial_backoff: 100,
                    compute_units_per_second: 10000,
                    window_size: 10,
                },
            }],
            socket_address: Some(([127, 0, 0, 1], 8080).into()),
            telemetry: None,
        };

        let serialized = toml::to_string(&config).unwrap();
        assert_eq!(serialized.trim(), S.trim());
    }
}
