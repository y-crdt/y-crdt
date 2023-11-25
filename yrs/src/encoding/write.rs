use crate::encoding::varint::{Signed, SignedVarInt, VarInt};

impl Write for Vec<u8> {
    fn write_all(&mut self, buf: &[u8]) {
        self.extend_from_slice(buf)
    }

    fn write_u8(&mut self, value: u8) {
        self.push(value)
    }
}

pub trait Write: Sized {
    fn write_all(&mut self, buf: &[u8]);

    fn write_u8(&mut self, value: u8) {
        self.write_all(&[value])
    }

    /// Write an unsigned integer (16bit)
    fn write_u16(&mut self, num: u16) {
        self.write_all(&[num as u8, (num >> 8) as u8])
    }

    /// Write an unsigned integer (32bit)
    fn write_u32(&mut self, num: u32) {
        self.write_all(&[
            num as u8,
            (num >> 8) as u8,
            (num >> 16) as u8,
            (num >> 24) as u8,
        ])
    }

    /// Write an unsigned integer (32bit) in big endian order (most significant byte first)
    fn write_u32_be(&mut self, num: u32) {
        self.write_all(&[
            (num >> 24) as u8,
            (num >> 16) as u8,
            (num >> 8) as u8,
            num as u8,
        ])
    }

    /// Write a variable length integer or unsigned integer.
    ///
    /// We don't use zig-zag encoding because we want to keep the option open
    /// to use the same function for BigInt and 53bit integers.
    ///
    /// We use the 7th bit instead for signaling that this is a negative number.
    #[inline]
    fn write_var<T: VarInt>(&mut self, num: T) {
        num.write(self)
    }

    /// Write a variable length integer or unsigned integer.
    ///
    /// We don't use zig-zag encoding because we want to keep the option open
    /// to use the same function for BigInt and 53bit integers.
    ///
    /// We use the 7th bit instead for signaling that this is a negative number.
    #[inline]
    fn write_var_signed<T: SignedVarInt>(&mut self, num: &Signed<T>) {
        T::write_signed(num, self)
    }

    /// Write variable length buffer (binary content).
    fn write_buf<B: AsRef<[u8]>>(&mut self, buf: B) {
        let buf = buf.as_ref();
        self.write_var(buf.len());
        self.write_all(buf)
    }

    /// Write variable-length utf8 string
    #[inline]
    fn write_string(&mut self, str: &str) {
        self.write_buf(str)
    }

    /// Write floating point number in 4 bytes
    #[inline]
    fn write_f32(&mut self, num: f32) {
        self.write_all(&num.to_be_bytes())
    }

    /// Write floating point number in 8 bytes
    #[inline]
    fn write_f64(&mut self, num: f64) {
        self.write_all(&num.to_be_bytes())
    }

    /// Write BigInt in 8 bytes in big endian order.
    #[inline]
    fn write_i64(&mut self, num: i64) {
        self.write_all(&num.to_be_bytes())
    }

    /// Write BigUInt in 8 bytes in big endian order.
    #[inline]
    fn write_u64(&mut self, num: u64) {
        self.write_all(&num.to_be_bytes())
    }
}
