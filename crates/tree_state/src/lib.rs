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
use tokio::sync::RwLock;
use tree::{canonical::CanonicalMetadata, Hash, PoseidonTree, TreeData, WorldTree};

pub struct TreeAvailabilityService<M: Middleware> {
    pub world_tree: Arc<WorldTree<TreeData<Canonical>, M>>,
    //TODO: add a field for join handles
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
                Arc::new(RwLock::new(TreeData::new(tree, 0, CanonicalMetadata {}))),
                world_tree_creation_block,
                middleware,
            )),
        }
    }

    pub async fn spawn(&self) -> Result<(), TreeAvailabilityError<M>> {
        // Create a new stream to listen for tree changed events
        let world_id_identity_manager = IWorldIdIdentityManager::new(
            self.world_tree.address,
            self.world_tree.middleware.clone(),
        );

        let filter = world_id_identity_manager.event::<TreeChangedFilter>();
        let mut event_stream = filter.stream().await?.with_meta();

        // Sync the tree to the current block
        self.world_tree.sync().await?;

        //TODO: spawn

        Ok(())
    }
}

//TODO: implement the api trait

#[cfg(test)]
mod tests {
    use ethers::providers::{Provider, PubsubClient, Ws};
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

        tree_availability_service.spawn().await?;

        Ok(())
    }
}
