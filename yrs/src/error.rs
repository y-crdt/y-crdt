use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    ReadError(#[from] crate::encoding::read::Error),
    #[error("Cannot execute this operation when document garbage collection is set")]
    Gc
}