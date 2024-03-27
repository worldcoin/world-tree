use std::net::SocketAddr;
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
use super::identity_tree::IdentityTree;
use super::tree_data::InclusionProof;
use super::{Hash, PoseidonTree};

pub type ChainId = u64;

/// Service that keeps the World Tree synced with `WorldIDIdentityManager` and exposes an API endpoint to serve inclusion proofs for a given World ID.
pub struct InclusionProofService<M: Middleware + 'static> {
    /// In-memory representation of the merkle tree containing all verified World IDs.
    pub identity_tree: Arc<IdentityTree<M>>,
}

impl<M: Middleware> InclusionProofService<M> {
    /// Initializes new instance of `InclusionProofService`,
    ///
    /// # Arguments
    ///TODO:
    ///
    /// # Returns
    ///
    /// New instance of `InclusionProofService`.
    pub fn new(identity_tree: IdentityTree<M>) -> Self {
        Self {
            identity_tree: Arc::new(identity_tree),
        }
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
        addr: SocketAddr,
    ) -> Vec<JoinHandle<Result<(), TreeAvailabilityError<M>>>> {
        let mut handles = vec![];

        // // Initialize a new router and spawn the server
        // tracing::info!(?port, "Initializing axum server");

        // let router = axum::Router::new()
        //     .route("/inclusionProof", axum::routing::post(inclusion_proof))
        //     .layer(middleware::from_fn(logging::middleware))
        //     .with_state(self.world_tree.clone());

        // let address =
        //     SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port);

        // let server_handle = tokio::spawn(async move {
        //     tracing::info!("Spawning server");
        //     axum::Server::bind(&address)
        //         .serve(router.into_make_service())
        //         .await
        //         .map_err(TreeAvailabilityError::HyperError)?;
        //     tracing::info!("Server spawned");

        //     Ok(())
        // });

        // handles.push(server_handle);

        // // Spawn a new task to keep the world tree synced to the chain head
        // handles.push(self.world_tree.spawn());

        handles
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InclusionProofRequest {
    pub identity_commitment: Hash,
    chain_id: Option<ChainId>,
}

impl InclusionProofRequest {
    pub fn new(
        identity_commitment: Hash,
        chain_id: Option<ChainId>,
    ) -> InclusionProofRequest {
        Self {
            identity_commitment,
            chain_id,
        }
    }
}

#[tracing::instrument(level = "debug", skip(identity_tree))]
pub async fn inclusion_proof<M: Middleware>(
    State(identity_tree): State<Arc<IdentityTree<M>>>,
    Json(req): Json<InclusionProofRequest>,
) -> Result<(StatusCode, Json<Option<InclusionProof>>), TreeError> {
    //TODO:
    // let inclusion_proof = identity_tree
    //     .tree_data
    //     .read()
    //     .await
    //     .get_inclusion_proof(req.identity_commitment, req.root);

    todo!()
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
