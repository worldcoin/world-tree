use ethers::middleware::contract::abigen;

abigen!(
    IWorldIDIdentityManager,
    r#"[
        function latestRoot() external returns (uint256)
        event TreeChanged(uint256 indexed preRoot, uint8 indexed kind, uint256 indexed postRoot)
        function registerIdentities(uint256[8] calldata insertionProof, uint256 preRoot, uint32 startIndex, uint256[] calldata identityCommitments, uint256 postRoot) external
        function deleteIdentities(uint256[8] calldata deletionProof, bytes calldata packedDeletionIndices, uint256 preRoot, uint256 postRoot) external
        function deleteIdentities(uint256[8] calldata deletionProof, uint32 batchSize, bytes calldata packedDeletionIndices, uint256 preRoot, uint256 postRoot) external
    ]"#;

    IStateBridge,
    r#"[
        function propagateRoot() external
    ]"#;

    IBridgedWorldID,
    r#"[
        event RootAdded(uint256 root, uint128 timestamp)
        function latestRoot() public view virtual returns (uint256)
        function receiveRoot(uint256 newRoot) external
    ]"#,
    event_derives(serde::Deserialize, serde::Serialize)

);
