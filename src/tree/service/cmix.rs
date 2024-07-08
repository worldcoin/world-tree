use std::sync::Arc;

use ethers::providers::Middleware;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use xxdk::service::{CMixServer, CMixServerConfig, IncomingRequest};

use crate::tree::error::WorldTreeError;
use crate::tree::service::InclusionProofRequest;
use crate::tree::{ChainId, Hash, WorldTree};

pub struct CmixInclusionProofService<M: Middleware + 'static> {
    pub world_tree: Arc<WorldTree<M>>,
}

impl<M: Middleware> CmixInclusionProofService<M> {
    pub fn new(world_tree: Arc<WorldTree<M>>) -> Self {
        Self { world_tree }
    }

    pub async fn serve(
        self,
        cmix_config: CMixServerConfig,
    ) -> eyre::Result<JoinHandle<Result<(), WorldTreeError<M>>>> {
        tracing::info!(?cmix_config, "Initializing cMix RPC server");

        let router = xxdk::service::Router::new(self.world_tree)
            .route("inclusionProof", inclusion_proof)
            .route("health", health)
            .route("computeRoot", compute_root);

        let server_handle = tokio::spawn(async move {
            tracing::info!("Starting cMix RPC server");
            CMixServer::serve(router, cmix_config)
                .await
                .map_err(|e| WorldTreeError::CMixError(e))?;
            Ok(())
        });

        Ok(server_handle)
    }
}

#[tracing::instrument(level = "debug", skip(state))]
async fn inclusion_proof<M: Middleware + 'static>(
    state: Arc<WorldTree<M>>,
    request: IncomingRequest,
) -> Result<Vec<u8>, String> {
    tracing::info!("cMix request to `inclusionProof`");
    let req: InclusionProofRequest =
        serde_json::from_slice(&request.request).map_err(|e| e.to_string())?;
    tracing::debug!(?req, "Request decoded");
    let proof = state.inclusion_proof(req.identity_commitment, None)
        .await
        .map_err(|e| e.to_string())?;
    let res = serde_json::to_vec(&proof).map_err(|e| e.to_string())?;
    tracing::debug!(response_len = res.len(), "cMix response");
    Ok(res)
}

#[tracing::instrument(level = "debug", skip(_state))]
async fn health<M: Middleware + 'static>(
    _state: Arc<WorldTree<M>>,
    _request: IncomingRequest,
) -> Result<Vec<u8>, String> {
    tracing::info!("cMix request to `health`");
    Ok(Vec::new())
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CmixComputeRootRequest {
    pub identity_commitments: Vec<Hash>,
    pub chain_id: Option<ChainId>,
}

#[tracing::instrument(level = "debug", skip(state))]
async fn compute_root<M: Middleware + 'static>(
    state: Arc<WorldTree<M>>,
    request: IncomingRequest,
) -> Result<Vec<u8>, String> {
    tracing::info!("cMix request to `computeRoot`");
    let req: CmixComputeRootRequest =
        serde_json::from_slice(&request.request).map_err(|e| e.to_string())?;
    tracing::debug!(?req, "Request decoded");

    let updated_root = state
        .compute_root(&req.identity_commitments, req.chain_id)
        .await
        .map_err(|e| e.to_string())?;

    let res = serde_json::to_vec(&updated_root).map_err(|e| e.to_string())?;
    tracing::debug!(response_len = res.len(), "cMix response");
    Ok(res)
}
