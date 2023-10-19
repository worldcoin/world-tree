use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;

use clap::Parser;
use ethers::providers::{Http, Provider};
use ethers::types::H160;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tree_availability::TreeAvailabilityService;

#[derive(Parser, Debug)]
#[clap(name = "Tree Availability Service", about = "")]
struct Opts {
    #[clap(long, help = "")]
    tree_depth: usize, //TODO: we might be able to fetch tree depth on chain somewhere
    #[clap(long, help = "")]
    tree_history_size: usize,
    #[clap(short, long, help = "")]
    dense_prefix_depth: usize,
    #[clap(short, long, help = "")]
    address: H160,
    #[clap(short, long, help = "")]
    creation_block: u64,
    #[clap(short, long, help = "")]
    rpc_endpoint: String,
    #[clap(short, long, help = "")]
    port: Option<u16>,
}

#[tokio::main]
pub async fn main() -> eyre::Result<()> {
    let opts = Opts::parse();
    dbg!("Parsed opts");

    let middleware = Arc::new(Provider::<Http>::try_from(opts.rpc_endpoint)?);
    let handles = TreeAvailabilityService::new(
        opts.tree_depth,
        opts.dense_prefix_depth,
        opts.tree_history_size,
        opts.address,
        opts.creation_block,
        middleware,
    )
    .serve(opts.port)
    .await;

    let mut handles = handles.into_iter().collect::<FuturesUnordered<_>>();
    while let Some(result) = handles.next().await {
        result??;
    }

    Ok(())
}
