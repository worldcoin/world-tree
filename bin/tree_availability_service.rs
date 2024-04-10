use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use common::metrics::init_statsd_exporter;
use common::shutdown_tracer_provider;
use common::tracing::{init_datadog_subscriber, init_subscriber};
use ethers::providers::{Http, Provider};
use ethers_throttle::ThrottledJsonRpcClient;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tracing::Level;
use world_tree::tree::config::ServiceConfig;
use world_tree::tree::service::InclusionProofService;
use world_tree::tree::tree_manager::{BridgedTree, CanonicalTree, TreeManager};
use world_tree::tree::WorldTree;

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

    let world_tree = initialize_world_tree(&config).await?;

    let handles = InclusionProofService::new(world_tree)
        .serve(config.socket_address)
        .await?;

    let mut handles = handles.into_iter().collect::<FuturesUnordered<_>>();
    while let Some(result) = handles.next().await {
        tracing::error!("TreeAvailabilityError: {:?}", result);
        result?;
    }

    shutdown_tracer_provider();

    Ok(())
}

async fn initialize_world_tree(
    config: &ServiceConfig,
) -> eyre::Result<Arc<WorldTree<Provider<ThrottledJsonRpcClient<Http>>>>> {
    let canonical_provider_config = &config.canonical_tree.provider;

    let http_provider =
        Http::new(canonical_provider_config.rpc_endpoint.clone());
    let throttled_provider = ThrottledJsonRpcClient::new(
        http_provider,
        canonical_provider_config.throttle,
        None,
    );
    let canonical_middleware = Arc::new(Provider::new(throttled_provider));

    let canonical_tree_config = &config.canonical_tree;
    let canonical_tree_manager = TreeManager::<_, CanonicalTree>::new(
        canonical_tree_config.address,
        canonical_tree_config.window_size,
        canonical_tree_config.last_synced_block,
        canonical_middleware,
    )
    .await?;

    let mut bridged_tree_managers = vec![];

    if let Some(bridged_trees) = &config.bridged_trees {
        for tree_config in bridged_trees.iter() {
            let bridged_provider_config = &tree_config.provider;
            let http_provider =
                Http::new(bridged_provider_config.rpc_endpoint.clone());

            let throttled_provider = ThrottledJsonRpcClient::new(
                http_provider,
                bridged_provider_config.throttle,
                None,
            );
            let bridged_middleware =
                Arc::new(Provider::new(throttled_provider));

            let tree_manager = TreeManager::<_, BridgedTree>::new(
                tree_config.address,
                tree_config.window_size,
                tree_config.last_synced_block,
                bridged_middleware,
            )
            .await?;

            bridged_tree_managers.push(tree_manager);
        }
    }

    Ok(Arc::new(WorldTree::new(
        config.tree_depth,
        canonical_tree_manager,
        bridged_tree_managers,
        PathBuf::from("tree-cache"),
    )?))
}
