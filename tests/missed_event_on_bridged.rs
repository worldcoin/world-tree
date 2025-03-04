use std::sync::Arc;
use std::time::Duration;

use alloy::network::EthereumWallet;
use alloy::primitives::{address, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::k256::ecdsa::SigningKey;
use alloy::signers::local::LocalSigner;
use eyre::ContextCompat;
use semaphore::cascading_merkle_tree::CascadingMerkleTree;
use semaphore::poseidon_tree::PoseidonHash;
use semaphore::Field;
use tempfile::TempDir;
use world_tree::abi::{IBridgedWorldID, IWorldIDIdentityManager};
use world_tree::tree::config::{
    CacheConfig, ProviderConfig, ServiceConfig, TreeConfig,
};

const TREE_DEPTH: usize = 20;

mod common;

use common::*;
use world_tree::tree::error::WorldTreeResult;
use world_tree::tree::provider;

#[tokio::test]
async fn missing_event_on_bridged() -> WorldTreeResult<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let cache_dir = TempDir::new()?;

    let (db_config, _db_container) = setup_db().await?;

    let mainnet_container = setup_mainnet().await?;
    let mainnet_rpc_port = mainnet_container.get_host_port_ipv4(8545).await?;
    let mainnet_rpc_url = format!("http://127.0.0.1:{mainnet_rpc_port}");

    let rollup_container = setup_rollup().await?;
    let rollup_rpc_port = rollup_container.get_host_port_ipv4(8545).await?;
    let rollup_rpc_url = format!("http://127.0.0.1:{rollup_rpc_port}");

    let mut tree = CascadingMerkleTree::<PoseidonHash, _>::new(
        vec![],
        TREE_DEPTH,
        &Field::ZERO,
    );

    let initial_root = tree.root();
    tracing::info!(?initial_root, "Initial root",);

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
    wait_until_contracts_deployed(
        &mainnet_container,
        &mainnet_provider,
        id_manager_address,
    )
    .await?;
    wait_until_contracts_deployed(
        &rollup_container,
        &rollup_provider,
        bridged_address,
    )
    .await?;

    let rollup_chain_id = rollup_provider.get_chain_id().await?;

    let wallet = EthereumWallet::from(
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
            .parse::<LocalSigner<SigningKey>>()?,
    );

    let mainnet_signer = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet.clone())
        .on_http(mainnet_rpc_url.parse()?);
    let mainnet_signer = Arc::new(mainnet_signer);

    let rollup_signer = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(rollup_rpc_url.parse()?);

    let first_batch = random_leaves(5);
    tree.extend_from_slice(&first_batch);
    let first_batch_root = tree.root();
    let second_batch = random_leaves(5);
    tree.extend_from_slice(&second_batch);
    let second_batch_root = tree.root();

    let world_id_manager = IWorldIDIdentityManager::new(
        id_manager_address,
        mainnet_signer.clone(),
    );

    let bridged_world_id =
        IBridgedWorldID::new(bridged_address, rollup_signer.clone());

    tracing::info!("Setting up world-tree service");

    let service_config = ServiceConfig {
        tree_depth: TREE_DEPTH,
        db: db_config,
        canonical_tree: TreeConfig {
            address: id_manager_address,
            creation_block: 0,
            provider: ProviderConfig {
                rpc_endpoint: mainnet_rpc_url.parse()?,
                max_rate_limit_retries: 1,
                compute_units_per_second: 10000,
                initial_backoff: 100,
                window_size: 10,
            },
        },
        cache: CacheConfig {
            dir: cache_dir.path().to_path_buf(),
            purge: true,
        },
        bridged_trees: vec![TreeConfig {
            address: bridged_address,
            creation_block: 0,
            provider: ProviderConfig {
                rpc_endpoint: rollup_rpc_url.parse()?,
                max_rate_limit_retries: 1,
                compute_units_per_second: 10000,
                initial_backoff: 100,
                window_size: 10,
            },
        }],
        socket_address: None,
        telemetry: None,
        shutdown_delay: Duration::from_secs(1),
    };

    let (local_addr, handles) = setup_world_tree(&service_config).await?;
    let client =
        TestClient::new(format!("http://127.0.0.1:{}", local_addr.port()));

    // Publish the first batch on mainnet
    world_id_manager
        .registerIdentities(
            [U256::ZERO; 8],
            f2u256(initial_root), // pre root,
            0,                    // start index
            first_batch.iter().cloned().map(f2u256).collect(), // commitments
            f2u256(first_batch_root), // post root
        )
        .send()
        .await?
        .get_receipt()
        .await?;

    tokio::time::sleep(Duration::from_secs(2)).await;

    // Publish the second batch on mainnet
    world_id_manager
        .registerIdentities(
            [U256::ZERO; 8],
            f2u256(first_batch_root), // pre root,
            first_batch.len() as u32, // start index
            second_batch.iter().cloned().map(f2u256).collect(), // commitments
            f2u256(second_batch_root), // post root
        )
        .send()
        .await?
        .get_receipt()
        .await?;

    // Publish the second batch on bridged network
    bridged_world_id
        .receiveRoot(f2u256(second_batch_root))
        .send()
        .await?
        .get_receipt()
        .await?;

    let ip_bridged = attempt_async! {
        async {
            client
                .inclusion_proof_by_chain_id(&first_batch[0], rollup_chain_id)
                .await
                .and_then(|maybe_proof| maybe_proof.context("Missing inclusion proof"))
        }
    };

    let ip = client
        .inclusion_proof_by_chain_id(&first_batch[0], rollup_chain_id)
        .await?
        .context("Missing inclusion proof")?;

    assert_eq!(ip.root, ip_bridged.root);
    assert_eq!(ip_bridged.root, second_batch_root);

    tracing::info!("Waiting for world-tree service to shutdown...");
    for handle in handles {
        handle.abort();
    }

    tracing::info!("Shutting down mainnet container...");
    mainnet_container.stop().await?;
    tracing::info!("Shutting down rollup container...");
    rollup_container.stop().await?;

    Ok(())
}
