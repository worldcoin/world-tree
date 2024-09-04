pub mod block_scanner;
pub mod config;
pub mod error;
pub mod identity_tree;
pub mod multi_tree_cache;
pub mod newtypes;
pub mod service;

use std::path::Path;
use std::process;
use std::sync::Arc;

use config::{ProviderConfig, ServiceConfig};
use ethers::providers::{Http, Middleware, Provider};
use ethers_throttle::ThrottledJsonRpcClient;
use eyre::ContextCompat;
use multi_tree_cache::MultiTreeCache;
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

    pub cache: Arc<MultiTreeCache<MmapVec<Hash>>>,

    pub canonical_chain_id: ChainId,

    pub chain_ids: Vec<ChainId>,
}

impl WorldTree {
    pub async fn new(
        config: ServiceConfig,
        db: Arc<Db>,
    ) -> WorldTreeResult<Self> {
        let tree_depth = config.tree_depth;

        let (canonical_chain_id, chain_ids) = fetch_chain_ids(&config).await?;

        let cache =
            MultiTreeCache::init(tree_depth, &config.cache.dir, &chain_ids)?;
        let cache = Arc::new(cache);

        let cache_dir = config.cache.dir.clone();

        let world_tree = Self {
            config,
            db,
            cache: cache.clone(),
            canonical_chain_id,
            chain_ids: chain_ids.clone(),
        };

        // TODO: Move to a higher level? A task handler of sorts?
        {
            let cache = cache.clone();
            let cache_dir = cache_dir.clone();

            tokio::task::spawn_blocking(move || {
                info!("Validating canonical tree");
                let start = std::time::Instant::now();
                if let Err(e) = cache.canonical.blocking_read().validate() {
                    tracing::error!("Tree validation failed: {e:?}");
                    tracing::info!("Deleting cache and exiting");
                    std::fs::remove_file(cache_dir).unwrap();
                    process::exit(1);
                }
                let elapsed = start.elapsed();
                let elapsed_ms = elapsed.as_millis();
                info!(
                    ?elapsed,
                    elapsed_ms, "Canonical tree validation complete"
                );
            });
        }

        for chain_id in chain_ids.iter() {
            let cache = cache.clone();
            let cache_dir = cache_dir.clone();
            let chain_id = *chain_id;

            tokio::task::spawn_blocking(move || {
                info!(%chain_id, "Validating tree");
                let start = std::time::Instant::now();
                if let Err(e) =
                    cache.trees[&chain_id].blocking_read().validate()
                {
                    tracing::error!("Tree validation failed: {e:?}");
                    tracing::info!("Deleting cache and exiting");
                    std::fs::remove_file(cache_dir).unwrap();
                    process::exit(1);
                }

                let elapsed = start.elapsed();
                let elapsed_ms = elapsed.as_millis();
                info!(%chain_id, ?elapsed, elapsed_ms, "Tree validation complete");
            });
        }

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
        let leaf_idx = self
            .db
            .leaf_index(identity_commitment)
            .await?
            .context("Missing leaf index")?;

        let tree_lock = if let Some(chain_id) = chain_id {
            self.cache
                .trees
                .get(&chain_id)
                .context("Missing tree")?
                .read()
                .await
        } else {
            self.cache.canonical.read().await
        };

        let proof = tree_lock.proof(leaf_idx as usize);
        let root = tree_lock.root();

        let inclusion_proof = InclusionProof { proof, root };

        Ok(Some(inclusion_proof))
    }

    // /// Computes the updated root given a set of identity commitments.
    // /// If a chain ID is provided, the updated root is calculated from the latest root on the specified chain.
    // /// If no chain ID is provided, the updated root is calculated from the latest root bridged to all chains.
    // pub async fn compute_root(
    //     &self,
    //     identity_commitements: &[Hash],
    //     chain_id: Option<ChainId>,
    // ) -> WorldTreeResult<Hash> {
    //     let identity_tree = self.identity_tree.read().await;

    //     let root = if let Some(chain_id) = chain_id {
    //         self.db.root_by_chain(chain_id.0).await?
    //     } else {
    //         None
    //     };

    //     let updated_root =
    //         identity_tree.compute_root(identity_commitements, root.as_ref())?;

    //     Ok(updated_root)
    // }
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

pub async fn fetch_chain_ids(
    config: &ServiceConfig,
) -> WorldTreeResult<(ChainId, Vec<ChainId>)> {
    let canonical_provider = provider(&config.canonical_tree.provider).await?;
    let canonical_chain_id = canonical_provider.get_chainid().await?;
    let canonical_chain_id = ChainId(canonical_chain_id.as_u64());

    let mut providers = vec![];
    for tree_config in &config.bridged_trees {
        let provider = provider(&tree_config.provider).await?;
        providers.push(provider);
    }

    let mut chain_ids = vec![canonical_chain_id];
    for provider in providers {
        let chain_id = provider.get_chainid().await?;
        chain_ids.push(ChainId(chain_id.as_u64()));
    }

    Ok((canonical_chain_id, chain_ids))
}
