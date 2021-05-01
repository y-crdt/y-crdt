use std::cmp::PartialEq;
use std::collections::HashMap;
use std::convert::TryInto;

#[derive(Debug, Clone, PartialEq)]
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

impl std::fmt::Display for Any {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Any::Null => write!(f, "null")?,
            Any::Undefined => write!(f, "undefined")?,
            Any::Bool(value) => write!(f, "{}", value)?,
            Any::Number(value) => write!(f, "{}", value)?,
            Any::BigInt(value) => write!(f, "{}", value)?,
            Any::String(value) => write!(f, "\"{}\"", value)?,
            Any::Buffer(value) => write!(f, "[binary: {} bytes]", value.len())?, //TODO: use base64?
            Any::Array(values) => {
                write!(f, "[")?;
                let mut i = values.iter();
                if let Some(value) = i.next() {
                    value.fmt(f)?;
                }
                while let Some(value) = i.next() {
                    write!(f, ", ")?;
                    value.fmt(f)?;
                }
                write!(f, "]")?;
            }
            Any::Map(entries) => {
                write!(f, "{{")?;
                let mut i = entries.iter();
                if let Some((key, value)) = i.next() {
                    write!(f, "\"{}\": {}", key, value)?;
                }
                while let Some((key, value)) = i.next() {
                    write!(f, ", \"{}\": {}", key, value)?;
                }
                write!(f, "}}")?;
            }
        }

        Ok(())
    }
}

impl Into<Any> for bool {
    fn into(self) -> Any {
        Any::Bool(self)
    }
}

impl Into<Any> for f64 {
    fn into(self) -> Any {
        Any::Number(self)
    }
}

impl Into<Any> for f32 {
    fn into(self) -> Any {
        Any::Number(self as f64)
    }
}

impl Into<Any> for u32 {
    fn into(self) -> Any {
        Any::Number(self as f64)
    }
}

impl Into<Any> for i32 {
    fn into(self) -> Any {
        Any::Number(self as f64)
    }
}

impl Into<Any> for String {
    fn into(self) -> Any {
        Any::String(self)
    }
}

impl Into<Any> for &str {
    fn into(self) -> Any {
        Any::String(self.to_string())
    }
}

impl Into<Any> for Box<[u8]> {
    fn into(self) -> Any {
        Any::Buffer(self)
    }
}

impl Into<Any> for Vec<u8> {
    fn into(self) -> Any {
        Any::Buffer(self.into_boxed_slice())
    }
}

impl<T> Into<Any> for Option<T>
where
    T: Into<Any>,
{
    fn into(self) -> Any {
        match self {
            None => Any::Null,
            Some(value) => value.into(),
        }
    }
}

impl<T> Into<Any> for Vec<T>
where
    T: Into<Any>,
{
    fn into(self) -> Any {
        let mut array = Vec::with_capacity(self.len());
        for value in self {
            array.push(value.into())
        }
        Any::Array(array)
    }
}

impl<T> Into<Any> for HashMap<String, T>
where
    T: Into<Any>,
{
    fn into(self) -> Any {
        let mut map = HashMap::with_capacity(self.len());
        for (key, value) in self {
            map.insert(key, value.into());
        }
        Any::Map(map)
    }
}

impl TryInto<Any> for u64 {
    type Error = &'static str;

    fn try_into(self) -> Result<Any, Self::Error> {
        if self < (1 << 53) {
            Ok(Any::Number(self as f64))
        } else if self <= (i64::MAX as u64) {
            Ok(Any::BigInt(self as i64))
        } else {
            Err("lib0::Any conversion is possible only for numbers up to 2^63")
        }
    }
}

impl TryInto<Any> for usize {
    type Error = &'static str;

    #[cfg(target_pointer_width = "32")]
    fn try_into(self) -> Result<Any, Self::Error> {
        // for 32-bit architectures we know that usize will always fit,
        // so there's no need to check for length, but we stick to TryInto
        // trait to keep API compatibility
        Ok(Any::Number(self as f64))
    }

    #[cfg(target_pointer_width = "64")]
    fn try_into(self) -> Result<Any, Self::Error> {
        (self as u64).try_into()
    }
}
