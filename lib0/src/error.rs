use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("internal I/O error")]
    IO(#[from] std::io::Error),

    #[error("decoded variable integer size was outside of expected bounds of {0} bits")]
    VarIntSizeExceeded(u8),

    #[error("while trying to read more data, an unexpected end of buffer was reached")]
    EndOfBuffer,

    #[error("while reading, an unexpected value was found")]
    UnexpectedValue,

    #[error("`{0}`")]
    Other(String),

    #[cfg(not(feature = "lib0-serde"))]
    #[error("JSON parsing error: {0}")]
    InvalidJSON(#[from] crate::json_parser::JsonParseError),

    #[cfg(feature = "lib0-serde")]
    #[error("JSON parsing error: {0}")]
    InvalidJSON(#[from] serde_json::Error),
}
