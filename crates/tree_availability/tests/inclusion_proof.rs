use std::str::FromStr;

use common::test_utilities::chain_mock::{spawn_mock_chain, MockChain};
use ethers::abi::Uint;
use ethers::types::U256;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use hyper::StatusCode;
use tree_availability::error::TreeAvailabilityError;
use tree_availability::server::InclusionProofRequest;
use tree_availability::world_tree::Hash;
use tree_availability::TreeAvailabilityService;

#[tokio::test]
async fn test_inclusion_proof() -> eyre::Result<()> {
    #[allow(unused_variables)]
    let MockChain {
        anvil,
        middleware,
        mock_world_id,
        ..
    } = spawn_mock_chain().await?;

    //let rpc_endpoint = std::env::var("GOERLI_RPC_ENDPOINT")?;
    //let middleware = Arc::new(Provider::<Http>::try_from(rpc_endpoint)?);

    //let world_tree_address =
    // H160::from_str("0x78eC127A3716D447F4575E9c834d452E397EE9E1")?;

    ///////////////////////////////////////////////////////////////////
    ///                          TEST DATA                          ///
    ///////////////////////////////////////////////////////////////////
    let insertion_start_index = 0;

    // Taken from https://github.com/worldcoin/world-id-contracts/blob/main/src/test/identity-manager/WorldIDIdentityManagerTest.sol
    let insertion_pre_root = U256::from_str(
        "0x2a7c7c9b6ce5880b9f6f228d72bf6a575a526f29c66ecceef8b753d38bba7323",
    )?;

    let insertion_post_root = U256::from_str(
        "0x193289951bec3e4a099d9f1b0fb22cf20fe9dc4ea75c253352f22848b08c888b",
    )?;

    let identity_commitments =
        vec![U256::from(1), U256::from(2), U256::from(3)];

    // proof is ignored in MockWorldIDIdentityManager
    let mock_insertion_proof: [U256; 8] = [
        U256::from_str("0x12bca28c242e87a12637280e957ee6eefa29446e11cd4e49f86df9fd320fbdba")?,
        U256::from_str("0x1f399a7953de230a6da3a70e49c284b60ee5040cd71ee668994f70f49ff73489")?,
        U256::from_str("0x9b28c42a4b34507ae0ae61f72342888b64a2f89c1c45e2e897a53ee2b946406")?,
        U256::from_str("0x28a5825073e62d01c9f5c8172664349180df8a09d68026a29df91500964a0a72")?,
        U256::from_str("0x24356f7d8e4565514fc8a575834b6b37bd7df399ae99424187f30591557e4c4a")?,
        U256::from_str("0x177dc415a6d6e866c0125a0f73f41e7a5b291f18a90864e5fbe106c67df82f67")?,
        U256::from_str("0x1fb872833d2373abaa26cca40f498b878cef8134cae18cc703e68f99b9938fa5")?,
        U256::from_str("0x1487bc571a30416ad9124f5f8ef9e2efe5a0d720a710f6abf0095004f250b306")?,
    ];

    let deletion_pre_root = U256::from_str(
        "0x18cb13df3e79b9f847a1494d0a2e6f3cc0041d9cae7e5ccb8cd1852ecdc4af58",
    )?;

    let deletion_post_root = U256::from_str(
        "0x82fcf94594d7363636338e2c29242cc77e3d04f36c8ad64d294d2ab4d251708",
    )?;

    // proof is ignored in MockWorldIDIdentityManager
    let mock_deletion_proof: [U256;8] = [
        U256::from_str("0x19233cf0c60aa740585125dd5936462b1795bc2c8da7b9d0b7e92392cf91e1fd")?,
        U256::from_str("0x244096da06de365f3bd8e7f428c2de4214096c4cd0feeba579642435ab15e90a")?,
        U256::from_str("0x107395cd3aa9bfe3bcaada7f171d43a1ffffd576325598e2b9c8fbe1cfd6d032")?,
        U256::from_str("0xac23f21fb0376055adeee2a78491ca13afc288c63c6450d0ce6ded6fda14344")?,
        U256::from_str("0x29022f4cf64701ff88807430b9e333d87c670a4bdfe7d495d76271044a2d3711")?,
        U256::from_str("0x134e41bef89e02289885852b368395b1b679dd243e5cf9e2f36b04ba990ab6a2")?,
        U256::from_str("0x280894db66e6a9f9bf8aa48ffa1de98b755adadcf5962fb308cd1802a1101a0c")?,
        U256::from_str("0x1484814b74243a07930c6af61079f94eefd843efe95e2388d9d49956cfacf3ab")?,
    ];

    mock_world_id
        .register_identities(
            mock_insertion_proof,
            insertion_pre_root,
            insertion_start_index,
            identity_commitments,
            insertion_post_root,
        )
        .send()
        .await?
        .await?;

    let world_tree_address = mock_world_id.address();

    let world_tree_creation_block = 0;

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
