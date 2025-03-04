use alloy::primitives::address;
use world_tree::tree::config::ProviderConfig;

mod common;

use common::*;
use world_tree::tree::error::WorldTreeResult;
use world_tree::tree::provider;

#[tokio::test]
async fn deploy_contracts() -> WorldTreeResult<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let mainnet_container = setup_mainnet().await?;
    let mainnet_rpc_port = mainnet_container.get_host_port_ipv4(8545).await?;
    let mainnet_rpc_url = format!("http://127.0.0.1:{mainnet_rpc_port}");

    let rollup_container = setup_rollup().await?;
    let rollup_rpc_port = rollup_container.get_host_port_ipv4(8545).await?;
    let rollup_rpc_url = format!("http://127.0.0.1:{rollup_rpc_port}");

    // The addresses are the same since we use the same account on both networks
    let id_manager_address =
        address!("5FbDB2315678afecb367f032d93F642f64180aa3");
    let bridged_address = address!("5FbDB2315678afecb367f032d93F642f64180aa3");

    let mainnet_provider = provider(&ProviderConfig {
        rpc_endpoint: mainnet_rpc_url.parse()?,
        max_rate_limit_retries: 1,
        compute_units_per_second: 10000,
        initial_backoff: 100,
        window_size: 10,
    })
    .await?;

    let rollup_provider = provider(&ProviderConfig {
        rpc_endpoint: rollup_rpc_url.parse()?,
        max_rate_limit_retries: 1,
        compute_units_per_second: 10000,
        initial_backoff: 100,
        window_size: 10,
    })
    .await?;

    tracing::info!("Waiting for contracts to deploy...");
    wait_until_contracts_deployed(&mainnet_provider, id_manager_address)
        .await?;
    tracing::info!("Waiting for rollup contracts to deploy...");
    wait_until_contracts_deployed(&rollup_provider, bridged_address).await?;

    Ok(())
}
