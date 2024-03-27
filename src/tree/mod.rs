pub mod block_scanner;
pub mod error;
pub mod identity_tree;
pub mod service;
pub mod tree_data;
pub mod tree_manager;

use semaphore::lazy_merkle_tree::LazyMerkleTree;
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::PoseidonHash;

pub type PoseidonTree<Version> = LazyMerkleTree<PoseidonHash, Version>;
pub type Hash = <PoseidonHash as Hasher>::Hash;

pub const SYNC_TO_HEAD_SLEEP_SECONDS: u64 = 5;
