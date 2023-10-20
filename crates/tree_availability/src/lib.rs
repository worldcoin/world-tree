pub mod error;
pub mod server;
pub mod world_tree;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use error::TreeAvailabilityError;
use ethers::providers::Middleware;
use ethers::types::H160;
use semaphore::lazy_merkle_tree::Canonical;
use tokio::task::JoinHandle;
use world_tree::{Hash, PoseidonTree, WorldTree};

use crate::server::{inclusion_proof, synced};

pub struct TreeAvailabilityService<M: Middleware + 'static> {
    pub world_tree: Arc<WorldTree<M>>,
}

impl<M: Middleware> TreeAvailabilityService<M> {
    pub fn new(
        tree_depth: usize,
        dense_prefix_depth: usize,
        tree_history_size: usize,
        world_tree_address: H160,
        world_tree_creation_block: u64,
        middleware: Arc<M>,
    ) -> Self {
        let tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
            tree_depth,
            dense_prefix_depth,
            &Hash::ZERO,
        );

        let world_tree = Arc::new(WorldTree::new(
            tree,
            tree_history_size,
            world_tree_address,
            world_tree_creation_block,
            middleware,
        ));

        Self { world_tree }
    }

    pub async fn serve(
        self,
        port: u16,
    ) -> Vec<JoinHandle<Result<(), TreeAvailabilityError<M>>>> {
        let mut handles = vec![];

        // Initialize a new router and spawn the server
        let router = axum::Router::new()
            .route("/inclusionProof", axum::routing::post(inclusion_proof))
            .route("/synced", axum::routing::post(synced))
            // .route("/verifyProof", axum::routing::post(verify_proof))
            .with_state(self.world_tree.clone());

        let address =
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port);

        let server_handle = tokio::spawn(async move {
            axum::Server::bind(&address)
                .serve(router.into_make_service())
                .await
                .map_err(TreeAvailabilityError::HyperError)?;
            // .with_graceful_shutdown(await_shutdown());

            Ok(())
        });

        handles.push(server_handle);

        // Spawn a new task to keep the world tree synced to the chain head
        handles.push(self.world_tree.spawn().await);

        handles
    }
}

#[cfg(test)]
mod tests {}
