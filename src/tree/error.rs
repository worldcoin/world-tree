use axum::response::IntoResponse;
use ethers::prelude::{AbiError, ContractError};
use ethers::providers::{Middleware, ProviderError};
use ethers::types::Log;
use hyper::StatusCode;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;

#[derive(Error, Debug)]
pub enum TreeAvailabilityError<M>
where
    M: Middleware + 'static,
{
    #[error("Missing transaction on log")]
    MissingTransaction,
    #[error("Unrecognized transaction")]
    UnrecognizedTransaction,
    #[error("Transaction hash was not found")]
    TransactionHashNotFound,
    #[error("Block number was not found")]
    BlockNumberNotFound,
    #[error("Transaction was not found from hash")]
    TransactionNotFound,
    #[error("Unrecognized function selector")]
    UnrecognizedFunctionSelector,
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
    #[error(transparent)]
    SendLogError(#[from] SendError<Log>),
}

#[derive(Error, Debug)]
pub enum WorldTreeError<M>
where
    M: Middleware + 'static,
{
    #[error("Tree manager error")]
    TreeManagerError(#[from] TreeManagerError<M>),
    #[error("Identity tree error")]
    IdentityTreeError(#[from] IdentityTreeError),
    #[error("Middleware error")]
    MiddlewareError(<M as Middleware>::Error),
    #[error("Contract error")]
    ContractError(#[from] ContractError<M>),
}

#[derive(Error, Debug)]
pub enum IdentityTreeError {
    #[error("Root not found")]
    RootNotFound,
    #[error("Leaf already exists")]
    LeafAlreadyExists,
}

impl<M> WorldTreeError<M>
where
    M: Middleware + 'static,
{
    fn to_status_code(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

impl<M> IntoResponse for WorldTreeError<M>
where
    M: Middleware + 'static,
{
    fn into_response(self) -> axum::response::Response {
        let status_code = self.to_status_code();
        let response_body = self.to_string();
        (status_code, response_body).into_response()
    }
}

#[derive(Error, Debug)]
pub enum TreeManagerError<M>
where
    M: Middleware + 'static,
{
    #[error("Middleware error")]
    MiddlewareError(<M as Middleware>::Error),
    #[error("Contract error")]
    ContractError(#[from] ContractError<M>),
    #[error("ABI Codec error")]
    ABICodecError(#[from] AbiError),
    #[error("Eth ABI error")]
    EthABIError(#[from] ethers::abi::Error),
}
