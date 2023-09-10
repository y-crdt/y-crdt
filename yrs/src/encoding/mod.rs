pub mod read;
pub mod serde;
pub mod varint;
pub mod write;

#[cfg(test)]
mod test {
    use proptest::prelude::*;
    use crate::Any;
    use crate::encoding::read::{Cursor, Read};
    use crate::encoding::write::Write;

    pub fn arb_any() -> impl Strategy<Value = Any> {
        let leaf = prop_oneof![
            Just(Any::Null),
            Just(Any::Undefined),
            any::<bool>().prop_map(Any::Bool),
            any::<f64>().prop_map(Any::from),
            any::<i64>().prop_map(Any::from),
            any::<String>().prop_map(Any::from),
            any::<Vec<u8>>().prop_map(Any::from),
        ]
        .boxed();

        leaf.prop_recursive(8, 256, 10, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..10).prop_map(Any::from),
                prop::collection::hash_map(".*", inner, 0..10).prop_map(Any::from),
            ]
        })
    }

    proptest! {
        #[test]
        fn encoding_any_prop(any in arb_any()) {
            let mut encoder = Vec::with_capacity(1024);
            any.encode(&mut encoder);
            let mut decoder = Cursor::new(encoder.as_slice());
            let copy = Any::decode(&mut decoder).unwrap();
            assert_eq!(any, copy);
        }
    }

    #[derive(Debug, proptest_derive::Arbitrary)]
    enum EncodingTypes {
        Byte(u8),
        Uint8(u8),
        Uint16(u16),
        Uint32(u32),
        Uint32BigEndian(u32),
        VarUint32(u32),
        VarUint64(u64),
        VarUint128(u128),
        VarUintUsize(usize),
        VarInt(i64),
        Buffer(Vec<u8>),
        VarBuffer(Vec<u8>),
        VarString(String),
        Float32(f32),
        Float64(f64),
        BigInt64(i64),
        BigUInt64(u64),
        #[proptest(strategy = "arb_any().prop_map(EncodingTypes::Any)")]
        Any(Any),
    }

    impl EncodingTypes {
        fn write<W: Write>(&self, encoder: &mut W) {
            match self {
                EncodingTypes::Byte(input) => encoder.write_u8(*input),
                EncodingTypes::Uint8(input) => {
                    encoder.write_u8(*input);
                }
                EncodingTypes::Uint16(input) => {
                    encoder.write_u16(*input);
                }
                EncodingTypes::Uint32(input) => {
                    encoder.write_u32(*input);
                }
                EncodingTypes::Uint32BigEndian(input) => {
                    encoder.write_u32_be(*input);
                }
                EncodingTypes::VarUint32(input) => {
                    encoder.write_var(*input);
                }
                EncodingTypes::VarUint64(input) => {
                    encoder.write_var(*input);
                }
                EncodingTypes::VarUint128(input) => {
                    encoder.write_var(*input);
                }
                EncodingTypes::VarUintUsize(input) => {
                    encoder.write_var(*input);
                }
                EncodingTypes::VarInt(input) => {
                    encoder.write_var(*input);
                }
                EncodingTypes::Buffer(input) => {
                    encoder.write_all(input);
                }
                EncodingTypes::VarBuffer(input) => {
                    encoder.write_buf(input);
                }
                EncodingTypes::VarString(input) => {
                    encoder.write_string(input);
                }
                EncodingTypes::Float32(input) => {
                    encoder.write_f32(*input);
                }
                EncodingTypes::Float64(input) => {
                    encoder.write_f64(*input);
                }
                EncodingTypes::BigInt64(input) => {
                    encoder.write_i64(*input);
                }
                EncodingTypes::BigUInt64(input) => {
                    encoder.write_u64(*input);
                }
                EncodingTypes::Any(input) => {
                    input.encode(encoder);
                }
            }
        }
        fn read(&self, decoder: &mut Cursor) {
            match self {
                EncodingTypes::Byte(input) => {
                    let read = decoder.read_u8().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::Uint8(input) => {
                    let read = decoder.read_u8().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::Uint16(input) => {
                    let read = decoder.read_u16().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::Uint32(input) => {
                    let read = decoder.read_u32().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::Uint32BigEndian(input) => {
                    let read = decoder.read_u32_be().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::VarUint32(input) => {
                    let read: u32 = decoder.read_var().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::VarUint64(input) => {
                    let read: u64 = decoder.read_var().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::VarUint128(input) => {
                    let read: u128 = decoder.read_var().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::VarUintUsize(input) => {
                    let read: usize = decoder.read_var().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::VarInt(input) => {
                    let read = decoder.read_var::<i64>().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::Buffer(input) => {
                    let read = decoder.read_exact(input.len()).unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::VarBuffer(input) => {
                    let read = decoder.read_buf().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::VarString(input) => {
                    let read = decoder.read_string().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::Float32(input) => {
                    let read = decoder.read_f32().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::Float64(input) => {
                    let read = decoder.read_f64().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::BigInt64(input) => {
                    let read = decoder.read_i64().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::BigUInt64(input) => {
                    let read = decoder.read_u64().unwrap();
                    assert_eq!(read, *input);
                }
                EncodingTypes::Any(input) => {
                    let read = Any::decode(decoder).unwrap();
                    assert_eq!(read, *input);
                }
            }
        }
    }

    proptest! {
        #[test]
        fn encoding_prop(val: EncodingTypes) {
            let mut encoder = Vec::new();
            val.write(&mut encoder);
            let mut decoder = Cursor::new(encoder.as_slice());
            val.read(&mut decoder)
        }
    }
}
