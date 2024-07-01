use std::net::{TcpListener};
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

use tokio::task::JoinHandle;
use world_tree::abi::{IBridgedWorldID, IWorldIDIdentityManager};
use world_tree::init_world_tree;
use world_tree::tree::config::{
    CacheConfig, ProviderConfig, ServiceConfig, TreeConfig,
};
use world_tree::tree::error::WorldTreeError;
use world_tree::tree::service::InclusionProofService;

const TREE_DEPTH: usize = 20;

mod test_client;

// Attempts a given block multiple times
// panics if the block fails (evaluates to an error) more than 10 times
macro_rules! attempt_async {
    ($e:expr) => {
        {
            const MAX_ATTEMPTS: usize = 10;
            let mut attempt = 0;

            loop {
                let res = $e.await;

                if attempt >= MAX_ATTEMPTS {
                    panic!("Too many attempts");
                }

                match res {
                    Ok(res) => break res,
                    Err(err) => {
                        attempt += 1;
                        tracing::warn!(attempt, %err, "Attempt failed");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    };
}

#[tokio::test]
async fn integration() -> eyre::Result<()> {
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

    let first_batch = random_leaves(5);

    let world_id_manager = IWorldIDIdentityManager::new(
        id_manager_address,
        mainnet_signer.clone(),
    );

    let bridged_world_id =
        IBridgedWorldID::new(bridged_address, rollup_signer.clone());

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

    tokio::time::sleep(Duration::from_secs(4)).await;

    tracing::info!("Setting up world-tree service");

    let listener = TcpListener::bind("0.0.0.0:0")?;
    let world_tree_port = listener.local_addr()?.port();

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
        socket_address: ([127, 0, 0, 1], world_tree_port).into(),
        telemetry: None,
    };

    let handles = setup_world_tree(&service_config).await?;
    let client =
        TestClient::new(format!("http://127.0.0.1:{}", world_tree_port));

    let ip = client
        .inclusion_proof(&first_batch[0])
        .await?
        .context("Missing inclusion proof")?;
    tracing::debug!(?ip, "Got first inclusion proof");

    assert_eq!(
        ip.root, first_batch_root,
        "First inclusion proof root should batch to first batch"
    );

    let ip_for_mainnet = client
        .inclusion_proof_by_chain_id(&first_batch[0], mainnet_chain_id.as_u64())
        .await?
        .context("Missing inclusion proof")?;

    tracing::debug!(?ip_for_mainnet, "Got first inclusion proof for mainnet");

    assert_eq!(
        ip_for_mainnet.root, first_batch_root,
        "First inclusion proof root should batch to first batch"
    );

    let ip_for_bridged = client
        .inclusion_proof_by_chain_id(&first_batch[0], rollup_chain_id.as_u64())
        .await?
        .context("Missing inclusion proof")?;

    tracing::debug!(?ip_for_bridged, "Got first inclusion proof for bridged");

    assert_eq!(
        ip_for_bridged.root, first_batch_root,
        "First inclusion proof bridged should batch to first batch"
    );

    let second_batch = random_leaves(5);
    tree.extend_from_slice(&second_batch);
    let second_batch_root = tree.root();

    // Publish the second batch
    world_id_manager
        .register_identities(
            [U256::zero(); 8],
            f2ethers(first_batch_root), // pre root,
            first_batch.len() as u32,   // start index
            second_batch.iter().cloned().map(f2ethers).collect(), // commitments
            f2ethers(second_batch_root), // post root
        )
        .send()
        .await?;

    // Repeated attempts until the new root & batch is available on mainnet
    let ip_for_mainnet = attempt_async! {
        async {
            client
                .inclusion_proof_by_chain_id(
                    &second_batch[0],
                    mainnet_chain_id.as_u64(),
                )
                .await
                .and_then(|maybe_proof| maybe_proof.context("Missing inclusion proof"))
        }
    };

    tracing::debug!(?ip_for_mainnet, "Got second inclusion proof for mainnet");

    assert_eq!(
        ip_for_mainnet.root, second_batch_root,
        "Second inclusion proof root should batch to second batch"
    );

    let ip = client.inclusion_proof(&second_batch[0]).await?;

    assert!(ip.is_none(), "The inclusion proof endpoint should return only roots finalized on chains (unless chain id is specified)");

    let ip = client
        .inclusion_proof(&first_batch[0])
        .await?
        .context("Missing inclusion proof")?;

    assert_eq!(
        ip.root, first_batch_root,
        "We should still be able to fetch the first batch proofs"
    );

    let ip_for_bridged = client
        .inclusion_proof_by_chain_id(&second_batch[0], rollup_chain_id.as_u64())
        .await?;

    assert!(
        ip_for_bridged.is_none(),
        "The bridged network did not receive the batch yet"
    );

    // Bridge the second batch
    bridged_world_id
        .receive_root(f2ethers(second_batch_root))
        .send()
        .await?;

    // Repeated attempts until the new root & batch is available on rollup
    let ip_for_bridged = attempt_async! {
        async {
            client
                .inclusion_proof_by_chain_id(
                    &second_batch[0],
                    rollup_chain_id.as_u64(),
                )
                .await
                .and_then(|maybe_proof| maybe_proof.context("Missing inclusion proof"))
        }
    };

    tracing::debug!(?ip_for_bridged, "Got second inclusion proof for bridged");

    assert_eq!(
        ip_for_bridged.root, second_batch_root,
        "Second inclusion proof root should batch to second batch"
    );

    tokio::time::sleep(Duration::from_secs(4)).await;

    let ip = client
        .inclusion_proof(&second_batch[0])
        .await?
        .context("Missing inclusion proof")?;

    assert_eq!(
        ip.root,
        second_batch_root,
        "The second batch is now bridged and should be merged into the canonical tree"
    );

    tracing::info!("Cancelling world-tree service");

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

async fn setup_world_tree(
    config: &ServiceConfig,
) -> eyre::Result<
    Vec<
        JoinHandle<
            Result<(), WorldTreeError<Provider<ThrottledJsonRpcClient<Http>>>>,
        >,
    >,
> {
    let world_tree = init_world_tree(config).await?;

    let service = InclusionProofService::new(world_tree);
    let handles = service.serve(config.socket_address).await?;

    Ok(handles)
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
