use axum::response::IntoResponse;
use hyper::StatusCode;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorldTreeError {
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
    #[error("Transaction not found")]
    TransactionNotFound,
    #[error("Transaction search error: {0}")]
    TransactionSearchError(String),
    #[error("Duplicate transaction")]
    DuplicateTransaction,
    #[error("Calldata does not have a function selector")]
    MissingFunctionSelector,
}

#[derive(Error, Debug)]
pub enum IdentityTreeError {
    #[error("Root not found")]
    RootNotFound,
    #[error("Tree not found")]
    TreeNotFound,
    #[error("Leaf already exists")]
    LeafAlreadyExists,
    #[error("Leaf does not exist in tree")]
    LeafNotFound,
    #[error("Update not found")]
    UpdateNotFound,
}

impl Status for IdentityTreeError {
    fn status_code(&self) -> StatusCode {
        match self {
            IdentityTreeError::RootNotFound
            | IdentityTreeError::LeafNotFound => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl Status for WorldTreeError {
    fn status_code(&self) -> StatusCode {
        match self {
            WorldTreeError::TreeNotSynced => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub trait Status {
    fn status_code(&self) -> StatusCode;
}

#[derive(Debug)]
pub struct WorldTreeEyre(pub color_eyre::eyre::Report);

impl std::fmt::Display for WorldTreeEyre {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

pub type WorldTreeResult<T> = Result<T, WorldTreeEyre>;

impl<E: Into<eyre::Report>> From<E> for WorldTreeEyre {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}

impl IntoResponse for WorldTreeEyre {
    fn into_response(self) -> axum::response::Response {
        if let Some(e) = self.0.downcast_ref::<WorldTreeError>() {
            let status_code = e.status_code();
            let response_body = e.to_string();
            (status_code, response_body).into_response()
        } else if let Some(e) = self.0.downcast_ref::<IdentityTreeError>() {
            let status_code = e.status_code();
            let response_body = e.to_string();
            (status_code, response_body).into_response()
        } else {
            let status_code = StatusCode::INTERNAL_SERVER_ERROR;
            let response_body = self.0.to_string();
            (status_code, response_body).into_response()
        }
    }
}
