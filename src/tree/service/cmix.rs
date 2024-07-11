use std::sync::Arc;

use ethers::providers::Middleware;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use xxdk::rpc::extractor::{Json, State};
use xxdk::rpc::{self, Router, RpcServerConfig};

use crate::tree::error::WorldTreeError;
use crate::tree::identity_tree::InclusionProof;
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
        cmix_config: RpcServerConfig,
    ) -> eyre::Result<JoinHandle<Result<(), WorldTreeError<M>>>> {
        tracing::info!(?cmix_config, "Initializing cMix RPC server");

        let router = Router::with_state(self.world_tree)
            .route("inclusionProof", inclusion_proof)
            .route("health", health)
            .route("computeRoot", compute_root);

        let server_handle = tokio::spawn(async move {
            tracing::info!("Starting cMix RPC server");
            rpc::serve(router, cmix_config)
                .await
                .map_err(|e| WorldTreeError::CMixError(e))?;
            Ok(())
        });

        Ok(server_handle)
    }
}

#[tracing::instrument(level = "debug", skip(tree))]
async fn inclusion_proof<M: Middleware + 'static>(
    State(tree): State<Arc<WorldTree<M>>>,
    Json(req): Json<InclusionProofRequest>,
) -> Result<Json<Option<InclusionProof>>, String> {
    tracing::info!(?req, "cMix request to `inclusionProof`");
    let proof = tree
        .inclusion_proof(req.identity_commitment, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Json(proof))
}

#[tracing::instrument(level = "debug")]
async fn health() {
    tracing::info!("cMix request to `health`");
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CmixComputeRootRequest {
    pub identity_commitments: Vec<Hash>,
    pub chain_id: Option<ChainId>,
}

#[tracing::instrument(level = "debug", skip(tree))]
async fn compute_root<M: Middleware + 'static>(
    State(tree): State<Arc<WorldTree<M>>>,
    Json(req): Json<CmixComputeRootRequest>,
) -> Result<Json<Hash>, String> {
    tracing::info!(?req, "cMix request to `computeRoot`");
    let updated_root = tree
        .compute_root(&req.identity_commitments, req.chain_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Json(updated_root))
}
