use crate::encoding::read::{Error, Read};
use crate::encoding::write::Write;
use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::PartialEq;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt::Formatter;
use std::sync::Arc;

pub const F64_MAX_SAFE_INTEGER: f64 = (i64::pow(2, 53) - 1) as f64;
pub const F64_MIN_SAFE_INTEGER: f64 = -F64_MAX_SAFE_INTEGER;

/// Any is an enum with a potentially associated value that is used to represent JSON values
/// and supports efficient encoding of those values.
#[derive(Debug, Clone, PartialEq)]
pub enum Any {
    Null,
    Undefined,
    Bool(bool),
    Number(Number),
    String(Arc<str>),
    Buffer(Arc<[u8]>),
    Array(Arc<[Any]>),
    Map(Arc<HashMap<String, Any>>),
}

impl Any {
    #[inline]
    pub fn cast<T>(self) -> Result<T, Self>
    where
        T: TryFrom<Any, Error = Any>,
    {
        // we create dedicated cast, so that we can parametrize it in a fluent fashion
        // ie. `any.cast::<u32>().unwrap_or_default()`.
        T::try_from(self)
    }

    pub fn decode<R: Read>(decoder: &mut R) -> Result<Self, Error> {
        Ok(match decoder.read_u8()? {
            // CASE 127: undefined
            127 => Any::Undefined,
            // CASE 126: null
            126 => Any::Null,
            // CASE 125: integer
            125 => Any::Number(Number::Int(decoder.read_var::<i64>()?)),
            // CASE 124: float32
            124 => Any::Number(Number::Float(decoder.read_f32()? as f64)),
            // CASE 123: float64
            123 => Any::Number(Number::Float(decoder.read_f64()?)),
            // CASE 122: bigint
            122 => Any::Number(Number::Int(decoder.read_i64()?)),
            // CASE 121: boolean (false)
            121 => Any::Bool(false),
            // CASE 120: boolean (true)
            120 => Any::Bool(true),
            // CASE 119: string
            119 => {
                let str = decoder.read_string()?;
                Any::String(Arc::from(str))
            }
            // CASE 118: Map<string,Any>
            118 => {
                let len: usize = decoder.read_var()?;
                // `len` is attacker-controlled; a fallible reservation avoids an allocation bomb
                // (see `Update::decode`'s `try_reserve`).
                let mut map = HashMap::new();
                map.try_reserve(len)?;
                for _ in 0..len {
                    let key = decoder.read_string()?;
                    map.insert(key.to_owned(), Any::decode(decoder)?);
                }
                Any::Map(Arc::new(map))
            }
            // CASE 117: Array<Any>
            117 => {
                let len: usize = decoder.read_var()?;
                // `len` is attacker-controlled; a fallible reservation avoids an allocation bomb
                // (see `Update::decode`'s `try_reserve`).
                let mut arr = Vec::new();
                arr.try_reserve(len)?;
                for _ in 0..len {
                    arr.push(Any::decode(decoder)?);
                }
                Any::Array(Arc::from(arr))
            }
            // CASE 116: buffer
            116 => Any::Buffer(Arc::from(decoder.read_buf()?)),
            _ => return Err(Error::UnexpectedValue),
        })
    }

