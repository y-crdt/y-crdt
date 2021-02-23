#[derive(Default)]
pub struct Decoder<'a> {
    pub buf: &'a [u8],
    next: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(buf: &'a [u8]) -> Decoder<'a> {
        Decoder { buf, next: 0 }
    }
    pub fn read(&mut self) -> u8 {
        let b = self.buf[self.next];
        self.next += 1;
        b
    }
    pub fn read_var_u32(&mut self) -> u32 {
        self.read() as u32
            | (self.read() as u32) << 8
            | (self.read() as u32) << 16
            | (self.read() as u32) << 24
    }
    pub fn read_var_buffer(&mut self) -> &[u8] {
        let len = self.read_var_u32();
        let slice = &self.buf[self.next..(self.next + len as usize)];
        self.next += len as usize;
        slice
    }
}
