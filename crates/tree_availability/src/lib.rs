//! # Tree Availability Service
//!
//! The tree availability service is able to create an in-memory representation of the World ID
//! merkle tree by syncing the state of the World ID contract `registerIdentities` and `deleteIdentities`
//! function calldata and `TreeChanged` events. Once it syncs the latest state of the state of the tree, it
//! is able to serve inclusion proofs on the `/inclusionProof` endpoint. It also keeps a historical roots list
//! of `tree_history_size` size in order to serve proofs against older tree roots (including the roots of
//! World IDs bridged to other networks).

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

/// Service that keeps the World Tree synced with `WorldIDIdentityManager` and exposes an API endpoint to serve inclusion proofs for a given World ID.
pub struct TreeAvailabilityService<M: Middleware + 'static> {
    /// In-memory representation of the merkle tree containing all verified World IDs.
    pub world_tree: Arc<WorldTree<M>>,
}

impl<M: Middleware> TreeAvailabilityService<M> {
    /// Initializes new instance of `TreeAvailabilityService`,
    ///
    /// # Arguments
    ///
    /// * `tree_depth` - Depth of the merkle tree
    /// * `dense_prefix_depth`: Depth of the tree that is densely populated. Nodes beyond the `dense_prefix_depth` will be stored through pointer based structures.
    /// * `tree_history_size`: Number of historical roots to store in memory. This is used to serve proofs against historical roots.
    /// * `world_tree_address`: Address of the `WorldIDIdentityManager` contract onchain
    /// * `world_tree_creation_block`: Block number where `WorldIDIdentityManager` was deployed
    /// * `middleware`: Provider to interact with Ethereum
    ///
    /// # Returns
    ///
    /// New instance of `TreeAvailabilityService`.
    pub fn new(
        tree_depth: usize,
        dense_prefix_depth: usize,
        tree_history_size: usize,
        world_tree_address: H160,
        world_tree_creation_block: u64,
        window_size: u64,
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
            window_size,
            middleware,
        ));

        Self { world_tree }
    }

    /// Spawns an axum server and exposes an API endpoint to serve inclusion proofs for a given World ID. This function also spawns a new task to keep the world tree synced to the chain head.
    ///
    /// # Arguments
    ///
    /// * `port` - Port to bind the server to.
    ///
    /// # Returns
    ///
    /// Vector of `JoinHandle`s for the spawned tasks.
    pub async fn serve(
        self,
        port: u16,
    ) -> Vec<JoinHandle<Result<(), TreeAvailabilityError<M>>>> {
        let mut handles = vec![];

        // Initialize a new router and spawn the server
        tracing::info!("Initializing axum server on port {port}");

        let router = axum::Router::new()
            .route("/inclusionProof", axum::routing::post(inclusion_proof))
            .route("/synced", axum::routing::post(synced))
            .with_state(self.world_tree.clone());

        let address =
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port);

        let server_handle = tokio::spawn(async move {
            tracing::info!("Spawning server");
            axum::Server::bind(&address)
                .serve(router.into_make_service())
                .await
                .map_err(TreeAvailabilityError::HyperError)?;
            tracing::info!("Server spawned");

            Ok(())
        });

        handles.push(server_handle);

        // Spawn a new task to keep the world tree synced to the chain head
        handles.push(self.world_tree.spawn().await);

        handles
    }
}
