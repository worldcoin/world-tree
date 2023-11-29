use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use common::metrics::init_statsd_exporter;
use common::shutdown_tracer_provider;
use common::tracing::{init_datadog_subscriber, init_subscriber};
use ethers::providers::{Http, Provider};
use ethers::types::H160;
use ethers_throttle::ThrottledProvider;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use governor::Jitter;
use tracing::Level;
use url::Url;
use world_tree::tree::service::TreeAvailabilityService;

#[derive(Parser, Debug)]
#[clap(
    name = "Tree Availability Service",
    about = "This service syncs the state of the World Tree and spawns a server that can deliver inclusion proofs for a given identity."
)]
struct Opts {
    #[clap(long, help = "Depth of the World Tree")]
    tree_depth: usize,
    #[clap(
        long,
        help = "Quantity of recent tree changes to cache. This allows inclusion proof requests to specify a historical root"
    )]
    tree_history_size: usize,
    #[clap(
        short,
        long,
        help = "Depth of merkle tree that should be represented as a densely populated prefix. The remainder of the tree will be represented with pointer-based structures."
    )]
    dense_prefix_depth: usize,
    #[clap(short, long, help = "Address of the World Tree")]
    address: H160,
    #[clap(short, long, help = "Creation block of the World Tree")]
    creation_block: u64,
    #[clap(
        short,
        long,
        help = "Maximum window size when scanning blocks for TreeChanged events",
        default_value = "1000"
    )]
    window_size: u64,
    #[clap(short, long, help = "Ethereum RPC endpoint")]
    rpc_endpoint: String,
    #[clap(
        short,
        long,
        help = "Port to expose for the tree-availability-service API",
        default_value = "8080"
    )]
    port: u16,
    #[clap(long, help = "Enable datadog backend for instrumentation")]
    datadog: bool,

    #[clap(
        short,
        long,
        help = "Request per minute limit for rpc endpoint",
        default_value = "0"
    )]
    throttle: u32,
}

const SERVICE_NAME: &str = "tree-availability-service";
const METRICS_HOST: &str = "localhost";
const METRICS_PORT: u16 = 8125;

#[tokio::main]
pub async fn main() -> eyre::Result<()> {
    let opts = Opts::parse();

    if opts.datadog {
        init_datadog_subscriber(SERVICE_NAME, Level::INFO);
        init_statsd_exporter(METRICS_HOST, METRICS_PORT);
    } else {
        init_subscriber(Level::INFO);
    }

    let http_provider = Http::new(Url::parse(&opts.rpc_endpoint)?);
    let throttled_http_provider = ThrottledProvider::new(
        http_provider,
        opts.throttle,
        Some(Jitter::new(
            Duration::from_millis(10),
            Duration::from_millis(100),
        )),
    );
    let middleware = Arc::new(Provider::new(throttled_http_provider));

    let handles = TreeAvailabilityService::new(
        opts.tree_depth,
        opts.dense_prefix_depth,
        opts.tree_history_size,
        opts.address,
        opts.creation_block,
        opts.window_size,
        middleware,
    )
    .serve(opts.port);

    let mut handles = handles.into_iter().collect::<FuturesUnordered<_>>();
    while let Some(result) = handles.next().await {
        tracing::error!("TreeAvailabilityError: {:?}", result);
        result??;
    }

    shutdown_tracer_provider();

    Ok(())
}
