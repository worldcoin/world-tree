use ethers::core::k256::pkcs8::der::Writer;
use ethers::types::Address;
use semaphore::cascading_merkle_tree::CascadingMerkleTree;
use semaphore::poseidon_tree::PoseidonHash;
use semaphore::Field;
use tempfile::NamedTempFile;
use world_tree::tree::config::{
    CacheConfig, ProviderConfig, ServiceConfig, TreeConfig,
};

const TREE_DEPTH: usize = 20;

mod common;

use common::*;
use world_tree::tree::error::WorldTreeResult;

/// Check that an invalid cache results in the process exiting.
/// We expect the process here to exit(1).
/// I don't think we can effectively test the exit code in a test
/// so I'm just ignoreing it.
#[ignore]
#[tokio::test]
async fn mmap_cache() -> WorldTreeResult<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let mut cache_file = NamedTempFile::new()?;

    let (db_config, _db_container) = setup_db().await?;

    // Write an invalid cache file
    cache_file.write(&[
        // length
        4, 0, 0, 0, 0, 0, 0, 0, //
        // num leaves
        2, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, //
        // 1
        0, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, //
        // 2
        0, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, //
        // 3
        0, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, //
    ])?;

    let mainnet_container = setup_mainnet().await?;
    let mainnet_rpc_port = mainnet_container.get_host_port_ipv4(8545).await?;
    let mainnet_rpc_url = format!("http://127.0.0.1:{mainnet_rpc_port}");

    let rollup_container = setup_rollup().await?;
    let rollup_rpc_port = rollup_container.get_host_port_ipv4(8545).await?;
    let rollup_rpc_url = format!("http://127.0.0.1:{rollup_rpc_port}");

    let tree = CascadingMerkleTree::<PoseidonHash, _>::new(
        vec![],
        TREE_DEPTH,
        &Field::ZERO,
    );

    let initial_root = tree.root();
    tracing::info!(?initial_root, "Initial root",);

    let id_manager_address: Address =
        "0x5FbDB2315678afecb367f032d93F642f64180aa3".parse()?;
    let bridged_address: Address =
        "0x5FbDB2315678afecb367f032d93F642f64180aa3".parse()?;

    tracing::info!("Setting up world-tree service");

    let service_config = ServiceConfig {
        tree_depth: TREE_DEPTH,
        db: db_config,
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
            dir: cache_file.path().to_path_buf(),
            purge: false,
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

    let (_, _) = setup_world_tree(&service_config).await?;

    Ok(())
}
