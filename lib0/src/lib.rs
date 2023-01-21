pub mod any;
pub mod binary;
pub mod decoding;
pub mod encoding;
pub mod error;
pub mod number;

#[cfg(not(feature = "lib0-serde"))]
mod json_parser;

mod macros;
#[cfg(feature = "lib0-serde")]
pub mod serde;
