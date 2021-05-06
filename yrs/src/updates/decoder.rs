use crate::types::TypeRefs;
use crate::*;
use lib0::{any::Any, decoding::Decoder};
use std::convert::TryInto;

pub trait DSDecoder {
    fn reset_ds_cur_val(&mut self);
    fn read_ds_clock(&mut self) -> u32;
    fn read_ds_len(&mut self) -> u32;
}

pub struct DecoderV1<'a> {
    pub rest_decoder: &'a mut Decoder<'a>,
}

impl<'a> DecoderV1<'a> {
    pub fn new(decoder: &'a mut Decoder<'a>) -> Self {
        DecoderV1 {
            rest_decoder: decoder,
        }
    }
}

impl<'a> DSDecoder for DecoderV1<'a> {
    fn reset_ds_cur_val(&mut self) {
        todo!()
    }

    fn read_ds_clock(&mut self) -> u32 {
        todo!()
    }

    fn read_ds_len(&mut self) -> u32 {
        todo!()
    }
}

pub trait UpdateDecoder: DSDecoder {
    fn read_left_id(&mut self) -> block::ID;
    fn read_right_id(&mut self) -> block::ID;
    fn read_client(&mut self) -> u64;
    fn read_info(&mut self) -> u8;
    fn read_string(&mut self) -> &str;
    fn read_parent_info(&mut self) -> bool;
    fn read_type_ref(&mut self) -> types::TypeRefs;
    fn read_len(&mut self) -> u32;
    fn read_any(&mut self) -> lib0::any::Any;
    fn read_buffer(&mut self) -> &[u8];
    fn read_key(&mut self) -> &str;
}

impl<'a> UpdateDecoder for DecoderV1<'a> {
    fn read_left_id(&mut self) -> block::ID {
        block::ID {
            client: self.rest_decoder.read_uvar(),
            clock: self.rest_decoder.read_uvar(),
        }
    }

    fn read_right_id(&mut self) -> block::ID {
        block::ID {
            client: self.rest_decoder.read_uvar(),
            clock: self.rest_decoder.read_uvar(),
        }
    }

    fn read_client(&mut self) -> u64 {
        self.rest_decoder.read_uvar()
    }

    fn read_info(&mut self) -> u8 {
        self.rest_decoder.read_u8()
    }

    fn read_string(&mut self) -> &str {
        self.rest_decoder.read_string()
    }

    fn read_parent_info(&mut self) -> bool {
        let info: u32 = self.rest_decoder.read_uvar();
        info == 1
    }

    fn read_type_ref(&mut self) -> TypeRefs {
        // In Yjs we use read_var_uint but use only 7 bit. So this is equivalent.
        let r = self.rest_decoder.read_u8();
        r.try_into().unwrap()
    }

    fn read_len(&mut self) -> u32 {
        self.rest_decoder.read_uvar()
    }

    fn read_any(&mut self) -> Any {
        Any::decode(&mut self.rest_decoder)
    }

    fn read_buffer(&mut self) -> &[u8] {
        self.rest_decoder.read_var_buffer()
    }

    fn read_key(&mut self) -> &str {
        self.rest_decoder.read_string()
    }
}
