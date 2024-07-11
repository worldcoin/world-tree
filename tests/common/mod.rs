#![allow(unused)]

use std::net::SocketAddr;
use std::time::Duration;

use ethers::providers::{Http, Middleware, Provider};
use ethers::types::{Address, U256};
use ethers_throttle::ThrottledJsonRpcClient;
use eyre::ContextCompat;
use rand::Rng;
use semaphore::Field;
use testcontainers::core::{ContainerPort, Mount};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use world_tree::init_world_tree;
use world_tree::tree::config::ServiceConfig;
use world_tree::tree::error::WorldTreeError;
use world_tree::tree::service::InclusionProofService;

mod test_client;

pub use test_client::TestClient;

// Attempts a given block multiple times
// panics if the block fails (evaluates to an error) more than 10 times
#[macro_export]
macro_rules! attempt_async {
    ($e:expr) => {
        {
            const MAX_ATTEMPTS: usize = 10;
            const SLEEP_DURATION: Duration = Duration::from_secs(5);
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
                        tokio::time::sleep(SLEEP_DURATION).await;
                    }
                }
            }
        }
    };
}

pub async fn setup_world_tree(
    config: &ServiceConfig,
) -> eyre::Result<(
    SocketAddr,
    Vec<
        JoinHandle<
            Result<(), WorldTreeError<Provider<ThrottledJsonRpcClient<Http>>>>,
        >,
    >,
)> {
    let world_tree = init_world_tree(config).await?;

    let service = InclusionProofService::new(world_tree);

    service.serve(config.socket_address).await
}

pub async fn setup_mainnet() -> eyre::Result<ContainerAsync<GenericImage>> {
    setup_chain("runMainnet.sh").await
}

pub async fn setup_rollup() -> eyre::Result<ContainerAsync<GenericImage>> {
    setup_chain("runRollup.sh").await
}

pub async fn setup_chain(
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

pub async fn wait_until_contracts_deployed(
    provider: &Provider<Http>,
    address: Address,
) -> eyre::Result<()> {
    const MAX_RETRIES: usize = 10;
    const SLEEP_DURATION: Duration = Duration::from_secs(2);

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

pub fn random_leaves(n: usize) -> Vec<Field> {
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

pub fn f2ethers(f: Field) -> U256 {
    U256::from_little_endian(&f.as_le_bytes())
}
