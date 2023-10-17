use ethers::prelude::abigen;

abigen!(
    MockWorldID,
    "sol/WorldIDIdentityManagerMock.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    MockStateBridge,
    "sol/MockStateBridge.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    MockBridgedWorldID,
    "sol/MockBridgedWorldID.json",
    event_derives(serde::Deserialize, serde::Serialize)
);
