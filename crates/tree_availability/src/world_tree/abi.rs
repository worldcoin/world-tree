use ethers::middleware::contract::abigen;
use ethers::types::Selector;

pub const REGISTER_IDENTITIES_SELECTOR: Selector = [34, 23, 178, 17];
pub const DELETE_IDENTITIES_SELECTOR: Selector = [234, 16, 251, 190];

abigen!(
    IWorldIDIdentityManager,
    r#"[
        event TreeChanged(uint256 indexed preRoot, uint8 indexed kind, uint256 indexed postRoot)
        function registerIdentities(uint256[8] calldata insertionProof, uint256 preRoot, uint32 startIndex, uint256[] calldata identityCommitments, uint256 postRoot) external
        function deleteIdentities(uint256[8] calldata deletionProof, bytes calldata packedDeletionIndices, uint256 preRoot, uint256 postRoot) external
    ]"#;
);
