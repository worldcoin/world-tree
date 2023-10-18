use ethers::prelude::{AbiError, ContractError};
use ethers::providers::{Middleware, ProviderError};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TreeAvailabilityError<M>
where
    M: Middleware + 'static,
{
    // Internal errors
    #[error("Missing transaction on log")]
    MissingTransaction,
    #[error("Unrecognized transaction")]
    UnrecognizedTransaction,

    // Third-party converted errors
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
    #[error(transparent)]
    HyperError(#[from] hyper::Error),
}

#[derive(Error, Debug)]
pub enum TreeError {}
