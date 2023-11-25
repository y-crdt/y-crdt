use crate::any::Any;
use serde::de::value::{MapAccessDeserializer, MapDeserializer, SeqDeserializer};
use serde::de::{IntoDeserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::any::type_name;
use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use thiserror::Error;

pub fn from_any<'de, T: Deserialize<'de>>(any: &'de Any) -> Result<T, AnyDeserializeError> {
    let deserializer = AnyDeserializer { value: any };
    T::deserialize(deserializer)
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
                E: serde::de::Error,
            {
                Ok(Any::from(v))
            }

            fn visit_i8<E>(self, v: i8) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::Number(v as f64))
            }

            fn visit_i16<E>(self, v: i16) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::from(v))
            }

            fn visit_i32<E>(self, v: i32) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::from(v))
            }

            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::from(v))
            }

            fn visit_u8<E>(self, v: u8) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::Number(v as f64))
            }

            fn visit_u16<E>(self, v: u16) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::from(v))
            }

            fn visit_u32<E>(self, v: u32) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::from(v))
            }

            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match Any::try_from(v) {
                    Ok(any) => Ok(any),
                    Err(v) => Err(serde::de::Error::custom(format!(
                        "Value {} out of range for i64",
                        v
                    ))),
                }
            }

            fn visit_f32<E>(self, v: f32) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::from(v))
            }

            fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::from(v))
            }

            fn visit_char<E>(self, v: char) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::from(v.to_string()))
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::from(v))
            }

            fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::from(v))
            }

            fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::from(v))
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::from(v))
            }

            fn visit_borrowed_bytes<E>(self, v: &'de [u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::from(v))
            }

            fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Any::from(v))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
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
                E: serde::de::Error,
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

                Ok(Any::Array(vec.into()))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut any_map = HashMap::new();
                while let Some((key, value)) = map.next_entry()? {
                    any_map.insert(key, value);
                }

                Ok(Any::Map(Arc::new(any_map)))
            }
        }

        deserializer.deserialize_any(AnyVisitor)
    }
}

pub struct AnyDeserializer<'a> {
    value: &'a Any,
}

impl<'de, 'a: 'de> IntoDeserializer<'de, AnyDeserializeError> for &'a Any {
    type Deserializer = AnyDeserializer<'de>;

    fn into_deserializer(self) -> Self::Deserializer {
        AnyDeserializer { value: self }
    }
}

