use lib0::decoding::Decoder;
use lib0::encoding::Encoder;
use lib0::any::Any;
use proptest::prelude::*;

pub fn arb_any() -> impl Strategy<Value = Any> {
    let leaf = prop_oneof![
        Just(Any::Null),
        Just(Any::Undefined),
        any::<bool>().prop_map(Any::Bool),
        any::<f64>().prop_map(Any::Number),
        any::<i64>().prop_map(|i| Any::Number(i as f64)),
        any::<String>().prop_map(Any::String),
        any::<Box<[u8]>>().prop_map(Any::Buffer),
    ].boxed();

    leaf.prop_recursive(
        8,
        256,
        10,
        |inner| prop_oneof![
            prop::collection::vec(inner.clone(), 0..10)
                .prop_map(Any::Array),
            prop::collection::hash_map(".*", inner, 0..10)
                .prop_map(Any::Map),
        ]
    )
}

proptest! {
    #[test]
    fn encoding_any_prop(any in arb_any()) {
        let mut encoder = Encoder::new();
        encoder.write_any(&any);
        let mut decoder = Decoder::new(&encoder.buf);
        let copy = decoder.read_any();
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
    VarUint(u64),
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
    fn write (&self, encoder: &mut Encoder) {
        match self {
            EncodingTypes::Byte(input) => {
                encoder.write(*input)
            }
            EncodingTypes::Uint8(input) => {
                encoder.write_uint8(*input);
            }
            EncodingTypes::Uint16(input) => {
                encoder.write_uint16(*input);
            }
            EncodingTypes::Uint32(input) => {
                encoder.write_uint32(*input);
            }
            EncodingTypes::Uint32BigEndian(input) => {
                encoder.write_uint32_big_endian(*input);
            }
            EncodingTypes::VarUint(input) => {
                encoder.write_var_uint(*input);
            }
            EncodingTypes::VarInt(input) => {
                encoder.write_var_int(*input);
            }
            EncodingTypes::Buffer(input) => {
                encoder.write_buffer(input);
            }
            EncodingTypes::VarBuffer(input) => {
                encoder.write_var_buffer(input);
            }
            EncodingTypes::VarString(input) => {
                encoder.write_var_string(input);
            }
            EncodingTypes::Float32(input) => {
                encoder.write_float32(*input);
            }
            EncodingTypes::Float64(input) => {
                encoder.write_float64(*input);
            }
            EncodingTypes::BigInt64(input) => {
                encoder.write_big_int64(*input);
            }
            EncodingTypes::BigUInt64(input) => {
                encoder.write_big_uint64(*input);
            }
            EncodingTypes::Any(input) => {
                encoder.write_any(input);
            }
        }
    }
    fn read (&self, decoder: &mut Decoder) {
        match self {
            EncodingTypes::Byte(input) => {
                let read = decoder.read();
                assert_eq!(read, *input);
            }
            EncodingTypes::Uint8(input) => {
                let read = decoder.read_uint8();
                assert_eq!(read, *input);
            }
            EncodingTypes::Uint16(input) => {
                let read = decoder.read_uint16();
                assert_eq!(read, *input);
            }
            EncodingTypes::Uint32(input) => {
                let read = decoder.read_uint32();
                assert_eq!(read, *input);
            }
            EncodingTypes::Uint32BigEndian(input) => {
                let read = decoder.read_uint32_big_endian();
                assert_eq!(read, *input);
            }
            EncodingTypes::VarUint(input) => {
                let read = decoder.read_var_uint();
                assert_eq!(read, *input);
            }
            EncodingTypes::VarInt(input) => {
                let read = decoder.read_var_int();
                assert_eq!(read, *input);
            }
            EncodingTypes::Buffer(input) => {
                let read = decoder.read_buffer(input.len() as u32);
                assert_eq!(read, *input);
            }
            EncodingTypes::VarBuffer(input) => {
                let read = decoder.read_var_buffer();
                assert_eq!(read, *input);
            }
            EncodingTypes::VarString(input) => {
                let read = decoder.read_var_string();
                assert_eq!(read, *input);
            }
            EncodingTypes::Float32(input) => {
                let read = decoder.read_float32();
                assert_eq!(read, *input);
            }
            EncodingTypes::Float64(input) => {
                let read = decoder.read_float64();
                assert_eq!(read, *input);
            }
            EncodingTypes::BigInt64(input) => {
                let read = decoder.read_bigint64();
                assert_eq!(read, *input);
            }
            EncodingTypes::BigUInt64(input) => {
                let read = decoder.read_biguint64();
                assert_eq!(read, *input);
            }
            EncodingTypes::Any(input) => {
                let read = decoder.read_any();
                assert_eq!(read, *input);
            }
        }
    }
}

proptest! {
    #[test]
    fn encoding_prop(val: EncodingTypes) {
        let mut encoder = Encoder::new();
        val.write(&mut encoder);
        let mut decoder = Decoder::new(&encoder.buf);
        val.read(&mut decoder)
    }
}
