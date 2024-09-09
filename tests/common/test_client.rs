use hyper::StatusCode;
use semaphore::Field;
use world_tree::tree::inclusion_proof::InclusionProof;
use world_tree::tree::service::InclusionProofRequest;

pub struct TestClient {
    pub client: reqwest::Client,
    pub world_tree_host: String,
}

impl TestClient {
    pub fn new(world_tree_host: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            world_tree_host,
        }
    }

    pub async fn inclusion_proof(
        &self,
        commitment: &Field,
    ) -> eyre::Result<Option<InclusionProof>> {
        let url = format!("{}/inclusionProof", self.world_tree_host);

        let response = self
            .client
            .post(&url)
            .json(&InclusionProofRequest {
                identity_commitment: *commitment,
            })
            .send()
            .await?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        } else {
            response.error_for_status_ref()?;
        }

        Ok(response.json().await?)
    }

    pub async fn inclusion_proof_by_chain_id(
        &self,
        commitment: &Field,
        chain_id: u64,
    ) -> eyre::Result<Option<InclusionProof>> {
        let url = format!(
            "{}/inclusionProof?chainId={chain_id}",
            self.world_tree_host
        );

        let response = self
            .client
            .post(&url)
            .json(&InclusionProofRequest {
                identity_commitment: *commitment,
            })
            .send()
            .await?;

        response.error_for_status_ref()?;

        Ok(response.json().await?)
    }
}
