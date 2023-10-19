use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;

use clap::Parser;
use ethers::providers::{Http, Provider};
use ethers::types::H160;
use tree_availability::TreeAvailabilityService;

#[derive(Parser, Debug)]
#[clap(name = "Tree Availability Service", about = "")]
struct Opts {
    #[clap(short, long, help = "")]
    tree_depth: usize,
    #[clap(short, long, help = "")]
    dense_prefix_depth: usize,
    #[clap(short, long, help = "")]
    tree_history_size: usize,
    #[clap(short, long, help = "")]
    world_tree_address: String,
    #[clap(short, long, help = "")]
    world_tree_creation_block: u64,
    #[clap(short, long, help = "")]
    rpc_endpoint: String,
    #[clap(short, long, help = "")]
    port: Option<u16>,
}

#[tokio::main]
pub async fn main() -> eyre::Result<()> {
    let opts = Opts::parse();

    let middleware = Arc::new(Provider::<Http>::try_from(opts.rpc_endpoint)?);
    let world_tree_address = H160::from_str(&opts.world_tree_address)?;

    let handles = TreeAvailabilityService::new(
        opts.tree_depth,
        opts.dense_prefix_depth,
        opts.tree_history_size,
        world_tree_address,
        opts.world_tree_creation_block,
        middleware,
    )
    .serve(opts.port)
    .await;

    for handle in handles {
        handle.await??;
    }

    Ok(())
}
