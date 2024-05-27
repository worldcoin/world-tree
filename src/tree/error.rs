use axum::response::IntoResponse;
use ethers::prelude::{AbiError, ContractError};
use ethers::providers::Middleware;
use hyper::StatusCode;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorldTreeError<M>
where
    M: Middleware + 'static,
{
    #[error("No canonical logs found")]
    CanonicalLogsNotFound,
    #[error("Roots are different when expected to be the same")]
    IncongruentRoots,
    #[error("Leaf channel closed")]
    LeafChannelClosed,
    #[error("Bridged root channel closed")]
    BridgedRootChannelClosed,
    #[error("Chain ID not found")]
    ChainIdNotFound,
    #[error("Tree not synced")]
    TreeNotSynced,
    #[error("Transaction hash not found")]
    TransactionHashNotFound,
    #[error("Transaction found")]
    TransactionNotFound,
    #[error("Calldata does not have a function selector")]
    MissingFunctionSelector,
    #[error(transparent)]
    IdentityTreeError(#[from] IdentityTreeError),
    #[error(transparent)]
    MiddlewareError(<M as Middleware>::Error),
    #[error(transparent)]
    ContractError(#[from] ContractError<M>),
    #[error(transparent)]
    ABICodecError(#[from] AbiError),
    #[error(transparent)]
    EthABIError(#[from] ethers::abi::Error),
    #[error(transparent)]
    HyperError(#[from] hyper::Error),
    #[error("cMix server error: {0}")]
    CMixError(String),
}

#[derive(Error, Debug)]
pub enum IdentityTreeError {
    #[error("Root not found")]
    RootNotFound,
    #[error("Leaf already exists")]
    LeafAlreadyExists,
    #[error(transparent)]
    MmapVecError(#[from] eyre::Report),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
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
