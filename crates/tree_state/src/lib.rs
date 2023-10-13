pub mod abi;
pub mod error;
pub mod tree;

use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use abi::{IWorldIdIdentityManager, TreeChangedFilter};
use error::TreeAvailabilityError;
use ethers::{
    contract::EthCall,
    types::{BlockNumber, U64},
};
use ethers::{
    providers::{Middleware, PubsubClient, StreamExt},
    types::{H160, U256},
};

use semaphore::lazy_merkle_tree::{Canonical, Derived};
use tokio::{sync::RwLock, task::JoinHandle};
use tree::{Hash, PoseidonTree, TreeData, WorldTree};

pub struct TreeAvailabilityService<M: Middleware + 'static> {
    pub world_tree: Arc<WorldTree<TreeData<Canonical>, M>>,
}

impl<M: Middleware> TreeAvailabilityService<M> {
    pub fn new(
        tree_depth: usize,
        dense_prefix_depth: usize,
        world_tree_address: H160,
        world_tree_creation_block: u64,
        middleware: Arc<M>,
    ) -> Self {
        let tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
            tree_depth,
            dense_prefix_depth,
            &Hash::ZERO,
        );

        Self {
            world_tree: Arc::new(WorldTree::new(
                world_tree_address,
                Arc::new(RwLock::new(TreeData::new(tree, 0))),
                world_tree_creation_block,
                middleware,
            )),
        }
    }

    pub async fn spawn(&self) -> JoinHandle<Result<(), TreeAvailabilityError<M>>> {
        let world_tree = self.world_tree.clone();
        tokio::spawn(async move {
            world_tree.sync_to_head().await?;
            world_tree.listen_for_updates().await?;

            Ok(())
        })
    }
}

//TODO: implement the api trait

#[cfg(test)]
mod tests {
    use ethers::providers::{Provider, Ws};
    use ethers::types::H160;
    use std::{str::FromStr, sync::Arc};

    use crate::TreeAvailabilityService;

    //TODO: set world tree address as const for tests

    async fn test_spawn_tree_availability_service() -> eyre::Result<()> {
        let world_tree_address = H160::from_str("0x78eC127A3716D447F4575E9c834d452E397EE9E1")?;

        let middleware =
            Arc::new(Provider::<Ws>::connect(std::env::var("GOERLI_WS_ENDPOINT")?).await?);

        let tree_availability_service =
            TreeAvailabilityService::new(30, 10, world_tree_address, 0, middleware);

        let handle = tree_availability_service.spawn().await;

        Ok(())
    }
}
