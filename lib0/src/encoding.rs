pub struct Encoder {
  pub buf: Vec<u8>
}

impl Encoder {
  pub fn new () -> Encoder {
    Encoder {
        buf: Vec::with_capacity(10000)
    }
  }
  pub fn write (&mut self, byte: u8) {
    self.buf.push(byte)
  }
}