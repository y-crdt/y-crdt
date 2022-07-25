mod de;
mod ser;

pub use de::from_any;
pub use ser::to_any;

#[cfg(test)]
mod test {
    use super::*;
    use crate::any::Any;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    #[test]
    fn test_struct_any_roundtrip() {
        #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
        struct Test {
            bool: bool,
            int: i64,
            negative_int: i64,
            max_int: i64,
            min_int: i64,
            real_number: f64,
            max_number: f64,
            min_number: f64,
            null: Option<bool>,
            some: Option<bool>,
            undefined: Option<bool>,
            nested: Nested,
            enum_a: StringEnum,
            enum_b: StringEnum,
            enum_c: StringEnum,
            array: Vec<Option<Nested>>,
        }

        #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
        struct Nested {
            int: i16,
            other: f32,
        }

        #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
        enum StringEnum {
            VariantA,
            VariantB(String),
            VariantC { test: bool },
        }

        let test = Test {
            bool: true,
            int: 1,
            negative_int: -1,
            max_int: i64::MAX,
            min_int: i64::MIN,
            real_number: -123.2387f64,
            max_number: f64::MIN,
            min_number: f64::MAX,
            null: None,
            some: Some(false),
            undefined: None,
            nested: Nested {
                int: 100,
                other: 100.0,
            },
            enum_a: StringEnum::VariantA,
            enum_b: StringEnum::VariantB("test".to_string()),
            enum_c: StringEnum::VariantC { test: true },
            array: vec![None, Some(Nested { int: 0, other: 0.0 }), None],
        };

        assert_eq!(test, from_any(&to_any(&test).unwrap()).unwrap())
    }

    #[test]
    fn test_enum_untagged_any_roundtrip() {
        #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
        struct Test {
            enum_a: StringEnum,
            enum_b: StringEnum,
            enum_c: StringEnum,
        }

        #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
        #[serde(untagged)]
        enum StringEnum {
            VariantA,
            VariantB(String),
            VariantC { test: bool },
        }

        let test = Test {
            enum_a: StringEnum::VariantA,
            enum_b: StringEnum::VariantB("test".to_string()),
            enum_c: StringEnum::VariantC { test: true },
        };

        assert_eq!(test, from_any(&to_any(&test).unwrap()).unwrap())
    }

    #[test]
    fn test_any_to_json_roundtrip() {
        let any = Any::Map(Box::new(HashMap::from([
            ("bool".into(), Any::Bool(true)),
            ("int".into(), Any::BigInt(1)),
            ("negativeInt".into(), Any::BigInt(-1)),
            ("maxInt".into(), Any::BigInt(i64::MAX)),
            ("minInt".into(), Any::BigInt(i64::MIN)),
            ("realNumber".into(), Any::Number(-123.2387f64)),
            ("maxNumber".into(), Any::Number(f64::MIN)),
            ("minNumber".into(), Any::Number(f64::MAX)),
            ("null".into(), Any::Null),
            (
                "map".into(),
                Any::Map(Box::new(HashMap::from([
                    ("bool".into(), Any::Bool(true)),
                    ("int".into(), Any::BigInt(1)),
                    ("negativeInt".into(), Any::BigInt(-1)),
                    ("maxInt".into(), Any::BigInt(i64::MAX)),
                    ("minInt".into(), Any::BigInt(i64::MIN)),
                    ("realNumber".into(), Any::Number(-123.2387f64)),
                    ("maxNumber".into(), Any::Number(f64::MIN)),
                    ("minNumber".into(), Any::Number(f64::MAX)),
                    ("null".into(), Any::Null),
                ]))),
            ),
            (
                "key6".into(),
                Any::Array(
                    vec![
                        Any::Bool(true),
                        Any::BigInt(1),
                        Any::BigInt(-1),
                        Any::BigInt(i64::MAX),
                        Any::BigInt(i64::MIN),
                        Any::Number(-123.2387f64),
                        Any::Number(f64::MIN),
                        Any::Number(f64::MAX),
                        Any::Null,
                    ]
                    .into_boxed_slice(),
                ),
            ),
        ])));

        assert_eq!(
            any,
            serde_json::from_str::<Any>(serde_json::to_string(&any).unwrap().as_str()).unwrap()
        );
    }
}
