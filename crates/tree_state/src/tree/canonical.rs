use ethers::{providers::Middleware, types::U256};
use semaphore::{
    lazy_merkle_tree::{self, Canonical},
    poseidon_tree::Proof,
};
use tracing::info;

use super::{
    Hash, TreeData, TreeItem, TreeMetadata, TreeReader, TreeVersion, TreeWriter, WorldTree,
};

impl<M: Middleware> WorldTree<TreeData<Canonical>, M> {
    pub async fn spawn(&self) {}

    pub async fn sync(&self) {}
}

pub struct CanonicalMetadata {
    pub identity_tx: tokio::sync::broadcast::Sender<Hash>,
    pub last_synced_block: u64, //TODO: probably update this type
}

impl TreeMetadata for Canonical {
    type Metadata = CanonicalMetadata;
}

impl TreeVersion for Canonical {}

impl TreeWriter for TreeData<Canonical> {
    fn update(&mut self, item: TreeItem) -> Hash {
        // Figure out if this will work or not
        take_mut::take(&mut self.tree, |tree| {
            tree.update_with_mutation(item.leaf_index, &item.element)
        });

        if item.element != Hash::ZERO {
            self.next_leaf = item.leaf_index + 1;
        }

        self.tree.root()
    }
}

impl TreeData<Canonical> {
    //TODO: FIXME: will probably need to update these
    /// Appends many identities to the tree, returns a list with the root, proof
    /// of inclusion and leaf index
    #[must_use]
    fn append_many(&self, identities: &[Hash]) -> Vec<(Hash, Proof, usize)> {
        let next_leaf = self.next_leaf;
        let mut output = Vec::with_capacity(identities.len());

        for (idx, identity) in identities.iter().enumerate() {
            let leaf_index = next_leaf + idx;
            //TODO: FIXME: update this to mutate the tree
            self.tree.update(leaf_index, identity);
            let proof = self.tree.proof(leaf_index);

            output.push((self.tree.root(), proof, leaf_index));
        }

        output
    }

    /// Deletes many identities from the tree, returns a list with the root
    /// and proof of inclusion
    #[must_use]
    fn delete_many(&self, leaf_indices: &[usize]) -> Vec<(Hash, Proof)> {
        let mut output = Vec::with_capacity(leaf_indices.len());

        for leaf_index in leaf_indices {
            //TODO: FIXME: update this to mutate the tree
            self.tree.update(*leaf_index, &Hash::ZERO);
            let proof = self.tree.proof(*leaf_index);
            output.push((self.tree.root(), proof));
        }

        output
    }
}
