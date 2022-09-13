use crate::decoding::Read;
use crate::encoding::Write;
use crate::error::Error;
use std::convert::TryInto;
use std::mem::size_of;

pub const F64_MAX_SAFE_INTEGER: f64 = (i64::pow(2, 53) - 1) as f64;
pub const F64_MIN_SAFE_INTEGER: f64 = -F64_MAX_SAFE_INTEGER;

pub trait VarInt: Sized + Copy {
    fn write<W: Write>(&self, w: &mut W);
    fn read<R: Read>(r: &mut R) -> Result<Self, Error>;
}

impl VarInt for usize {
    #[inline]
    fn write<W: Write>(&self, w: &mut W) {
        write_var_u64(*self as u64, w)
    }

    #[inline]
    fn read<R: Read>(r: &mut R) -> Result<Self, Error> {
        Ok(read_var_u64(r)? as Self)
    }
}

impl VarInt for u128 {
    fn write<W: Write>(&self, w: &mut W) {
        let mut value = *self;
        while value >= 0b10000000 {
            let b = ((value & 0b01111111) as u8) | 0b10000000;
            w.write_u8(b);
            value = value >> 7;
        }

        w.write_u8((value & 0b01111111) as u8)
    }

    #[inline]
    fn read<R: Read>(r: &mut R) -> Result<Self, Error> {
        let mut num = 0u128;
        let mut len: usize = 0;
        loop {
            let r = r.read_u8()?;
            num |= u128::wrapping_shl((r & 0b01111111) as u128, len as u32);
            len += 7;
            if r < 0b10000000 {
                return Ok(num);
            }
            if len > 180 {
                return Err(Error::VarIntSizeExceeded(180));
            }
        }
    }
}

impl VarInt for u64 {
    #[inline]
    fn write<W: Write>(&self, w: &mut W) {
        write_var_u64(*self, w)
    }

    #[inline]
    fn read<R: Read>(r: &mut R) -> Result<Self, Error> {
        read_var_u64(r)
    }
}

impl VarInt for u32 {
    #[inline]
    fn write<W: Write>(&self, w: &mut W) {
        write_var_u32(*self, w)
    }

    #[inline]
    fn read<R: Read>(r: &mut R) -> Result<Self, Error> {
        read_var_u32(r)
    }
}

impl VarInt for u16 {
    #[inline]
    fn write<W: Write>(&self, w: &mut W) {
        write_var_u32(*self as u32, w)
    }

    fn read<R: Read>(r: &mut R) -> Result<Self, Error> {
        let value = read_var_u32(r)?;
        if let Ok(value) = value.try_into() {
            Ok(value)
        } else {
            Err(Error::VarIntSizeExceeded((size_of::<Self>() * 8) as u8))
        }
    }
}

impl VarInt for u8 {
    #[inline]
    fn write<W: Write>(&self, w: &mut W) {
        write_var_u32(*self as u32, w)
    }

    fn read<R: Read>(r: &mut R) -> Result<Self, Error> {
        let value = read_var_u32(r)?;
        if let Ok(value) = value.try_into() {
            Ok(value)
        } else {
            Err(Error::VarIntSizeExceeded((size_of::<Self>() * 8) as u8))
        }
    }
}

impl VarInt for isize {
    #[inline]
    fn write<W: Write>(&self, w: &mut W) {
        write_var_i64(*self as i64, w)
    }

    fn read<R: Read>(r: &mut R) -> Result<Self, Error> {
        let value = read_var_i64(r)?;
        if let Ok(value) = value.try_into() {
            Ok(value)
        } else {
            Err(Error::VarIntSizeExceeded((size_of::<Self>() * 8) as u8))
        }
    }
}

impl VarInt for i64 {
    #[inline]
    fn write<W: Write>(&self, w: &mut W) {
        write_var_i64(*self, w)
    }

    #[inline]
    fn read<R: Read>(r: &mut R) -> Result<Self, Error> {
        read_var_i64(r)
    }
}

impl VarInt for i32 {
    #[inline]
    fn write<W: Write>(&self, w: &mut W) {
        write_var_i64(*self as i64, w)
    }

    fn read<R: Read>(r: &mut R) -> Result<Self, Error> {
        let value = read_var_i64(r)?;
        if let Ok(value) = value.try_into() {
            Ok(value)
        } else {
            Err(Error::VarIntSizeExceeded((size_of::<Self>() * 8) as u8))
        }
    }
}

impl VarInt for i16 {
    #[inline]
    fn write<W: Write>(&self, w: &mut W) {
        write_var_i64(*self as i64, w)
    }

    fn read<R: Read>(r: &mut R) -> Result<Self, Error> {
        let value = read_var_i64(r)?;
        if let Ok(value) = value.try_into() {
            Ok(value)
        } else {
            Err(Error::VarIntSizeExceeded((size_of::<Self>() * 8) as u8))
        }
    }
}

impl VarInt for i8 {
    #[inline]
    fn write<W: Write>(&self, w: &mut W) {
        write_var_i64(*self as i64, w)
    }

