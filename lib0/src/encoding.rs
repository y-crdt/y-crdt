use crate::binary;
use crate::{any::Any, number::Uint};
use std::io::Write;

#[derive(Default)]
pub struct Encoder {
    pub buf: Vec<u8>,
}
impl Encoder {
    pub fn new() -> Encoder {
        Encoder::with_capacity(10000)
    }

    pub fn with_capacity(capacity: usize) -> Encoder {
        Encoder {
            buf: Vec::with_capacity(capacity),
        }
    }

    /// Write a single byte to the encoder
    pub fn write_u8(&mut self, byte: u8) {
        self.buf.push(byte);
    }

    /// Write an unsigned integer (16bit)
    pub fn write_u16(&mut self, num: u16) {
        self.write_u8(num as u8);
        self.write_u8((num >> 8) as u8);
    }

    /// Write an unsigned integer (32bit)
    pub fn write_u32(&mut self, num: u32) {
        self.write_u8(num as u8);
        self.write_u8((num >> 8) as u8);
        self.write_u8((num >> 16) as u8);
        self.write_u8((num >> 24) as u8);
    }

    /// Write an unsigned integer (32bit) in big endian order (most significant byte first)
    pub fn write_u32_be(&mut self, num: u32) {
        self.write_u8((num >> 24) as u8);
        self.write_u8((num >> 16) as u8);
        self.write_u8((num >> 8) as u8);
        self.write_u8(num as u8);
    }

    /// Write a variable length unsigned integer.
    pub fn write_uvar(&mut self, mut num: impl Uint) {
        while {
            let rest = num.shift7_rest_to_byte();
            let c = !num.is_null();
            self.write_u8(if c { 0b10000000 | rest } else { rest });
            c
        } {}
    }

    /// Write a variable length integer.
    ///
    /// We don't use zig-zag encoding because we want to keep the option open
    /// to use the same function for BigInt and 53bit integers.
    ///
    /// We use the 7th bit instead for signaling that this is a negative number.
    // @todo Support up to 128 bit
    pub fn write_ivar(&mut self, mut num: i64) {
        let is_negative = num < 0;
        num = if is_negative { -num } else { num };
        self.write_u8(
            // whether to continue reading
            (if num > binary::BITS6 as i64 { binary::BIT8 as u8 } else { 0 })
                // whether number is negative
                | (if is_negative { binary::BIT7 as u8 } else { 0 })
                // number
                | (binary::BITS6 as i64 & num) as u8,
        );
        num >>= 6;
        while num > 0 {
            self.write_u8(
                if num > binary::BITS7 as i64 {
                    binary::BIT8 as u8
                } else {
                    0
                } | (binary::BITS7 as i64 & num) as u8,
            );
            num >>= 7;
        }
    }

    /// Write buffer without storing the length of the buffer
    pub fn write(&mut self, buf: &[u8]) {
        self.buf.write(buf).unwrap();
    }

    /// Write variable length buffer (binary content).
    pub fn write_buf<B: AsRef<[u8]>>(&mut self, buf: B) {
        let buf = buf.as_ref();
        self.write_uvar(buf.len());
        self.write(buf);
    }

    /// Write variable-length utf8 string
    pub fn write_string(&mut self, str: &str) {
        self.write_buf(str);
    }

    /// Write floating point number in 4 bytes
    pub fn write_f32(&mut self, num: f32) {
        self.write(&num.to_be_bytes());
    }

    /// Write floating point number in 8 bytes
    pub fn write_f64(&mut self, num: f64) {
        self.write(&num.to_be_bytes());
    }

    /// Write BigInt in 8 bytes in big endian order.
    // @deprecated This method is here for compatibility to lib0/encoding. Instead you should use
    // write_int_64;
    pub fn write_i64(&mut self, num: i64) {
        self.write(&num.to_be_bytes());
    }

    /// Write BigUInt in 8 bytes in big endian order.
    // @deprecated This method is here for compatibility to lib0/encoding. Instead you should use
    // write_int_64;
    pub fn write_u64(&mut self, num: u64) {
        self.write(&num.to_be_bytes());
    }
}
