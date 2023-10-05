use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use ethers::{
    providers::{Middleware, MiddlewareError, PubsubClient},
    types::{Filter, H160, U256},
};
use semaphore::{
    merkle_tree::Hasher,
    poseidon_tree::{PoseidonHash, Proof},
};

pub type Hash = <PoseidonHash as Hasher>::Hash;

use ethers::prelude::abigen;
use tokio::task::JoinHandle;

use crate::error::StateBridgeError;

abigen!(
    IWorldIdIdentityManager,
    r#"[
        function latestRoot() external returns (uint256)
        event TreeChanged(uint256 indexed preRoot, uint8 indexed kind, uint256 indexed postRoot)
    ]"#;
);

pub struct WorldTreeRoot<M: Middleware + PubsubClient + 'static> {
    pub root: Hash,
    pub world_id_identity_manager: IWorldIdIdentityManager<M>,
    pub middleware: Arc<M>,
    pub root_tx: tokio::sync::broadcast::Sender<Hash>,
}

impl<M: Middleware + PubsubClient> WorldTreeRoot<M> {
    pub async fn new(
        world_tree_address: H160,
        middleware: Arc<M>,
    ) -> Result<Self, StateBridgeError<M>> {
        let (root_tx, _) = tokio::sync::broadcast::channel::<Hash>(1000);

        let world_id_identity_manager =
            IWorldIdIdentityManager::new(world_tree_address, middleware.clone());

        let latest_root: U256 = world_id_identity_manager.latest_root().await?;

        Ok(Self {
            root: ruint::Uint::from_limbs(latest_root.0),
            world_id_identity_manager,
            middleware,
            root_tx,
        })
    }

    pub async fn spawn(&self) -> JoinHandle<Result<(), StateBridgeError<M>>> {
        tokio::spawn(async move {
            //TODO: create a filter to subscribe to the TreeChanged event from the WorldIdIdentityManager contract

            //TODO: Listen to a stream of events, when a new event is received, update the root and block number

            //TODO: send it through the tx, you can convert ethers U256 to ruint with Uint::from_limbs()

            Ok(())
        })
    }
}
