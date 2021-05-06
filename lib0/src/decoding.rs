use crate::any::Any;
use crate::binary;
use core::panic;
use std::collections::HashMap;
use std::io::Read;

#[derive(Default)]
pub struct Decoder<'a> {
    pub buf: &'a [u8],
    next: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(buf: &'a [u8]) -> Decoder<'a> {
        Decoder { buf, next: 0 }
    }

    /// Read a single byte.
    pub fn read_u8(&mut self) -> u8 {
        let b = self.buf[self.next];
        self.next += 1;
        b
    }

    /// Take a slice of the next `len` bytes and advance the position by `len`.
    pub fn read_buffer(&mut self, len: u32) -> &[u8] {
        let slice = &self.buf[self.next..(self.next + len as usize)];
        self.next += len as usize;
        slice
    }

    /// Read a variable length buffer.
    pub fn read_var_buffer(&mut self) -> &[u8] {
        let len: u32 = self.read_uvar();
        self.read_buffer(len)
    }

    /// Read 2 bytes as unsigned integer
    pub fn read_u16(&mut self) -> u16 {
        self.read_u8() as u16 | ((self.read_u8() as u16) << 8)
    }

    /// Read 4 bytes as unsigned integer
    pub fn read_u32(&mut self) -> u32 {
        self.read_u8() as u32
            | (self.read_u8() as u32) << 8
            | (self.read_u8() as u32) << 16
            | (self.read_u8() as u32) << 24
    }

    /// Read 4 bytes as unsigned integer in big endian order.
    /// (most significant byte first)
    pub fn read_u32_be(&mut self) -> u32 {
        (self.read_u8() as u32) << 24
            | (self.read_u8() as u32) << 16
            | (self.read_u8() as u32) << 8
            | self.read_u8() as u32
    }

    /// Read unsigned integer with variable length.
    /// * numbers < 2^7 are stored in one byte
    /// * numbers < 2^14 are stored in two bytes
    // @todo currently, only 32 bits supported
    pub fn read_uvar<T: crate::number::Uint>(&mut self) -> T {
        let mut num: T = Default::default();
        let mut len: usize = 0;
        loop {
            let r = self.read_u8();
            num.unshift_add(len, r & binary::BITS7);
            len += 7;
            if r < binary::BIT8 {
                return num;
            }
            if len > 128 {
                panic!("Integer out of range!");
            }
        }
    }

    /// Read signed integer with variable length.
    /// * numbers < 2^7 are stored in one byte
    /// * numbers < 2^14 are stored in two bytes
    // @todo currently, only 32 bits supported
    pub fn read_ivar(&mut self) -> i64 {
        let mut r = self.read_u8();
        let mut num = (r & binary::BITS6 as u8) as i64;
        let mut len: u32 = 6;
        let is_negative = r & binary::BIT7 as u8 > 0;
        if r & binary::BIT8 as u8 == 0 {
            return if is_negative { -num } else { num };
        }
        loop {
            r = self.read_u8();
            num |= (r as i64 & binary::BITS7 as i64) << len;
            len += 7;
            if r < binary::BIT8 as u8 {
                return if is_negative { -num } else { num };
            }
            if len > 128 {
                panic!("Integer out of range!");
            }
        }
    }

    /// Look ahead and read var_uint without incrementing position
    pub fn peek_var_uint(&mut self) -> u64 {
        let pos = self.next;
        let s = self.read_uvar();
        self.next = pos;
        s
    }

    /// Look ahead and read var_int without incrementing position
    pub fn peek_var_int(&mut self) -> i64 {
        let pos = self.next;
        let s = self.read_ivar();
        self.next = pos;
        s
    }

    /// Read string of variable length.
    pub fn read_string(&mut self) -> &str {
        let buf = self.read_var_buffer();
        unsafe { std::str::from_utf8_unchecked(buf) }
    }

    /// Read buffer of 4 bytes as fixed-length array
    pub fn read_buffer_fixed4(&mut self) -> [u8; 4] {
        let buf = self.read_buffer(4);
        let mut res: [u8; 4] = Default::default();
        res.clone_from_slice(buf);
        res
    }

    /// Read buffer of 8 bytes as fixed-length array
    pub fn read_buffer_fixed8(&mut self) -> [u8; 8] {
        let buf = self.read_buffer(8);
        let mut res: [u8; 8] = Default::default();
        res.clone_from_slice(buf);
        res
    }

    /// Read float32 in big endian order
    pub fn read_f32(&mut self) -> f32 {
        f32::from_be_bytes(self.read_buffer_fixed4())
    }

    /// Read float64 in big endian order
    // @todo there must be a more elegant way to convert a slice to a fixed-length buffer.
    pub fn read_f64(&mut self) -> f64 {
        f64::from_be_bytes(self.read_buffer_fixed8())
    }

    /// Read BigInt64 in big endian order
    pub fn read_i64(&mut self) -> i64 {
        i64::from_be_bytes(self.read_buffer_fixed8())
    }

    /// read BigUInt64 in big endian order
    pub fn read_u64(&mut self) -> u64 {
        u64::from_be_bytes(self.read_buffer_fixed8())
    }
}