    // Encode data with efficient binary format.
    //
    // Differences to JSON:
    // • Transforms data to a binary format (not to a string)
    // • Encodes undefined, NaN, and ArrayBuffer (these can't be represented in JSON)
    // • Numbers are efficiently encoded either as a variable length integer, as a
    //   32 bit float, as a 64 bit float, or as a 64 bit bigint.
    //
    // Encoding table:
    //
    // | Data Type           | Prefix   | Encoding Method    | Comment |
    // | ------------------- | -------- | ------------------ | ------- |
    // | undefined           | 127      |                    | Functions, symbol, and everything that cannot be identified is encoded as undefined |
    // | null                | 126      |                    | |
    // | integer             | 125      | writeVarInt        | Only encodes 32 bit signed integers |
    // | float32             | 124      | writeFloat32       | |
    // | float64             | 123      | writeFloat64       | |
    // | bigint              | 122      | writeBigInt64      | |
    // | boolean (false)     | 121      |                    | True and false are different data types so we save the following byte |
    // | boolean (true)      | 120      |                    | - 0b01111000 so the last bit determines whether true or false |
    // | string              | 119      | writeVarString     | |
    // | object<string,any>  | 118      | custom             | Writes {length} then {length} key-value pairs |
    // | array<any>          | 117      | custom             | Writes {length} then {length} json values |
    // | Uint8Array          | 116      | writeVarUint8Array | We use Uint8Array for any kind of binary data |
    //
    // Reasons for the decreasing prefix:
    // We need the first bit for extendability (later we may want to encode the
    // prefix with writeVarUint). The remaining 7 bits are divided as follows:
    // [0-30]   the beginning of the data range is used for custom purposes
    //          (defined by the function that uses this library)
    // [31-127] the end of the data range is used for data encoding by
    //          lib0/encoding.js
    pub fn encode<W: Write>(&self, encoder: &mut W) {
        match self {
            Any::Undefined => {
                // TYPE 127: undefined
                encoder.write_u8(127)
            }
            Any::Null => {
                // TYPE 126: null
                encoder.write_u8(126)
            }
            Any::Bool(bool) => {
                // TYPE 120/121: boolean (true/false)
                encoder.write_u8(if *bool { 120 } else { 121 })
            }
            Any::String(str) => {
                // TYPE 119: String
                encoder.write_u8(119);
                encoder.write_string(&str)
            }
            Any::Number(num) => {
                match *num {
                    Number::Int(n) => {
                        // ensure max compatibility with yjs lib0 number encoding
                        if n >= -0x7FFFFFFF && n <= 0x7FFFFFFF {
                            // TYPE 125: INTEGER
                            encoder.write_u8(125);
                            encoder.write_var(n)
                        } else if (n as f32) as i64 == n {
                            // TYPE 124: FLOAT32
                            encoder.write_u8(124);
                            encoder.write_f32(n as f32)
                        } else if (n as f64) as i64 == n {
                            // TYPE 123: FLOAT64
                            encoder.write_u8(123);
                            encoder.write_f64(n as f64)
                        } else {
                            // TYPE 122: BigInt
                            encoder.write_u8(122);
                            encoder.write_i64(n)
                        }
                    }
                    Number::Float(n) => {
                        if (n as f32) as f64 == n {
                            // TYPE 124: FLOAT32
                            encoder.write_u8(124);
                            encoder.write_f32(n as f32)
                        } else {
                            // TYPE 123: FLOAT64
                            encoder.write_u8(123);
                            encoder.write_f64(n)
                        }
                    }
                }
            }
            Any::Array(arr) => {
                // TYPE 117: Array
                encoder.write_u8(117);
                encoder.write_var(arr.len() as u64);
                for el in arr.iter() {
                    el.encode(encoder);
                }
            }
            Any::Map(map) => {
                // TYPE 118: Map
                encoder.write_u8(118);
                encoder.write_var(map.len() as u64);
                for (key, value) in map.as_ref() {
                    encoder.write_string(&key);
                    value.encode(encoder);
                }
            }
            Any::Buffer(buf) => {
                // TYPE 116: Buffer
                encoder.write_u8(116);
                encoder.write_buf(&buf)
            }
        }
    }

    pub fn from_json(src: &str) -> Result<Self, Error> {
        Ok(serde_json::from_str(src)?)
    }

    pub fn to_json(&self, buf: &mut String) {
        use serde::Serialize;
        use serde_json::Serializer;

        let buf = unsafe { buf.as_mut_vec() };
        let cursor = std::io::Cursor::new(buf);

        let mut s = Serializer::new(cursor);
        self.serialize(&mut s).unwrap();
    }

