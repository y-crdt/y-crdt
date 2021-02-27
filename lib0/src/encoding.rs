use crate::any::Any;
use crate::binary;
use std::io::Write;

#[derive(Default)]
pub struct Encoder {
    pub buf: Vec<u8>,
}
impl Encoder {
    pub fn new() -> Encoder {
        Encoder {
            buf: Vec::with_capacity(10000),
        }
    }
    // Write a single byte to the encoder
    pub fn write(&mut self, byte: u8) {
        self.buf.push(byte);
    }
    // Returns the byte-length of the data written to the encoder.
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    // Write one byte at a specific position.
    // The position must already be written (i.e. encoder.len > pos).
    pub fn set(&mut self, pos: usize, byte: u8) {
        self.buf[pos] = byte;
    }
    // Write an unsigned integer (8bit)
    pub fn write_uint8(&mut self, num: u8) {
        self.write(num);
    }
    // Write an unsigned integer (8bit)
    pub fn set_uint8(&mut self, pos: usize, num: u8) {
        self.set(pos, num);
    }
    // Write an unsigned integer (16bit)
    pub fn write_uint16(&mut self, num: u16) {
        self.write(num as u8);
        self.write((num >> 8) as u8);
    }
    // Write an unsigned integer (16bit)
    pub fn set_uint16(&mut self, pos: usize, num: u16) {
        self.set(pos, num as u8);
        self.set(pos + 1, (num >> 8) as u8);
    }
    // Write an unsigned integer (32bit)
    pub fn write_uint32(&mut self, num: u32) {
        self.write(num as u8);
        self.write((num >> 8) as u8);
        self.write((num >> 16) as u8);
        self.write((num >> 24) as u8);
    }
    // Write an unsigned integer (32bit)
    pub fn set_uint32(&mut self, pos: usize, num: u32) {
        self.set(pos, num as u8);
        self.set(pos + 1, (num >> 8) as u8);
        self.set(pos + 2, (num >> 16) as u8);
        self.set(pos + 3, (num >> 24) as u8);
    }
    // Write an unsigned integer (32bit) in big endian order (most significant byte first)
    pub fn write_uint32_big_endian(&mut self, num: u32) {
        self.write((num >> 24) as u8);
        self.write((num >> 16) as u8);
        self.write((num >> 8) as u8);
        self.write(num as u8);
    }
    // Write a variable length unsigned integer.
    //
    // Encodes integers in the range from [0, 4294967295] / [0, 0xffffffff]. (max 32 bit unsigned integer).
    // @todo Support 53, 64, and possibly 128 bit integers.
    pub fn write_var_uint(&mut self, num: u64) {
        let mut rest = num;
        while rest > binary::BITS7 as u64 {
            self.write(binary::BIT8 as u8 | (binary::BITS7 as u8 & rest as u8));
            rest >>= 7;
        }
        self.write(binary::BITS7 as u8 & rest as u8);
    }
    // Write a variable length integer.
    //
    // We don't use zig-zag encoding because we want to keep the option open
    // to use the same function for BigInt and 53bit integers.
    //
    // We use the 7th bit instead for signaling that this is a negative number.
    // @todo Support up to 128 bit
    pub fn write_var_int(&mut self, num: i64) {
        let is_negative = num < 0;
        let mut rest = if is_negative { -num } else { num };
        self.write(
            // whether to continue reading
            (if rest > binary::BITS6 as i64 { binary::BIT8 as u8 } else { 0 })
                // whether number is negative
                | (if is_negative { binary::BIT7 as u8 } else { 0 })
                // number
                | (binary::BITS6 as i64 & rest) as u8,
        );
        rest >>= 6;
        while rest > 0 {
            self.write(
                if rest > binary::BITS7 as i64 { binary::BIT8 as u8 } else { 0 }
                | (binary::BITS7 as i64 & rest) as u8
            );
            rest >>= 7;
        }
    }
    // Write buffer without storing the length of the buffer
    pub fn write_buffer(&mut self, buf: &[u8]) {
        self.buf.write(buf).expect("");
    }
    // Write variable length buffer (binary content).
    pub fn write_var_buffer(&mut self, buf: &[u8]) {
        self.write_var_uint(buf.len() as u64);
        self.write_buffer(buf);
    }
    // Write variable-length utf8 string
    pub fn write_var_string(&mut self, str: &str) {
        self.write_var_buffer(str.as_bytes());
    }
    // Write floating point number in 4 bytes
    pub fn write_float32(&mut self, num: f32) {
        self.write_buffer(&num.to_be_bytes());
    }
    // Write floating point number in 8 bytes
    pub fn write_float64(&mut self, num: f64) {
        self.write_buffer(&num.to_be_bytes());
    }
    // Write BigInt in 8 bytes in big endian order.
    // @deprecated This method is here for compatibility to lib0/encoding. Instead you should use
    // write_int_64;
    pub fn write_big_int64(&mut self, num: i64) {
        self.write_buffer(&num.to_be_bytes());
    }
    // Write BigUInt in 8 bytes in big endian order.
    // @deprecated This method is here for compatibility to lib0/encoding. Instead you should use
    // write_int_64;
    pub fn write_big_uint64(&mut self, num: u64) {
        self.write_buffer(&num.to_be_bytes());
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
    pub fn write_any(&mut self, obj: &Any) {
        match obj {
            Any::Undefined => {
                // TYPE 127: undefined
                self.write(127)
            }
            Any::Null => {
                // TYPE 126: null
                self.write(126)
            }
            Any::Bool(bool) => {
                // TYPE 120/121: boolean (true/false)
                self.write(if *bool { 120 } else { 121 })
            }
            Any::String(str) => {
                // TYPE 119: String
                self.write(119);
                self.write_var_string(&str);
            }
            Any::Number(num) => {
                let num_truncated = num.trunc();
                if num_truncated == *num && num_truncated <= crate::number::F64_MAX_SAFE_INTEGER && num_truncated >= crate::number::F64_MIN_SAFE_INTEGER {
                    // TYPE 125: INTEGER
                    self.write(125);
                    self.write_var_int(num_truncated as i64);
                } else if is_float_32(*num) {
                    // TYPE 124: FLOAT32
                    self.write(124);
                    self.write_float32(*num as f32);
                } else {
                    // TYPE 123: FLOAT64
                    self.write(123);
                    self.write_float64(*num);
                }
            }
            Any::BigInt(num) => {
                // TYPE 122: BigInt
                self.write(122);
                self.write_big_int64(*num);
            }
            Any::Array(arr) => {
                // TYPE 117: Array
                self.write(117);
                self.write_var_uint(arr.len() as u64);
                for el in arr.iter() {
                    self.write_any(el);
                }
            }
            Any::Map(map) => {
                // TYPE 118: Map
                self.write(118);
                self.write_var_uint(map.len() as u64);
                for (key, value) in map {
                    self.write_var_string(&key);
                    self.write_any(value);
                }
            }
            Any::Buffer(buf) => {
                // TYPE 116: Buffer
                self.write(116);
                self.write_var_buffer(&buf)
            }
        }
    }
}

pub fn is_float_32(num: f64) -> bool {
    ((num as f32) as f64) == num
}
