use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use common::metrics::init_statsd_exporter;
use common::shutdown_tracer_provider;
use common::tracing::{init_datadog_subscriber, init_subscriber};
use ethers::providers::{Http, Provider};
use ethers_throttle::ThrottledProvider;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use governor::Jitter;
use tracing::Level;
use world_tree::tree::config::ServiceConfig;
use world_tree::tree::service::InclusionProofService;

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
    // dotenv::dotenv().ok();

    // let opts = Opts::parse();

    // let config = ServiceConfig::load(opts.config.as_deref())?;

    // if opts.datadog {
    //     init_datadog_subscriber(SERVICE_NAME, Level::INFO);
    //     init_statsd_exporter(METRICS_HOST, METRICS_PORT);
    // } else {
    //     init_subscriber(Level::INFO);
    // }

    // let http_provider = Http::new(config.provider.rpc_endpoint);

    // let throttled_http_provider = ThrottledProvider::new(
    //     http_provider,
    //     config.provider.throttle.unwrap_or(u32::MAX),
    //     Some(Jitter::new(
    //         Duration::from_millis(10),
    //         Duration::from_millis(100),
    //     )),
    // );

    // let middleware = Arc::new(Provider::new(throttled_http_provider));

    // let handles = InclusionProofService::new(
    //     config.world_tree.tree_depth,
    //     config.world_tree.dense_prefix_depth,
    //     config.world_tree.tree_history_size,
    //     config.world_tree.world_id_contract_address,
    //     config.world_tree.creation_block,
    //     config.world_tree.window_size,
    //     middleware,
    // )
    // .serve(config.world_tree.socket_address);

    // let mut handles = handles.into_iter().collect::<FuturesUnordered<_>>();
    // while let Some(result) = handles.next().await {
    //     tracing::error!("TreeAvailabilityError: {:?}", result);
    //     result??;
    // }

    // shutdown_tracer_provider();

    Ok(())
}
