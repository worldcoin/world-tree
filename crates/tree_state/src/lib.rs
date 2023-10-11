pub mod abi;
pub mod block_scanner;
pub mod error;
pub mod tree;
pub mod tree_updater;

use std::sync::Arc;

use error::TreeAvailabilityError;
use ethers::providers::Middleware;
use ethers::types::H160;
use semaphore::lazy_merkle_tree::Canonical;
use tokio::task::JoinHandle;
use tree::{Hash, PoseidonTree, WorldTree};
use tree_updater::TreeUpdater;

pub struct TreeAvailabilityService<M: Middleware + 'static> {
    pub world_tree: Arc<WorldTree>,
    pub tree_updater: TreeUpdater<M>,
    pub middleware: Arc<M>,
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

        let world_tree = Arc::new(WorldTree::new(tree));

        let tree_updater = TreeUpdater::new(
            middleware.clone(),
            world_tree_creation_block,
            world_tree_address,
        );

        Self {
            world_tree,
            tree_updater,
            middleware,
        }
    }

    pub async fn spawn(&self) -> JoinHandle<Result<(), TreeAvailabilityError<M>>> {
        let world_tree = self.world_tree.clone();
        tokio::spawn(async move { Ok(()) })
    }
}

//TODO: implement the api trait

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::Arc;

    use ethers::providers::{Provider, Ws};
    use ethers::types::H160;

    use crate::TreeAvailabilityService;

    //TODO: set world tree address as const for tests

    async fn test_spawn_tree_availability_service() -> eyre::Result<()> {
        let world_tree_address = H160::from_str("0x78eC127A3716D447F4575E9c834d452E397EE9E1")?;

        let middleware =
            Arc::new(Provider::<Ws>::connect(std::env::var("GOERLI_WS_ENDPOINT")?).await?);

        let tree_availability_service =
            TreeAvailabilityService::new(30, 10, world_tree_address, 0, middleware);

        let _handle = tree_availability_service.spawn().await;

        Ok(())
    }
}
