use std::collections::HashMap;
use std::cmp::{PartialEq};

#[derive(PartialEq, Debug, Clone)]
pub enum Any {
    Null,
    Undefined,
    Bool(bool),
    Number(f64),
    BigInt(i64),
    String(String),
    Buffer(Box<[u8]>),
    Array(Vec<Any>),
    Map(HashMap<String, Any>),
}
