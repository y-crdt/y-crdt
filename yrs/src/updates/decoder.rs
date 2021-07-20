use crate::*;
use lib0::decoding::Read;
use lib0::{any::Any, decoding::Cursor};

pub trait Decode: Sized {
    fn decode<D: Decoder>(decoder: &mut D) -> Self;

    /// Helper function for decoding
    fn decode_v1(data: &[u8]) -> Self {
        let mut decoder = DecoderV1::from(data);
        Self::decode(&mut decoder)
    }
}

pub trait Decoder: Read {
    fn reset_ds_cur_val(&mut self);
    fn read_ds_clock(&mut self) -> u32;
    fn read_ds_len(&mut self) -> u32;
    fn read_left_id(&mut self) -> block::ID;
    fn read_right_id(&mut self) -> block::ID;
    fn read_client(&mut self) -> u64;
    fn read_info(&mut self) -> u8;
    fn read_parent_info(&mut self) -> bool;
    fn read_type_ref(&mut self) -> types::TypeRefs;
    fn read_len(&mut self) -> u32;
    fn read_any(&mut self) -> lib0::any::Any;
    fn read_key(&mut self) -> &str;
    fn read_to_end(&mut self) -> &[u8];
}

pub struct DecoderV1<'a> {
    cursor: Cursor<'a>,
}

impl<'a> DecoderV1<'a> {
    pub fn new(cursor: Cursor<'a>) -> Self {
        DecoderV1 { cursor }
    }

    fn read_id(&mut self) -> block::ID {
        ID::new(self.read_uvar(), self.read_uvar())
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
    fn read_u8(&mut self) -> u8 {
        self.cursor.read_u8()
    }

    fn read(&mut self, len: usize) -> &[u8] {
        self.cursor.read(len)
    }
}

impl<'a> Decoder for DecoderV1<'a> {
    fn reset_ds_cur_val(&mut self) {
        /* no op */
    }

    fn read_ds_clock(&mut self) -> u32 {
        self.read_uvar()
    }

    fn read_ds_len(&mut self) -> u32 {
        self.read_uvar()
    }

    fn read_left_id(&mut self) -> ID {
        self.read_id()
    }

    fn read_right_id(&mut self) -> ID {
        self.read_id()
    }

    fn read_client(&mut self) -> u64 {
        self.cursor.read_uvar()
    }

    fn read_info(&mut self) -> u8 {
        self.cursor.read_u8()
    }

    fn read_parent_info(&mut self) -> bool {
        let info: u32 = self.cursor.read_uvar();
        info == 1
    }

    fn read_type_ref(&mut self) -> u8 {
        // In Yjs we use read_var_uint but use only 7 bit. So this is equivalent.
        self.cursor.read_u8()
    }

    fn read_len(&mut self) -> u32 {
        self.read_uvar()
    }

    fn read_any(&mut self) -> Any {
        Any::decode(self)
    }

    fn read_key(&mut self) -> &str {
        self.read_string()
    }

    fn read_to_end(&mut self) -> &[u8] {
        &self.cursor.buf[self.cursor.next..]
    }
}
