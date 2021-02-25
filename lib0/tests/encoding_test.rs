use lib0::decoding::Decoder;
use lib0::encoding::Encoder;
use lib0::any::Any;
use proptest::prelude::*;

pub fn arb_any() -> impl Strategy<Value = Any> {
    let leaf = prop_oneof![
        Just(Any::Null),
        Just(Any::Undefined),
        any::<bool>().prop_map(Any::Bool),
        any::<i32>().prop_map(|i| Any::Number(i as f64)),
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
        assert!(any == copy)
    }
}