    /// Returns an iterator over an inner values of an array or a map, current reference represents.
    pub fn try_iter(&self) -> Option<AnyIter<'_>> {
        match self {
            Any::Array(values) => Some(AnyIter::Array(values.iter())),
            Any::Map(entries) => Some(AnyIter::Map(entries.iter())),
            _ => None,
        }
    }

    pub fn try_into_iter(self) -> Option<AnyIntoIter> {
        match self {
            Any::Array(values) => Some(AnyIntoIter::from(values.clone())),
            Any::Map(entries) => Some(AnyIntoIter::from(entries.clone())),
            _ => None,
        }
    }
}

impl std::fmt::Display for Any {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Any::Null => f.write_str("null"),
            Any::Undefined => f.write_str("undefined"),
            Any::Bool(value) => write!(f, "{}", value),
            Any::Number(value) => write!(f, "{}", value),
            Any::String(value) => f.write_str(value.as_ref()),
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
                write!(f, "]")
            }
            Any::Map(entries) => {
                write!(f, "{{")?;
                let mut i = entries.iter();
                if let Some((key, value)) = i.next() {
                    write!(f, "{}: {}", key, value)?;
                }
                while let Some((key, value)) = i.next() {
                    write!(f, ", {}: {}", key, value)?;
                }
                write!(f, "}}")
            }
            Any::Buffer(value) => {
                f.write_str("0x")?;
                for &byte in value.iter() {
                    write!(f, "{:02x}", byte)?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Number {
    Int(i64),
    Float(f64),
}

impl Number {
    pub const I64_MAX_SAFE_INTEGER: i64 = i64::pow(2, 53) - 1;
    pub const I64_MIN_SAFE_INTEGER: i64 = -Self::I64_MAX_SAFE_INTEGER;
    pub const F64_MAX_SAFE_INTEGER: f64 = Self::I64_MAX_SAFE_INTEGER as f64;
    pub const F64_MIN_SAFE_INTEGER: f64 = -Self::F64_MAX_SAFE_INTEGER;

    pub fn try_i64(value: f64) -> Self {
        if value.trunc() == value
            && value >= Self::F64_MIN_SAFE_INTEGER
            && value <= Self::F64_MAX_SAFE_INTEGER
        {
            Number::Int(value as i64)
        } else {
            Number::Float(value)
        }
    }

    pub fn as_i64(self) -> Option<i64> {
        match self {
            Number::Int(value) => Some(value),
            Number::Float(value) => {
                // check if conversion is lossless
                let converted = value as i64;
                if converted as f64 == value {
                    Some(converted)
                } else {
                    None
                }
            }
        }
    }

    pub fn as_f64(self) -> Option<f64> {
        match self {
            Number::Int(value) => {
                let n = value as f64;
                if n as i64 == value {
                    Some(n)
                } else {
                    None
                }
            }
            Number::Float(value) => Some(value),
        }
    }
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Number::Int(a), Number::Int(b)) => a == b,
            (Number::Float(a), Number::Float(b)) => a == b,
            _ => match (self.as_f64(), other.as_f64()) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        }
    }
}

impl From<f64> for Number {
    #[inline]
    fn from(value: f64) -> Self {
        Number::Float(value)
    }
}

impl From<i64> for Number {
    #[inline]
    fn from(value: i64) -> Self {
        Number::Int(value)
    }
}

impl TryFrom<u64> for Number {
    type Error = u64;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value <= i64::MAX as u64 {
            Ok(Number::Int(value as i64))
        } else {
            Err(value)
        }
    }
}

impl Serialize for Number {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::Error;
        match self.as_f64() {
            Some(v) => serializer.serialize_f64(v),
            None => match self.as_i64() {
                Some(v) => serializer.serialize_i64(v),
                None => Err(S::Error::custom("cannot serialize number")),
            },
        }
    }
}

impl<'de> Deserialize<'de> for Number {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct NumberVisitor;
        impl<'de> Visitor<'de> for NumberVisitor {
            type Value = Number;

            fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
                write!(formatter, "number")
            }

            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if v > (i64::MAX as u64) {
                    return Err(E::custom("integer outside of bounds of i64"));
                }
                Ok(Number::Int(v as i64))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Number, E> {
                Ok(Number::Int(value))
            }

            fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if v <= Number::F64_MAX_SAFE_INTEGER
                    && v > Number::F64_MIN_SAFE_INTEGER
                    && v.trunc() == v
                {
                    Ok(Number::Int(v as i64))
                } else {
                    Ok(Number::Float(v))
                }
            }
        }

        deserializer.deserialize_any(NumberVisitor)
    }
}

impl std::fmt::Display for Number {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Number::Int(value) => write!(f, "{}", value),
            Number::Float(value) => write!(f, "{}", value),
        }
    }
}

pub enum AnyIter<'a> {
    Array(std::slice::Iter<'a, Any>),
    Map(std::collections::hash_map::Iter<'a, String, Any>),
}

impl<'a> Iterator for AnyIter<'a> {
    type Item = (Option<&'a str>, &'a Any);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            AnyIter::Array(iter) => {
                let value = iter.next()?;
                Some((None, value))
            }
            AnyIter::Map(iter) => {
                let (key, value) = iter.next()?;
                Some((Some(key), value))
            }
        }
    }
}

pub struct AnyArrayIter {
    source: Arc<[Any]>,
    index: usize,
}

impl AnyArrayIter {
    pub fn new(source: Arc<[Any]>) -> Self {
        Self { source, index: 0 }
    }
}

impl Iterator for AnyArrayIter {
    type Item = Any;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.source.len() {
            let value = self.source[self.index].clone();
            self.index += 1;
            Some(value)
        } else {
            None
        }
    }
}

pub enum AnyIntoIter {
    Array(AnyArrayIter),
    //TODO: we need to clone the map, because we need to consume its iterator
    // try to figure out something better
    Map(std::collections::hash_map::IntoIter<String, Any>),
}

impl From<Arc<[Any]>> for AnyIntoIter {
    fn from(value: Arc<[Any]>) -> Self {
        Self::Array(AnyArrayIter::new(value))
    }
}

impl From<Arc<HashMap<String, Any>>> for AnyIntoIter {
    fn from(value: Arc<HashMap<String, Any>>) -> Self {
        Self::Map((&*value).clone().into_iter())
    }
}

impl Iterator for AnyIntoIter {
    type Item = (Option<String>, Any);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            AnyIntoIter::Array(iter) => {
                let n = iter.next()?;
                Some((None, n))
            }
            AnyIntoIter::Map(iter) => {
                let (key, value) = iter.next()?;
                Some((Some(key.clone()), value.clone()))
            }
        }
    }
}

macro_rules! impl_from_float {
    ($t:ty) => {
        impl From<$t> for Any {
            #[inline]
            fn from(v: $t) -> Self {
                Self::Number(Number::Float(v as f64))
            }
        }

        impl TryFrom<Any> for $t {
            type Error = Any;

            fn try_from(v: Any) -> Result<Self, Self::Error> {
                match v {
                    Any::Number(num) => match num.as_f64() {
                        Some(n) => Ok(n as Self),
                        None => Err(v),
                    },
                    other => Err(other),
                }
            }
        }
    };
}
macro_rules! impl_from_int {
    ($t:ty) => {
        impl From<$t> for Any {
            fn from(value: $t) -> Self {
                Any::Number(Number::from(value as i64))
            }
        }

        impl TryFrom<Any> for $t {
            type Error = Any;

            fn try_from(v: Any) -> Result<Self, Self::Error> {
                match v {
                    Any::Number(num) => match num.as_i64() {
                        Some(n) => Ok(n as Self),
                        None => Err(v),
                    },
                    other => Err(other),
                }
            }
        }
    };
}

