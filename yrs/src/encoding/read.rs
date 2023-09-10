use thiserror::Error;
use crate::encoding::varint::{Signed, SignedVarInt, VarInt};

#[derive(Error, Debug)]
pub enum Error {
    #[error("decoded variable integer size was outside of expected bounds of {0} bits")]
    VarIntSizeExceeded(u8),

    #[error("while trying to read more data (expected: {0} bytes), an unexpected end of buffer was reached")]
    EndOfBuffer(usize),

    #[error("while reading, an unexpected value was found")]
    UnexpectedValue,

    #[error("JSON parsing error: {0}")]
    InvalidJSON(#[from] serde_json::Error),
}

#[derive(Default)]
pub struct Cursor<'a> {
    pub buf: &'a [u8],
    pub next: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(buf: &'a [u8]) -> Cursor<'a> {
        Cursor { buf, next: 0 }
    }

    pub fn has_content(&self) -> bool {
        self.next != self.buf.len()
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
    /// Take a slice of the next `len` bytes and advance the position by `len`.
    fn read_exact(&mut self, len: usize) -> Result<&[u8], Error> {
        if self.next + len > self.buf.len() {
            Err(Error::EndOfBuffer(len))
        } else {
            let slice = &self.buf[self.next..(self.next + len)];
            self.next += len as usize;
            Ok(slice)
        }
    }

    /// Read a single byte.
    fn read_u8(&mut self) -> Result<u8, Error> {
        if let Some(&b) = self.buf.get(self.next) {
            self.next += 1;
            Ok(b)
        } else {
            Err(Error::EndOfBuffer(1))
        }
    }
}

pub trait Read: Sized {
    fn read_exact(&mut self, len: usize) -> Result<&[u8], Error>;

    /// Read a single byte.
    fn read_u8(&mut self) -> Result<u8, Error> {
        let buf = self.read_exact(1)?;
        Ok(buf[0])
    }

    /// Read a variable length buffer.
    fn read_buf(&mut self) -> Result<&[u8], Error> {
        let len: u32 = self.read_var()?;
        self.read_exact(len as usize)
    }

    /// Read 2 bytes as unsigned integer
    fn read_u16(&mut self) -> Result<u16, Error> {
        let buf = self.read_exact(2)?;
        Ok(buf[0] as u16 | ((buf[1] as u16) << 8))
    }

    /// Read 4 bytes as unsigned integer
    fn read_u32(&mut self) -> Result<u32, Error> {
        let buf = self.read_exact(4)?;
        Ok(buf[0] as u32 | (buf[1] as u32) << 8 | (buf[2] as u32) << 16 | (buf[3] as u32) << 24)
    }

    /// Read 4 bytes as unsigned integer in big endian order.
    /// (most significant byte first)
    fn read_u32_be(&mut self) -> Result<u32, Error> {
        let buf = self.read_exact(4)?;
        Ok((buf[0] as u32) << 24 | (buf[1] as u32) << 16 | (buf[2] as u32) << 8 | buf[3] as u32)
    }

    /// Read unsigned integer with variable length.
    /// * numbers < 2^7 are stored in one byte
    /// * numbers < 2^14 are stored in two bytes
    #[inline]
    fn read_var<T: VarInt>(&mut self) -> Result<T, Error> {
        T::read(self)
    }

    /// Read unsigned integer with variable length.
    /// * numbers < 2^7 are stored in one byte
    /// * numbers < 2^14 are stored in two bytes
    #[inline]
    fn read_var_signed<T: SignedVarInt>(&mut self) -> Result<Signed<T>, Error> {
        T::read_signed(self)
    }

    /// Read string of variable length.
    fn read_string(&mut self) -> Result<&str, Error> {
        let buf = self.read_buf()?;
        Ok(unsafe { std::str::from_utf8_unchecked(buf) })
    }

    /// Read float32 in big endian order
    fn read_f32(&mut self) -> Result<f32, Error> {
        let mut buf = [0; 4];
        let slice = self.read_exact(4)?;
        buf.copy_from_slice(slice);
        Ok(f32::from_be_bytes(buf))
    }

    /// Read float64 in big endian order
    // @todo there must be a more elegant way to convert a slice to a fixed-length buffer.
    fn read_f64(&mut self) -> Result<f64, Error> {
        let mut buf = [0; 8];
        let slice = self.read_exact(8)?;
        buf.copy_from_slice(slice);
        Ok(f64::from_be_bytes(buf))
    }

    /// Read BigInt64 in big endian order
    fn read_i64(&mut self) -> Result<i64, Error> {
        let mut buf = [0; 8];
        let slice = self.read_exact(8)?;
        buf.copy_from_slice(slice);
        Ok(i64::from_be_bytes(buf))
    }

    /// read BigUInt64 in big endian order
    fn read_u64(&mut self) -> Result<u64, Error> {
        let mut buf = [0; 8];
        let slice = self.read_exact(8)?;
        buf.copy_from_slice(slice);
        Ok(u64::from_be_bytes(buf))
    }
}
