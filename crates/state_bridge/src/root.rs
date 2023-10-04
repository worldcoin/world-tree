use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use ethers::{
    providers::{Middleware, PubsubClient},
    types::{H160, U256},
};
use semaphore::{
    merkle_tree::Hasher,
    poseidon_tree::{PoseidonHash, Proof},
};

pub type Hash = <PoseidonHash as Hasher>::Hash;

//TODO: good first issue, create the spawn method for the WorldTreeRoot to listen to new roots from the canonical tree and send them through the channel.

pub struct WorldTreeRoot<M: Middleware + PubsubClient> {
    pub root: Hash,
    pub block_number: u64, //TODO: update to use a different type
    pub world_tree_address: H160,
    pub middleware: Arc<M>,
    pub root_tx: tokio::sync::broadcast::Sender<Hash>,
}

impl<M: Middleware + PubsubClient> WorldTreeRoot<M> {
    pub async fn new(world_tree_address: H160, middleware: Arc<M>) -> Self {
        let (root_tx, _) = tokio::sync::broadcast::channel::<Hash>(1000);

        //TODO: get the root and current block number

        todo!()
    }
}
