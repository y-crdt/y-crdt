use crate::block::ClientID;
use crate::encoding::read::{Cursor, Error, Read};
use crate::*;
use std::sync::Arc;

/// A trait that can be implemented by any other type in order to support lib0 decoding capability.
pub trait Decode: Sized {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error>;

    /// Helper function for decoding 1st version of lib0 encoding.
    fn decode_v1(data: &[u8]) -> Result<Self, Error> {
        let mut decoder = DecoderV1::from(data);
        Self::decode(&mut decoder)
    }

    /// Helper function for decoding 2nd version of lib0 encoding.
    fn decode_v2(data: &[u8]) -> Result<Self, Error> {
        let mut decoder = DecoderV2::new(Cursor::new(data))?;
        Self::decode(&mut decoder)
    }
}

/// Trait used by lib0 decoders. Natively lib0 encoding supports two versions:
///
/// 1. 1st version (implemented in Yrs) uses simple optimization techniques like var int encoding.
/// 2. 2nd version optimizes bigger batches of blocks by using run-length encoding.
///
/// Both of these define a common set of operations defined in this trait.  
pub trait Decoder: Read {
    /// Reset the value of current delete set state.
    fn reset_ds_cur_val(&mut self);

    /// Read next [DeleteSet] clock value.
    fn read_ds_clock(&mut self) -> Result<u32, Error>;

    /// Read the number of clients stored in encoded [DeleteSet].
    fn read_ds_len(&mut self) -> Result<u32, Error>;

    /// Read left origin of a currently decoded [Block].
    fn read_left_id(&mut self) -> Result<ID, Error>;

    /// Read right origin of a currently decoded [Block].
    fn read_right_id(&mut self) -> Result<ID, Error>;

    /// Read currently decoded client identifier.
    fn read_client(&mut self) -> Result<ClientID, Error>;

    /// Read info bit flags of a currently decoded [Block].
    fn read_info(&mut self) -> Result<u8, Error>;

    /// Read bit flags determining type of parent of a currently decoded [Block].
    fn read_parent_info(&mut self) -> Result<bool, Error>;

    /// Read type ref info of a currently decoded [Block] parent.
    fn read_type_ref(&mut self) -> Result<u8, Error>;

    /// Read length parameter.
    fn read_len(&mut self) -> Result<u32, Error>;

    /// Decode a JSON-like data type. It's a complex type which is an extension of native JavaScript
    /// Object Notation.
    fn read_any(&mut self) -> Result<Any, Error>;

    /// Decode an embedded JSON string into [Any] struct. It's a complex type which is an extension
    /// of native JavaScript Object Notation.
    fn read_json(&mut self) -> Result<Any, Error>;

    /// Read key string.
    fn read_key(&mut self) -> Result<Arc<str>, Error>;

    /// Consume a rest of the decoded buffer data and return it without parsing.
    fn read_to_end(&mut self) -> Result<&[u8], Error>;
}

/// Version 1 of lib0 decoder.
pub struct DecoderV1<'a> {
    cursor: Cursor<'a>,
}

impl<'a> DecoderV1<'a> {
    pub fn new(cursor: Cursor<'a>) -> Self {
        DecoderV1 { cursor }
    }

    fn read_id(&mut self) -> Result<ID, Error> {
        #[cfg(not(feature = "small-client"))]
        let client: u64 = self.read_var()?;
        #[cfg(feature = "small-client")]
        let client: u32 = self.read_var()?;
        let clock = self.read_var()?;
        Ok(ID::new(ClientID::new(client as u64), clock))
    }
}

impl<'a> From<Cursor<'a>> for DecoderV1<'a> {
    fn from(cursor: Cursor<'a>) -> Self {
        Self::new(cursor)
    }
}

impl<'a> From<&'a [u8]> for DecoderV1<'a> {
    fn from(buf: &'a [u8]) -> Self {
        Self::new(Cursor::new(buf))
    }
}

impl<'a> Read for DecoderV1<'a> {
    #[inline]
    fn read_u8(&mut self) -> Result<u8, Error> {
        self.cursor.read_u8()
    }

