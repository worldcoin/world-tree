use std::collections::{HashMap, VecDeque};

use semaphore::lazy_merkle_tree::{Canonical, Derived, VersionMarker};
use semaphore::poseidon_tree::{Branch, Proof};
use semaphore::Field;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use super::{Hash, PoseidonTree};

macro_rules! current_unix_timestamp {
    () => {{
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs()
    }};
}

/// Represents the in-memory state of the World Tree, caching historical roots up to `tree_history_size`.
#[derive(Clone)]
pub struct TreeData {
    /// A canonical in-memory representation of the World Tree.
    pub tree: PoseidonTree<Derived>,
    /// Depth of the merkle tree.
    pub depth: usize,
    /// The number of historical tree roots to cache for serving older proofs.
    pub tree_history_size: usize,
    /// Timestamp representing when the most recent root was received.
    pub latest_root_timestamp: u64,
    /// Cache of historical tree state, used to serve proofs against older roots. If the cache becomes larger than `tree_history_size`, the oldest roots are removed on a FIFO basis.
    pub tree_history: VecDeque<HistoricalTree>,
    /// Maintains the valid leaves in tree as well as the timestamp that they were inserted.
    pub leaves: HashMap<Hash, u64>,
}

impl TreeData {
    /// * `tree` - PoseidonTree representing the World Tree onchain, which will be used to generate inclusion proofs.
    /// * `tree_history_size` - Number of previous tree states to retain for serving proofs with historical roots.
    pub fn new(
        tree: PoseidonTree<Canonical>,
        tree_history_size: usize,
    ) -> Self {
        Self {
            tree_history_size,
            depth: tree.depth(),
            tree: tree.derived(),
            tree_history: VecDeque::new(),
            leaves: HashMap::new(),
            latest_root_timestamp: 0,
        }
    }

    /// Inserts multiple identity commitments starting from a specified index. The tree state before the insert operation is cached to tree history.
    ///
    /// # Arguments
    ///
    /// * `start_index` - The leaf index in the tree to begin inserting identity commitments.
    /// * `identities` - The array of identity commitments to insert.
    pub fn insert_many_at(&mut self, start_index: usize, identities: &[Hash]) {
        self.cache_tree_history();

        let timestamp = current_unix_timestamp!();
        for (i, identity) in identities.iter().enumerate() {
            let idx = start_index + i;
            self.tree = self.tree.update(idx, identity);
            self.leaves.insert(*identity, timestamp);

            tracing::info!(?identity, ?idx, "Inserted identity");
        }

        self.latest_root_timestamp = timestamp;
    }

    /// Deletes multiple identity commitments at specified indices. The tree state before the delete operation is cached to tree history.
    ///
    /// # Arguments
    ///
    /// * `delete_indices` - The indices of the leaves in the tree to delete.
    pub fn delete_many(&mut self, delete_indices: &[usize]) {
        self.cache_tree_history();

        for idx in delete_indices.iter() {
            let identity = self.tree.get_leaf(*idx);
            self.leaves.remove(&identity);

            self.tree = self.tree.update(*idx, &Hash::ZERO);
            tracing::info!(?idx, "Deleted identity");
        }

        self.latest_root_timestamp = current_unix_timestamp!();
    }

    /// Caches the current tree state to `tree_history` if `tree_history_size` is greater than 0.
    pub fn cache_tree_history(&mut self) {
        if self.tree_history_size != 0 {
            if self.tree_history.len() == self.tree_history_size {
                let historical_tree = self
                    .tree_history
                    .pop_back()
                    .expect("Tree history length should be > 0");

                let historical_root = historical_tree.tree.root();
                tracing::info!(?historical_root, "Popping tree from history",);
            }

            let new_root = self.tree.root();
            tracing::info!(?new_root, "Pushing tree to history",);

            self.tree_history.push_front(HistoricalTree::new(
                self.tree.clone(),
                self.latest_root_timestamp,
            ));
        }
    }