impl_from_float!(f32);
impl_from_float!(f64);
impl_from_int!(i16);
impl_from_int!(i32);
impl_from_int!(u16);
impl_from_int!(u32);
impl_from_int!(i64);
impl_from_int!(isize);

impl TryFrom<u64> for Any {
    type Error = u64;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Ok(Any::Number(Number::try_from(value)?))
    }
}

impl TryFrom<Any> for u64 {
    type Error = Any;

    fn try_from(v: Any) -> Result<Self, Self::Error> {
        match v {
            Any::Number(num) => match num.as_i64() {
                Some(n) if n >= 0 => Ok(n as u64),
                _ => Err(Any::Number(num)),
            },
            other => Err(other),
        }
    }
}

impl TryFrom<usize> for Any {
    type Error = usize;

    #[cfg(target_pointer_width = "32")]
    fn try_from(value: usize) -> Result<Self, Self::Error> {
        // for 32-bit architectures we know that usize will always fit,
        // so there's no need to check for length, but we stick to TryInto
        // trait to keep API compatibility
        Ok(Any::Number(Number::Int(value as i64)))
    }

    #[cfg(target_pointer_width = "64")]
    fn try_from(value: usize) -> Result<Self, Self::Error> {
        use std::convert::TryInto;
        if let Ok(v) = (value as u64).try_into() {
            Ok(v)
        } else {
            Err(value)
        }
    }
}

impl TryFrom<Any> for usize {
    type Error = Any;

    fn try_from(v: Any) -> Result<Self, Self::Error> {
        match v {
            Any::Number(num) => match num.as_i64() {
                Some(n) if n >= 0 => Ok(n as usize),
                _ => Err(v),
            },
            other => Err(other),
        }
    }
}

impl From<bool> for Any {
    #[inline]
    fn from(value: bool) -> Self {
        Any::Bool(value)
    }
}

impl TryFrom<Any> for bool {
    type Error = Any;

    fn try_from(v: Any) -> Result<Self, Self::Error> {
        match v {
            Any::Bool(num) => Ok(num),
            other => Err(other),
        }
    }
}

impl From<String> for Any {
    #[inline]
    fn from(value: String) -> Self {
        Any::String(value.into())
    }
}

impl TryFrom<Any> for String {
    type Error = Any;

    fn try_from(v: Any) -> Result<Self, Self::Error> {
        match v {
            Any::String(value) => Ok(String::from(value.as_ref())),
            other => Err(other),
        }
    }
}

impl From<&str> for Any {
    #[inline]
    fn from(value: &str) -> Self {
        Any::String(value.into())
    }
}

impl From<Arc<str>> for Any {
    #[inline]
    fn from(value: Arc<str>) -> Self {
        Any::String(value.clone())
    }
}

impl TryFrom<Any> for Arc<str> {
    type Error = Any;

    fn try_from(v: Any) -> Result<Self, Self::Error> {
        match v {
            Any::String(value) => Ok(value),
            other => Err(other),
        }
    }
}

impl From<Vec<u8>> for Any {
    #[inline]
    fn from(value: Vec<u8>) -> Self {
        Any::Buffer(Arc::from(value))
    }
}

impl TryFrom<Any> for Vec<u8> {
    type Error = Any;

    fn try_from(v: Any) -> Result<Self, Self::Error> {
        match v {
            Any::Buffer(value) => Ok(Vec::from(value.as_ref())),
            other => Err(other),
        }
    }
}

impl From<Arc<[u8]>> for Any {
    #[inline]
    fn from(value: Arc<[u8]>) -> Self {
        Any::Buffer(value)
    }
}

impl TryFrom<Any> for Arc<[u8]> {
    type Error = Any;

    fn try_from(v: Any) -> Result<Self, Self::Error> {
        match v {
            Any::Buffer(value) => Ok(value),
            other => Err(other),
        }
    }
}

