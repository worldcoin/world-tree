use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::{middleware, Json};
use axum_middleware::logging;
use ethers::providers::Middleware;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use super::error::WorldTreeError;
use super::{ChainId, Hash, InclusionProof, WorldTree};

/// Service that keeps the World Tree synced with `WorldIDIdentityManager` and exposes an API endpoint to serve inclusion proofs for a given World ID.

pub struct InclusionProofService<M: Middleware + 'static> {
    /// In-memory representation of the merkle tree containing all verified World IDs.
    pub world_tree: Arc<WorldTree<M>>,
    pub cancel_tx: broadcast::Sender<()>,
}

impl<M> InclusionProofService<M>
where
    M: Middleware,
{
    pub fn new(world_tree: Arc<WorldTree<M>>) -> Self {
        let (cancel_tx, _) = broadcast::channel(1);

        Self {
            world_tree,
            cancel_tx,
        }
    }

    /// Spawns an axum server and exposes an API endpoint to serve inclusion proofs for requested identity commitments.
    /// This function spawns a task to sync and maintain the state of the world tree across all monitored chains.
    ///
    /// # Arguments
    ///
    /// * `addr` - Socket address to bind the server to
    ///
    /// # Returns
    ///
    /// Vector of `JoinHandle`s for the spawned tasks.
    pub async fn serve(
        self,
        addr: Option<SocketAddr>,
    ) -> eyre::Result<(
        SocketAddr,
        Vec<JoinHandle<Result<(), WorldTreeError<M>>>>,
    )> {
        let mut handles = vec![];

        // Initialize a new router and spawn the server
        tracing::info!(?addr, "Initializing axum server");

        let router = axum::Router::new()
            .route("/inclusionProof", axum::routing::post(inclusion_proof))
            .route("/computeRoot", axum::routing::post(compute_root))
            .route("/health", axum::routing::get(health))
            .layer(middleware::from_fn(logging::middleware))
            .with_state(self.world_tree.clone());

        let tcp_listener = match addr {
            Some(addr) => std::net::TcpListener::bind(addr)?,
            None => std::net::TcpListener::bind("0.0.0.0:0")?,
        };

        let bound_server = axum::Server::from_tcp(tcp_listener)?;
        let local_addr = bound_server.local_addr();

        let server_handle = tokio::spawn(async move {
            tracing::info!("Spawning server");
            bound_server.serve(router.into_make_service()).await?;

            Ok(())
        });

        // Spawn a task to sync and maintain the state of the world tree
        tracing::info!("Spawning world tree");
        handles.extend(self.world_tree.spawn().await?);

        handles.push(server_handle);

        Ok((local_addr, handles))
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InclusionProofRequest {
    pub identity_commitment: Hash,
}

impl InclusionProofRequest {
    pub fn new(identity_commitment: Hash) -> InclusionProofRequest {
        Self {
            identity_commitment,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComputeRootRequest {
    pub identity_commitments: Vec<Hash>,
}

impl ComputeRootRequest {
    pub fn new(identity_commitments: Vec<Hash>) -> ComputeRootRequest {
        Self {
            identity_commitments,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChainIdQueryParams {
    chain_id: Option<ChainId>,
}

#[tracing::instrument(skip(world_tree))]
pub async fn inclusion_proof<M: Middleware + 'static>(
    State(world_tree): State<Arc<WorldTree<M>>>,
    Query(query_params): Query<ChainIdQueryParams>,
    Json(req): Json<InclusionProofRequest>,
) -> Result<(StatusCode, Json<Option<InclusionProof>>), WorldTreeError<M>> {
    let chain_id = query_params.chain_id;
    let inclusion_proof = world_tree
        .inclusion_proof(req.identity_commitment, chain_id)
        .await?;

    Ok((StatusCode::OK, Json(inclusion_proof)))
}

#[tracing::instrument(level = "debug", skip(world_tree))]
#[allow(clippy::complexity)]
pub async fn health<M: Middleware + 'static>(
    State(world_tree): State<Arc<WorldTree<M>>>,
) -> Result<Json<HashMap<u64, Hash>>, WorldTreeError<M>> {
    let chain_state = world_tree.chain_state.read().await.clone();
    Ok(Json(chain_state))
}

#[tracing::instrument(level = "debug", skip(world_tree))]
pub async fn compute_root<M: Middleware + 'static>(
    State(world_tree): State<Arc<WorldTree<M>>>,
    Query(query_params): Query<ChainIdQueryParams>,
    Json(req): Json<ComputeRootRequest>,
) -> Result<(StatusCode, Json<Hash>), WorldTreeError<M>> {
    let chain_id = query_params.chain_id;
    let updated_root = world_tree
        .compute_root(&req.identity_commitments, chain_id)
        .await?;

    Ok((StatusCode::OK, Json(updated_root)))
}
