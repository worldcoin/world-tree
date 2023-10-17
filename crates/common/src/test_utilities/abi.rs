use ethers::prelude::abigen;

abigen!(
    MockWorldID,
    "src/test_utilities/abi/WorldIDIdentityManagerMock.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    MockStateBridge,
    "src/test_utilities/abi/MockStateBridge.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    MockBridgedWorldID,
    "src/test_utilities/abi/MockBridgedWorldID.json",
    event_derives(serde::Deserialize, serde::Serialize)
);
