use ethers::prelude::abigen;

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
