pub mod error;
pub mod service;
pub mod transaction;

use std::ops::Sub;
use std::sync::Arc;

use ethers::providers::Middleware;
use ethers::signers::{LocalWallet, Signer};
use ethers::types::H160;
use ruint::Uint;
use tokio::select;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant};
use tracing::instrument;

use self::error::StateBridgeError;
use crate::abi::{self, IBridgedWorldID};
use crate::tree::Hash;

/// The `StateBridge` is responsible for monitoring root changes from the `WorldRoot`, propagating the root to the corresponding Layer 2.
pub struct StateBridge<L1M: Middleware + 'static, L2M: Middleware + 'static> {
    //TODO:
    l1_state_bridge: H160,
    wallet: LocalWallet,
    //TODO:
    l1_middleware: Arc<L1M>,
    /// Interface for the `BridgedWorldID` contract
    pub l2_world_id: IBridgedWorldID<L2M>,
    /// Time delay between `propagateRoot()` transactions
    pub relaying_period: Duration,
    /// The number of block confirmations before a `propagateRoot()` transaction is considered finalized
    pub block_confirmations: usize,
}

impl<L1M: Middleware, L2M: Middleware> StateBridge<L1M, L2M> {
    //TODO: update

    /// # Arguments
    ///
    /// * l1_state_bridge - Interface to the StateBridge smart contract.
    /// * l2_world_id - Interface to the BridgedWorldID smart contract.
    /// * relaying_period - Duration between successive propagateRoot() invocations.
    /// * block_confirmations - Number of block confirmations required to consider a propagateRoot() transaction as finalized.
    pub fn new(
        l1_state_bridge: H160,
        wallet: LocalWallet,
        l1_middleware: Arc<L1M>,
        l2_world_id: IBridgedWorldID<L2M>,
        relaying_period: Duration,
        block_confirmations: usize,
    ) -> Result<Self, StateBridgeError<L1M, L2M>> {
        Ok(Self {
            l1_state_bridge,
            wallet,
            l1_middleware,
            l2_world_id,
            relaying_period,
            block_confirmations,
        })
    }

    //TODO: update
    /// # Arguments
    ///
    /// * `l1_state_bridge` - Address of the StateBridge contract.
    /// * `l1_middleware` - Middleware for interacting with the chain where StateBridge is deployed.
    /// * `l2_world_id` - Address of the BridgedWorldID contract.
    /// * `l2_middleware` - Middleware for interacting with the chain where BridgedWorldID is deployed.
    /// * `relaying_period` - Duration between `propagateRoot()` transactions.
    /// * `block_confirmations` - Number of block confirmations before a`propagateRoot()` transaction is considered finalized.
    pub fn new_from_parts(
        l1_state_bridge: H160,
        wallet: LocalWallet,
        l1_middleware: Arc<L1M>,
        l2_world_id: H160,
        l2_middleware: Arc<L2M>,
        relaying_period: Duration,
        block_confirmations: usize,
    ) -> Result<Self, StateBridgeError<L1M, L2M>> {
        let l2_world_id = IBridgedWorldID::new(l2_world_id, l2_middleware);

        Ok(Self {
            l1_state_bridge,
            wallet,
            l1_middleware,
            l2_world_id,
            relaying_period,
            block_confirmations,
        })
    }

    /// Spawns a `StateBridge` task to listen for `TreeChanged` events from `WorldRoot` and propagate new roots.
    ///
    /// # Arguments
    ///
    /// * `root_rx` - Receiver channel for roots from `WorldRoot`.
    #[instrument(skip(self, root_rx))]
    pub fn spawn(
        &self,
        mut root_rx: tokio::sync::broadcast::Receiver<Hash>,
    ) -> JoinHandle<Result<(), StateBridgeError<L1M, L2M>>> {
        let l2_world_id = self.l2_world_id.clone();
        let l1_state_bridge = self.l1_state_bridge.clone();
        let relaying_period = self.relaying_period;
        let block_confirmations = self.block_confirmations;
        let wallet = self.wallet.clone();
        let l2_world_id_address = l2_world_id.address();
        let l1_middleware = self.l1_middleware.clone();

        tracing::info!(
            ?l2_world_id_address,
            ?l1_state_bridge,
            ?relaying_period,
            ?block_confirmations,
            "Spawning bridge"
        );

        tokio::spawn(async move {
            let mut latest_root = Uint::from_limbs(
                l2_world_id
                    .latest_root()
                    .call()
                    .await
                    .map_err(StateBridgeError::L2ContractError)?
                    .0,
            );

            let mut last_propagation = Instant::now().sub(relaying_period);

            loop {
                select! {
                    root = root_rx.recv() => {
                        tracing::info!(?l1_state_bridge, ?root, "Root received from rx");
                        latest_root = root?;
                    }

                    _ = tokio::time::sleep(relaying_period) => {
                        tracing::info!(?l1_state_bridge, "Sleep time elapsed");
                    }
                }

                let time_since_last_propagation =
                    Instant::now() - last_propagation;

                if time_since_last_propagation >= relaying_period {
                    tracing::info!(?l1_state_bridge, "Relaying period elapsed");

                    let latest_bridged_root = Uint::from_limbs(
                        l2_world_id
                            .latest_root()
                            .call()
                            .await
                            .map_err(StateBridgeError::L2ContractError)?
                            .0,
                    );

                    if latest_root != latest_bridged_root {
                        tracing::info!(
                            ?l1_state_bridge,
                            ?latest_root,
                            ?latest_bridged_root,
                            "Propagating root"
                        );

                        Self::propagate_root(
                            l1_state_bridge,
                            &wallet,
                            block_confirmations,
                            l1_middleware.clone(),
                        )
                        .await?;

                        last_propagation = Instant::now();
                    } else {
                        tracing::info!(
                            ?l1_state_bridge,
                            "Root already propagated"
                        );
                    }
                }
            }
        })
    }

    pub async fn propagate_root(
        l1_state_bridge: H160,
        wallet: &LocalWallet,
        block_confirmations: usize,
        l1_middleware: Arc<L1M>,
    ) -> Result<(), StateBridgeError<L1M, L2M>> {
        let calldata = abi::ISTATEBRIDGE_ABI
            .function("propagateRoot")?
            .encode_input(&vec![])?;

        let tx = transaction::fill_and_simulate_eip1559_transaction(
            calldata.into(),
            l1_state_bridge,
            wallet.address(),
            wallet.chain_id(),
            l1_middleware.clone(),
        )
        .await?;

        transaction::sign_and_send_transaction(
            tx,
            &wallet,
            block_confirmations,
            l1_middleware,
        )
        .await?;

        Ok(())
    }
}