#[derive(Error, Debug)]
pub enum AnyDeserializeError {
    #[error("deserialized int does not fit in field int type")]
    IntDoesNotFit,
    #[error("types do not match. Expected {0}")]
    TypeMismatch(&'static str),
    #[error("{0}")]
    Custom(String),
}

impl AnyDeserializeError {
    pub fn type_mismatch<T>() -> Self {
        AnyDeserializeError::TypeMismatch(type_name::<T>())
    }
}

impl serde::de::Error for AnyDeserializeError {
    fn custom<T>(msg: T) -> Self
    where
        T: Display,
    {
        AnyDeserializeError::Custom(msg.to_string())
    }
}

impl<'de> Deserializer<'de> for AnyDeserializer<'de> {
    type Error = AnyDeserializeError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Any::Null => self.deserialize_unit(visitor),
            Any::Undefined => self.deserialize_unit(visitor),
            Any::Bool(_) => self.deserialize_bool(visitor),
            Any::Number(_) => self.deserialize_f64(visitor),
            Any::BigInt(_) => self.deserialize_i64(visitor),
            Any::String(_) => self.deserialize_string(visitor),
            Any::Buffer(_) => self.deserialize_byte_buf(visitor),
            Any::Array(_) => self.deserialize_seq(visitor),
            Any::Map(_) => self.deserialize_map(visitor),
        }
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Any::Bool(b) => visitor.visit_bool(*b),
            _ => Err(AnyDeserializeError::TypeMismatch(type_name::<bool>())),
        }
    }

    fn deserialize_i8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let value = match self.value {
            Any::Number(i) => {
                if i.fract() == 0.0 {
                    *i as i64
                } else {
                    return Err(AnyDeserializeError::type_mismatch::<i8>());
                }
            }
            Any::BigInt(i) => *i,
            _ => return Err(AnyDeserializeError::type_mismatch::<i8>()),
        }
        .try_into()
        .map_err(|_| AnyDeserializeError::IntDoesNotFit)?;
        visitor.visit_i8(value)
    }

    fn deserialize_i16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let value = match self.value {
            Any::Number(i) => {
                if i.fract() == 0.0 {
                    *i as i64
                } else {
                    return Err(AnyDeserializeError::type_mismatch::<i16>());
                }
            }
            Any::BigInt(i) => *i,
            _ => return Err(AnyDeserializeError::type_mismatch::<i16>()),
        }
        .try_into()
        .map_err(|_| AnyDeserializeError::IntDoesNotFit)?;
        visitor.visit_i16(value)
    }

    fn deserialize_i32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let value = match self.value {
            Any::Number(i) => {
                if i.fract() == 0.0 {
                    *i as i64
                } else {
                    return Err(AnyDeserializeError::type_mismatch::<i32>());
                }
            }
            Any::BigInt(i) => *i,
            _ => return Err(AnyDeserializeError::type_mismatch::<i32>()),
        }
        .try_into()
        .map_err(|_| AnyDeserializeError::IntDoesNotFit)?;
        visitor.visit_i32(value)
    }

    fn deserialize_i64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let value = match self.value {
            Any::Number(i) => {
                if i.fract() == 0.0 {
                    *i as i64
                } else {
                    return Err(AnyDeserializeError::type_mismatch::<i64>());
                }
            }
            Any::BigInt(i) => *i,
            _ => return Err(AnyDeserializeError::type_mismatch::<i64>()),
        };
        visitor.visit_i64(value)
    }

    fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let value = match self.value {
            Any::Number(i) => {
                if i.fract() == 0.0 {
                    *i as i64
                } else {
                    return Err(AnyDeserializeError::type_mismatch::<u8>());
                }
            }
            Any::BigInt(i) => *i,
            _ => return Err(AnyDeserializeError::type_mismatch::<u8>()),
        }
        .try_into()
        .map_err(|_| AnyDeserializeError::IntDoesNotFit)?;
        visitor.visit_u8(value)
    }

    fn deserialize_u16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let value = match self.value {
            Any::Number(i) => {
                if i.fract() == 0.0 {
                    *i as i64
                } else {
                    return Err(AnyDeserializeError::type_mismatch::<u16>());
                }
            }
            Any::BigInt(i) => *i,
            _ => return Err(AnyDeserializeError::type_mismatch::<u16>()),
        }
        .try_into()
        .map_err(|_| AnyDeserializeError::IntDoesNotFit)?;
        visitor.visit_u16(value)
    }

    fn deserialize_u32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let value = match self.value {
            Any::Number(i) => {
                if i.fract() == 0.0 {
                    *i as i64
                } else {
                    return Err(AnyDeserializeError::type_mismatch::<u32>());
                }
            }
            Any::BigInt(i) => *i,
            _ => return Err(AnyDeserializeError::type_mismatch::<u32>()),
        }
        .try_into()
        .map_err(|_| AnyDeserializeError::IntDoesNotFit)?;
        visitor.visit_u32(value)
    }

    fn deserialize_u64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let value = match self.value {
            Any::Number(i) => {
                if i.fract() == 0.0 {
                    *i as i64
                } else {
                    return Err(AnyDeserializeError::type_mismatch::<u64>());
                }
            }
            Any::BigInt(i) => *i,
            _ => return Err(AnyDeserializeError::type_mismatch::<u64>()),
        }
        .try_into()
        .map_err(|_| AnyDeserializeError::IntDoesNotFit)?;
        visitor.visit_u64(value)
    }

    fn deserialize_f32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Any::Number(f) => visitor.visit_f32(*f as f32),
            Any::BigInt(f) => visitor.visit_f32(*f as f32),
            _ => Err(AnyDeserializeError::type_mismatch::<f32>()),
        }
    }

    fn deserialize_f64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Any::Number(f) => visitor.visit_f64(*f),
            Any::BigInt(f) => visitor.visit_f64(*f as f64),
            _ => Err(AnyDeserializeError::type_mismatch::<f64>()),
        }
    }

    fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Any::String(s) => {
                let mut chars = s.chars();
                // Match on first two characters
                match (chars.next(), chars.next()) {
                    (Some(c), None) => visitor.visit_char(c),
                    _ => Err(AnyDeserializeError::type_mismatch::<char>()),
                }
            }
            _ => Err(AnyDeserializeError::type_mismatch::<char>()),
        }
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Any::String(s) => visitor.visit_borrowed_str(s),
            _ => Err(AnyDeserializeError::type_mismatch::<&str>()),
        }
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Any::String(s) => visitor.visit_string(s.to_string()),
            _ => Err(AnyDeserializeError::type_mismatch::<String>()),
        }
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Any::Buffer(b) => visitor.visit_borrowed_bytes(b),
            _ => Err(AnyDeserializeError::type_mismatch::<&[u8]>()),
        }
    }

    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Any::Buffer(b) => visitor.visit_bytes(b),
            _ => Err(AnyDeserializeError::type_mismatch::<Vec<u8>>()),
        }
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Any::Null | Any::Undefined => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Any::Null | Any::Undefined => visitor.visit_unit(),
            _ => Err(AnyDeserializeError::type_mismatch::<()>()),
        }
    }

    fn deserialize_unit_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Any::Array(a) => visitor.visit_seq(SeqDeserializer::new(a.iter())),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Any::Map(m) => visitor.visit_map(MapDeserializer::new(
                m.iter().map(|(key, value)| (key.as_str(), value)),
            )),
            _ => Err(AnyDeserializeError::type_mismatch::<V::Value>()),
        }
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Any::String(s) => visitor.visit_enum(s.into_deserializer()),
            Any::Map(m) => visitor.visit_enum(MapAccessDeserializer::new(MapDeserializer::new(
                m.iter().map(|(key, value)| (key.as_str(), value)),
            ))),
            _ => Err(AnyDeserializeError::type_mismatch::<V::Value>()),
        }
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_any(visitor)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::any::Any;
    use serde::Deserialize;
    use std::collections::HashMap;

    #[test]
    fn test_deserialize_any_from_bool() {
        assert_eq!(
            serde_json::from_str::<Any>("true").unwrap(),
            Any::from(true)
        );
    }

    #[test]
    fn test_deserialize_any_from_float() {
        assert_eq!(
            serde_json::from_str::<Any>("18.812036").unwrap(),
            Any::from(18.812036f64)
        );
    }

    #[test]
    fn test_deserialize_any_from_int() {
        assert_eq!(serde_json::from_str::<Any>("18").unwrap(), Any::from(18));
    }

    #[test]
    fn test_deserialize_any_from_int_unrepresentable() {
        assert!(serde_json::from_str::<Any>(&u64::MAX.to_string()).is_err());
    }

    #[test]
    fn test_deserialize_any_from_string() {
        assert_eq!(
            serde_json::from_str::<Any>("\"string\"").unwrap(),
            Any::from("string")
        );
    }

    #[test]
    fn test_deserialize_any_from_null() {
        assert_eq!(serde_json::from_str::<Any>("null").unwrap(), Any::Null);
    }

    #[test]
    fn test_deserialize_any_from_array() {
        assert_eq!(
            serde_json::from_str::<Any>("[true, -101]").unwrap(),
            Any::from(vec![Any::from(true), Any::from(-101)])
        );
    }

    #[test]
    fn test_deserialize_any_from_map() {
        assert_eq!(
            serde_json::from_str::<Any>("{\"key1\":true,\"key2\":-12307.2138}").unwrap(),
            Any::from(HashMap::from([
                ("key1".into(), Any::from(true)),
                ("key2".into(), Any::from(-12307.2138f64))
            ]))
        );
    }

    #[test]
    fn test_deserialize_any_from_nested_map() {
        assert_eq!(
            serde_json::from_str::<Any>("{\"key1\":true,\"key2\":1.1,\"key3\":{\"key4\":true,\"key5\":1},\"key6\":[true,1,null]}").unwrap(),
            Any::from(HashMap::from([
                    ("key1".into(), Any::from(true)),
                    ("key2".into(), Any::from(1.1f64)),
                    ("key3".into(), Any::from(
                        HashMap::from([
                            ("key4".into(), Any::from(true)),
                            ("key5".into(), Any::from(1))
                        ])
                    )),
                    ("key6".into(), Any::from(vec![Any::from(true), Any::from(1), Any::Null]))
                ])
            )
        );
    }

    #[test]
    fn test_any_deserializer_many_fields() {
        #[derive(Debug, Deserialize, PartialEq)]
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
        }

        #[derive(Debug, Deserialize, PartialEq)]
        struct Nested {
            int: i16,
            other: f32,
        }

        #[derive(Debug, Deserialize, PartialEq)]
        enum StringEnum {
            VariantA,
            VariantB,
        }

        let any = Any::from(HashMap::from([
            ("bool".to_string(), Any::from(true)),
            ("int".to_string(), Any::from(1)),
            ("negative_int".to_string(), Any::from(-1)),
            ("max_int".to_string(), Any::from(i64::MAX)),
            ("min_int".to_string(), Any::from(i64::MIN)),
            ("real_number".to_string(), Any::from(-123.2387f64)),
            ("max_number".to_string(), Any::from(f64::MIN)),
            ("min_number".to_string(), Any::from(f64::MAX)),
            ("null".to_string(), Any::Null),
            ("some".to_string(), Any::from(false)),
            ("undefined".to_string(), Any::Undefined),
            (
                "nested".to_string(),
                Any::from(HashMap::from([
                    ("int".to_string(), Any::from(100)),
                    ("other".to_string(), Any::from(100.0)),
                ])),
            ),
            ("enum_a".to_string(), "VariantA".into()),
            ("enum_b".to_string(), "VariantB".into()),
        ]));

        assert_eq!(
            Test {
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
                    other: 100.0
                },
                enum_a: StringEnum::VariantA,
                enum_b: StringEnum::VariantB
            },
            from_any(&any).unwrap()
        )
    }

    #[test]
    fn test_deserialize_any_to_any() {
        let any = Any::from(HashMap::from([
            ("bool".to_string(), Any::from(true)),
            ("int".to_string(), Any::from(1)),
            ("negative_int".to_string(), Any::from(-1)),
            ("max_int".to_string(), Any::from(i64::MAX)),
            ("min_int".to_string(), Any::from(i64::MIN)),
            ("real_number".to_string(), Any::from(-123.2387f64)),
            ("max_number".to_string(), Any::from(f64::MIN)),
            ("min_number".to_string(), Any::from(f64::MAX)),
            ("null".to_string(), Any::Null),
            ("some".to_string(), Any::from(false)),
            (
                "nested".to_string(),
                Any::from(HashMap::from([
                    ("int".to_string(), Any::from(1i64 << 54)),
                    ("other".to_string(), Any::from(100.0)),
                ])),
            ),
            ("enum_a".to_string(), "VariantA".into()),
            ("enum_b".to_string(), "VariantB".into()),
        ]));

        assert_eq!(any, from_any(&any.clone()).unwrap())
    }

    #[test]
    fn test_any_deserializer_multiple_borrows() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Test<'a> {
            str: &'a str,
            bytes: &'a [u8],
        }

        let any = Any::from(HashMap::from([
            ("str".to_string(), Any::from("String")),
            ("bytes".to_string(), b"Bytes".to_vec().into()),
        ]));

        assert_eq!(
            Test {
                str: "String",
                bytes: b"Bytes"
            },
            from_any(&any).unwrap()
        )
    }

    #[test]
    fn test_any_deserializer_nested_array() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Test {
            array: Vec<Vec<bool>>,
        }

        let any: Any = HashMap::from([(
            "array".to_string(),
            Any::from(vec![
                Any::from(vec![Any::Bool(true), Any::Bool(false)]),
                Any::from(vec![Any::Bool(true)]),
            ]),
        )])
        .into();

        assert_eq!(
            Test {
                array: vec![vec![true, false], vec![true]]
            },
            from_any(&any).unwrap()
        )
    }

    #[test]
    fn test_any_deserializer_undefined() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Test {
            undefined: Option<bool>,
        }

        let any: Any = HashMap::from([("undefined".to_string(), Any::Undefined)]).into();

        assert_eq!(Test { undefined: None }, from_any(&any).unwrap())
    }

    #[test]
    fn test_any_deserializer_int_does_not_fit_error() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Test {
            test: i8,
        }

        let any: Any = HashMap::from([("test".to_string(), Any::BigInt(1000))]).into();

        assert!(matches!(
            from_any::<Test>(&any).unwrap_err(),
            AnyDeserializeError::IntDoesNotFit,
        ))
    }

    #[test]
    fn test_any_deserializer_type_mismatch_error() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Test {
            test: i8,
        }

        let any: Any = HashMap::from([("test".to_string(), Any::Number(1000.1f64))]).into();

        let error = from_any::<Test>(&any).unwrap_err();
        match error {
            AnyDeserializeError::TypeMismatch("i8") => { /* ok */ }
            other => panic!("unexpected error {}", other),
        }
    }
}
