use crate::*;
use lib0::any::Any;
use lib0::encoding::Write;

pub trait Encode {
    fn encode<E: Encoder>(&self, encoder: &mut E);

    fn encode_v1(&self) -> Vec<u8> {
        let mut encoder = EncoderV1::new();
        self.encode(&mut encoder);
        encoder.to_vec()
    }
}

pub trait Encoder: Write {
    fn to_vec(self) -> Vec<u8>;
    fn reset_ds_cur_val(&mut self);
    fn write_ds_clock(&mut self, clock: u32);
    fn write_ds_len(&mut self, len: u32);
    fn write_left_id(&mut self, id: &block::ID);
    fn write_right_id(&mut self, id: &block::ID);
    fn write_client(&mut self, client: u64);
    fn write_info(&mut self, info: u8);
    fn write_parent_info(&mut self, is_y_key: bool);
    fn write_type_ref(&mut self, info: u8);
    fn write_len(&mut self, len: u32);
    fn write_any(&mut self, any: &lib0::any::Any);
    fn write_key(&mut self, string: &str);
}

pub struct EncoderV1 {
    buf: Vec<u8>,
}

impl EncoderV1 {
    pub fn new() -> Self {
        EncoderV1 {
            buf: Vec::with_capacity(1024),
        }
    }

    fn write_id(&mut self, id: &ID) {
        self.write_uvar(id.client);
        self.write_uvar(id.clock);
    }
}

impl Write for EncoderV1 {
    fn write_u8(&mut self, value: u8) {
        self.buf.write_u8(value)
    }

    fn write(&mut self, buf: &[u8]) {
        self.buf.write(buf)
    }
}

impl Encoder for EncoderV1 {
    fn to_vec(self) -> Vec<u8> {
        self.buf
    }

    fn reset_ds_cur_val(&mut self) {
        /* no op */
    }

    fn write_ds_clock(&mut self, clock: u32) {
        self.write_uvar(clock)
    }

    fn write_ds_len(&mut self, len: u32) {
        self.write_uvar(len)
    }

    fn write_left_id(&mut self, id: &ID) {
        self.write_id(id)
    }

    fn write_right_id(&mut self, id: &ID) {
        self.write_id(id)
    }

    fn write_client(&mut self, client: u64) {
        self.write_uvar(client)
    }

    fn write_info(&mut self, info: u8) {
        self.write_u8(info)
    }

    fn write_parent_info(&mut self, is_y_key: bool) {
        self.write_uvar(if is_y_key { 1 as u32 } else { 0 as u32 })
    }

    fn write_type_ref(&mut self, info: u8) {
        self.write_u8(info)
    }

    fn write_len(&mut self, len: u32) {
        self.write_uvar(len)
    }

    fn write_any(&mut self, any: &Any) {
        any.encode(self)
    }

    fn write_key(&mut self, key: &str) {
        self.write_string(key)
    }
}
