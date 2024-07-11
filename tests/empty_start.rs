use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use ethers::middleware::SignerMiddleware;
use ethers::providers::{Http, Middleware, Provider};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{Address, U256};
use eyre::ContextCompat;
use semaphore::cascading_merkle_tree::CascadingMerkleTree;
use semaphore::poseidon_tree::PoseidonHash;
use semaphore::Field;
use tempfile::NamedTempFile;
use world_tree::abi::{IBridgedWorldID, IWorldIDIdentityManager};
use world_tree::tree::config::{
    CacheConfig, ProviderConfig, ServiceConfig, TreeConfig,
};

const TREE_DEPTH: usize = 20;

mod common;

use common::*;

#[tokio::test]
async fn empty_start() -> eyre::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let cache_file = NamedTempFile::new()?;

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
    let id_manager_address: Address =
        "0x5FbDB2315678afecb367f032d93F642f64180aa3".parse()?;
    let bridged_address: Address =
        "0x5FbDB2315678afecb367f032d93F642f64180aa3".parse()?;

    let mainnet_provider = Provider::<Http>::new(mainnet_rpc_url.parse()?);
    let rollup_provider = Provider::<Http>::new(rollup_rpc_url.parse()?);

    tracing::info!("Waiting for contracts to deploy...");
    wait_until_contracts_deployed(&mainnet_provider, id_manager_address)
        .await?;
    wait_until_contracts_deployed(&rollup_provider, bridged_address).await?;

    let mainnet_chain_id = mainnet_provider.get_chainid().await?;
    let rollup_chain_id = rollup_provider.get_chainid().await?;

    let wallet = LocalWallet::from_str(
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
    )?;
    let mainnet_wallet =
        wallet.clone().with_chain_id(mainnet_chain_id.as_u64());
    let rollup_wallet = wallet.with_chain_id(rollup_chain_id.as_u64());

    let mainnet_signer =
        SignerMiddleware::new(mainnet_provider, mainnet_wallet);
    let mainnet_signer = Arc::new(mainnet_signer);

    let rollup_signer = SignerMiddleware::new(rollup_provider, rollup_wallet);
    let rollup_signer = Arc::new(rollup_signer);

    let world_id_manager = IWorldIDIdentityManager::new(
        id_manager_address,
        mainnet_signer.clone(),
    );

    let bridged_world_id =
        IBridgedWorldID::new(bridged_address, rollup_signer.clone());

    tracing::info!("Setting up world-tree service");

    let service_config = ServiceConfig {
        tree_depth: TREE_DEPTH,
        canonical_tree: TreeConfig {
            address: id_manager_address,
            window_size: 10,
            creation_block: 0,
            provider: ProviderConfig {
                rpc_endpoint: mainnet_rpc_url.parse()?,
                throttle: 150,
            },
        },
        cache: CacheConfig {
            cache_file: cache_file.path().to_path_buf(),
            purge_cache: true,
        },
        bridged_trees: vec![TreeConfig {
            address: bridged_address,
            window_size: 10,
            creation_block: 0,
            provider: ProviderConfig {
                rpc_endpoint: rollup_rpc_url.parse()?,
                throttle: 150,
            },
        }],
        socket_address: None,
        telemetry: None,
    };

    let (local_addr, handles) = setup_world_tree(&service_config).await?;
    let client =
        TestClient::new(format!("http://127.0.0.1:{}", local_addr.port()));

    let first_batch = random_leaves(5);
    tree.extend_from_slice(&first_batch);
    let first_batch_root = tree.root();

    // We need some initial test data to start
    world_id_manager
        .register_identities(
            [U256::zero(); 8],
            f2ethers(initial_root), // pre root,
            0,                      // start index
            first_batch.iter().cloned().map(f2ethers).collect(), // commitments
            f2ethers(first_batch_root), // post root
        )
        .send()
        .await?;

    bridged_world_id
        .receive_root(f2ethers(first_batch_root))
        .send()
        .await?;

    let ip = attempt_async! {
        async {
            client
                .inclusion_proof(&first_batch[0])
                .await
                .and_then(|maybe_proof| maybe_proof.context("Missing inclusion proof"))
        }
    };

    assert_eq!(
        ip.root, first_batch_root,
        "Should fetch inclusion proof for first batch"
    );

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
