#![cfg_attr(test, allow(clippy::needless_range_loop))]

use std::fs;
use std::sync::Arc;

use db::Db;
use tree::config::ServiceConfig;
use tree::WorldTree;

use self::tree::error::WorldTreeResult;

pub mod abi;
pub mod db;
pub mod serde_utils;
pub mod tasks;
pub mod tree;
pub mod util;

pub async fn init_world_tree(
    config: &ServiceConfig,
) -> WorldTreeResult<Arc<WorldTree>> {
    if config.cache.purge_cache {
        tracing::info!("Purging tree cache");
        if config.cache.cache_file.exists() {
            fs::remove_file(&config.cache.cache_file)?;
        }
    }

    let db = Arc::new(Db::init(&config.db).await?);

    let world_tree = Arc::new(WorldTree::new(
        config.clone(),
        db,
        config.tree_depth,
        &config.cache.cache_file,
    )?);

    Ok(world_tree)
}
