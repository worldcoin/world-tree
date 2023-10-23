use ethers::prelude::abigen;

// Generates ABI interfaces for `MockWorldID`
abigen!(
    MockWorldID,
    "src/test_utilities/abi/MockWorldIDIdentityManager.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

// Generates ABI interfaces for `MockStateBridge`
abigen!(
    MockStateBridge,
    "src/test_utilities/abi/MockStateBridge.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

// Generates ABI interfaces for `MockStateBridge`
abigen!(
    MockBridgedWorldID,
    "src/test_utilities/abi/MockBridgedWorldID.json",
    event_derives(serde::Deserialize, serde::Serialize)
);
