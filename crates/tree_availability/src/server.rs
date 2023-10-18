use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use ethers::providers::Middleware;

use crate::error::TreeAvailabilityError;
use crate::tree::{Hash, WorldTree};

pub struct InclusionProofRequest {
    pub identity_commitment: Hash,
    pub root: Hash,
}

pub struct InclusionProofResponse {
    //TODO:
}

pub async fn inclusion_proof<M: Middleware>(
    State(world_tree): State<Arc<WorldTree>>,
    Json(inclusion_proof_request): Json<InclusionProofRequest>,
) -> Result<(StatusCode, Json<InclusionProofResponse>), TreeAvailabilityError<M>>
{
    todo!();
}
