pub mod abi;
pub mod block_scanner;
pub mod error;
pub mod index_packing;
pub mod server;
pub mod tree;
pub mod tree_updater;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Json, Router};
use error::TreeAvailabilityError;
use ethers::providers::Middleware;
use ethers::types::H160;
use semaphore::lazy_merkle_tree::Canonical;
use tokio::task::JoinHandle;
use tree::{Hash, PoseidonTree, WorldTree};
use tree_updater::TreeUpdater;

use crate::server::inclusion_proof;

// TODO: Change to a configurable parameter
const TREE_HISTORY_SIZE: usize = 1000;

pub struct TreeAvailabilityService<M: Middleware + 'static> {
    pub world_tree: Arc<WorldTree>,
    pub tree_updater: Arc<TreeUpdater<M>>,
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

        let world_tree = Arc::new(WorldTree::new(tree, TREE_HISTORY_SIZE));

        let tree_updater = Arc::new(TreeUpdater::new(
            middleware.clone(),
            world_tree_creation_block,
            world_tree_address,
        ));

        Self {
            world_tree,
            tree_updater,
            middleware,
        }
    }

    //TODO: maybe move this spawn function to the World Tree and then the tree avail service will only have one spawn function instead
    //TODO: or maybe we can use a trait that will allow the service to extend an api like tas.server() which returns a builder and then we can call
    //TODO: spawn on the server builder.
    pub async fn spawn(
        &self,
    ) -> JoinHandle<Result<(), TreeAvailabilityError<M>>> {
        let world_tree = self.world_tree.clone();
        let tree_updater: Arc<TreeUpdater<M>> = self.tree_updater.clone();

        tokio::spawn(async move {
            loop {
                tree_updater.sync_to_head(&world_tree).await?;

                // Sleep a little to unblock the executor
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        })
    }

    //TODO: rename this, this function spawns the api service
    pub async fn serve(
        self,
    ) -> JoinHandle<Result<(), TreeAvailabilityError<M>>> {
        let handle = self.spawn().await;

        let router = axum::Router::new()
            .route(
                "/inclusionProof",
                axum::routing::post(Self::inclusion_proof),
            )
            .with_state(self.world_tree.clone());

        todo!();
    }

    async fn inclusion_proof(
        State(world_tree): State<Arc<WorldTree>>,
        Json(inclusion_proof_request): Json<InclusionProofRequest>,
    ) -> Result<
        (StatusCode, Json<InclusionProofResponse>),
        TreeAvailabilityError<M>,
    > {
        todo!();
    }
}

pub struct InclusionProofRequest {
    pub identity_commitment: Hash,
    pub root: Hash,
}

pub struct InclusionProofResponse {
    //TODO:
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
        let world_tree_address =
            H160::from_str("0x78eC127A3716D447F4575E9c834d452E397EE9E1")?;

        let middleware = Arc::new(
            Provider::<Ws>::connect(std::env::var("GOERLI_WS_ENDPOINT")?)
                .await?,
        );

        let tree_availability_service = TreeAvailabilityService::new(
            30,
            10,
            world_tree_address,
            0,
            middleware,
        );

        let _handle = tree_availability_service.spawn().await;

        Ok(())
    }
}
