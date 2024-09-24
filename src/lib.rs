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
    if config.cache.purge {
        tracing::info!("Purging tree cache");
        if config.cache.dir.exists() {
            if config.cache.dir.is_dir() {
                fs::remove_dir_all(&config.cache.dir)?;
            } else {
                fs::remove_file(&config.cache.dir)?;
            }
        }
    }

    let db = Arc::new(Db::init(&config.db).await?);

    let world_tree = Arc::new(WorldTree::new(config.clone(), db).await?);

    Ok(world_tree)
}
