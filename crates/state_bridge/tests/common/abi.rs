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
        function latestRoot() public view virtual returns (uint256)
        function receiveRoot(uint256 newRoot) external
    ]"#,
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    MockStateBridge,
    r#"[
        event RootPropagated(uint256 root)
        function propagateRoot() external
        function worldID() external view returns (address)
        function mockBridgedWorldID() external view returns (address)
    ]"#,
    event_derives(serde::Deserialize, serde::Serialize)
);
