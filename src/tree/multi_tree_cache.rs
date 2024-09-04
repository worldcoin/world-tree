use std::collections::HashMap;
use std::path::{Path, PathBuf};

use semaphore::cascading_merkle_tree::CascadingMerkleTree;
use semaphore::generic_storage::MmapVec;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::sync::RwLock;

use super::error::WorldTreeResult;
use super::{ChainId, Hash};

pub struct MultiTreeCache<S> {
    pub canonical: RwLock<CascadingMerkleTree<PoseidonHash, S>>,
    pub trees: HashMap<ChainId, RwLock<CascadingMerkleTree<PoseidonHash, S>>>,
}

impl MultiTreeCache<MmapVec<Hash>> {
    pub fn init(
        depth: usize,
        cache_dir: impl AsRef<Path>,
        chain_ids: &[ChainId],
    ) -> WorldTreeResult<Self> {
        let cache_dir = cache_dir.as_ref();

        std::fs::create_dir_all(cache_dir)?;

        let mmap_vecs: HashMap<ChainId, MmapVec<Hash>> = chain_ids
            .iter()
            .map(|chain_id| {
                let path = Self::tree_cache_path(cache_dir, *chain_id);
                let storage = Self::init_storage(&path)?;
                WorldTreeResult::Ok((*chain_id, storage))
            })
            .collect::<Result<_, _>>()?;

        let trees: HashMap<
            ChainId,
            RwLock<CascadingMerkleTree<PoseidonHash, MmapVec<Hash>>>,
        > = mmap_vecs
            .into_iter()
            .map(|(chain_id, mmap_vec)| {
                let path = Self::tree_cache_path(cache_dir, chain_id);
                let tree = Self::init_tree(depth, path, mmap_vec)?;

                let tree = RwLock::new(tree);

                WorldTreeResult::Ok((chain_id, tree))
            })
            .collect::<Result<_, _>>()?;

        let canonical_cache_path = cache_dir.join("canonical.cache");
        let canonical_storage = Self::init_storage(&canonical_cache_path)?;
        let canonical =
            Self::init_tree(depth, canonical_cache_path, canonical_storage)?;

        Ok(Self {
            canonical: RwLock::new(canonical),
            trees,
        })
    }

    fn tree_cache_path(
        cache_dir: impl AsRef<Path>,
        chain_id: ChainId,
    ) -> PathBuf {
        cache_dir.as_ref().join(format!("{}.cache", chain_id))
    }

    fn init_storage(path: impl AsRef<Path>) -> WorldTreeResult<MmapVec<Hash>> {
        let path = path.as_ref();

        let mmap_vec: MmapVec<Hash> =
            match unsafe { MmapVec::restore_from_path(path) } {
                Ok(mmap_vec) => mmap_vec,

                Err(e) => unsafe {
                    tracing::error!("Cache restore error: {:?}", e);
                    MmapVec::create_from_path(path)?
                },
            };

        WorldTreeResult::Ok(mmap_vec)
    }

    fn init_tree(
        depth: usize,
        path: impl AsRef<Path>,
        mmap_vec: MmapVec<Hash>,
    ) -> WorldTreeResult<CascadingMerkleTree<PoseidonHash, MmapVec<Hash>>> {
        let path = path.as_ref();

        let tree =
            match CascadingMerkleTree::<PoseidonHash, _>::restore_unchecked(
                mmap_vec,
                depth,
                &Hash::ZERO,
            ) {
                Ok(tree) => tree,
                Err(e) => {
                    tracing::error!(
                        "Error to restoring tree from cache {e:?}, purging cache and creating new tree"
                    );

                    // Remove the existing cache and create a new cache file
                    std::fs::remove_file(path)?;
                    let mmap_vec = unsafe { MmapVec::create_from_path(path)? };

                    CascadingMerkleTree::<PoseidonHash, _>::new(
                        mmap_vec,
                        depth,
                        &Hash::ZERO,
                    )
                }
            };
        Ok(tree)
    }
}