    #[inline]
    fn read_exact(&mut self, len: usize) -> Result<&[u8], Error> {
        self.cursor.read_exact(len)
    }
}

impl<'a> Decoder for DecoderV1<'a> {
    #[inline]
    fn reset_ds_cur_val(&mut self) {
        /* no op */
    }

    #[inline]
    fn read_ds_clock(&mut self) -> Result<u32, Error> {
        self.read_var()
    }

    #[inline]
    fn read_ds_len(&mut self) -> Result<u32, Error> {
        self.read_var()
    }

    #[inline]
    fn read_left_id(&mut self) -> Result<ID, Error> {
        self.read_id()
    }

    #[inline]
    fn read_right_id(&mut self) -> Result<ID, Error> {
        self.read_id()
    }

    #[inline]
    fn read_client(&mut self) -> Result<ClientID, Error> {
        #[cfg(not(feature = "small-client"))]
        let client: u64 = self.cursor.read_var()?;
        #[cfg(feature = "small-client")]
        let client: u32 = self.cursor.read_var()?;
        Ok(ClientID::new(client as u64))
    }

    #[inline]
    fn read_info(&mut self) -> Result<u8, Error> {
        self.cursor.read_u8()
    }

    #[inline]
    fn read_parent_info(&mut self) -> Result<bool, Error> {
        let info: u32 = self.cursor.read_var()?;
        Ok(info == 1)
    }

    #[inline]
    fn read_type_ref(&mut self) -> Result<u8, Error> {
        // In Yjs we use read_var_uint but use only 7 bit. So this is equivalent.
        self.cursor.read_u8()
    }

    #[inline]
    fn read_len(&mut self) -> Result<u32, Error> {
        self.read_var()
    }

    #[inline]
    fn read_any(&mut self) -> Result<Any, Error> {
        Any::decode(self)
    }

    fn read_json(&mut self) -> Result<Any, Error> {
        let src = self.read_string()?;
        Any::from_json(src)
    }

    #[inline]
    fn read_key(&mut self) -> Result<Arc<str>, Error> {
        let str: Arc<str> = self.read_string()?.into();
        Ok(str)
    }

    #[inline]
    fn read_to_end(&mut self) -> Result<&[u8], Error> {
        Ok(&self.cursor.buf[self.cursor.next..])
    }
}

/// Version 2 of lib0 decoder.
pub struct DecoderV2<'a> {
    cursor: Cursor<'a>,
    keys: Vec<Arc<str>>,
    ds_curr_val: u32,
    key_clock_decoder: IntDiffOptRleDecoder<'a>,
    client_decoder: UIntOptRleDecoder<'a>,
    left_clock_decoder: IntDiffOptRleDecoder<'a>,
    right_clock_decoder: IntDiffOptRleDecoder<'a>,
    info_decoder: RleDecoder<'a>,
    string_decoder: StringDecoder<'a>,
    parent_info_decoder: RleDecoder<'a>,
    type_ref_decoder: UIntOptRleDecoder<'a>,
    len_decoder: UIntOptRleDecoder<'a>,
}

