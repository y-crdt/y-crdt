use crate::binary;
use core::panic;
use std::mem::MaybeUninit;

#[derive(Default)]
pub struct Cursor<'a> {
    pub buf: &'a [u8],
    pub next: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(buf: &'a [u8]) -> Cursor<'a> {
        Cursor { buf, next: 0 }
    }
}

impl<'a, R> From<&'a R> for Cursor<'a>
where
    R: AsRef<[u8]>,
{
    fn from(buf: &'a R) -> Self {
        Self::new(buf.as_ref())
    }
}

impl<'a> Read for Cursor<'a> {
    /// Read a single byte.
    fn read_u8(&mut self) -> u8 {
        let b = self.buf[self.next];
        self.next += 1;
        b
    }

    /// Take a slice of the next `len` bytes and advance the position by `len`.
    fn read(&mut self, len: usize) -> &[u8] {
        let slice = &self.buf[self.next..(self.next + len)];
        self.next += len as usize;
        slice
    }
}

pub trait Read {
    /// Read a single byte.
    fn read_u8(&mut self) -> u8;

    fn read(&mut self, len: usize) -> &[u8];

    /// Read a variable length buffer.
    fn read_buf(&mut self) -> &[u8] {
        let len: u32 = self.read_uvar();
        self.read(len as usize)
    }

    /// Read 2 bytes as unsigned integer
    fn read_u16(&mut self) -> u16 {
        self.read_u8() as u16 | ((self.read_u8() as u16) << 8)
    }

    /// Read 4 bytes as unsigned integer
    fn read_u32(&mut self) -> u32 {
        self.read_u8() as u32
            | (self.read_u8() as u32) << 8
            | (self.read_u8() as u32) << 16
            | (self.read_u8() as u32) << 24
    }

    /// Read 4 bytes as unsigned integer in big endian order.
    /// (most significant byte first)
    fn read_u32_be(&mut self) -> u32 {
        (self.read_u8() as u32) << 24
            | (self.read_u8() as u32) << 16
            | (self.read_u8() as u32) << 8
            | self.read_u8() as u32
    }

    /// Read unsigned integer with variable length.
    /// * numbers < 2^7 are stored in one byte
    /// * numbers < 2^14 are stored in two bytes
    // @todo currently, only 32 bits supported
    fn read_uvar<T: crate::number::Uint>(&mut self) -> T {
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
    fn read_ivar(&mut self) -> i64 {
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

    /// Read string of variable length.
    fn read_string(&mut self) -> &str {
        let buf = self.read_buf();
        unsafe { std::str::from_utf8_unchecked(buf) }
    }

    /// Read float32 in big endian order
    fn read_f32(&mut self) -> f32 {
        let buf = [
            self.read_u8(),
            self.read_u8(),
            self.read_u8(),
            self.read_u8(),
        ];
        f32::from_be_bytes(buf)
    }

    /// Read float64 in big endian order
    // @todo there must be a more elegant way to convert a slice to a fixed-length buffer.
    fn read_f64(&mut self) -> f64 {
        let mut bytes = init_buf();
        bytes.clone_from_slice(self.read(8));
        f64::from_be_bytes(bytes)
    }

    /// Read BigInt64 in big endian order
    fn read_i64(&mut self) -> i64 {
        let mut bytes = init_buf();
        bytes.clone_from_slice(self.read(8));
        i64::from_be_bytes(bytes)
    }

    /// read BigUInt64 in big endian order
    fn read_u64(&mut self) -> u64 {
        let mut bytes = init_buf();
        bytes.clone_from_slice(self.read(8));
        u64::from_be_bytes(bytes)
    }
}

/// Create non-zeroed fixed array of 8-bytes and returns it.
/// Since it's not zeroed it should be filled before use.
#[inline(always)]
fn init_buf() -> [u8; 8] {
    unsafe {
        let b: [MaybeUninit<u8>; 8] = MaybeUninit::uninit().assume_init();
        std::mem::transmute::<_, [u8; 8]>(b)
    }
}
