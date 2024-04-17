use axum::response::IntoResponse;
use ethers::prelude::{AbiError, ContractError};
use ethers::providers::{Middleware, ProviderError};
use ethers::types::Log;
use hyper::StatusCode;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;

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
    #[error("No canonical logs found")]
    CanonicalLogsNotFound,
    #[error("Roots are different when expected to be the same")]
    IncongruentRoots,
    #[error("Unexpected tree root")]
    UnexpectedRoot,
    #[error("Leaf channel closed")]
    LeafChannelClosed,
    #[error("Bridged root channel closed")]
    BridgedRootChannelClosed,
    #[error("Chain ID not found")]
    ChainIdNotFound,
}

#[derive(Error, Debug)]
pub enum IdentityTreeError {
    #[error("Root not found")]
    RootNotFound,
    #[error("Leaf already exists")]
    LeafAlreadyExists,
    #[error("Could not create mmap vec")]
    MmapVecError,
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    // #[error(transparent)]
    // EyreError(#[from] eyre::Error),
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
