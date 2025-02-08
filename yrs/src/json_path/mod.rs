use std::fmt::{Display, Formatter};
use std::ops::Range;

mod iter_any;
mod parse;

#[derive(Debug, Clone, PartialEq)]
pub struct JsonPath<'a> {
    tokens: Vec<JsonPathToken<'a>>,
}

impl<'a> AsRef<[JsonPathToken<'a>]> for JsonPath<'a> {
    fn as_ref(&self) -> &[JsonPathToken<'a>] {
        &self.tokens
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum JsonPathToken<'a> {
    Root,
    Current,
    Member(&'a str),
    Index(i32),
    Wildcard,
    Descend,
    Slice(u32, u32, u32),
}

impl<'a> Display for JsonPathToken<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            JsonPathToken::Root => write!(f, r#"$"#),
            JsonPathToken::Current => write!(f, "@"),
            JsonPathToken::Member(key) => {
                if key.chars().any(char::is_whitespace) {
                    write!(f, "['{}']", key)
                } else {
                    write!(f, ".{}", key)
                }
            }
            JsonPathToken::Index(index) => write!(f, "[{}]", index),
            JsonPathToken::Wildcard => write!(f, ".*"),
            JsonPathToken::Descend => write!(f, ".."),
            JsonPathToken::Slice(from, to, by) => write!(f, "[{}:{}:{}]", from, to, by),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("{0}")]
    InvalidJsonPath(String),
}
