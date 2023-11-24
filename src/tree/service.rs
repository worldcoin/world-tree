use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{middleware, Json};
use axum_middleware::logging;
use ethers::providers::Middleware;
use ethers::types::H160;
use semaphore::lazy_merkle_tree::Canonical;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

use super::error::{TreeAvailabilityError, TreeError};
use super::tree_data::InclusionProof;
use super::{Hash, PoseidonTree, WorldTree};

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
    pub fn serve(
        self,
        port: u16,
    ) -> Vec<JoinHandle<Result<(), TreeAvailabilityError<M>>>> {
        let mut handles = vec![];

        // Initialize a new router and spawn the server
        tracing::info!(?port, "Initializing axum server");

        let router = axum::Router::new()
            .route("/inclusionProof", axum::routing::post(inclusion_proof))
            .route("/synced", axum::routing::post(synced))
            .layer(middleware::from_fn(logging::middleware))
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
        handles.push(self.world_tree.spawn());

        handles
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InclusionProofRequest {
    pub identity_commitment: Hash,
    pub root: Option<Hash>,
}

impl InclusionProofRequest {
    pub fn new(
        identity_commitment: Hash,
        root: Option<Hash>,
    ) -> InclusionProofRequest {
        Self {
            identity_commitment,
            root,
        }
    }
}

#[tracing::instrument(level = "debug", skip(world_tree))]
pub async fn inclusion_proof<M: Middleware>(
    State(world_tree): State<Arc<WorldTree<M>>>,
    Json(req): Json<InclusionProofRequest>,
) -> Result<(StatusCode, Json<Option<InclusionProof>>), TreeError> {
    if world_tree.synced.load(Ordering::Relaxed) {
        let inclusion_proof = world_tree
            .tree_data
            .read()
            .await
            .get_inclusion_proof(req.identity_commitment, req.root)
            .await;

        Ok((StatusCode::OK, inclusion_proof.into()))
    } else {
        Err(TreeError::TreeNotSynced)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResponse {
    pub synced: bool,
    pub block_number: Option<u64>,
}

impl SyncResponse {
    pub fn new(synced: bool, block_number: Option<u64>) -> SyncResponse {
        Self {
            synced,
            block_number,
        }
    }
}

#[tracing::instrument(level = "debug", skip(world_tree))]
pub async fn synced<M: Middleware>(
    State(world_tree): State<Arc<WorldTree<M>>>,
) -> (StatusCode, Json<SyncResponse>) {
    if world_tree.synced.load(Ordering::Relaxed) {
        (StatusCode::OK, SyncResponse::new(true, None).into())
    } else {
        let latest_synced_block = Some(
            world_tree
                .tree_updater
                .latest_synced_block
                .load(Ordering::SeqCst),
        );
        (
            StatusCode::OK,
            SyncResponse::new(false, latest_synced_block).into(),
        )
    }
}

impl TreeError {
    fn to_status_code(&self) -> StatusCode {
        match self {
            TreeError::TreeNotSynced => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl IntoResponse for TreeError {
    fn into_response(self) -> axum::response::Response {
        let status_code = self.to_status_code();
        let response_body = self.to_string();
        (status_code, response_body).into_response()
    }
}
