use ethers::prelude::abigen;

abigen!(
    WorldIDIdentityManagerMock,
    r#"[
        function latestRoot() external view returns (uint256)
    ]"#,
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    BridgedWorldID,
    r#"[
        event RootAdded(uint256 root, uint128 timestamp)
        event RootHistoryExpirySet(uint256 newExpiry)
        function latestRoot() public view virtual returns (uint256)
        function receiveRoot(uint256 newRoot) external
        function setRootHistoryExpiry(uint256 expiryTime) public
    ]"#,
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    StateBridge,
    r#"[
        event RootPropagated(uint256 root)
        function propagateRoot() external
    ]"#,
    event_derives(serde::Deserialize, serde::Serialize)
);
