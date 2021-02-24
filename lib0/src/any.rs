use std::collections::HashMap;

pub enum Any {
    Null,
    Undefined,
    Bool(bool),
    Number(f64),
    BigInt(i64),
    String(String),
    Array(Vec<Any>),
    Map(HashMap<String, Any>),
    Buffer(Box<[u8]>),
}
