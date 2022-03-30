use crate::any::Any;
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::fmt::Formatter;

impl Serialize for Any {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Any::Null => serializer.serialize_none(),
            Any::Undefined => serializer.serialize_none(),
            Any::Bool(value) => serializer.serialize_bool(*value),
            Any::Number(value) => serializer.serialize_f64(*value),
            Any::BigInt(value) => serializer.serialize_i64(*value),
            Any::String(value) => serializer.serialize_str(value.as_ref()),
            Any::Array(values) => {
                let mut seq = serializer.serialize_seq(Some(values.len()))?;
                for value in values.iter() {
                    seq.serialize_element(value)?;
                }
                seq.end()
            }
            Any::Map(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries.iter() {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
            Any::Buffer(buf) => serializer.serialize_bytes(buf),
        }
    }
}

impl<'de> Deserialize<'de> for Any {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct AnyVisitor;
        impl<'de> Visitor<'de> for AnyVisitor {
            type Value = Any;

            fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
                formatter.write_str("enum Any")
            }

            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::Bool(v))
            }

            fn visit_i8<E>(self, v: i8) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::BigInt(i64::from(v)))
            }

            fn visit_i16<E>(self, v: i16) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::BigInt(i64::from(v)))
            }

            fn visit_i32<E>(self, v: i32) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::BigInt(i64::from(v)))
            }

            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::BigInt(v))
            }

            fn visit_u8<E>(self, v: u8) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::BigInt(i64::from(v)))
            }

            fn visit_u16<E>(self, v: u16) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::BigInt(i64::from(v)))
            }

            fn visit_u32<E>(self, v: u32) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::BigInt(i64::from(v)))
            }

            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if v > i64::MAX as u64 {
                    Err(de::Error::custom(format!(
                        "Value {} out of range for i64",
                        v
                    )))
                } else {
                    Ok(Any::BigInt(v as i64))
                }
            }

            fn visit_f32<E>(self, v: f32) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::Number(f64::from(v)))
            }

            fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::Number(v))
            }

            fn visit_char<E>(self, v: char) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::String(v.to_string().into_boxed_str()))
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::String(v.to_string().into_boxed_str()))
            }

            fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::String(v.to_string().into_boxed_str()))
            }

            fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::String(v.into_boxed_str()))
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::Buffer(v.to_owned().into_boxed_slice()))
            }

            fn visit_borrowed_bytes<E>(self, v: &'de [u8]) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::Buffer(v.to_owned().into_boxed_slice()))
            }

            fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::Buffer(v.into_boxed_slice()))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::Null)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                Any::deserialize(deserializer)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Any::Null)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut vec = Vec::new();

                while let Some(value) = seq.next_element()? {
                    vec.push(value);
                }

                Ok(Any::Array(vec.into_boxed_slice()))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut any_map = HashMap::new();
                while let Some((key, value)) = map.next_entry()? {
                    any_map.insert(key, value);
                }

                Ok(Any::Map(Box::new(any_map)))
            }
        }

        deserializer.deserialize_any(AnyVisitor)
    }
}

#[cfg(test)]
mod test {
    use crate::any::Any;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn test_deserialize_bool() {
        assert_eq!(
            serde_json::from_str::<Any>("true").unwrap(),
            Any::Bool(true)
        );
    }

    #[test]
    fn test_serialize_bool() {
        assert_eq!(serde_json::to_string(&Any::Bool(false)).unwrap(), "false");
    }

    #[test]
    fn test_deserialize_float() {
        assert_eq!(
            serde_json::from_str::<Any>("18.812036").unwrap(),
            Any::Number(18.812036f64)
        );
    }

    #[test]
    fn test_serialize_float() {
        assert_eq!(
            serde_json::to_string(&Any::Number(-1298.283f64)).unwrap(),
            "-1298.283"
        );
    }

    #[test]
    fn test_deserialize_int() {
        assert_eq!(serde_json::from_str::<Any>("18").unwrap(), Any::BigInt(18));
    }

    #[test]
    fn test_deserialize_int_unrepresentable() {
        assert!(serde_json::from_str::<Any>(&u64::MAX.to_string()).is_err());
    }