    fn read<R: Read>(r: &mut R) -> Result<Self, Error> {
        let value = read_var_i64(r)?;
        if let Ok(value) = value.try_into() {
            Ok(value)
        } else {
            Err(Error::VarIntSizeExceeded((size_of::<Self>() * 8) as u8))
        }
    }
}

fn write_var_u32<W: Write>(mut value: u32, w: &mut W) {
    while value >= 0b10000000 {
        let b = ((value & 0b01111111) as u8) | 0b10000000;
        w.write_u8(b);
        value = value >> 7;
    }

    w.write_u8((value & 0b01111111) as u8)
}

fn write_var_u64<W: Write>(mut value: u64, w: &mut W) {
    while value >= 0b10000000 {
        let b = ((value & 0b01111111) as u8) | 0b10000000;
        w.write_u8(b);
        value = value >> 7;
    }

    w.write_u8((value & 0b01111111) as u8)
}

fn write_var_i64<W: Write>(mut value: i64, w: &mut W) {
    let is_negative = value < 0;
    value = if is_negative { -value } else { value };
    w.write_u8(
        // whether to continue reading
        (if value > 0b00111111 as i64 { 0b10000000 as u8 } else { 0 })
            // whether number is negative
            | (if is_negative { 0b01000000 as u8 } else { 0 })
            // number
            | (0b00111111 as i64 & value) as u8,
    );
    value >>= 6;
    while value > 0 {
        w.write_u8(
            if value > 0b01111111 as i64 {
                0b10000000 as u8
            } else {
                0
            } | (0b01111111 as i64 & value) as u8,
        );
        value >>= 7;
    }
}

fn read_var_u64<R: Read>(r: &mut R) -> Result<u64, Error> {
    let mut num = 0;
    let mut len: usize = 0;
    loop {
        let r = r.read_u8()?;
        num |= u64::wrapping_shl((r & 0b01111111) as u64, len as u32);
        len += 7;
        if r < 0b10000000 {
            return Ok(num);
        }
        if len > 70 {
            return Err(Error::VarIntSizeExceeded(70));
        }
    }
}

fn read_var_u32<R: Read>(r: &mut R) -> Result<u32, Error> {
    let mut num = 0;
    let mut len: usize = 0;
    loop {
        let r = r.read_u8()?;
        num |= u32::wrapping_shl((r & 0b01111111) as u32, len as u32);
        len += 7;
        if r < 0b10000000 {
            return Ok(num);
        }
        if len > 70 {
            // a proper setting for 32bit int would be 35 bits, however for Yjs compatibility
            // we allow wrap up up to 64bit ints (with int overflow wrap)
            return Err(Error::VarIntSizeExceeded(70));
        }
    }
}

fn read_var_i64<R: Read>(reader: &mut R) -> Result<i64, Error> {
    let mut r = reader.read_u8()?;
    let mut num = (r & 0b00111111 as u8) as i64;
    let mut len: u32 = 6;
    let is_negative = r & 0b01000000 as u8 > 0;
    if r & 0b10000000 as u8 == 0 {
        return Ok(if is_negative { -num } else { num });
    }
    loop {
        r = reader.read_u8()?;
        num |= (r as i64 & 0b01111111 as i64) << len;
        len += 7;
        if r < 0b10000000 as u8 {
            return Ok(if is_negative { -num } else { num });
        }
        if len > 70 {
            return Err(Error::VarIntSizeExceeded(70));
        }
    }
}

impl VarInt for f64 {
    #[inline]
    fn write<W: Write>(&self, w: &mut W) {
        write_var_f64(*self, w)
    }

    fn read<R: Read>(r: &mut R) -> Result<Self, Error> {
        read_var_f64(r)
    }
}

fn write_var_f64<W: Write>(mut value: f64, w: &mut W) {
    let is_negative = value.is_sign_negative();
    value = if is_negative { -value } else { value };
    w.write_u8(
        // whether to continue reading
        (if value as i64 > 0b00111111  { 0b10000000 as u8 } else { 0 })
            // whether number is negative
            | (if is_negative { 0b01000000 as u8 } else { 0 })
            // number
            | (0b00111111 & value as i64) as u8,
    );
    value = ((value as i64) >> 6) as f64;
    while value > 0.0 {
        w.write_u8(
            if value as i64 > 0b01111111 {
                0b10000000 as u8
            } else {
                0
            } | (0b01111111 & value as i64) as u8,
        );
        value = ((value as i64) >> 7) as f64;
    }
}

fn read_var_f64<R: Read>(reader: &mut R) -> Result<f64, Error> {
    let mut r = reader.read_u8()?;
    let mut num = (r & 0b00111111 as u8) as i64;
    let mut len: u32 = 6;
    let is_negative = r & 0b01000000 as u8 > 0;
    if r & 0b10000000 as u8 == 0 {
        return Ok(if is_negative {
            -(num as f64)
        } else {
            num as f64
        });
    }
    loop {
        r = reader.read_u8()?;
        num |= (r as i64 & 0b01111111 as i64) << len;
        len += 7;
        if r < 0b10000000 as u8 {
            return Ok(if is_negative {
                -(num as f64)
            } else {
                num as f64
            });
        }
        if len > 70 {
            return Err(Error::VarIntSizeExceeded(70));
        }
    }
}
