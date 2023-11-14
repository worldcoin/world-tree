use ethers::prelude::{AbiError, ContractError};
use ethers::providers::{Middleware, ProviderError};
use ethers::signers::WalletError;
use thiserror::Error;

use crate::tree::Hash;

#[derive(Error, Debug)]
pub enum StateBridgeError<L1M, L2M>
where
    L1M: Middleware,
    L2M: Middleware,
{
    #[error("L1 middleware error")]
    L1MiddlewareError(<L1M as Middleware>::Error),
    #[error("L2 middleware error")]
    L2MiddlewareError(<L2M as Middleware>::Error),
    #[error("Provider error")]
    ProviderError(#[from] ProviderError),
    #[error("L1 contract error")]
    L1ContractError(ContractError<L1M>),
    #[error("L2 contract error")]
    L2ContractError(ContractError<L2M>),
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
    #[error("Transaction error")]
    TransactionError(#[from] TransactionError<L1M>),
}

#[derive(Error, Debug)]
pub enum TransactionError<M>
where
    M: Middleware,
{
    #[error("Middleware error")]
    MiddlewareError(<M as Middleware>::Error),
    #[error("Middleware error")]
    WalletError(#[from] WalletError),
    #[error("Wallet has insufficient funds")]
    InsufficientWalletFunds,
}
