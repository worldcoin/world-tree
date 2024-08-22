use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use ethers::middleware::SignerMiddleware;
use ethers::providers::{Http, Middleware, Provider};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{Address, U256};
use eyre::ContextCompat;
use rand::Rng;
use semaphore::cascading_merkle_tree::CascadingMerkleTree;
use semaphore::poseidon_tree::PoseidonHash;
use semaphore::Field;
use tempfile::NamedTempFile;
use world_tree::abi::{IBridgedWorldID, IWorldIDIdentityManager};
use world_tree::tree::config::{
    CacheConfig, ProviderConfig, ServiceConfig, TreeConfig,
};

const TREE_DEPTH: usize = 30;

mod common;

use common::*;
use world_tree::tree::error::WorldTreeResult;

const NUM_BATCHES: usize = 40;
const BATCH_SIZE: usize = 10;

#[tokio::test]
async fn many_batches() -> WorldTreeResult<()> {
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
            creation_block: 0,
            provider: ProviderConfig {
                rpc_endpoint: mainnet_rpc_url.parse()?,
                throttle: 150,
                window_size: 10,
            },
        },
        cache: CacheConfig {
            cache_file: cache_file.path().to_path_buf(),
            purge_cache: true,
        },
        bridged_trees: vec![TreeConfig {
            address: bridged_address,
            creation_block: 0,
            provider: ProviderConfig {
                rpc_endpoint: rollup_rpc_url.parse()?,
                throttle: 150,
                window_size: 10,
            },
        }],
        socket_address: None,
        telemetry: None,
    };

    let (local_addr, handles) = setup_world_tree(&service_config).await?;
    let client =
        TestClient::new(format!("http://127.0.0.1:{}", local_addr.port()));

    // Each batch is (Pre Root, Vec<Leaf>, Post Root)
    let mut batches = vec![];

    tracing::info!("Prepare batches");
    for _i in 0..NUM_BATCHES {
        let leaves = random_leaves(BATCH_SIZE);
        let pre_root = tree.root();
        tree.extend_from_slice(&leaves);
        let post_root = tree.root();
        batches.push((pre_root, leaves.clone(), post_root));

        tracing::info!(?pre_root, ?post_root, "Batch prepared");
        tracing::debug!(?pre_root, ?post_root, ?leaves, "Batch");
    }

    let (batch_idx_tx, mut batch_idx_rx) = tokio::sync::mpsc::channel(1);

    let submit_batches_on_mainnet = async {
        let mut rng = rand::thread_rng();

        for (i, (pre_root, leaves, post_root)) in batches.iter().enumerate() {
            tracing::info!(?pre_root, ?post_root, "Publishing batch");

            world_id_manager
                .register_identities(
                    [U256::zero(); 8],
                    f2ethers(*pre_root),     // pre root,
                    (BATCH_SIZE * i) as u32, // start index
                    leaves.iter().cloned().map(f2ethers).collect(), // commitments
                    f2ethers(*post_root),                           // post root
                )
                .send()
                .await?;

            let wait_offset: f32 = rng.gen();
            tokio::time::sleep(Duration::from_secs_f32(2.0 + wait_offset))
                .await;

            batch_idx_tx.send(i).await?;
        }

        drop(batch_idx_tx);

        tracing::info!("All batches submitted");
        eyre::Result::<()>::Ok(())
    };

    let submit_batches_on_bridged = async {
        let mut rng = rand::thread_rng();

        for (pre_root, _leaves, post_root) in batches.iter() {
            tracing::info!(?pre_root, ?post_root, "Bridging batch");

            // Receive root on bridged network first
            bridged_world_id
                .receive_root(f2ethers(*post_root))
                .send()
                .await?;

            let wait_offset: f32 = rng.gen();
            tokio::time::sleep(Duration::from_secs_f32(2.0 + wait_offset))
                .await;
        }

        tracing::info!("All batches bridged");
        eyre::Result::<()>::Ok(())
    };

    let check_batches = async {
        while let Some(batch_idx) = batch_idx_rx.recv().await {
            let (pre_root, batch_leaves, post_root) = &batches[batch_idx];

            tracing::info!(?pre_root, ?post_root, "Checking batch");

            for leaf in batch_leaves {
                let ip = attempt_async! {
                    async {
                        tracing::info!(?leaf, "Fetching inclusion proof for leaf");
                        client
                            .inclusion_proof(leaf)
                            .await
                            .and_then(|maybe_proof| maybe_proof.context("Missing inclusion proof"))
                    }
                };

                assert!(ip.verify(*leaf));
            }
        }

        tracing::info!("All batches verified");
        eyre::Result::<()>::Ok(())
    };

    let (submit_batches_on_mainnet, submit_batches_on_bridged, check_batches) = tokio::join!(
        submit_batches_on_mainnet,
        submit_batches_on_bridged,
        check_batches
    );

    submit_batches_on_mainnet?;
    submit_batches_on_bridged?;
    check_batches?;

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
