use lib0::decoding::Decoder;
use lib0::encoding::Encoder;
use quickcheck::quickcheck;

#[cfg(test)]
quickcheck! {
    fn write(xs: Vec<u8>) -> bool {
        let mut encoder = Encoder::new();
        for elem in xs.iter() {
            encoder.write(*elem);
        }
        let mut decoder = Decoder::new(&encoder.buf);
        for i in 0..xs.len() {
            let b = decoder.read();
            assert_eq!(b, xs[i]);
            if b != xs[i] {
                return false
            }
        }
        true
    }
}

#[test]
fn it_adds_two() {
    let num = 1;
    assert_eq!((num as u32), 1);
    let mut encoder = Encoder::new();
    encoder.write(4);
    encoder.write(3);
    let mut decoder = Decoder::new(&encoder.buf);
    assert_eq!(decoder.read(), 4);
}
