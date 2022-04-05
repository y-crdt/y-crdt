#[cfg(test)]
mod test {
    use lib0::any::Any;
    use std::collections::HashMap;

    fn roundtrip(any: &Any) -> Any {
        // encode
        let mut buf = String::new();
        any.to_json(&mut buf);
        // decode
        Any::from_json(buf.as_str())
    }

    #[test]
    fn json_any_null() {
        let expected = Any::Null;
        let actual = roundtrip(&expected);
        assert_eq!(actual, expected);
    }

    #[test]
    fn json_any_bool() {
        for v in [true, false] {
            let expected = Any::Bool(v);
            let actual = roundtrip(&expected);
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn json_any_number() {
        for v in [1.8, -1.5, 0.2, 0.25, 2349464814556456.4] {
            let expected = Any::Number(v);
            let actual = roundtrip(&expected);
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn json_any_string() {
        for v in ["", "hello ", "hello \nworld", "hello \"world\"", "hello 😀"] {
            let expected = Any::String(v.into());
            let actual = roundtrip(&expected);
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn json_any_array() {
        let expected = Any::Array(
            vec![
                Any::Number(-10.0),
                Any::String("hello \\world".into()),
                Any::Null,
                Any::Bool(true),
            ]
            .into_boxed_slice(),
        );
        let actual = roundtrip(&expected);
        assert_eq!(actual, expected);
    }

    #[test]
    fn json_any_map() {
        let expected = Any::Map(Box::new(HashMap::from([
            ("a".to_string(), Any::Number(-10.2)),
            ("b".to_string(), Any::String("hello world".into())),
            ("c".to_string(), Any::Null),
            ("d".to_string(), Any::Bool(true)),
            (
                "e".to_string(),
                Any::Array(vec![Any::String("abc".into())].into_boxed_slice()),
            ),
            (
                "f".to_string(),
                Any::Map(Box::new(HashMap::from([(
                    "fa".to_string(),
                    Any::Number(1.5),
                )]))),
            ),
        ])));
        let actual = roundtrip(&expected);
        assert_eq!(actual, expected);
    }
}
