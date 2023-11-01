use std::str::FromStr;

use common::test_utilities::chain_mock::{spawn_mock_chain, MockChain};
use ethers::providers::Middleware;
use ethers::types::U256;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use hyper::StatusCode;
use tree_availability::error::TreeAvailabilityError;
use tree_availability::server::{InclusionProof, InclusionProofRequest};
use tree_availability::world_tree::tree_updater::pack_indices;
use tree_availability::world_tree::Hash;
use tree_availability::TreeAvailabilityService;
#[tokio::test]
async fn test_inclusion_proof() -> eyre::Result<()> {
    // Initialize a new mock tree
    let MockChain {
        anvil: _anvil,
        middleware,
        mock_world_id,
        ..
    } = spawn_mock_chain().await?;

    // Register identities
    let identity_commitments =
        vec![U256::from(1), U256::from(2), U256::from(3)];

    let world_tree_creation_block =
        middleware.get_block_number().await?.as_u64() - 1;

    mock_world_id
        .register_identities(
            [U256::zero(); 8],
            U256::zero(),
            0,
            identity_commitments,
            U256::zero(),
        )
        .send()
        .await?
        .await?;

    // Delete an identity
    mock_world_id
        .delete_identities(
            [U256::zero(); 8],
            pack_indices(&[1]).into(),
            U256::zero(),
            U256::zero(),
        )
        .send()
        .await?
        .await?;

    // Initialize the tree availability service
    let world_tree_address = mock_world_id.address();
    let tree_availability_service = TreeAvailabilityService::new(
        3,
        1,
        5,
        world_tree_address,
        world_tree_creation_block,
        Some(10000),
        middleware,
    );

    let world_tree = tree_availability_service.world_tree.clone();

    // Spawn the service in a separate task
    let server_handle = tokio::spawn(async move {
        let handles = tree_availability_service.serve(8080).await;

        let mut handles = handles.into_iter().collect::<FuturesUnordered<_>>();
        while let Some(result) = handles.next().await {
            result.expect("TODO: propagate this error")?;
        }

        Ok::<(), TreeAvailabilityError<_>>(())
    });

    // Wait for the tree to be synced
    loop {
        if world_tree
            .tree_updater
            .synced
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            break;
        }

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    // Send a request to get an inclusion proof
    let client = reqwest::Client::new();
    let response = client
        .post("http://127.0.0.1:8080/inclusionProof")
        .json(&InclusionProofRequest {
            identity_commitment: Hash::from(0x01),
            root: None,
        })
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let proof: Option<InclusionProof> = response.json().await?;
    assert!(proof.is_some());

    // Try to get an inclusion proof for an identity at a historical root
    let client = reqwest::Client::new();
    let response = client
        .post("http://127.0.0.1:8080/inclusionProof")
        .json(&InclusionProofRequest {
            identity_commitment: Hash::from(0x01),
            root: Some(Hash::from_str("0x05c1e52b41a571293b30efacd2afdb7173b20cfaf1f646c4ac9f96eb75848270")?),
        })
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let proof: Option<InclusionProof> = response.json().await?;
    assert!(proof.is_some());

    // Try to get an inclusion proof for an identity that has been deleted
    let client = reqwest::Client::new();
    let response = client
        .post("http://127.0.0.1:8080/inclusionProof")
        .json(&InclusionProofRequest {
            identity_commitment: Hash::from(0x02),
            root: None,
        })
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let proof: Option<InclusionProof> = response.json().await?;
    assert!(proof.is_none());

    // Cleanup the server resources
    server_handle.abort();

    Ok(())
}
