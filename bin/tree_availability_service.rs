use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use common::metrics::init_statsd_exporter;
use common::shutdown_tracer_provider;
use common::tracing::{init_datadog_subscriber, init_subscriber};
use ethers::providers::{Http, Middleware, Provider};
use ethers_throttle::{Throttle, ThrottledProvider};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use governor::Jitter;
use tracing::Level;
use world_tree::tree::config::{ServiceConfig, WorldTreeConfig};
use world_tree::tree::identity_tree::{self, IdentityTree};
use world_tree::tree::service::InclusionProofService;
use world_tree::tree::tree_manager::{BridgedTree, CanonicalTree, TreeManager};

/// This service syncs the state of the World Tree and spawns a server that can deliver inclusion proofs for a given identity.
#[derive(Parser, Debug)]
#[clap(name = "Tree Availability Service")]
#[clap(version)]
struct Opts {
    /// Path to the configuration file
    #[clap(short, long)]
    config: Option<PathBuf>,

    /// Enable datadog backend for instrumentation
    #[clap(long, env)]
    datadog: bool,
}

const SERVICE_NAME: &str = "tree-availability-service";
const METRICS_HOST: &str = "localhost";
const METRICS_PORT: u16 = 8125;

#[tokio::main]
pub async fn main() -> eyre::Result<()> {
    dotenv::dotenv().ok();

    let opts = Opts::parse();

    let config = ServiceConfig::load(opts.config.as_deref())?;

    if opts.datadog {
        init_datadog_subscriber(SERVICE_NAME, Level::INFO);
        init_statsd_exporter(METRICS_HOST, METRICS_PORT);
    } else {
        init_subscriber(Level::INFO);
    }

    let identity_tree = initialize_identity_tree(&config.world_tree).await?;

    let handles = InclusionProofService::new(identity_tree)
        .serve(config.world_tree.socket_address);

    let mut handles = handles.into_iter().collect::<FuturesUnordered<_>>();
    while let Some(result) = handles.next().await {
        tracing::error!("TreeAvailabilityError: {:?}", result);
        result??;
    }

    shutdown_tracer_provider();

    Ok(())
}

async fn initialize_identity_tree(
    config: &WorldTreeConfig,
) -> eyre::Result<IdentityTree<Provider<Http>>> {
    let http_provider =
        Http::new(config.canonical_tree.provider.rpc_endpoint.clone());
    let canonical_middleware = Arc::new(Provider::new(http_provider));

    let canonical_tree_config = &config.canonical_tree;
    let canonical_tree_manager = TreeManager::<_, CanonicalTree>::new(
        canonical_tree_config.address,
        canonical_tree_config.window_size,
        canonical_tree_config.last_synced_block,
        canonical_middleware,
    )
    .await?;

    let mut bridged_tree_managers = vec![];
    for tree_config in config.bridged_trees.iter() {
        let http_provider =
            Http::new(tree_config.provider.rpc_endpoint.clone());
        let middleware = Arc::new(Provider::new(http_provider));

        let tree_manager = TreeManager::<_, BridgedTree>::new(
            tree_config.address,
            tree_config.window_size,
            tree_config.last_synced_block,
            middleware,
        )
        .await?;

        bridged_tree_managers.push(tree_manager);
    }

    Ok(IdentityTree::new(
        config.tree_depth,
        canonical_tree_manager,
        bridged_tree_managers,
    ))
}
