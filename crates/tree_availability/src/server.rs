use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use ethers::providers::Middleware;
use semaphore::poseidon_tree::Proof;
use semaphore::Field;
use serde::{Deserialize, Serialize};

use crate::error::{TreeAvailabilityError, TreeError};
use crate::tree::{Hash, WorldTree};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InclusionProofRequest {
    pub identity_commitment: Hash,
    pub root: Option<Hash>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InclusionProof {
    pub root: Field,
    pub proof: Proof,
    pub message: Option<String>,
}

impl InclusionProof {
    pub fn new(
        root: Field,
        proof: Proof,
        message: Option<String>,
    ) -> InclusionProof {
        Self {
            root,
            proof,
            message,
        }
    }
}

pub async fn inclusion_proof<M: Middleware>(
    State(world_tree): State<Arc<WorldTree<M>>>,
    Json(req): Json<InclusionProofRequest>,
) -> Result<(StatusCode, Json<Option<InclusionProof>>), TreeError> {
    let inclusion_proof = world_tree
        .get_inclusion_proof(req.identity_commitment, req.root)
        .await;

    Ok((StatusCode::OK, inclusion_proof.into()))
}

impl TreeError {
    fn to_status_code(&self) -> StatusCode {
        //TODO: update this
        StatusCode::BAD_REQUEST
    }
}
impl IntoResponse for TreeError {
    fn into_response(self) -> axum::response::Response {
        let status_code = self.to_status_code();
        let response_body = self.to_string();
        (status_code, response_body).into_response()
    }
}
