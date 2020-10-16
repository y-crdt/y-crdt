use crate::*;

pub const BIT8: u8 = 0b10000000;
pub const BIT7: u8 = 0b01000000;
// pub const BIT6: u8 = 0b00100000;
// pub const BIT5: u8 = 0b00010000;
// pub const BIT4: u8 = 0b00001000;
// pub const BIT3: u8 = 0b00000100;
// pub const BIT2: u8 = 0b00000010;
// pub const BIT1: u8 = 0b00000001;

// pub const BITS8: u8 = 0b11111111;
// pub const BITS7: u8 = 0b01111111;
// pub const BITS6: u8 = 0b00111111;
// pub const BITS5: u8 = 0b00011111;
// pub const BITS4: u8 = 0b00001111;
// pub const BITS3: u8 = 0b00000111;
// pub const BITS2: u8 = 0b00000011;
// pub const BITS1: u8 = 0b00000001;

pub struct Encoder {
    pub buf: Vec<u8>
}

impl Encoder {
    pub fn new () -> Encoder {
        Encoder {
            buf: Vec::with_capacity(10000)
        }
    }
    pub fn with_capacity (size: usize) -> Encoder {
        Encoder {
            buf: Vec::with_capacity(size)
        }
    }
    pub fn write (&mut self, byte: u8) {
        self.buf.push(byte)
    }
    pub fn write_var_u32 (&mut self, mut num: u32) {
        for _ in 0..4 {
            self.buf.push(num as u8);
            num >>= 8
        }
    }
    pub fn as_bytes (&self) -> &[u8] {
        &self.buf[..]
    }
    pub fn write_var_buffer (&mut self, buffer: &[u8]) {
        self.write_var_u32(buffer.len() as u32);
        for elemen in buffer.iter() {
            self.write(*elemen);
        }
    }
}

pub struct Decoder <'a> {
    pub buf: &'a [u8],
    next: usize
}

impl <'a> Decoder <'a> {
    pub fn new (buf: &'a [u8]) -> Decoder<'a> {
        Decoder {
            buf,
            next: 0
        }
    }
    pub fn read (&mut self) -> u8 {
        let b = self.buf[self.next];
        self.next += 1;
        b
    }
    pub fn read_var_u32 (&mut self) -> u32 {
        self.read() as u32 | (self.read() as u32) << 8 | (self.read() as u32) << 16 | (self.read() as u32) << 24
    }
    pub fn read_var_buffer (&mut self) -> &[u8] {
        let len = self.read_var_u32();
        let slice = &self.buf[self.next..(self.next + len as usize)];
        self.next += len as usize;
        slice
    }
}

pub struct UpdateEncoder {
    pub rest_encoder: Encoder
}

impl UpdateEncoder {
    pub fn new () -> UpdateEncoder {
        UpdateEncoder {
            rest_encoder: Encoder::new()
        }
    }
    pub fn buffer (&self) -> &[u8] {
        self.rest_encoder.as_bytes()
    }
    pub fn write_left_id (&mut self, id: &ID) {
        self.rest_encoder.write_var_u32(id.client);
        self.rest_encoder.write_var_u32(id.clock);
    }
    pub fn write_right_id (&mut self, id: &ID) {
        self.rest_encoder.write_var_u32(id.client);
        self.rest_encoder.write_var_u32(id.clock);
    }
    pub fn write_client (&mut self, client: u32) {
        self.rest_encoder.write_var_u32(client);
    }
    pub fn write_info (&mut self, info: u8) {
        self.rest_encoder.write(info);
    }
    pub fn write_string (&mut self, string: &str) {
        let bytes = string.as_bytes();
        self.write_var_buffer(bytes);
        // self.rest_encoder.push(string as u32)
    }
    pub fn write_char (&mut self, string: char) {
        self.rest_encoder.write(string as u8)
    }
    /*
    pub fn write_parent_info (&mut self, parent_info: bool) {
        self.rest_encoder.write(if parent_info { 1 } else { 0 })
    }
    pub fn write_type_ref (&mut self, type_ref: u8) {
        self.rest_encoder.write(type_ref);
    }
    pub fn write_len (&mut self, len: u32) {
        self.rest_encoder.write_var_u32(len)
    }
    */
    pub fn write_var_buffer (&mut self, buffer: &[u8]) {
        self.rest_encoder.write_var_buffer(buffer);
    }
}

pub struct UpdateDecoder <'a> {
    pub rest_decoder: Decoder<'a>
}

impl <'a> UpdateDecoder <'a> {
    pub fn new (buf: &[u8]) -> UpdateDecoder {
        UpdateDecoder {
            rest_decoder: Decoder::new(buf)
        }
    }
    pub fn read_left_id (&mut self) -> ID {
        ID {
            client: self.rest_decoder.read_var_u32(),
            clock: self.rest_decoder.read_var_u32()
        }
    }
    pub fn read_right_id (&mut self) -> ID {
        ID {
            client: self.rest_decoder.read_var_u32(),
            clock: self.rest_decoder.read_var_u32()
        }
    }
    pub fn read_client (&mut self) -> u32 {
        self.rest_decoder.read_var_u32()
    }
    pub fn read_info (&mut self) -> u8 {
        self.rest_decoder.read()
    }
    pub fn read_string (&mut self) -> String {
        let buf = self.read_var_buffer();
        String::from_utf8(buf.to_vec()).expect("malformatted string")
    }
    pub fn read_char (&mut self) -> char {
        self.rest_decoder.read() as char
    }
    /*
    pub fn read_parent_info (&mut self) -> bool {
        let flag = self.rest_decoder.read();
        flag == 1
    }
    pub fn read_type_ref (&mut self) -> u8 {
        self.rest_decoder.read()
    }
    pub fn read_len (&mut self) -> u32 {
        self.rest_decoder.read_var_u32()
    }
    */
    pub fn read_var_buffer (&mut self) -> &[u8] {
        self.rest_decoder.read_var_buffer()
    }
}

