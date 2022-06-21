use std::num::TryFromIntError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("internal I/O error")]
    IO(#[from] std::io::Error),

    #[error("decoded variable integer size was outside of expected bounds")]
    VarIntSizeExceeded,

    #[error("while trying to read more data, an unexpected end of buffer was reached")]
    EndOfBuffer,

    #[error("while reading, an unexpected value was found")]
    UnexpectedValue,

    #[error("`{0}`")]
    Other(String),
}

impl From<TryFromIntError> for Error {
    #[inline]
    fn from(_: TryFromIntError) -> Self {
        Error::VarIntSizeExceeded
    }
}