impl From<&[u8]> for Any {
    #[inline]
    fn from(value: &[u8]) -> Self {
        Any::Buffer(Arc::from(value))
    }
}

impl<T> From<Option<T>> for Any
where
    T: Into<Any>,
{
    fn from(v: Option<T>) -> Any {
        match v {
            None => Any::Null,
            Some(value) => value.into(),
        }
    }
}

impl<T> From<Vec<T>> for Any
where
    T: Into<Any>,
{
    fn from(v: Vec<T>) -> Any {
        let mut array = Vec::with_capacity(v.len());
        for value in v {
            array.push(value.into())
        }
        Any::Array(Arc::from(array))
    }
}

impl<T> From<HashMap<String, T>> for Any
where
    T: Into<Any>,
{
    fn from(v: HashMap<String, T>) -> Any {
        let mut map = HashMap::with_capacity(v.len());
        for (key, value) in v {
            map.insert(key, value.into());
        }
        Any::Map(Arc::new(map))
    }
}

// This code is based on serde_json::json! macro (see: https://docs.rs/serde_json/latest/src/serde_json/macros.rs.html#53-58).
// Kudos to the original authors.

/// Construct a lib0 [Any] value literal.
///
/// # Examples
///
/// ```rust
///
/// use yrs::any;
///
/// let value = any!({
///   "code": 200,
///   "success": true,
///   "payload": {
///     "features": [
///       "lib0",
///       true
///     ]
///   }
/// });
/// ```
#[macro_export(local_inner_macros)]
macro_rules! any {
    // Hide distracting implementation details from the generated rustdoc.
    ($($any:tt)+) => {
        any_internal!($($any)+)
    };
}

