use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{middleware, Json};
use axum_middleware::logging;
use ethers::providers::Middleware;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

use super::error::TreeError;
use super::{Hash, InclusionProof, WorldTree};

pub type ChainId = u64;

/// Service that keeps the World Tree synced with `WorldIDIdentityManager` and exposes an API endpoint to serve inclusion proofs for a given World ID.
pub struct InclusionProofService<M: Middleware + 'static> {
    /// In-memory representation of the merkle tree containing all verified World IDs.
    pub world_tree: Arc<WorldTree<M>>,
}

impl<M: Middleware> InclusionProofService<M> {
    pub fn new(world_tree: Arc<WorldTree<M>>) -> Self {
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
        addr: SocketAddr,
    ) -> eyre::Result<Vec<JoinHandle<eyre::Result<()>>>> {
        let mut handles = vec![];

        handles.extend(self.world_tree.spawn().await?);

        // Initialize a new router and spawn the server
        tracing::info!(?addr, "Initializing axum server");

        let router = axum::Router::new()
            .route("/inclusionProof", axum::routing::post(inclusion_proof))
            .layer(middleware::from_fn(logging::middleware))
            .with_state(self.world_tree.clone());

        let server_handle = tokio::spawn(async move {
            tracing::info!("Spawning server");
            axum::Server::bind(&addr)
                .serve(router.into_make_service())
                .await?;

            Ok(())
        });

        handles.push(server_handle);

        Ok(handles)
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

#[tracing::instrument(level = "debug", skip(world_tree))]
pub async fn inclusion_proof<M: Middleware>(
    State(world_tree): State<Arc<WorldTree<M>>>,
    Json(req): Json<InclusionProofRequest>,
) -> Result<(StatusCode, Json<Option<InclusionProof>>), TreeError> {
    // let inclusion_proof = world_tree
    //     .inclusion_proof(identity_commitment, chain_id)
    //     .await

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
