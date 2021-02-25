use core::{panic};
use std::collections::HashMap;
use crate::binary;
use crate::any::Any;

#[derive(Default)]
pub struct Decoder<'a> {
    pub buf: &'a [u8],
    next: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(buf: &'a [u8]) -> Decoder<'a> {
        Decoder { buf, next: 0 }
    }
    // Read a single byte.
    pub fn read(&mut self) -> u8 {
        let b = self.buf[self.next];
        self.next += 1;
        b
    }
    // Check if there is still content to be read.
    pub fn has_content(&self) -> bool {
        self.buf.len() > self.next
    }
    // Clone this decoder. This operation is pretty cheap as it only copies references.
    pub fn clone(&self) -> Decoder<'a> {
        Decoder {
            buf: self.buf,
            next: self.next
        }
    }
    // Take a slice of the next `len` bytes and advance the position by `len`.
    pub fn read_buffer(&mut self, len: u32) -> &[u8] {
        let slice = &self.buf[self.next..(self.next + len as usize)];
        self.next += len as usize;
        slice
    }
    // Read a variable length buffer.
    pub fn read_var_buffer(&mut self) -> &[u8] {
        let len = self.read_var_uint() as u32;
        self.read_buffer(len)
    }
    // Read a variable length buffer.
    pub fn peek_var_buffer(&mut self) -> &[u8] {
        let next = self.next;
        let len = self.read_var_uint() as u32;
        let buffer_next = self.next;
        self.next = next;
        let slice = &self.buf[buffer_next..(buffer_next + len as usize)];
        self.next += len as usize;
        slice
    }
    // Read the remaining bytes as buffer
    pub fn read_tail_as_buffer(&mut self) -> &[u8] {
        self.read_buffer((self.buf.len() - self.next) as u32)
    }
    // Skip one byte, jump to the next position
    pub fn skip8 (&mut self) {
        self.next += 1;
    }
    // Read one byte as unsigned integer.
    pub fn read_uint8 (&mut self) -> u8 {
        self.read()
    }
    // Read 2 bytes as unsigned integer
    pub fn read_uint16 (&mut self) -> u16 {
        self.read() as u16 | ((self.read() as u16) << 8)
    }
    // Read 4 bytes as unsigned integer
    pub fn read_uint32(&mut self) -> u32 {
        self.read() as u32
            | (self.read() as u32) << 8
            | (self.read() as u32) << 16
            | (self.read() as u32) << 24
    }
    // Read 4 bytes as unsigned integer in big endian order.
    // (most significant byte first)
    pub fn read_uint32_big_endian(&mut self) -> u32 {
        (self.read() as u32) << 24
            | (self.read() as u32) << 16
            | (self.read() as u32) << 8
            | self.read() as u32
    }
    // Look ahead without incrementing the position
    // to the next byte and read it as unsigned integer.
    pub fn peek_uint8(&mut self) -> u8 {
        self.buf[self.next]
    }
    // Look ahead without incrementing the position
    // to the next byte and read it as unsigned integer.
    pub fn peek_uint16(&mut self) -> u16 {
        (self.buf[self.next] as u16) | (self.buf[self.next + 1] as u16) << 8
    }
    // Look ahead without incrementing the position
    // to the next byte and read it as unsigned integer.
    pub fn peek_uint32(&self) -> u32 {
        self.buf[self.next] as u32
            | (self.buf[self.next + 1] as u32) << 8
            | (self.buf[self.next + 2] as u32) << 16
            | (self.buf[self.next + 3] as u32) << 24
    }
    // Read unsigned integer with variable length.
    // * numbers < 2^7 are stored in one byte
    // * numbers < 2^14 are stored in two bytes
    // @todo currently, only 32 bits supported
    pub fn read_var_uint(&mut self) -> u64 {
        let mut num: u64 = 0;
        let mut len: usize = 0;
        loop {
            let r = self.read() as u64;
            num |= (r & binary::BITS7 as u64) << len;
            len += 7;
            if r < binary::BIT8 as u64 {
                return num
            }
            if len > 35 {
                panic!("Integer out of range!");
            }
        }
    }
    // Read signed integer with variable length.
    // * numbers < 2^7 are stored in one byte
    // * numbers < 2^14 are stored in two bytes
    // @todo currently, only 32 bits supported
    pub fn read_var_int(&mut self) -> i64 {
        let mut r = self.read();
        let mut num = (r & binary::BITS6 as u8) as i64;
        let mut len: u32 = 6;
        let is_negative = r & binary::BIT7 as u8 > 0;
        if r & binary::BIT8 as u8 == 0 {
            return if is_negative { -num } else { num }
        }
        loop {
            r = self.read();
            num |= (r as i64 & binary::BITS7 as i64) << len;
            len += 7;
            if r < binary::BIT8 as u8 {
                return if is_negative { -num } else { num }
            }
            if len > 128 {
                panic!("Integer out of range!");
            }
        }
    }
    // Look ahead and read var_uint without incrementing position
    pub fn peek_var_uint (&mut self) -> u64 {
        let pos = self.next;
        let s = self.read_var_uint();
        self.next = pos;
        s
    }
    // Look ahead and read var_int without incrementing position
    pub fn peek_var_int (&mut self) -> i64 {
        let pos = self.next;
        let s = self.read_var_int();
        self.next = pos;
        s
    }
    // Read string of variable length.
    // read_var_uint is used to read the length of the string.
    pub fn read_var_string (&mut self) -> &str {
        let buf = self.read_var_buffer();
        unsafe {
            std::str::from_utf8_unchecked(buf)
        }
    }
    // Look ahead and read var_string without incrementing position
    pub fn peek_var_string (&mut self) -> &str {
        let buf = self.peek_var_buffer();
        unsafe {
            std::str::from_utf8_unchecked(buf)
        }
    }
    // read buffer of 4 bytes as fixed-length array
    pub fn read_buffer_fixed4 (&mut self) -> [u8; 4] {
        let buf = self.read_buffer(4);
        let mut res: [u8; 4] = Default::default();
        res.clone_from_slice(buf);
        res
    }
    // read buffer of 8 bytes as fixed-length array
    pub fn read_buffer_fixed8 (&mut self) -> [u8; 8] {
        let buf = self.read_buffer(4);
        let mut res: [u8; 8] = Default::default();
        res.clone_from_slice(buf);
        res
    }
    // Read float32 in big endian order
    pub fn read_float32 (&mut self) -> f32 {
        f32::from_be_bytes(self.read_buffer_fixed4())
    }
    // Read float64 in big endian order
    // @todo there must be a more elegant way to convert a slice to a fixed-length buffer.
    pub fn read_float64 (&mut self) -> f64 {
        f64::from_be_bytes(self.read_buffer_fixed8())
    }
    // read BigInt64 in big endian order
    pub fn read_bigint64 (&mut self) -> i64{
        i64::from_be_bytes(self.read_buffer_fixed8())
    }
    // read BigUInt64 in big endian order
    pub fn read_biguint64 (&mut self) -> u64 {
        u64::from_be_bytes(self.read_buffer_fixed8())
    }
    pub fn read_any (&mut self) -> Any {
        match self.read_uint8() {
            // CASE 127: undefined
            127 => {
                Any::Undefined
            }
            // CASE 126: null
            126 => {
                Any::Null
            }
            // CASE 125: integer
            125 => {
                Any::Number(self.read_var_int() as f64)
            }
            // CASE 124: float32
            124 => {
                Any::Number(self.read_float32() as f64)
            }
            // CASE 123: float64
            123 => {
                Any::Number(self.read_float64())
            }
            // CASE 122: bigint
            122 => {
                Any::BigInt(self.read_bigint64())
            }
            // CASE 121: boolean (false)
            121 => {
                Any::Bool(false)
            }
            // CASE 120: boolean (true)
            120 => {
                Any::Bool(true)
            }
            // CASE 119: string
            119 => {
                Any::String(self.read_var_string().to_owned())
            }
            // CASE 118: Map<string,Any>
            118 => {
                let len = self.read_var_uint();
                let mut map = HashMap::new();
                for _ in 0..len {
                    let key = self.read_var_string();
                    map.insert(key.to_owned(), self.read_any());
                }
                Any::Map(map)
            }
            // CASE 117: Array<Any>
            117 => {
                let len = self.read_var_uint();
                let mut arr = Vec::new();
                for _ in 0..len {
                    arr.push(self.read_any());
                }
                Any::Array(arr)
            }
            // CASE 116: buffer
            116 => {
                Any::Buffer(Box::from(self.read_var_buffer().to_owned()))
            }
            _ => {
                panic!("Unable to read Any content");
            }
        }
    }
}