    /// Fetches the inclusion proof for a given identity against a specified root. If no root is specified, the latest root is used. Returns `None` if root or identity is not found.
    ///
    /// # Arguments
    ///
    /// * `identity` - The identity commitment for which to fetch the inclusion proof.
    /// * `root` - Optional root hash to serve the inclusion proof against. If `None`, uses the latest root.
    pub fn get_inclusion_proof(
        &self,
        identity: Hash,
        root: Option<Hash>,
    ) -> Option<InclusionProof> {
        // Get the timestamp that the leaf was inserted into the tree. If the leaf does not exist, None will be returned.
        let leaf_timestamp = self.leaves.get(&identity)?;

        if let Some(root) = root {
            // If the root is the latest root, use the current version of the tree
            if root == self.tree.root() {
                tracing::info!(?identity, ?root, "Getting inclusion proof");

                return Some(InclusionProof::new(
                    root,
                    Self::proof(&self.tree, identity)?,
                ));
            } else {
                // Otherwise, search the tree history for the root and use the corresponding tree
                for prev_tree in self.tree_history.iter() {
                    if prev_tree.tree.root() == root {
                        // If the tree root was committed after the leaf, the leaf does not exist in this tree
                        if prev_tree.root_timestamp < *leaf_timestamp {
                            //TODO: return some error here if the leaf is not in the tree
                            return None;
                        }

                        tracing::info!(
                            ?identity,
                            ?root,
                            "Getting inclusion proof"
                        );

                        return Some(InclusionProof::new(
                            root,
                            Self::proof(&prev_tree.tree, identity)?,
                        ));
                    }
                }
            }

            //TODO: should return an error if the tree root is specified but not in history
            tracing::warn!(
                ?identity,
                ?root,
                "Could not get inclusion proof. Root not in tree history."
            );
            None
        } else {
            let latest_root = self.tree.root();
            tracing::info!(?identity, ?latest_root, "Getting inclusion proof");
            // If the root is not specified, return a proof at the latest root
            Some(InclusionProof::new(
                latest_root,
                Self::proof(&self.tree, identity)?,
            ))
        }
    }