impl<'a> DecoderV2<'a> {
    pub fn new(mut cursor: Cursor<'a>) -> Result<Self, Error> {
        if cursor.has_content() {
            // read feature flag - currently unused
            let _: u8 = cursor.read_u8()?;
        }
        let mut idx = cursor.next;
        let buf = cursor.buf;

        let key_clock_buf = Self::read_buf(buf, &mut idx)?;
        let client_buf = Self::read_buf(buf, &mut idx)?;
        let left_clock_buf = Self::read_buf(buf, &mut idx)?;
        let right_clock_buf = Self::read_buf(buf, &mut idx)?;
        let info_buf = Self::read_buf(buf, &mut idx)?;
        let string_buf = Self::read_buf(buf, &mut idx)?;
        let parent_info_buf = Self::read_buf(buf, &mut idx)?;
        let type_ref_buf = Self::read_buf(buf, &mut idx)?;
        let len_buf = Self::read_buf(buf, &mut idx)?;
        let cursor = Cursor {
            buf: &buf[idx..],
            next: 0,
        };
        Ok(DecoderV2 {
            cursor,
            ds_curr_val: 0,
            keys: Vec::new(),
            key_clock_decoder: IntDiffOptRleDecoder::new(Cursor::new(key_clock_buf)),
            client_decoder: UIntOptRleDecoder::new(Cursor::new(client_buf)),
            left_clock_decoder: IntDiffOptRleDecoder::new(Cursor::new(left_clock_buf)),
            right_clock_decoder: IntDiffOptRleDecoder::new(Cursor::new(right_clock_buf)),
            info_decoder: RleDecoder::new(Cursor::new(info_buf)),
            string_decoder: StringDecoder::new(Cursor::new(string_buf))?,
            parent_info_decoder: RleDecoder::new(Cursor::new(parent_info_buf)),
            type_ref_decoder: UIntOptRleDecoder::new(Cursor::new(type_ref_buf)),
            len_decoder: UIntOptRleDecoder::new(Cursor::new(len_buf)),
        })
    }

    fn read_usize(buf: &[u8], idx: &mut usize) -> Result<usize, Error> {
        let mut num: usize = 0;
        let mut len: usize = 0;
        loop {
            // buffer can end in the middle of a varint, so every byte must be bounds-checked
            let r = *buf.get(*idx).ok_or(Error::InvalidVarInt)?;
            *idx += 1;
            if len >= usize::BITS as usize {
                // number cannot be represented as usize
                return Err(Error::InvalidVarInt);
            }
            num |= (r as usize & 127) << len;
            len += 7;
            if r < 128 {
                return Ok(num);
            }
        }
    }

    fn read_buf(buf: &'a [u8], idx: &mut usize) -> Result<&'a [u8], Error> {
        let len = Self::read_usize(buf, idx)?;
        let start = *idx;
        match start.checked_add(len) {
            Some(end) if end <= buf.len() => {
                *idx = end;
                Ok(&buf[start..end])
            }
            _ => Err(Error::EndOfBuffer(len)),
        }
    }
}

impl<'a> Read for DecoderV2<'a> {
    #[inline]
    fn read_exact(&mut self, len: usize) -> Result<&[u8], Error> {
        self.cursor.read_exact(len)
    }

    #[inline]
    fn read_u8(&mut self) -> Result<u8, Error> {
        self.cursor.read_u8()
    }

    #[inline]
    fn read_string(&mut self) -> Result<&str, Error> {
        self.string_decoder.read_str()
    }
}

impl<'a> Decoder for DecoderV2<'a> {
    fn reset_ds_cur_val(&mut self) {
        self.ds_curr_val = 0;
    }

    fn read_ds_clock(&mut self) -> Result<u32, Error> {
        self.ds_curr_val += self.cursor.read_var::<u32>()?;
        Ok(self.ds_curr_val)
    }

    fn read_ds_len(&mut self) -> Result<u32, Error> {
        let diff = self.cursor.read_var::<u32>()? + 1;
        self.ds_curr_val += diff;
        Ok(diff)
    }

    fn read_left_id(&mut self) -> Result<ID, Error> {
        Ok(ID::new(
            ClientID::new(self.client_decoder.read_u64()?),
            self.left_clock_decoder.read_u32()?,
        ))
    }

    fn read_right_id(&mut self) -> Result<ID, Error> {
        Ok(ID::new(
            ClientID::new(self.client_decoder.read_u64()?),
            self.right_clock_decoder.read_u32()?,
        ))
    }

    fn read_client(&mut self) -> Result<ClientID, Error> {
        Ok(ClientID::new(self.client_decoder.read_u64()?))
    }

    fn read_info(&mut self) -> Result<u8, Error> {
        self.info_decoder.read_u8()
    }

    fn read_parent_info(&mut self) -> Result<bool, Error> {
        Ok(self.parent_info_decoder.read_u8()? == 1)
    }

