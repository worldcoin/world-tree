use std::collections::HashMap;

use super::{Hash, LeafIndex};

pub type Leaves = HashMap<LeafIndex, Hash>;

#[derive(Debug, Clone)]
pub enum LeafUpdates {
    Insert(Leaves),
    Delete(Leaves),
}

impl From<LeafUpdates> for Leaves {
    fn from(val: LeafUpdates) -> Self {
        match val {
            LeafUpdates::Insert(leaves) => leaves,
            LeafUpdates::Delete(leaves) => leaves,
        }
    }
}
