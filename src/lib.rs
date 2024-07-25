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
        if config.cache.cache_file.exists() {
            fs::remove_file(&config.cache.cache_file)?;
        }

        // There's something wrong with CascadingMerkleTree impl
        // in some cases it'll fail if the file doesn't exist
        fs::write(&config.cache.cache_file, [])?;
    }

    Ok(Arc::new(WorldTree::new(
        config.tree_depth,
        canonical_tree_manager,
        bridged_tree_managers,
        &config.cache.cache_file,
    )?))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use semaphore::cascading_merkle_tree::CascadingMerkleTree;
    use semaphore::generic_storage::MmapVec;
    use semaphore::poseidon_tree::PoseidonHash;
    use tree::identity_tree::node_to_leaf_idx;
    use tree::{Hash, NodeIndex};

    use super::*;

    #[tokio::test]
    async fn sanity_check() -> eyre::Result<()> {
        let storage = unsafe { MmapVec::restore_from_path("./cache.mmap")? };
        let tree: CascadingMerkleTree<PoseidonHash, _> =
            CascadingMerkleTree::restore_unchecked(storage, 30, &Hash::ZERO)?;

        println!("tree.num_leaves() = {}", tree.num_leaves());

        let mut all_leaves: Vec<_> = tree.leaves().collect();
        all_leaves.sort();

        // Count duplicates
        let mut counts: HashMap<Hash, usize> = HashMap::new();
        for leaf in &all_leaves {
            *counts.entry(leaf.clone()).or_insert(0) += 1;
        }

        // Collect duplicates and sort by frequency
        let mut duplicates: Vec<_> =
            counts.iter().filter(|&(_, &count)| count > 1).collect();
        duplicates.sort_by(|a, b| b.1.cmp(a.1));

        // Display duplicates sorted by the most amount of duplicates descending
        println!("Duplicates sorted by frequency:");
        for (leaf, count) in duplicates {
            println!("{:?}: {}", leaf, count);
        }

        Ok(())
    }

    #[test]
    fn only_leaf_idxs() {
        let fname = "updates/r14476_6394958320965700475475854235126049092479678984786377875350991357779760415846.json";

        let content = std::fs::read(fname).unwrap();

        let updates: HashMap<NodeIndex, Hash> = serde_json::from_slice(&content).unwrap();

        let mut leaf_idxs: Vec<_> = updates.keys().filter_map(|node_idx| {
            let leaf_idx = node_to_leaf_idx(node_idx.0, 30)?;

            Some(leaf_idx)
        }).collect();

        leaf_idxs.sort();

        println!("leaf_idxs = {:?}", leaf_idxs);
    }
}
