use std::cmp::PartialEq;
use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::sync::Arc;
use crate::encoding::read::{Error, Read};
use crate::encoding::write::Write;

pub const F64_MAX_SAFE_INTEGER: f64 = (i64::pow(2, 53) - 1) as f64;
pub const F64_MIN_SAFE_INTEGER: f64 = -F64_MAX_SAFE_INTEGER;

/// Any is an enum with a potentially associated value that is used to represent JSON values
/// and supports efficient encoding of those values.
#[derive(Debug, Clone, PartialEq)]
pub enum Any {
    Null,
    Undefined,
    Bool(bool),
    Number(f64),
    BigInt(i64),
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
            125 => Any::Number(decoder.read_var::<i64>()? as f64),
            // CASE 124: float32
            124 => Any::Number(decoder.read_f32()? as f64),
            // CASE 123: float64
            123 => Any::Number(decoder.read_f64()?),
            // CASE 122: bigint
            122 => Any::BigInt(decoder.read_i64()?),
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
                let mut map = HashMap::with_capacity(len);
                for _ in 0..len {
                    let key = decoder.read_string()?;
                    map.insert(key.to_owned(), Any::decode(decoder)?);
                }
                Any::Map(Arc::new(map))
            }
            // CASE 117: Array<Any>
            117 => {
                let len: usize = decoder.read_var()?;
                let mut arr = Vec::with_capacity(len);
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
                let num_truncated = num.trunc();
                if num_truncated == *num
                    && num_truncated <= F64_MAX_SAFE_INTEGER
                    && num_truncated >= F64_MIN_SAFE_INTEGER
                {
                    // TYPE 125: INTEGER
                    encoder.write_u8(125);
                    encoder.write_var(num_truncated as i64)
                } else if ((*num as f32) as f64) == *num {
                    // TYPE 124: FLOAT32
                    encoder.write_u8(124);
                    encoder.write_f32(*num as f32)
                } else {
                    // TYPE 123: FLOAT64
                    encoder.write_u8(123);
                    encoder.write_f64(*num)
                }
            }
            Any::BigInt(num) => {
                // TYPE 122: BigInt
                encoder.write_u8(122);
                encoder.write_i64(*num)
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
}

impl std::fmt::Display for Any {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Any::Null => f.write_str("null"),
            Any::Undefined => f.write_str("undefined"),
            Any::Bool(value) => write!(f, "{}", value),
            Any::Number(value) => write!(f, "{}", value),
            Any::BigInt(value) => write!(f, "{}", value),
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

macro_rules! impl_from_num {
    ($t:ty) => {
        impl From<$t> for Any {
            #[inline]
            fn from(v: $t) -> Self {
                Self::Number(v as f64)
            }
        }

        impl TryFrom<Any> for $t {
            type Error = Any;

            fn try_from(v: Any) -> Result<Self, Self::Error> {
                match v {
                    Any::Number(num) => Ok(num as Self),
                    Any::BigInt(num) => Ok(num as Self),
                    other => Err(other),
                }
            }
        }
    };
}
macro_rules! impl_from_bigint {
    ($t:ty) => {
        impl From<$t> for Any {
            fn from(value: $t) -> Self {
                let v = value as f64;
                if v <= F64_MAX_SAFE_INTEGER && v >= F64_MIN_SAFE_INTEGER {
                    Self::Number(v)
                } else {
                    Self::BigInt(value as i64)
                }
            }
        }

        impl TryFrom<Any> for $t {
            type Error = Any;

            fn try_from(v: Any) -> Result<Self, Self::Error> {
                match v {
                    Any::Number(num) => Ok(num as Self),
                    Any::BigInt(num) => Ok(num as Self),
                    other => Err(other),
                }
            }
        }
    };
}

impl_from_num!(f32);
impl_from_num!(f64);
impl_from_num!(i16);
impl_from_num!(i32);
impl_from_num!(u16);
impl_from_num!(u32);
impl_from_bigint!(i64);
impl_from_bigint!(isize);

impl TryFrom<u64> for Any {
    type Error = u64;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value > i64::MAX.abs() as u64 {
            Err(value)
        } else {
            let v = value as f64;
            if v <= F64_MAX_SAFE_INTEGER && v >= F64_MIN_SAFE_INTEGER {
                Ok(Any::Number(v))
            } else {
                Ok(Any::BigInt(v as i64))
            }
        }
    }
}

impl TryFrom<Any> for u64 {
    type Error = Any;

    fn try_from(v: Any) -> Result<Self, Self::Error> {
        match v {
            Any::Number(num) => Ok(num as Self),
            Any::BigInt(num) => Ok(num as Self),
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
        Ok(Any::Number(value as f64))
    }

    #[cfg(target_pointer_width = "64")]
    fn try_from(value: usize) -> Result<Self, Self::Error> {
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
            Any::Number(num) => Ok(num as Self),
            Any::BigInt(num) => Ok(num as Self),
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
