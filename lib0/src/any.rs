use crate::decoding::Read;
use crate::encoding::Write;
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

impl Any {
    pub fn decode<R: Read>(decoder: &mut R) -> Self {
        match decoder.read_u8() {
            // CASE 127: undefined
            127 => Any::Undefined,
            // CASE 126: null
            126 => Any::Null,
            // CASE 125: integer
            125 => Any::Number(decoder.read_ivar() as f64),
            // CASE 124: float32
            124 => Any::Number(decoder.read_f32() as f64),
            // CASE 123: float64
            123 => Any::Number(decoder.read_f64()),
            // CASE 122: bigint
            122 => Any::BigInt(decoder.read_i64()),
            // CASE 121: boolean (false)
            121 => Any::Bool(false),
            // CASE 120: boolean (true)
            120 => Any::Bool(true),
            // CASE 119: string
            119 => Any::String(decoder.read_string().to_owned()),
            // CASE 118: Map<string,Any>
            118 => {
                let len: usize = decoder.read_uvar();
                let mut map = HashMap::with_capacity(len);
                for _ in 0..len {
                    let key = decoder.read_string();
                    map.insert(key.to_owned(), Any::decode(decoder));
                }
                Any::Map(map)
            }
            // CASE 117: Array<Any>
            117 => {
                let len: usize = decoder.read_uvar();
                let mut arr = Vec::with_capacity(len);
                for _ in 0..len {
                    arr.push(Any::decode(decoder));
                }
                Any::Array(arr)
            }
            // CASE 116: buffer
            116 => Any::Buffer(Box::from(decoder.read_buf().to_owned())),
            _ => {
                panic!("Unable to read Any content");
            }
        }
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
                encoder.write_string(&str);
            }
            Any::Number(num) => {
                let num_truncated = num.trunc();
                if num_truncated == *num
                    && num_truncated <= crate::number::F64_MAX_SAFE_INTEGER
                    && num_truncated >= crate::number::F64_MIN_SAFE_INTEGER
                {
                    // TYPE 125: INTEGER
                    encoder.write_u8(125);
                    encoder.write_ivar(num_truncated as i64);
                } else if ((*num as f32) as f64) == *num {
                    // TYPE 124: FLOAT32
                    encoder.write_u8(124);
                    encoder.write_f32(*num as f32);
                } else {
                    // TYPE 123: FLOAT64
                    encoder.write_u8(123);
                    encoder.write_f64(*num);
                }
            }
            Any::BigInt(num) => {
                // TYPE 122: BigInt
                encoder.write_u8(122);
                encoder.write_i64(*num);
            }
            Any::Array(arr) => {
                // TYPE 117: Array
                encoder.write_u8(117);
                encoder.write_uvar(arr.len() as u64);
                for el in arr.iter() {
                    el.encode(encoder);
                }
            }
            Any::Map(map) => {
                // TYPE 118: Map
                encoder.write_u8(118);
                encoder.write_uvar(map.len() as u64);
                for (key, value) in map {
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
