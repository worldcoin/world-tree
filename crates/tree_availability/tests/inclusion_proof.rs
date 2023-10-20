use core::panic;
use std::str::FromStr;
use std::sync::Arc;

use ethers::providers::{Http, Provider};
use ethers::types::H160;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use hyper::StatusCode;
use tree_availability::error::TreeAvailabilityError;
use tree_availability::server::{InclusionProof, InclusionProofRequest};
use tree_availability::tree::Hash;
use tree_availability::TreeAvailabilityService;

#[tokio::test]
async fn test_inclusion_proof() -> eyre::Result<()> {
    let rpc_endpoint = std::env::var("GOERLI_RPC_ENDPOINT")?;
    let middleware = Arc::new(Provider::<Http>::try_from(rpc_endpoint)?);

    let world_tree_address =
        H160::from_str("0x78eC127A3716D447F4575E9c834d452E397EE9E1")?;

    let world_tree_creation_block = 9888280;

    let tree_availability_service = TreeAvailabilityService::new(
        30,
        10,
        10,
        world_tree_address,
        world_tree_creation_block,
        middleware,
    );

    let world_tree = tree_availability_service.world_tree.clone();

    // Spawn the service in a separate task
    let server_handle = tokio::spawn(async move {
        let handles = tree_availability_service.serve(None).await;

        dbg!("getting here");

        let mut handles = handles.into_iter().collect::<FuturesUnordered<_>>();
        while let Some(result) = handles.next().await {
            result.expect("TODO: propagate this error")?;
        }

        Ok::<(), TreeAvailabilityError<_>>(())
    });

    // Wait for the tree to be synced
    loop {
        if world_tree.synced.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    // Send a request to get an inclusion proof
    let client = reqwest::Client::new();
    let response = client
        .post("http://127.0.0.1:8080/inclusionProof")
        .json(&InclusionProofRequest {
            identity_commitment: Hash::from(0x01), //TODO: update to use a commitment in the tree
            root: None,
        })
        .send()
        .await?;

    // Check response
    assert_eq!(response.status(), StatusCode::OK);
    // let proof: Option<InclusionProof> = response.json().await?;
    // assert!(proof.is_some());

    // Cleanup: Shutdown the server task
    server_handle.abort();

    Ok(())
}
