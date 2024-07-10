use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use ethers::providers::{Http, Provider};
use ethers_throttle::ThrottledJsonRpcClient;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use opentelemetry;
use opentelemetry_datadog::DatadogPropagator;
use telemetry_batteries::metrics::statsd::StatsdBattery;
use telemetry_batteries::tracing::datadog::DatadogBattery;
use telemetry_batteries::tracing::TracingShutdownHandle;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use world_tree::tree::config::ServiceConfig;
use world_tree::tree::service::InclusionProofService;
#[cfg(feature = "xxdk")]
use world_tree::tree::service::CmixInclusionProofService;
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
}

#[tokio::main]
pub async fn main() -> eyre::Result<()> {
    dotenv::dotenv().ok();

    let opts = Opts::parse();

    let config = ServiceConfig::load(opts.config.as_deref())?;

    let _tracing_shutdown_handle = if let Some(telemetry) = &config.telemetry {
        opentelemetry::global::set_text_map_propagator(DatadogPropagator::new());
        let tracing_shutdown_handle = DatadogBattery::init(
            telemetry.traces_endpoint.as_deref(),
            &telemetry.service_name,
            None,
            true,
        );

        if let Some(metrics_config) = &telemetry.metrics {
            StatsdBattery::init(
                &metrics_config.host,
                metrics_config.port,
                metrics_config.queue_size,
                metrics_config.buffer_size,
                Some(&metrics_config.prefix),
            )?;
        }

        tracing_shutdown_handle
    } else {
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().pretty().compact())
            .with(tracing_subscriber::EnvFilter::from_default_env())
            .init();

        TracingShutdownHandle
    };

    tracing::info!(?config, "Starting World Tree service");

    let world_tree = initialize_world_tree(&config).await?;

    #[allow(unused_mut)]
    let mut handles = InclusionProofService::new(world_tree.clone())
        .serve(config.socket_address)
        .await?;

    #[cfg(feature = "xxdk")]
    handles.push(
        CmixInclusionProofService::new(world_tree)
            .serve(config.cmix)
            .await?,
    );

    let mut handles = handles.into_iter().collect::<FuturesUnordered<_>>();
    while let Some(result) = handles.next().await {
        tracing::error!("TreeAvailabilityError: {:?}", result);
        result??;
    }

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
        canonical_tree_config.creation_block,
        canonical_middleware,
    )
    .await?;

    let mut bridged_tree_managers = vec![];

    for tree_config in config.bridged_trees.iter() {
        let bridged_provider_config = &tree_config.provider;
        let http_provider =
            Http::new(bridged_provider_config.rpc_endpoint.clone());

        let throttled_provider = ThrottledJsonRpcClient::new(
            http_provider,
            bridged_provider_config.throttle,
            None,
        );
        let bridged_middleware = Arc::new(Provider::new(throttled_provider));

        let tree_manager = TreeManager::<_, BridgedTree>::new(
            tree_config.address,
            tree_config.window_size,
            tree_config.creation_block,
            bridged_middleware,
        )
        .await?;

        bridged_tree_managers.push(tree_manager);
    }

    if config.cache.purge_cache {
        tracing::info!("Purging tree cache");
        fs::remove_file(&config.cache.cache_file)?;
    }

    Ok(Arc::new(WorldTree::new(
        config.tree_depth,
        canonical_tree_manager,
        bridged_tree_managers,
        &config.cache.cache_file,
    )?))
}
