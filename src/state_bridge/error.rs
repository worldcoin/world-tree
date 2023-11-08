use ethers::prelude::{AbiError, ContractError};
use ethers::providers::{Middleware, ProviderError};
use thiserror::Error;

use crate::tree::Hash;

#[derive(Error, Debug)]
pub enum StateBridgeError<M>
where
    M: Middleware,
{
    #[error("Middleware error")]
    MiddlewareError(<M as Middleware>::Error),
    #[error("Provider error")]
    ProviderError(#[from] ProviderError),
    #[error("Contract error")]
    ContractError(#[from] ContractError<M>),
    #[error("ABI Codec error")]
    ABICodecError(#[from] AbiError),
    #[error("Eth ABI error")]
    EthABIError(#[from] ethers::abi::Error),
    #[error("Could not send root through channel")]
    RootSendError(#[from] tokio::sync::broadcast::error::SendError<Hash>),
    #[error("Could not send root through channel")]
    RecvError(#[from] tokio::sync::broadcast::error::RecvError),
    #[error("No state bridge was added to WorldTreeRoot")]
    BridgesNotInitialized,
}
