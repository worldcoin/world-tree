use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use common::test_utilities::chain_mock::{spawn_mock_chain, MockChain};
use ethers::core::k256::elliptic_curve::bigint::modular::constant_mod::ResidueParams;
use ethers::providers::{Http, Middleware, Provider};
use ethers::types::{H160, U256};
use ethers_throttle::ThrottledProvider;
use eyre::anyhow;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use governor::Jitter;
use hyper::StatusCode;
use rand::seq::IteratorRandom;
use semaphore::lazy_merkle_tree::Canonical;
use semaphore::poseidon_tree::Proof;
use semaphore::Field;
use serde::{Deserialize, Serialize};
use tracing::Level;
use url::Url;
use world_tree::tree::error::TreeAvailabilityError;
use world_tree::tree::service::{
    InclusionProofRequest, TreeAvailabilityService,
};
use world_tree::tree::tree_data::{
    deserialize_proof, InclusionProof, TreeData,
};
use world_tree::tree::tree_updater::pack_indices;
use world_tree::tree::{Hash, PoseidonTree, WorldTree};

pub const TREE_DEPTH: usize = 30;
pub const TREE_HISTORY_SIZE: usize = 24;

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
        1000,
        middleware,
    );

    let world_tree = tree_availability_service.world_tree.clone();

    // Spawn the service in a separate task
    let server_handle = tokio::spawn(async move {
        let handles = tree_availability_service.serve(8080);

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
            root: Some(Hash::from_str(
                "0x05c1e52b41a571293b30efacd2afdb7173b20cfaf1f646c4ac9f96eb75848270",
            )?),
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

//TODO: check that roots are the same as the tree changed event
#[tokio::test]
#[ignore]
async fn validate_inclusion_proof_against_signup_sequencer() -> eyre::Result<()>
{
    let args = std::env::args().collect::<String>();
    if args.contains("--nocapture") {
        common::tracing::init_subscriber(Level::INFO);
    }

    // Initialize a new WorldTree
    let rpc_endpoint = std::env::var("MAINNET_RPC_ENDPOINT")?;
    let http_provider = Http::new(Url::parse(&rpc_endpoint)?);
    let throttled_http_provider = ThrottledProvider::new(
        http_provider,
        5,
        Some(Jitter::new(
            Duration::from_millis(10),
            Duration::from_millis(100),
        )),
    );

    let middleware = Arc::new(Provider::new(throttled_http_provider));

    let world_tree_address =
        H160::from_str("0xf7134CE138832c1456F2a91D64621eE90c2bddEa")?;
    let tree = PoseidonTree::<Canonical>::new(TREE_DEPTH, Hash::ZERO);

    let world_tree: WorldTree<Provider<ThrottledProvider<Http>>> =
        WorldTree::new(
            tree,
            TREE_HISTORY_SIZE,
            world_tree_address,
            17636832,
            10000,
            middleware,
        );

    // Sync the WorldTree to chain head
    let tree_data = world_tree.tree_data.clone();
    world_tree.tree_updater.sync_to_head(&tree_data).await?;

    dbg!("Tree is synced");
    // Get random identities from the tree
    let tree = tree_data.tree.read().await;
    dbg!("Tree");

    let random_identities: Vec<ruint::Uint<256, 4>> = tree
        .leaves()
        .filter(|val| *val != Hash::ZERO)
        .take(1) //TODO: make this 10
        .collect::<Vec<Hash>>();

    dbg!(&random_identities);

    // Get expected proofs from the signup sequencer
    let sequencer_endpoint = std::env::var("SIGNUP_SEQUENCER_ENDPOINT")?;
    let client = reqwest::Client::new();

    let mut expected_proofs = vec![];
    for identity in &random_identities {
        let payload = InclusionProofRequest {
            identity_commitment: *identity,
            root: None,
        };

        let response = client
            .post(sequencer_endpoint.clone())
            .json(&payload)
            .send()
            .await?;

        let val = response.json::<serde_json::Value>().await?;

        let proof = InclusionProof {
            root: serde_json::from_value(
                val.get("root").expect("Could not get root").clone(),
            )?,

            proof: deserialize_proof(
                val.get("proof")
                    .expect("Could not get proof from Signup Sequencer"),
            )?,
            message: None,
        };

        expected_proofs.push(proof);
    }

    // Get proofs from the WorldTree
    let mut proofs = vec![];
    for (identity, expected_proof) in
        random_identities.iter().zip(expected_proofs.iter())
    {
        let proof = tree_data
            //TODO: make a note that we need to account for the mined root since the two roots are different?
            .get_inclusion_proof(*identity, Some(expected_proof.root))
            .await
            .expect("Could not get proof from WorldTree");

        proofs.push(proof);
    }

    dbg!(&proofs);
    dbg!(&expected_proofs);

    // // Assert proofs are equal
    // for (proof, expected_proof) in proofs.iter().zip(expected_proofs) {
    //     assert_eq!(proof.proof, expected_proof);
    // }

    //TODO: also assert that the root is the same

    Ok(())
}