#[macro_export(local_inner_macros)]
#[doc(hidden)]
macro_rules! any_internal {
    (@array [$($items:expr,)*]) => {
        any_internal_array![$($items,)*]
    };

    // Done without trailing comma.
    (@array [$($items:expr),*]) => {
        any_internal_array![$($items),*]
    };

    // Next item is `null`.
    (@array [$($items:expr,)*] null $($rest:tt)*) => {
        any_internal!(@array [$($items,)* any_internal!(null)] $($rest)*)
    };

    // Next item is `true`.
    (@array [$($items:expr,)*] true $($rest:tt)*) => {
        any_internal!(@array [$($items,)* any_internal!(true)] $($rest)*)
    };

    // Next item is `false`.
    (@array [$($items:expr,)*] false $($rest:tt)*) => {
        any_internal!(@array [$($items,)* any_internal!(false)] $($rest)*)
    };

    // Next item is an array.
    (@array [$($items:expr,)*] [$($array:tt)*] $($rest:tt)*) => {
        any_internal!(@array [$($items,)* any_internal!([$($array)*])] $($rest)*)
    };

    // Next item is a map.
    (@array [$($items:expr,)*] {$($map:tt)*} $($rest:tt)*) => {
        any_internal!(@array [$($items,)* any_internal!({$($map)*})] $($rest)*)
    };

    // Next item is an expression followed by comma.
    (@array [$($items:expr,)*] $next:expr, $($rest:tt)*) => {
        any_internal!(@array [$($items,)* any_internal!($next),] $($rest)*)
    };

    // Last item is an expression with no trailing comma.
    (@array [$($items:expr,)*] $last:expr) => {
        any_internal!(@array [$($items,)* any_internal!($last)])
    };

    // Comma after the most recent item.
    (@array [$($items:expr),*] , $($rest:tt)*) => {
        any_internal!(@array [$($items,)*] $($rest)*)
    };

    // Unexpected token after most recent item.
    (@array [$($items:expr),*] $unexpected:tt $($rest:tt)*) => {
        any_unexpected!($unexpected)
    };

    (@object $object:ident () () ()) => {};

    // Insert the current entry followed by trailing comma.
    (@object $object:ident [$($key:tt)+] ($value:expr) , $($rest:tt)*) => {
        let _ = $object.insert(($($key)+).into(), $value);
        any_internal!(@object $object () ($($rest)*) ($($rest)*));
    };

    // Current entry followed by unexpected token.
    (@object $object:ident [$($key:tt)+] ($value:expr) $unexpected:tt $($rest:tt)*) => {
        any_unexpected!($unexpected);
    };

    // Insert the last entry without trailing comma.
    (@object $object:ident [$($key:tt)+] ($value:expr)) => {
        let _ = $object.insert(($($key)+).into(), $value);
    };

    // Next value is `null`.
    (@object $object:ident ($($key:tt)+) (: null $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object [$($key)+] (any_internal!(null)) $($rest)*);
    };

    // Next value is `true`.
    (@object $object:ident ($($key:tt)+) (: true $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object [$($key)+] (any_internal!(true)) $($rest)*);
    };

    // Next value is `false`.
    (@object $object:ident ($($key:tt)+) (: false $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object [$($key)+] (any_internal!(false)) $($rest)*);
    };

    // Next value is an array.
    (@object $object:ident ($($key:tt)+) (: [$($array:tt)*] $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object [$($key)+] (any_internal!([$($array)*])) $($rest)*);
    };

    // Next value is a map.
    (@object $object:ident ($($key:tt)+) (: {$($map:tt)*} $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object [$($key)+] (any_internal!({$($map)*})) $($rest)*);
    };

    // Next value is an expression followed by comma.
    (@object $object:ident ($($key:tt)+) (: $value:expr , $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object [$($key)+] (any_internal!($value)) , $($rest)*);
    };

    // Last value is an expression with no trailing comma.
    (@object $object:ident ($($key:tt)+) (: $value:expr) $copy:tt) => {
        any_internal!(@object $object [$($key)+] (any_internal!($value)));
    };

    // Missing value for last entry. Trigger a reasonable error message.
    (@object $object:ident ($($key:tt)+) (:) $copy:tt) => {
        // "unexpected end of macro invocation"
        any_internal!();
    };

    // Missing colon and value for last entry. Trigger a reasonable error
    // message.
    (@object $object:ident ($($key:tt)+) () $copy:tt) => {
        // "unexpected end of macro invocation"
        any_internal!();
    };

    // Misplaced colon. Trigger a reasonable error message.
    (@object $object:ident () (: $($rest:tt)*) ($colon:tt $($copy:tt)*)) => {
        // Takes no arguments so "no rules expected the token `:`".
        any_unexpected!($colon);
    };

    // Found a comma inside a key. Trigger a reasonable error message.
    (@object $object:ident ($($key:tt)*) (, $($rest:tt)*) ($comma:tt $($copy:tt)*)) => {
        // Takes no arguments so "no rules expected the token `,`".
        any_unexpected!($comma);
    };

    // Key is fully parenthesized. This avoids clippy double_parens false
    // positives because the parenthesization may be necessary here.
    (@object $object:ident () (($key:expr) : $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object ($key) (: $($rest)*) (: $($rest)*));
    };

    // Refuse to absorb colon token into key expression.
    (@object $object:ident ($($key:tt)*) (: $($unexpected:tt)+) $copy:tt) => {
        json_expect_expr_comma!($($unexpected)+);
    };

    // Munch a token into the current key.
    (@object $object:ident ($($key:tt)*) ($tt:tt $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object ($($key)* $tt) ($($rest)*) ($($rest)*));
    };

    //////////////////////////////////////////////////////////////////////////
    // The main implementation.
    //
    // Must be invoked as: any_internal!($($json)+)
    //////////////////////////////////////////////////////////////////////////

    (null) => {
        $crate::any::Any::Null
    };

    (true) => {
        $crate::any::Any::Bool(true)
    };

    (false) => {
        $crate::any::Any::Bool(false)
    };

    ([]) => {
        $crate::any::Any::Array(any_internal_array![])
    };

    ([ $($tt:tt)+ ]) => {
        $crate::any::Any::Array(any_internal!(@array [] $($tt)+))
    };

    ({}) => {
        $crate::any::Any::Map(std::sync::Arc::new(std::collections::HashMap::new()))
    };

    ({ $($tt:tt)+ }) => {
        $crate::any::Any::Map({
            let mut object = std::collections::HashMap::new();
            any_internal!(@object object () ($($tt)+) ($($tt)+));
            std::sync::Arc::new(object)
        })
    };

    // Any Serialize type: numbers, strings, struct literals, variables etc.
    // Must be below every other rule.
    ($other:expr) => {
        ($other).into()
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! any_internal_array {
    ($($content:tt)*) => {
        std::sync::Arc::from([$($content)*])
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! any_unexpected {
    () => {};
}

#[macro_export]
#[doc(hidden)]
macro_rules! any_expect_expr_comma {
    ($e:expr , $($tt:tt)*) => {};
}

#[cfg(test)]
mod test {
    use crate::any::Any;
    use crate::encoding::read::Cursor;
    use crate::Number;

    #[test]
    fn decode_map_rejects_length_amplification() {
        // Regression: an `Any::Map` whose length prefix declares a huge entry count (a few bytes)
        // must not trigger an eager multi-hundred-MB allocation. `try_reserve` turns it into a
        // recoverable decode error instead of an abort under a hard memory limit.
        // Bytes: tag 118 (Map) + var-int len ~250M (0xFF 0xFF 0xFF 0x7A) + no data.
        let adversarial = [118u8, 0xFF, 0xFF, 0xFF, 0x7A];
        let mut cursor = Cursor::new(&adversarial);
        assert!(
            Any::decode(&mut cursor).is_err(),
            "an oversized Any::Map length must be rejected, not eagerly allocated"
        );
    }

    #[test]
    fn decode_array_rejects_length_amplification() {
        // As above, for `Any::Array`. Bytes: tag 117 (Array) + var-int len ~250M + no data.
        let adversarial = [117u8, 0xFF, 0xFF, 0xFF, 0x7A];
        let mut cursor = Cursor::new(&adversarial);
        assert!(
            Any::decode(&mut cursor).is_err(),
            "an oversized Any::Array length must be rejected, not eagerly allocated"
        );
    }

    fn hex(n: Number) -> String {
        let mut buf = Vec::new();
        Any::Number(n).encode(&mut buf);
        buf.iter().map(|b| format!("{:02x}", b)).collect()
    }

    #[test]
    fn number_encoding_yjs_compat() {
        assert_eq!(hex(Number::Int(0)), "7d00");
        assert_eq!(hex(Number::Int(42)), "7d2a");
        assert_eq!(hex(Number::Int(-42)), "7d6a");
        assert_eq!(hex(Number::Int(2147483647)), "7dbfffffff0f");
        // above 2^31 lib0 stops using the var int tag, even for whole numbers
        assert_eq!(hex(Number::Int(2147483648)), "7c4f000000");
        assert_eq!(hex(Number::Int(-2147483648)), "7ccf000000");
        assert_eq!(hex(Number::Int(4294967295)), "7b41efffffffe00000");
        assert_eq!(hex(Number::Int(9007199254740991)), "7b433fffffffffffff");
        assert_eq!(hex(Number::Float(1.5)), "7c3fc00000");
        assert_eq!(hex(Number::Float(1.1)), "7b3ff199999999999a");
        assert_eq!(hex(Number::Float(2147483648.0)), "7c4f000000");
        assert_eq!(hex(Number::Float(5e9)), "7c4f9502f9");
        assert_eq!(hex(Number::Float(1e30)), "7b46293e5939a08cea");
        assert_eq!(hex(Number::Float(f64::NAN)), "7b7ff8000000000000");
        assert_eq!(hex(Number::Float(f64::INFINITY)), "7c7f800000");
        assert_eq!(hex(Number::Float(f64::NEG_INFINITY)), "7cff800000");
    }
}
