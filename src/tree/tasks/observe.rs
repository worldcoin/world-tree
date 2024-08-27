//! Log observation logic
//! this module defines tasks responsible for observing bridged
//! chains and extracting update information

use std::sync::Arc;

use ethers::abi::RawLog;
use ethers::contract::EthEvent;
use ethers::providers::Middleware;
use ethers::types::{Filter, Log, ValueOrArray};
use eyre::ContextCompat;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::pin;

use crate::abi::RootAddedFilter;
use crate::db::DbMethods;
use crate::tree::block_scanner::BlockScanner;
use crate::tree::error::WorldTreeResult;
use crate::tree::{provider, Hash, WorldTree};

pub async fn observe(world_tree: Arc<WorldTree>) -> WorldTreeResult<()> {
    let mut handles = FuturesUnordered::new();

    for idx in 0..world_tree.config.bridged_trees.len() {
        let world_tree = world_tree.clone();

        handles.push(tokio::spawn(async move {
            observe_bridged(world_tree, idx).await
        }));
    }

    while let Some(result) = handles.next().await {
        result??;
    }

    Ok(())
}

pub async fn observe_bridged(
    world_tree: Arc<WorldTree>,
    idx: usize,
) -> WorldTreeResult<()> {
    let provider =
        provider(&world_tree.config.bridged_trees[idx].provider).await?;
    let chain_id = provider.get_chainid().await?.as_u64();

    let latest_block_number =
        world_tree.db.fetch_latest_block_number(chain_id).await?;

    let latest_block_number = latest_block_number
        .map(|x| x.as_u64())
        .unwrap_or(world_tree.config.bridged_trees[idx].creation_block);

    let filter = Filter::new()
        .address(world_tree.config.bridged_trees[idx].address)
        .topic0(ValueOrArray::Value(RootAddedFilter::signature()));

    let scanner = BlockScanner::new(
        provider.clone(),
        world_tree.config.bridged_trees[idx].provider.window_size,
        latest_block_number,
        filter,
    )
    .await?;

    // TODO: Make buffer size configurable?
    let stream = scanner.block_stream().buffered(10);
    pin!(stream);

    while let Some(log_batch) = stream.next().await {
        let logs: Vec<Log> = log_batch?;

        for log in logs {
            let block_number =
                log.block_number.context("Missing block_number")?;
            let tx_hash = log.transaction_hash.context("Missing tx_hash")?;

            let data = RootAddedFilter::decode_log(&RawLog::from(log))?;
            let root = Hash::from_limbs(data.root.0);

            let mut tx = world_tree.db.begin().await?;

            // 1. Insert tx metadata
            let tx_id = tx.insert_tx(chain_id, block_number, tx_hash).await?;

            // 2. Insert bridged update data
            tx.insert_root(root, tx_id).await?;

            tx.commit().await?;
        }
    }

    Ok(())
}
