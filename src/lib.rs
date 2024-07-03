use std::fs;
use std::sync::Arc;

use ethers::providers::{Http, Provider};
use ethers_throttle::ThrottledJsonRpcClient;
use tree::config::ServiceConfig;
use tree::tree_manager::{BridgedTree, CanonicalTree, TreeManager};
use tree::WorldTree;

pub mod abi;
mod error;
pub mod serde_utils;
pub mod tree;

pub async fn init_world_tree(
    config: &ServiceConfig,
) -> eyre::Result<Arc<WorldTree<Provider<ThrottledJsonRpcClient<Http>>>>> {
    let canonical_provider_config = &config.canonical_tree.provider;

    let http_provider =
        Http::new(canonical_provider_config.rpc_endpoint.clone());
    let throttled_provider = ThrottledJsonRpcClient::new(
        http_provider,
        canonical_provider_config.throttle,
        None,
    );
    let canonical_middleware = Arc::new(Provider::new(throttled_provider));

    let canonical_tree_config = &config.canonical_tree;
    let canonical_tree_manager = TreeManager::<_, CanonicalTree>::new(
        canonical_tree_config.address,
        canonical_tree_config.window_size,
        canonical_tree_config.creation_block,
        canonical_middleware,
    )
    .await?;

    let mut bridged_tree_managers = vec![];

    for tree_config in config.bridged_trees.iter() {
        let bridged_provider_config = &tree_config.provider;
        let http_provider =
            Http::new(bridged_provider_config.rpc_endpoint.clone());

        let throttled_provider = ThrottledJsonRpcClient::new(
            http_provider,
            bridged_provider_config.throttle,
            None,
        );
        let bridged_middleware = Arc::new(Provider::new(throttled_provider));

        let tree_manager = TreeManager::<_, BridgedTree>::new(
            tree_config.address,
            tree_config.window_size,
            tree_config.creation_block,
            bridged_middleware,
        )
        .await?;

        bridged_tree_managers.push(tree_manager);
    }

    if config.cache.purge_cache {
        tracing::info!("Purging tree cache");
        fs::remove_file(&config.cache.cache_file)?;
    }

    Ok(Arc::new(WorldTree::new(
        config.tree_depth,
        canonical_tree_manager,
        bridged_tree_managers,
        &config.cache.cache_file,
    )?))
}
