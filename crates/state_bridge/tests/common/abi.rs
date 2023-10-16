use ethers::{prelude::abigen, providers::Middleware};
use state_bridge::root::IWorldIdIdentityManager;

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
