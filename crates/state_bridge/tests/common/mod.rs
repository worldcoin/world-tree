pub mod abi;

mod chain_mock;

pub mod prelude {
    pub use std::time::Duration;

    pub use ethers::abi::{AbiEncode, Address};
    pub use ethers::core::abi::Abi;
    pub use ethers::core::k256::ecdsa::SigningKey;
    pub use ethers::core::rand;
    pub use ethers::prelude::{
        ContractFactory, Http, LocalWallet, NonceManagerMiddleware, Provider, Signer,
        SignerMiddleware, Wallet,
    };
    pub use ethers::providers::Middleware;
    pub use ethers::types::{Bytes, H256, U256};
    pub use ethers::utils::{Anvil, AnvilInstance};
    pub use ethers_solc::artifacts::Bytecode;
    pub use semaphore::identity::Identity;
    pub use semaphore::merkle_tree::{self, Branch};
    pub use semaphore::poseidon_tree::{PoseidonHash, PoseidonTree};
    pub use semaphore::protocol::{self, generate_nullifier_hash, generate_proof};
    pub use semaphore::{hash_to_field, Field};
    pub use serde::{Deserialize, Serialize};
    pub use serde_json::json;
    pub use tokio::spawn;
    pub use tokio::task::JoinHandle;
    pub use tracing::{error, info, instrument};
}

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::str::FromStr;
use std::sync::Arc;

use state_bridge::bridge::{IStateBridge, StateBridge};
use state_bridge::StateBridgeService;

use self::chain_mock::{spawn_mock_chain, MockChain};
use self::prelude::*;

#[derive(Deserialize, Serialize, Debug)]
struct CompiledContract {
    abi: Abi,
    bytecode: Bytecode,
}

#[tokio::test]
pub async fn test_relay_root() -> eyre::Result<()> {
    let MockChain {
        anvil,
        private_key,
        state_bridge,
        mock_bridged_world_id,
        mock_world_id,
        middleware,
    } = spawn_mock_chain().await?;

    let state_bridge = IStateBridge::new(state_bridge.address(), middleware.clone());

    let relaying_period = std::time::Duration::from_secs(5);

    let mut state_bridge_service = StateBridgeService::new(mock_world_id)
        .await
        .expect("couldn't create StateBridgeService");

    state_bridge_service
        .spawn()
        .await
        .expect("failed to spawn a state bridge service");

    Ok(())
}
