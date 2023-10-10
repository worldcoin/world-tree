use ethers::{middleware::contract::abigen, types::H256};

//TODO: update event sig for tree changed
pub const TREE_CHANGE_EVENT_SIGNATURE: H256 = H256([
    28, 65, 30, 154, 150, 224, 113, 36, 28, 47, 33, 247, 114, 107, 23, 174, 137, 227, 202, 180,
    199, 139, 229, 14, 6, 43, 3, 169, 255, 251, 186, 209,
]);

abigen!(
    IWorldIdIdentityManager,
    r#"[
        event TreeChanged(uint256 indexed preRoot, uint8 indexed kind, uint256 indexed postRoot)
    ]"#;
);