    #[test]
    fn test_serialize_int() {
        assert_eq!(serde_json::to_string(&Any::BigInt(-1298)).unwrap(), "-1298");
    }

    #[test]
    fn test_deserialize_string() {
        assert_eq!(
            serde_json::from_str::<Any>("\"string\"").unwrap(),
            Any::String("string".into())
        );
    }

    #[test]
    fn test_serialize_string() {
        assert_eq!(
            serde_json::to_string(&Any::String("string".into())).unwrap(),
            "\"string\""
        );
    }

    #[test]
    fn test_deserialize_null() {
        assert_eq!(serde_json::from_str::<Any>("null").unwrap(), Any::Null);
    }

    #[test]
    fn test_serialize_null() {
        assert_eq!(serde_json::to_string(&Any::Null).unwrap(), "null");
    }

    #[test]
    fn test_serialize_undefined() {
        assert_eq!(serde_json::to_string(&Any::Undefined).unwrap(), "null");
    }

    #[test]
    fn test_serialize_buffer() {
        assert_eq!(
            serde_json::to_string(&Any::Buffer("buffer".as_bytes().into())).unwrap(),
            "[98,117,102,102,101,114]"
        );
    }

    #[test]
    fn test_serialize_array() {
        assert_eq!(
            serde_json::to_string(&Any::Array(
                vec![Any::Bool(true), Any::BigInt(1)].into_boxed_slice()
            ))
            .unwrap(),
            "[true,1]"
        );
    }

    #[test]
    fn test_deserialize_array() {
        assert_eq!(
            serde_json::from_str::<Any>("[true, -101]").unwrap(),
            Any::Array(vec![Any::Bool(true), Any::BigInt(-101)].into_boxed_slice())
        );
    }

    #[test]
    fn test_serialize_map() {
        assert_eq!(
            serde_json::to_value(&Any::Map(Box::new(HashMap::from([
                ("key1".into(), Any::Bool(true)),
                ("key2".into(), Any::BigInt(1))
            ]))))
            .unwrap(),
            json!({"key1":true, "key2":1})
        );
    }

    #[test]
    fn test_deserialize_map() {
        assert_eq!(
            serde_json::from_str::<Any>("{\"key1\":true,\"key2\":-12307.2138}").unwrap(),
            Any::Map(Box::new(HashMap::from([
                ("key1".into(), Any::Bool(true)),
                ("key2".into(), Any::Number(-12307.2138f64))
            ])))
        );
    }

    #[test]
    fn test_serialize_complex_map() {
        assert_eq!(
            serde_json::to_value(&Any::Map(Box::new(HashMap::from([
                ("key1".into(), Any::Bool(true)),
                ("key2".into(), Any::BigInt(1)),
                (
                    "key3".into(),
                    Any::Map(Box::new(HashMap::from([
                        ("key4".into(), Any::Bool(true)),
                        ("key5".into(), Any::BigInt(1))
                    ])))
                ),
                (
                    "key6".into(),
                    Any::Array(vec![Any::Bool(true), Any::BigInt(1)].into_boxed_slice())
                )
            ]))))
            .unwrap(),
            json!({"key1": true, "key2":1, "key3":{"key4":true, "key5":1}, "key6": [true,1]})
        );
    }

    #[test]
    fn test_deserialize_complex_map() {
        assert_eq!(
            serde_json::from_str::<Any>("{\"key1\":true,\"key2\":1.1,\"key3\":{\"key4\":true,\"key5\":1},\"key6\":[true,1,null]}").unwrap(),
            Any::Map(Box::new(
                HashMap::from([
                    ("key1".into(), Any::Bool(true)),
                    ("key2".into(), Any::Number(1.1f64)),
                    ("key3".into(), Any::Map(Box::new(
                        HashMap::from([
                            ("key4".into(), Any::Bool(true)),
                            ("key5".into(), Any::BigInt(1))
                        ])
                    ))),
                    ("key6".into(), Any::Array(vec![Any::Bool(true), Any::BigInt(1), Any::Null].into_boxed_slice()))
                ])
            ))
        );
    }

    #[test]
    fn test_roundtrip() {
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
