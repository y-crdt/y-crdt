use crate::decoding::Read;
use crate::encoding::Write;
use crate::error::Error;
use std::cmp::PartialEq;
use std::collections::HashMap;
use std::convert::TryInto;

/// Any is an enum with a potentially associated value that is used to represent JSON values
/// and supports efficient encoding of those values.
#[derive(Debug, Clone, PartialEq)]
pub enum Any {
    Null,
    Undefined,
    Bool(bool),
    Number(f64),
    BigInt(i64),
    String(Box<str>),
    Buffer(Box<[u8]>),
    Array(Box<[Any]>),
    Map(Box<HashMap<String, Any>>),
}

impl Any {
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
                Any::String(Box::from(str))
            }
            // CASE 118: Map<string,Any>
            118 => {
                let len: usize = decoder.read_var()?;
                let mut map = HashMap::with_capacity(len);
                for _ in 0..len {
                    let key = decoder.read_string()?;
                    map.insert(key.to_owned(), Any::decode(decoder)?);
                }
                Any::Map(Box::new(map))
            }
            // CASE 117: Array<Any>
            117 => {
                let len: usize = decoder.read_var()?;
                let mut arr = Vec::with_capacity(len);
                for _ in 0..len {
                    arr.push(Any::decode(decoder)?);
                }
                Any::Array(arr.into_boxed_slice())
            }
            // CASE 116: buffer
            116 => Any::Buffer(Box::from(decoder.read_buf()?.to_owned())),
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
                    && num_truncated <= crate::number::F64_MAX_SAFE_INTEGER
                    && num_truncated >= crate::number::F64_MIN_SAFE_INTEGER
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

    #[cfg(not(feature = "lib0-serde"))]
    pub fn to_json(&self, buf: &mut String) {
        use std::fmt::Write;

        fn quoted(buf: &mut String, s: &str) {
            buf.reserve(s.len() + 2);
            buf.push('"');
            for c in s.chars() {
                match c {
                    '\\' => buf.push_str("\\\\"),
                    '\u{0008}' => buf.push_str("\\b"),
                    '\u{000c}' => buf.push_str("\\f"),
                    '\n' => buf.push_str("\\n"),
                    '\r' => buf.push_str("\\r"),
                    '\t' => buf.push_str("\\t"),
                    '"' => buf.push_str("\\\""),
                    c if c.is_control() => write!(buf, "\\u{:04x}", c as u32).unwrap(),
                    c => buf.push(c),
                }
            }
            buf.push('"');
        }

        match self {
            Any::Null => buf.push_str("null"),
            Any::Bool(value) => write!(buf, "{}", value).unwrap(),
            Any::Number(value) => write!(buf, "{}", value).unwrap(),
            Any::BigInt(value) => write!(buf, "{}", value).unwrap(),
            Any::String(value) => quoted(buf, value.as_ref()),
            Any::Array(values) => {
                buf.push('[');
                let mut i = values.iter();
                if let Some(value) = i.next() {
                    value.to_json(buf);
                }
                while let Some(value) = i.next() {
                    buf.push(',');
                    value.to_json(buf);
                }
                buf.push(']');
            }
            Any::Map(entries) => {
                buf.push('{');
                let mut i = entries.iter();
                if let Some((key, value)) = i.next() {
                    quoted(buf, key.as_str());
                    buf.push(':');
                    value.to_json(buf);
                }
                while let Some((key, value)) = i.next() {
                    buf.push(',');
                    quoted(buf, key.as_str());
                    buf.push(':');
                    value.to_json(buf);
                }
                buf.push('}');
            }
            other => panic!(
                "Serialization of {} into JSON representation is not supported",
                other
            ),
        }
    }

    #[cfg(not(feature = "lib0-serde"))]
    pub fn from_json(src: &str) -> Result<Self, Error> {
        use crate::json_parser::JsonParser;
        let mut parser = JsonParser::new(src.chars());
        Ok(parser.parse()?)
    }

    #[cfg(feature = "lib0-serde")]
    pub fn from_json(src: &str) -> Result<Self, Error> {
        Ok(serde_json::from_str(src)?)
    }

    #[cfg(feature = "lib0-serde")]
    pub fn to_json(&self, buf: &mut String) {
        use serde::Serialize;
        use serde_json::Serializer;

        let buf = unsafe { buf.as_mut_vec() };
        let mut cursor = std::io::Cursor::new(buf);

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

impl Into<Any> for i64 {
    fn into(self) -> Any {
        Any::BigInt(self)
    }
}

impl Into<Any> for String {
    fn into(self) -> Any {
        Any::String(self.into_boxed_str())
    }
}

impl Into<Any> for &str {
    fn into(self) -> Any {
        Any::String(self.into())
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
        Any::Array(array.into_boxed_slice())
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
        Any::Map(Box::new(map))
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