    fn read_type_ref(&mut self) -> Result<u8, Error> {
        Ok(self.type_ref_decoder.read_u64()? as u8)
    }

    fn read_len(&mut self) -> Result<u32, Error> {
        Ok(self.len_decoder.read_u64()? as u32)
    }

    fn read_any(&mut self) -> Result<Any, Error> {
        Any::decode(&mut self.cursor)
    }

    fn read_json(&mut self) -> Result<Any, Error> {
        Any::decode(&mut self.cursor)
    }

    fn read_key(&mut self) -> Result<Arc<str>, Error> {
        let key_clock = self.key_clock_decoder.read_u32()?;
        if let Some(key) = self.keys.get(key_clock as usize) {
            Ok(key.clone())
        } else {
            let key: Arc<str> = self.string_decoder.read_str()?.into();
            self.keys.push(key.clone());
            Ok(key)
        }
    }

    #[inline]
    fn read_to_end(&mut self) -> Result<&[u8], Error> {
        Ok(&self.cursor.buf[self.cursor.next..])
    }
}

struct IntDiffOptRleDecoder<'a> {
    cursor: Cursor<'a>,
    last: u32,
    count: u32,
    diff: i32,
}

impl<'a> IntDiffOptRleDecoder<'a> {
    fn new(cursor: Cursor<'a>) -> Self {
        IntDiffOptRleDecoder {
            cursor,
            last: 0,
            count: 0,
            diff: 0,
        }
    }

    fn read_u32(&mut self) -> Result<u32, Error> {
        if self.count == 0 {
            let diff = self.cursor.read_var::<i32>()?;
            // if the first bit is set, we read more data
            let has_count = diff & 1;
            self.diff = (diff >> 1) as i32;
            self.count = if has_count != 0 {
                self.cursor.read_var::<u32>()? + 2
            } else {
                1
            };
        }
        self.last = ((self.last as i32) + self.diff) as u32;
        self.count -= 1;
        Ok(self.last)
    }
}

struct UIntOptRleDecoder<'a> {
    cursor: Cursor<'a>,
    last: u64,
    count: u32,
}

impl<'a> UIntOptRleDecoder<'a> {
    fn new(cursor: Cursor<'a>) -> Self {
        UIntOptRleDecoder {
            cursor,
            last: 0,
            count: 0,
        }
    }

    fn read_u64(&mut self) -> Result<u64, Error> {
        if self.count == 0 {
            let s = self.cursor.read_var_signed::<i64>()?;
            // if the sign is negative, we read the count too, otherwise count is 1
            let is_negative = s.is_negative();
            if is_negative {
                self.count = self.cursor.read_var::<u32>()? + 2;
                self.last = (-s.value()) as u64;
            } else {
                self.count = 1;
                self.last = s.value() as u64;
            }
        }
        self.count -= 1;
        Ok(self.last)
    }
}

struct RleDecoder<'a> {
    cursor: Cursor<'a>,
    last: u8,
    count: i32,
}

impl<'a> RleDecoder<'a> {
    fn new(cursor: Cursor<'a>) -> Self {
        RleDecoder {
            cursor,
            last: 0,
            count: 0,
        }
    }

    fn read_u8(&mut self) -> Result<u8, Error> {
        if self.count == 0 {
            self.last = self.cursor.read_u8()?;
            if self.cursor.has_content() {
                self.count = (self.cursor.read_var::<u32>()? as i32) + 1; // see encoder implementation for the reason why this is incremented
            } else {
                self.count = -1; // read the current value forever
            }
        }
        self.count -= 1;
        Ok(self.last)
    }
}

struct StringDecoder<'a> {
    buf: &'a str,
    len_decoder: UIntOptRleDecoder<'a>,
    pos: usize,
}

impl<'a> StringDecoder<'a> {
    fn new(cursor: Cursor<'a>) -> Result<Self, Error> {
        let buf = cursor.buf;
        let mut next = cursor.next;
        let str_bin = DecoderV2::read_buf(buf, &mut next)?;
        // Wire bytes must be valid UTF-8.
        let str = std::str::from_utf8(str_bin).map_err(|_| Error::UnexpectedValue)?;
        let len_decoder = UIntOptRleDecoder::new(Cursor { buf, next });
        Ok(StringDecoder {
            pos: 0,
            buf: str,
            len_decoder,
        })
    }

