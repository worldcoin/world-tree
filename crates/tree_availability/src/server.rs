use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use ethers::providers::Middleware;
use semaphore::poseidon_tree::{Branch, Proof};
use semaphore::Field;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use tokio::time::Instant;

use crate::error::TreeError;
use crate::world_tree::{Hash, WorldTree};

#[derive(Serialize, Deserialize)]
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InclusionProof {
    pub root: Field,
    //TODO: Implement `Deserialize` for Proof within semaphore-rs instead of using `deserialize_with`
    #[serde(deserialize_with = "deserialize_proof")]
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

fn deserialize_proof<'de, D>(deserializer: D) -> Result<Proof, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Value = Deserialize::deserialize(deserializer)?;
    if let Value::Array(array) = value {
        let mut branches = vec![];
        for value in array {
            let branch = serde_json::from_value::<Branch>(value)
                .map_err(serde::de::Error::custom)?;
            branches.push(branch);
        }

        Ok(semaphore::merkle_tree::Proof(branches))
    } else {
        Err(D::Error::custom("Expected an array"))
    }
}

pub async fn inclusion_proof<M: Middleware>(
    State(world_tree): State<Arc<WorldTree<M>>>,
    Json(req): Json<InclusionProofRequest>,
) -> Result<(StatusCode, Json<Option<InclusionProof>>), TreeError> {
    if world_tree.synced.load(Ordering::Relaxed) {
        let inclusion_proof_start_time = Instant::now();

        let inclusion_proof = world_tree
            .tree_data
            .get_inclusion_proof(req.identity_commitment, req.root)
            .await;

        metrics::histogram!(
            "tree_availability.server.inclusion_proof_duration_ms",
            inclusion_proof_start_time.elapsed().as_millis() as f64
        );

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
