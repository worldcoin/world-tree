use ethers::middleware::contract::abigen;

abigen!(
    IWorldIdIdentityManager,
    r#"[
        event TreeChanged(uint256 indexed preRoot, uint8 indexed kind, uint256 indexed postRoot)
        function registerIdentities(uint256[8] calldata insertionProof, uint256 preRoot, uint32 startIndex, uint256[] calldata identityCommitments, uint256 postRoot) external
        function deleteIdentities(uint256[8] calldata deletionProof, uint32 batchSize, bytes calldata packedDeletionIndices, uint256 preRoot, uint256 postRoot) external
    ]"#;
);
