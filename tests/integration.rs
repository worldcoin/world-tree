use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use ethers::middleware::SignerMiddleware;
use ethers::providers::{Http, Middleware, Provider};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{Address, U256};
use ethers_throttle::ThrottledJsonRpcClient;
use eyre::ContextCompat;
use rand::Rng;
use semaphore::cascading_merkle_tree::CascadingMerkleTree;
use semaphore::poseidon_tree::PoseidonHash;
use semaphore::Field;
use tempfile::NamedTempFile;
use test_client::TestClient;
use testcontainers::core::{ContainerPort, Mount};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use world_tree::abi::IWorldIDIdentityManager;
use world_tree::init_world_tree;
use world_tree::tree::config::{
    CacheConfig, ProviderConfig, ServiceConfig, TreeConfig,
};
use world_tree::tree::error::WorldTreeError;
use world_tree::tree::service::InclusionProofService;

const TREE_DEPTH: usize = 20;

mod test_client;

#[tokio::test]
async fn integration() -> eyre::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let cache_file = NamedTempFile::new()?;

    let mainnet_container = setup_mainnet().await?;
    let mainnet_rpc_port = mainnet_container.get_host_port_ipv4(8545).await?;
    let mainnet_rpc_url = format!("http://127.0.0.1:{}", mainnet_rpc_port);

    let rollup_container = setup_rollup().await?;
    let _rollup_rpc_port = rollup_container.get_host_port_ipv4(8545).await?;
    let rollup_rpc_url = format!("http://127.0.0.1:{}", mainnet_rpc_port);

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
    wait_until_contracts_deployed(&mainnet_provider, bridged_address).await?;

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

    let first_batch = random_leaves(5);

    let world_id_manager = IWorldIDIdentityManager::new(
        id_manager_address,
        mainnet_signer.clone(),
    );

    let _bridged_world_id =
        IWorldIDIdentityManager::new(bridged_address, rollup_signer.clone());

    tree.extend_from_slice(&first_batch);

    let first_batch_root = tree.root();

    tracing::info!("Latest root = {:?}", world_id_manager.latest_root().await?);

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

    tracing::info!("Latest root = {:?}", world_id_manager.latest_root().await?);

    tracing::info!("Setting up world-tree service");
    let (world_tree_socket_addr, cancel_tx, handles) =
        setup_world_tree(&ServiceConfig {
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
            socket_address: ([127, 0, 0, 1], 0).into(),
            telemetry: None,
        })
        .await?;

    let client = TestClient::new(format!("http://{world_tree_socket_addr}"));

    let ip = client.inclusion_proof(&first_batch[0]).await?;
    tracing::info!(?ip, "Got first inclusion proof!");

    assert_eq!(
        ip.root, first_batch_root,
        "First inclusion proof root should batch to first batch"
    );

    tokio::time::sleep(Duration::from_secs_f32(1.0)).await;

    tracing::info!("Cancelling world-tree service");
    cancel_tx.send(())?;

    tracing::info!("Waiting for world-tree service to shutdown...");
    for handle in handles {
        // Ignore errors - channels might close causing recv errors at exit
        let _ = handle.await?;
    }

    Ok(())
}

async fn setup_world_tree(
    config: &ServiceConfig,
) -> eyre::Result<(
    SocketAddr,
    broadcast::Sender<()>,
    Vec<
        JoinHandle<
            Result<(), WorldTreeError<Provider<ThrottledJsonRpcClient<Http>>>>,
        >,
    >,
)> {
    let world_tree = init_world_tree(config).await?;

    let service = InclusionProofService::new(world_tree);
    let cancel_tx = service.cancel_tx.clone();

    let (socket_addr, handles) =
        service.bind_serve(config.socket_address).await?;

    Ok((socket_addr, cancel_tx, handles))
}

async fn setup_mainnet() -> eyre::Result<ContainerAsync<GenericImage>> {
    setup_chain("runMainnet.sh").await
}

async fn setup_rollup() -> eyre::Result<ContainerAsync<GenericImage>> {
    setup_chain("runRollup.sh").await
}

async fn setup_chain(
    script_file: &str,
) -> eyre::Result<ContainerAsync<GenericImage>> {
    let current_path = std::env::current_dir()?;
    let mount_dir = current_path.join("tests/fixtures/integration_contracts");
    let mount_dir = mount_dir.canonicalize()?;
    let mount_dir = mount_dir.to_str().context("Invalid path")?;

    let container = GenericImage::new("ghcr.io/foundry-rs/foundry", "latest")
        .with_entrypoint("/bin/sh")
        .with_exposed_port(ContainerPort::Tcp(8545))
        .with_mount(Mount::bind_mount(mount_dir, "/app"))
        .with_cmd(["-c", &format!("cd /app; ./{script_file}")])
        .start()
        .await?;

    Ok(container)
}

async fn wait_until_contracts_deployed(
    provider: &Provider<Http>,
    address: Address,
) -> eyre::Result<()> {
    const MAX_RETRIES: usize = 10;
    const SLEEP_DURATION: Duration = Duration::from_secs(1);

    for _ in 0..MAX_RETRIES {
        let resp = provider.get_code(address, None).await;

        match resp {
            Ok(code) if !code.is_empty() => return Ok(()),
            Ok(_) => {
                tracing::warn!("Contracts not deployed yet");
            }
            Err(err) => {
                tracing::warn!(%err, err_debug = ?err, "Failed to get code");
            }
        }

        tokio::time::sleep(SLEEP_DURATION).await;
    }

    eyre::bail!("Contracts not deployed")
}

fn random_leaves(n: usize) -> Vec<Field> {
    let mut rng = rand::thread_rng();

    (0..n)
        .map(|_| {
            let mut limbs = [0u64; 4];

            limbs[0] = rng.gen();
            limbs[1] = rng.gen();
            limbs[2] = rng.gen();

            Field::from_limbs(limbs)
        })
        .collect()
}

fn f2ethers(f: Field) -> U256 {
    U256::from_little_endian(&f.as_le_bytes())
}
