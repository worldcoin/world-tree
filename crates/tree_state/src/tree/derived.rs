use semaphore::lazy_merkle_tree::{self, Derived};
use serde_json::map::IterMut;

use super::{
    Hash, PoseidonTree, TreeData, TreeItem, TreeMetadata, TreeReader, TreeVersion, TreeWriter,
};

/// Additional data held by any derived tree version. Includes the list of
/// updates performed since previous version.

//TODO: do we need this, can we update in a way where the diff is only fed to the derived tree from the canonical tree
pub struct DerivedMetadata {
    pub identity_rx: tokio::sync::broadcast::Receiver<Hash>,
}

impl TreeMetadata for Derived {
    type Metadata = DerivedMetadata;
}

impl TreeVersion for Derived {}

impl TreeWriter for TreeData<Derived> {
    //TODO: docs as to why we return the root
    fn update(&mut self, item: TreeItem) -> Hash {
        self.tree = self.tree.update(item.leaf_index, &item.element);

        if item.element != Hash::ZERO {
            self.next_leaf = item.leaf_index + 1;
        }

        self.tree.root()
    }
}