    fn read_str(&mut self) -> Result<&'a str, Error> {
        let mut remaining = self.len_decoder.read_u64()? as usize;
        let mut i = 0;
        let start = &self.buf[self.pos..];
        for c in start.chars() {
            if remaining == 0 {
                break;
            }
            i += c.len_utf8(); // rust uses offsets as utf-8 bytes
            remaining -= c.len_utf16(); // but yjs provides them as utf-16
        }
        let result = &start[..i];
        self.pos += i;
        Ok(result)
    }
}

#[cfg(test)]
mod test {
    use crate::updates::decoder::{Decode, DecoderV2};
    use crate::Update;

    #[test]
    fn decode_v2_unterminated_varint_returns_error() {
        // first byte is the feature flag, second one is a varint continuation byte with no
        // following data
        assert!(Update::decode_v2(&[0x80, 0xFF]).is_err());
        assert!(Update::decode_v2(&[0x00, 0xFF]).is_err());
        assert!(Update::decode_v2(&[0x00, 0x80]).is_err());
        assert!(Update::decode_v2(&[0x00, 0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn decode_v2_long_varint_returns_error() {
        // continuation bytes which never terminate, or terminate after exceeding usize bounds
        for len in 1..40 {
            let mut buf = vec![0x00];
            buf.extend(std::iter::repeat(0xFF).take(len));
            assert!(
                Update::decode_v2(&buf).is_err(),
                "unterminated, {len} bytes"
            );

            buf.push(0x01);
            assert!(Update::decode_v2(&buf).is_err(), "terminated, {len} bytes");
        }
    }

    #[test]
    fn decoder_v2_read_usize_bounds() {
        let read = |buf: &[u8]| {
            let mut idx = 0;
            DecoderV2::read_usize(buf, &mut idx)
        };
        assert!(read(&[]).is_err());
        assert!(read(&[0xFF]).is_err());
        assert!(read(&[0xFF, 0xFF]).is_err());
        assert_eq!(read(&[0x00]).unwrap(), 0);
        assert_eq!(read(&[0x7F]).unwrap(), 127);
        assert_eq!(read(&[0x80, 0x01]).unwrap(), 128);

        // largest value representable in 64 bits takes 10 bytes
        let max = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
        if usize::BITS == 64 {
            assert_eq!(read(&max).unwrap(), usize::MAX);
        }
        // an 11-byte varint can never fit into a usize
        let too_long = [
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01,
        ];
        assert!(read(&too_long).is_err());
    }

    #[test]
    fn decoder_v2_read_buf_length_overflow() {
        // declared length is close to usize::MAX, so `start + len` must not overflow
        let mut buf = vec![0xFF; 9];
        buf.push(0x01);
        buf.push(0x00);
        let mut idx = 0;
        assert!(DecoderV2::read_buf(&buf, &mut idx).is_err());
    }

    #[test]
    fn decode_v2_malformed_input_never_panics() {
        // exhaustive over all 1 and 2-byte inputs, for both encoding versions
        for a in 0..=u8::MAX {
            let _ = Update::decode_v1(&[a]);
            let _ = Update::decode_v2(&[a]);
            for b in 0..=u8::MAX {
                let _ = Update::decode_v1(&[a, b]);
                let _ = Update::decode_v2(&[a, b]);
            }
        }

        // deterministic pseudo-random short inputs (xorshift, fixed seed)
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..5_000 {
            let len = (next() % 24) as usize;
            let buf: Vec<u8> = (0..len)
                .map(|_| match next() % 4 {
                    // bias towards varint continuation bytes, the interesting case
                    0 => 0xFF,
                    1 => 0x80 | (next() as u8 & 0x7F),
                    _ => next() as u8,
                })
                .collect();
            let _ = Update::decode_v2(&buf);
        }
    }
}
