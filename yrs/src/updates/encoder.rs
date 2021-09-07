use crate::*;
use lib0::any::Any;
use lib0::encoding::Write;

/// A trait that can be implemented by any other type in order to support lib0 encoding capability.
pub trait Encode {
    fn encode<E: Encoder>(&self, encoder: &mut E);

    /// Helper function for encoding 1st version of lib0 encoding.
    fn encode_v1(&self) -> Vec<u8> {
        let mut encoder = EncoderV1::new();
        self.encode(&mut encoder);
        encoder.to_vec()
    }
}

/// Trait used by lib0 encoders. Natively lib0 encoding supports two versions:
///
/// 1. 1st version (implemented in Yrs) uses simple optimization techniques like var int encoding.
/// 2. 2nd version optimizes bigger batches of blocks by using run-length encoding.
///
/// Both of these define a common set of operations defined in this trait.
pub trait Encoder: Write {
    /// Consume current encoder and return a binary with all data encoded so far.
    fn to_vec(self) -> Vec<u8>;

    /// Reset the state of currently encoded [DeleteSet].
    fn reset_ds_cur_val(&mut self);

    /// Write a clock value of currently encoded [DeleteSet] client.
    fn write_ds_clock(&mut self, clock: u32);

    /// Write a number of client entries used by currently encoded [DeleteSet].
    fn write_ds_len(&mut self, len: u32);

    /// Write unique identifier of a currently encoded [Block]'s left origin.
    fn write_left_id(&mut self, id: &block::ID);

    /// Write unique identifier of a currently encoded [Block]'s right origin.
    fn write_right_id(&mut self, id: &block::ID);

    /// Write currently encoded client identifier.
    fn write_client(&mut self, client: u64);

    /// Write currently encoded [Block]'s info flags. These contain information about which fields
    /// have been provided and which should be skipped during decoding process as well as a type of
    /// block currently encoded.
    fn write_info(&mut self, info: u8);

    /// Write info flag about currently encoded [Block]'s parent. Is is another block or root type.
    fn write_parent_info(&mut self, is_y_key: bool);

    /// Writes type ref data of currently encoded [Block]'s parent.
    fn write_type_ref(&mut self, info: u8);

    /// Write length parameter.
    fn write_len(&mut self, len: u32);

    /// Encode JSON-like data type. This is a complex structure which is an extension to JavaScript
    /// Object Notation with some extra cases.
    fn write_any(&mut self, any: &lib0::any::Any);

    /// Write a string key.
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
