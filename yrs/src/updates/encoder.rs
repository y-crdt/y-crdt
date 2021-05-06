use crate::*;
use lib0::{any::Any, encoding::Encoder};

pub trait DSEncoder {
    fn rest_encoder(&mut self) -> &mut Encoder;
    fn to_buffer(self) -> Vec<u8>;
    fn reset_ds_cur_val(&mut self);
    fn write_ds_clock(&mut self, clock: u32);
    fn write_ds_len(&mut self, len: u32);
}

pub struct EncoderV1 {
    pub rest_encoder: Encoder,
}

impl EncoderV1 {
    pub fn new() -> Self {
        EncoderV1 {
            rest_encoder: Encoder::new(),
        }
    }
}

impl DSEncoder for EncoderV1 {
    fn rest_encoder(&mut self) -> &mut Encoder {
        &mut self.rest_encoder
    }
    fn to_buffer(self) -> Vec<u8> {
        self.rest_encoder.buf
    }
    fn reset_ds_cur_val(&mut self) {
        // nop
    }
    fn write_ds_clock(&mut self, clock: u32) {
        self.rest_encoder.write_uvar(clock);
    }
    fn write_ds_len(&mut self, len: u32) {
        self.rest_encoder.write_uvar(len);
    }
}

pub trait UpdateEncoder: DSEncoder {
    fn write_left_id(&mut self, id: &block::ID);
    fn write_right_id(&mut self, id: &block::ID);
    fn write_client(&mut self, client: u64);
    fn write_info(&mut self, info: u8);
    fn write_string(&mut self, s: &str);
    fn write_parent_info(&mut self, is_y_key: bool);
    fn write_type_ref(&mut self, info: u8);
    fn write_len(&mut self, len: u32);
    fn write_any(&mut self, any: &lib0::any::Any);
    fn write_buffer(&mut self, buffer: &[u8]);
    fn write_key(&mut self, string: &str);
}

impl UpdateEncoder for EncoderV1 {
    fn write_left_id(&mut self, id: &block::ID) {
        self.rest_encoder.write_uvar(id.client);
        self.rest_encoder.write_uvar(id.clock);
    }

    fn write_right_id(&mut self, id: &block::ID) {
        self.rest_encoder.write_uvar(id.client);
        self.rest_encoder.write_uvar(id.clock);
    }

    fn write_client(&mut self, client: u64) {
        self.rest_encoder.write_uvar(client);
    }

    fn write_info(&mut self, info: u8) {
        self.rest_encoder.write_u8(info);
    }

    fn write_string(&mut self, s: &str) {
        self.rest_encoder.write_string(s);
    }

    fn write_parent_info(&mut self, is_y_key: bool) {
        self.rest_encoder
            .write_uvar(if is_y_key { 1 as u32 } else { 0 as u32 });
    }

    fn write_type_ref(&mut self, info: u8) {
        // In Yjs we use read_var_uint but use only 7 bit. So this is equivalent.
        self.rest_encoder.write_u8(info);
    }

    fn write_len(&mut self, len: u32) {
        self.rest_encoder.write_uvar(len);
    }

    fn write_any(&mut self, any: &Any) {
        any.encode(&mut self.rest_encoder);
    }

    fn write_buffer(&mut self, buffer: &[u8]) {
        self.rest_encoder.write_buf(buffer);
    }

    fn write_key(&mut self, string: &str) {
        self.rest_encoder.write_string(string);
    }
}
