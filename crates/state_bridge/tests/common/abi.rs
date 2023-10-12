use ethers::{prelude::abigen, providers::Middleware};
use state_bridge::root::IWorldIdIdentityManager;

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

abigen!(
    MockWorldID,
    r#"[
        function latestRoot() external returns (uint256)
        event TreeChanged(uint256 indexed preRoot, uint8 indexed kind, uint256 indexed postRoot)
        function insertRoot(uint256 postRoot) public
    ]"#,
    event_derives(serde::Deserialize, serde::Serialize)
);
