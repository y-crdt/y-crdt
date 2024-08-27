use crate::ID;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    ReadError(#[from] crate::encoding::read::Error),
    #[error("failed to apply update: {0}")]
    UpdateError(#[from] UpdateError),
    #[error("Cannot execute this operation when document garbage collection is set")]
    Gc,
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("block parent {0} must be deleted or shared ref type. Type: {1}")]
    InvalidParent(ID, u8),
}
