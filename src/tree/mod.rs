pub mod block_scanner;
pub mod error;
pub mod identity_tree;
pub mod service;
pub mod tree_data;
pub mod tree_manager;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use error::TreeAvailabilityError;
use ethers::contract::ContractError;
use ethers::middleware::gas_oracle::MiddlewareError;
use ethers::providers::Middleware;
use ethers::types::{H160, U256};
use futures::stream::FuturesUnordered;
use ruint::Uint;
use semaphore::dynamic_merkle_tree::DynamicMerkleTree;
use semaphore::lazy_merkle_tree::{Canonical, LazyMerkleTree};
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::sync::mpsc::Sender;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::instrument;

use self::block_scanner::BlockScanner;
use self::tree_data::TreeData;
use crate::abi::IBridgedWorldID;

pub type PoseidonTree<Version> = LazyMerkleTree<PoseidonHash, Version>;
pub type Hash = <PoseidonHash as Hasher>::Hash;

pub const SYNC_TO_HEAD_SLEEP_SECONDS: u64 = 5;