    /// Generates an inclusion proof for a specific identity commitment from a given `PoseidonTree`.
    ///
    /// # Arguments
    ///
    /// * `tree` - The Poseidon tree to fetch the inclusion proof against.
    /// * `idx` - Leaf index of the target identity to generate the inclusion proof for.
    fn proof<V: VersionMarker>(
        tree: &PoseidonTree<V>,
        identity: Hash,
    ) -> Option<Proof> {
        let idx = tree.leaves().position(|leaf| leaf == identity)?;

        Some(tree.proof(idx))
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InclusionProof {
    pub root: Field,
    //TODO: Open a PR to semaphore-rs to deserialize proof instead of implementing deserialization here
    #[serde(deserialize_with = "deserialize_proof")]
    pub proof: Proof,
}

impl InclusionProof {
    pub fn new(root: Field, proof: Proof) -> InclusionProof {
        Self { root, proof }
    }
}

#[derive(Clone)]
pub struct HistoricalTree {
    pub tree: PoseidonTree<Derived>,
    pub root_timestamp: u64,
}

impl HistoricalTree {
    pub fn new(tree: PoseidonTree<Derived>, root_timestamp: u64) -> Self {
        HistoricalTree {
            tree,
            root_timestamp,
        }
    }
}

fn deserialize_proof<'de, D>(deserializer: D) -> Result<Proof, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Value = Deserialize::deserialize(deserializer)?;
    if let Value::Array(array) = value {
        let mut branches = vec![];
        for value in array {
            let branch = serde_json::from_value::<Branch>(value)
                .map_err(serde::de::Error::custom)?;
            branches.push(branch);
        }

        Ok(semaphore::merkle_tree::Proof(branches))
    } else {
        Err(D::Error::custom("Expected an array"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE_DEPTH: usize = 10;
    const NUM_IDENTITIES: usize = 10;
    const TREE_HISTORY_SIZE: usize = 5;

    fn initialize_tree_data(
        tree_depth: usize,
        tree_history_size: usize,
        num_identities: usize,
    ) -> (TreeData, PoseidonTree<Canonical>, Vec<Hash>) {
        let poseidon_tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
            tree_depth,
            tree_depth,
            &Hash::ZERO,
        );
        let ref_tree = PoseidonTree::<Canonical>::new_with_dense_prefix(
            tree_depth,
            tree_depth,
            &Hash::ZERO,
        );

        let identities: Vec<_> = (0..num_identities).map(Hash::from).collect();

        let tree: TreeData = TreeData::new(poseidon_tree, tree_history_size);

        (tree, ref_tree, identities)
    }

    #[tokio::test]
    async fn test_get_inclusion_proof() {
        let (mut tree_data, mut ref_tree, identities) =
            initialize_tree_data(TREE_DEPTH, TREE_HISTORY_SIZE, NUM_IDENTITIES);

        tree_data.insert_many_at(0, &identities);

        for (idx, identity) in identities.iter().enumerate() {
            ref_tree = ref_tree.update_with_mutation(idx, identity);
        }

        assert_eq!(
            tree_data.tree_history.len(),
            1,
            "We should have 1 entry in tree history"
        );

        let root = ref_tree.root();

        for (i, identity) in identities.iter().enumerate().take(NUM_IDENTITIES)
        {
            let proof_from_world_tree = tree_data
                .get_inclusion_proof(*identity, Some(root))
                .unwrap();

            assert_eq!(ref_tree.proof(i), proof_from_world_tree.proof);
        }
    }

    #[tokio::test]
    async fn test_get_inclusion_proof_for_intermediate_root() {
        let (mut tree_data, mut ref_tree, identities) =
            initialize_tree_data(TREE_DEPTH, TREE_HISTORY_SIZE, NUM_IDENTITIES);

        for (idx, identity) in identities.iter().enumerate().take(5) {
            ref_tree = ref_tree.update_with_mutation(idx, identity);
        }

        let root = ref_tree.root();

        // Since the tree state is cached to tree history before a sequence of updates, we need to apply the first 5 updates to
        // ensure that the intermediate root is in the tree history
        tree_data.insert_many_at(0, &identities[0..5]);

        // Then you can apply the remaining updates
        tree_data.insert_many_at(5, &identities[5..]);

        for (i, _identity) in identities.iter().enumerate().take(5) {
            let proof_from_world_tree = tree_data
                .get_inclusion_proof(identities[i], Some(root))
                .unwrap();

            assert_eq!(ref_tree.proof(i), proof_from_world_tree.proof);
        }
    }

    #[tokio::test]
    async fn test_tree_history_capacity() {
        let (mut tree_data, _, identities) =
            initialize_tree_data(TREE_DEPTH, TREE_HISTORY_SIZE, NUM_IDENTITIES);

        // Apply an update to the tree one identity at a time to apply all changes to the tree history cache
        for (idx, identity) in identities.into_iter().enumerate() {
            tree_data.insert_many_at(idx, &[identity]);
        }

        // The tree history should not be larger than the tree history size
        assert_eq!(tree_data.tree_history.len(), tree_data.tree_history_size,);
    }

    #[tokio::test]
    async fn test_get_inclusion_proof_after_deletions() {
        let (mut tree_data, mut ref_tree, identities) =
            initialize_tree_data(TREE_DEPTH, TREE_HISTORY_SIZE, NUM_IDENTITIES);

        // Apply all identity updates to the ref tree and test tree
        for (idx, identity) in identities.iter().enumerate() {
            ref_tree = ref_tree.update_with_mutation(idx, identity);
        }

        tree_data.insert_many_at(0, &identities);

        // Initialize a vector of indices to delete
        let deleted_identity_idxs = &[3, 7];
        let non_deleted_identity_idxs: Vec<_> = (0..NUM_IDENTITIES)
            .filter(|idx| !deleted_identity_idxs.contains(idx))
            .collect();

        // Delete the identities at the specified indices for the ref tree and test tree
        for idx in deleted_identity_idxs {
            ref_tree = ref_tree.update_with_mutation(*idx, &Hash::ZERO);
        }
        tree_data.delete_many(deleted_identity_idxs);

        let root = ref_tree.root();

        // Ensure that an inclusion proof can be generated for all identities that were not deleted
        for i in non_deleted_identity_idxs {
            let proof_from_world_tree = tree_data
                .get_inclusion_proof(identities[i], Some(root))
                .unwrap();

            assert_eq!(ref_tree.proof(i), proof_from_world_tree.proof);
        }

        // Ensure that an inclusion proof cannot be generated for deleted identities
        for i in deleted_identity_idxs {
            let proof_from_world_tree =
                tree_data.get_inclusion_proof(identities[*i], Some(root));
            assert!(proof_from_world_tree.is_none());
        }
    }
}
