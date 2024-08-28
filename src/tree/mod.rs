pub mod block_scanner;
pub mod config;
pub mod error;
pub mod identity_tree;
pub mod newtypes;
pub mod service;

use std::path::Path;
use std::process;
use std::sync::Arc;

use config::{ProviderConfig, ServiceConfig};
use ethers::providers::{Http, Middleware, Provider};
use ethers_throttle::ThrottledJsonRpcClient;
use eyre::ContextCompat;
use semaphore::generic_storage::MmapVec;
use semaphore::lazy_merkle_tree::LazyMerkleTree;
use semaphore::merkle_tree::Hasher;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::sync::RwLock;
use tracing::info;

use self::error::WorldTreeResult;
use self::identity_tree::{IdentityTree, InclusionProof};
pub use self::newtypes::{ChainId, LeafIndex, NodeIndex};
use crate::db::{Db, DbMethods};

pub type PoseidonTree<Version> = LazyMerkleTree<PoseidonHash, Version>;
pub type Hash = <PoseidonHash as Hasher>::Hash;

pub type WorldTreeProvider = Arc<Provider<ThrottledJsonRpcClient<Http>>>;

/// The main state struct of this service
///
/// It holds all the stateful parts of the service as well as a copy of the running config
pub struct WorldTree {
    pub config: ServiceConfig,

    pub db: Arc<Db>,

    /// The identity tree is the main data structure that holds the state of the tree including latest roots, leaves, and an in-memory representation of the tree
    pub identity_tree: Arc<RwLock<IdentityTree<MmapVec<Hash>>>>,

    pub canonical_chain_id: ChainId,
}

impl WorldTree {
    pub async fn new(
        config: ServiceConfig,
        db: Arc<Db>,
        tree_depth: usize,
        cache: &Path,
    ) -> WorldTreeResult<Self> {
        let identity_tree =
            IdentityTree::new_with_cache_unchecked(tree_depth, cache)?;

        let canonical_provider =
            provider(&config.canonical_tree.provider).await?;
        let canonical_chain_id = canonical_provider.get_chainid().await?;
        let canonical_chain_id = ChainId(canonical_chain_id.as_u64());

        let world_tree = Self {
            config,
            db,
            identity_tree: Arc::new(RwLock::new(identity_tree)),
            canonical_chain_id,
        };

        let tree = world_tree.identity_tree.clone();
        let cache = cache.to_owned();

        // TODO: Move to a higher level? A task handler of sorts?
        tokio::task::spawn_blocking(move || {
            info!("Validating tree");
            let start = std::time::Instant::now();
            if let Err(e) = tree.blocking_read().tree.validate() {
                tracing::error!("Tree validation failed: {e:?}");
                tracing::info!("Deleting cache and exiting");
                std::fs::remove_file(cache).unwrap();
                process::exit(1);
            }
            info!("Tree validation completed in {:?}", start.elapsed());
        });

        Ok(world_tree)
    }

    // TODO: Cache?
    pub async fn canonical_provider(
        &self,
    ) -> WorldTreeResult<WorldTreeProvider> {
        provider(&self.config.canonical_tree.provider).await
    }

    /// Returns an inclusion proof for a given identity commitment.
    /// If a chain ID is provided, the proof is generated for the given chain.
    pub async fn inclusion_proof(
        &self,
        identity_commitment: Hash,
        chain_id: Option<ChainId>,
    ) -> WorldTreeResult<Option<InclusionProof>> {
        let identity_tree = self.identity_tree.read().await;

        let root = if let Some(chain_id) = chain_id {
            let root = self.db.root_by_chain(chain_id.0).await?;

            // match root {
            //     Some(root) => Some(root),
            //     None => {
            //         tracing::warn!("Missing root for chain: {:?}", chain_id);
            //         return Ok(None);
            //     }
            // }
            root
        } else {
            None
        };

        tracing::warn!("Root: {:?}", root);

        let leaf_idx = self
            .db
            .leaf_index(identity_commitment)
            .await?
            .context("Missing leaf index")?;

        tracing::warn!("Leaf index: {:?}", leaf_idx);

        let inclusion_proof =
            identity_tree.inclusion_proof(leaf_idx, root.as_ref())?;

        Ok(inclusion_proof)
    }

    /// Computes the updated root given a set of identity commitments.
    /// If a chain ID is provided, the updated root is calculated from the latest root on the specified chain.
    /// If no chain ID is provided, the updated root is calculated from the latest root bridged to all chains.
    pub async fn compute_root(
        &self,
        identity_commitements: &[Hash],
        chain_id: Option<ChainId>,
    ) -> WorldTreeResult<Hash> {
        let identity_tree = self.identity_tree.read().await;

        let root = if let Some(chain_id) = chain_id {
            self.db.root_by_chain(chain_id.0).await?
        } else {
            None
        };

        let updated_root =
            identity_tree.compute_root(identity_commitements, root.as_ref())?;

        Ok(updated_root)
    }
}

pub async fn provider(
    config: &ProviderConfig,
) -> WorldTreeResult<WorldTreeProvider> {
    let http_provider = Http::new(config.rpc_endpoint.clone());
    let throttled_provider =
        ThrottledJsonRpcClient::new(http_provider, config.throttle, None);

    let middleware = Arc::new(Provider::new(throttled_provider));

    Ok(middleware)
}
