//! # Tree Availability Service
//!
//! The tree availability service is able to create an in-memory representation of the World ID
//! merkle tree by syncing the state of the World ID contract `registerIdentities` and `deleteIdentities`
//! function calldata and `TreeChanged` events. Once it syncs the latest state of the state of the tree, it
//! is able to serve inclusion proofs on the `/inclusionProof` endpoint. It also keeps a historical roots list
//! of `tree_history_size` size in order to serve proofs against older tree roots (including the roots of
//! World IDs bridged to other networks).
//!
//! ### Usage
//!
//! #### CLI
//!
//! In order to run the tree availability service you can run the following commands
//!
//! ```bash
//! # Build the binary
//! cargo build --bin tree-availability-service --release
//! # Run the command
//! ./target/release/tree-availability-service  
//!  --tree-depth <TREE_DEPTH>                  
//!  --tree-history-size <TREE_HISTORY_SIZE>    
//!  --dense-prefix-depth <DENSE_PREFIX_DEPTH>  
//!  --address <ADDRESS>                        
//!  --creation-block <CREATION_BLOCK>          
//!  --rpc-endpoint <RPC_ENDPOINT>              
//! ```
//!
//! #### Library
//!
//! A good example of how to use the `StateBridgeService` struct in a library is demonstrated in the [`inclusion_proof.rs`](https://github.com/worldcoin/identity-sequencer/blob/main/crates/tree_availability/tests/inclusion_proof.rs) test file.
//!
//! ```
//! use std::str::FromStr;
//!
//! use common::test_utilities::chain_mock::{spawn_mock_chain, MockChain};
//! use ethers::providers::Middleware;
//! use ethers::types::U256;
//! use futures::stream::FuturesUnordered;
//! use futures::StreamExt;
//! use hyper::StatusCode;
//! use tree_availability::error::TreeAvailabilityError;
//! use tree_availability::server::{InclusionProof, InclusionProofRequest};
//! use tree_availability::world_tree::tree_updater::pack_indices;
//! use tree_availability::world_tree::Hash;
//! use tree_availability::TreeAvailabilityService;
//! #[tokio::test]
//! async fn test_inclusion_proof() -> eyre::Result<()> {
//!     // Initialize a new mock tree
//!     let MockChain {
//!         anvil: _anvil,
//!         middleware,
//!         mock_world_id,
//!         ..
//!     } = spawn_mock_chain().await?;
//!
//!     // Register identities
//!     let identity_commitments =
//!         vec![U256::from(1), U256::from(2), U256::from(3)];
//!
//!     let world_tree_creation_block =
//!         middleware.get_block_number().await?.as_u64() - 1;
//!
//!     mock_world_id
//!         .register_identities(
//!             [U256::zero(); 8],
//!             U256::zero(),
//!             0,
//!             identity_commitments,
//!             U256::zero(),
//!         )
//!         .send()
//!         .await?
//!         .await?;
//!
//!     // Delete an identity
//!     mock_world_id
//!         .delete_identities(
//!             [U256::zero(); 8],
//!             pack_indices(&[1]).into(),
//!             U256::zero(),
//!             U256::zero(),
//!         )
//!         .send()
//!         .await?
//!         .await?;
//!
//!     // Initialize the tree availability service
//!     let world_tree_address = mock_world_id.address();
//!     let tree_availability_service = TreeAvailabilityService::new(
//!         3,
//!         1,
//!         5,
//!         world_tree_address,
//!         world_tree_creation_block,
//!         middleware,
//!     );
//!
//!     let world_tree = tree_availability_service.world_tree.clone();
//!
//!     // Spawn the service in a separate task
//!     let server_handle = tokio::spawn(async move {
//!         let handles = tree_availability_service.serve(8080).await;
//!
//!         let mut handles = handles.into_iter().collect::<FuturesUnordered<_>>();
//!         while let Some(result) = handles.next().await {
//!             result.expect("TODO: propagate this error")?;
//!         }
//!
//!         Ok::<(), TreeAvailabilityError<_>>(())
//!     });
//!
//!     // Wait for the tree to be synced
//!     loop {
//!         if world_tree
//!             .tree_updater
//!             .synced
//!             .load(std::sync::atomic::Ordering::Relaxed)
//!         {
//!             break;
//!         }
//!
//!         tokio::time::sleep(std::time::Duration::from_secs(1)).await;
//!     }
//!
//!     // Send a request to get an inclusion proof
//!     let client = reqwest::Client::new();
//!     let response = client
//!         .post("http://127.0.0.1:8080/inclusionProof")
//!         .json(&InclusionProofRequest {
//!             identity_commitment: Hash::from(0x01),
//!             root: None,
//!         })
//!         .send()
//!         .await?;
//!
//!     assert_eq!(response.status(), StatusCode::OK);
//!
//!     // Return an inclusion proof
//!     let proof: Option<InclusionProof> = response.json().await?;
//!     assert!(proof.is_some());
//! }
//! ```

pub mod error;
pub mod server;
pub mod world_tree;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use error::TreeAvailabilityError;
use ethers::providers::Middleware;
use ethers::types::H160;
use semaphore::lazy_merkle_tree::Canonical;
use tokio::task::JoinHandle;
use world_tree::{Hash, PoseidonTree, WorldTree};

use crate::server::{inclusion_proof, synced};

/// The tree availability service data structure
pub struct TreeAvailabilityService<M: Middleware + 'static> {
    /// The World ID merkle tree synced from the `WorldIDIdentityManager` contract
    pub world_tree: Arc<WorldTree<M>>,
}

impl<M: Middleware> TreeAvailabilityService<M> {
    /// Constructor
    /// `tree_depth`: the depth of the World ID contract (currently 30 in production - Nov 2023)
    /// `dense_prefix_depth`: what is the depth of the tree that is densely populated (currently depth 10)
    /// `tree_history_size`: how many historical roots and derived trees to hold in memory to serve proofs for
    /// `world_tree_address`: `WorldIDIdentityManager` contract address
    /// `world_tree_creation_block`: block at which `WorldIDIdentityManager` contract was deployed
    /// `middleware`: Ethereum provider
    pub fn new(
        tree_depth: usize,
        dense_prefix_depth: usize,
        tree_history_size: usize,
        world_tree_address: H160,
        world_tree_creation_block: u64,
        middleware: Arc<M>,
    ) -> Self {
        let tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
            tree_depth,
            dense_prefix_depth,
            &Hash::ZERO,
        );

        let world_tree = Arc::new(WorldTree::new(
            tree,
            tree_history_size,
            world_tree_address,
            world_tree_creation_block,
            middleware,
        ));

        Self { world_tree }
    }

    /// Spins up the /inclusionProof endpoint to serve proofs of inclusion for the World ID tree
    /// `port`: which port on the machine will serve HTTP requests
    pub async fn serve(
        self,
        port: u16,
    ) -> Vec<JoinHandle<Result<(), TreeAvailabilityError<M>>>> {
        let mut handles = vec![];

        // Initialize a new router and spawn the server
        let router = axum::Router::new()
            .route("/inclusionProof", axum::routing::post(inclusion_proof))
            .route("/synced", axum::routing::post(synced))
            .with_state(self.world_tree.clone());

        let address =
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port);

        let server_handle = tokio::spawn(async move {
            axum::Server::bind(&address)
                .serve(router.into_make_service())
                .await
                .map_err(TreeAvailabilityError::HyperError)?;

            Ok(())
        });

        handles.push(server_handle);

        // Spawn a new task to keep the world tree synced to the chain head
        handles.push(self.world_tree.spawn().await);

        handles
    }
}
