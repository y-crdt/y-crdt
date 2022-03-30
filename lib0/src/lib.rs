pub mod any;
pub mod binary;
pub mod decoding;
pub mod encoding;
pub mod number;

#[cfg(not(feature = "lib0-serde"))]
mod json_parser;

#[cfg(feature = "lib0-serde")]
pub mod serde;
