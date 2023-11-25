use crate::any::Any;
use serde::ser::{
    SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant,
};
use serde::{Serialize, Serializer};
use std::collections::HashMap;
use std::convert::TryInto;
use std::fmt::Display;
use thiserror::Error;

pub fn to_any<T: Serialize>(value: T) -> Result<Any, AnySerializeError> {
    value.serialize(AnySerializer)
}

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

struct AnySerializer;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AnySerializeError {
    #[error("integers above i64::MAX cannot be represented as Any")]
    UnrepresentableInt,
    #[error("Any only supports Strings as map keys")]
    MapKeyNotString,
    #[error("{0}")]
    Custom(String),
}

impl serde::ser::Error for AnySerializeError {
    fn custom<T>(msg: T) -> Self
    where
        T: Display,
    {
        AnySerializeError::Custom(msg.to_string())
    }
}

impl Serializer for AnySerializer {
    type Ok = Any;
    type Error = AnySerializeError;
    type SerializeSeq = AnyArraySerializer;
    type SerializeTuple = AnyArraySerializer;
    type SerializeTupleStruct = AnyArraySerializer;
    type SerializeTupleVariant = AnyTupleVariantSerializer;
    type SerializeMap = AnyMapSerializer;
    type SerializeStruct = AnyMapSerializer;
    type SerializeStructVariant = AnyStructVariantSerializer;

    #[inline]
    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        Ok(Any::Bool(v))
    }

    #[inline]
    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        self.serialize_i64(v.into())
    }

    #[inline]
    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        self.serialize_i64(v.into())
    }

    #[inline]
    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        self.serialize_i64(v.into())
    }

    #[inline]
    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        Ok(Any::from(v))
    }

    #[inline]
    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        self.serialize_i64(v.into())
    }

    #[inline]
    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        self.serialize_i64(v.into())
    }

    #[inline]
    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        self.serialize_i64(v.into())
    }

    #[inline]
    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        self.serialize_i64(
            v.try_into()
                .map_err(|_e| AnySerializeError::UnrepresentableInt)?,
        )
    }

    #[inline]
    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.serialize_f64(v.into())
    }

    #[inline]
    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        Ok(Any::Number(v))
    }

    #[inline]
    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        self.serialize_str(&v.to_string())
    }

    #[inline]
    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        Ok(Any::String(v.into()))
    }

    #[inline]
    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
        Ok(Any::Buffer(v.into()))
    }

    #[inline]
    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        self.serialize_unit()
    }

    #[inline]
    fn serialize_some<T: ?Sized>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize,
    {
        value.serialize(self)
    }

    #[inline]
    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(Any::Null)
    }

    #[inline]
    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.serialize_unit()
    }

    #[inline]
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.serialize_str(variant)
    }

    #[inline]
    fn serialize_newtype_struct<T: ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize,
    {
        value.serialize(self)
    }

    #[inline]
    fn serialize_newtype_variant<T: ?Sized>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize,
    {
        Ok(Any::from(HashMap::from([(
            variant.to_string(),
            value.serialize(AnySerializer)?,
        )])))
    }

    #[inline]
    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        let array = if let Some(len) = len {
            Vec::with_capacity(len)
        } else {
            Vec::new()
        };
        Ok(AnyArraySerializer { array })
    }

    #[inline]
    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        self.serialize_seq(Some(len))
    }

    #[inline]
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        self.serialize_seq(Some(len))
    }

    #[inline]
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(AnyTupleVariantSerializer {
            variant,
            array_serializer: self.serialize_seq(Some(len))?,
        })
    }

    #[inline]
    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        let map = if let Some(len) = len {
            HashMap::with_capacity(len)
        } else {
            HashMap::new()
        };

        Ok(AnyMapSerializer {
            map,
            next_key: None,
        })
    }

    #[inline]
    fn serialize_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        self.serialize_map(Some(len))
    }

    #[inline]
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(AnyStructVariantSerializer {
            variant,
            map_serializer: self.serialize_map(Some(len))?,
        })
    }
}

struct AnyArraySerializer {
    array: Vec<Any>,
}

impl SerializeSeq for AnyArraySerializer {
    type Ok = Any;
    type Error = AnySerializeError;

    fn serialize_element<T: ?Sized>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        self.array.push(value.serialize(AnySerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(Any::from(self.array))
    }
}

impl SerializeTuple for AnyArraySerializer {
    type Ok = Any;
    type Error = AnySerializeError;

    fn serialize_element<T: ?Sized>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        serde::ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        serde::ser::SerializeSeq::end(self)
    }
}

impl SerializeTupleStruct for AnyArraySerializer {
    type Ok = Any;
    type Error = AnySerializeError;

    fn serialize_field<T: ?Sized>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        serde::ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        serde::ser::SerializeSeq::end(self)
    }
}

struct AnyMapSerializer {
    map: HashMap<String, Any>,
    next_key: Option<String>,
}

impl SerializeMap for AnyMapSerializer {
    type Ok = Any;
    type Error = AnySerializeError;

    fn serialize_key<T: ?Sized>(&mut self, key: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        self.next_key = Some(match key.serialize(AnySerializer)? {
            Any::String(s) => s.to_string(),
            _ => return Err(AnySerializeError::MapKeyNotString),
        });
        Ok(())
    }

    fn serialize_value<T: ?Sized>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        let key = self.next_key.take();
        let key = key.expect("serialize_value must not be called before serialize_key");
        self.map.insert(key, value.serialize(AnySerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.map.into())
    }
}

impl SerializeStruct for AnyMapSerializer {
    type Ok = Any;
    type Error = AnySerializeError;

    fn serialize_field<T: ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        self.map
            .insert(key.to_string(), value.serialize(AnySerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        serde::ser::SerializeMap::end(self)
    }
}

struct AnyTupleVariantSerializer {
    variant: &'static str,
    array_serializer: AnyArraySerializer,
}

impl SerializeTupleVariant for AnyTupleVariantSerializer {
    type Ok = Any;
    type Error = AnySerializeError;

    fn serialize_field<T: ?Sized>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        SerializeSeq::serialize_element(&mut self.array_serializer, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(HashMap::from([(
            self.variant.to_string(),
            SerializeSeq::end(self.array_serializer)?,
        )])
        .into())
    }
}

struct AnyStructVariantSerializer {
    variant: &'static str,
    map_serializer: AnyMapSerializer,
}

impl SerializeStructVariant for AnyStructVariantSerializer {
    type Ok = Any;
    type Error = AnySerializeError;

    fn serialize_field<T: ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        SerializeStruct::serialize_field(&mut self.map_serializer, key, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(HashMap::from([(
            self.variant.to_string(),
            SerializeStruct::end(self.map_serializer)?,
        )])
        .into())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::any::Any;
    use serde::Serialize;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[test]
    fn test_serialize_any_to_bool() {
        assert_eq!(serde_json::to_string(&Any::Bool(false)).unwrap(), "false");
    }

    #[test]
    fn test_serialize_any_to_float() {
        assert_eq!(
            serde_json::to_string(&Any::Number(-1298.283f64)).unwrap(),
            "-1298.283"
        );
    }

    #[test]
    fn test_serialize_any_to_int() {
        assert_eq!(serde_json::to_string(&Any::BigInt(-1298)).unwrap(), "-1298");
    }

    #[test]
    fn test_serialize_any_to_string() {
        assert_eq!(
            serde_json::to_string(&Any::String("string".into())).unwrap(),
            "\"string\""
        );
    }

    #[test]
    fn test_serialize_any_to_null() {
        assert_eq!(serde_json::to_string(&Any::Null).unwrap(), "null");
    }

    #[test]
    fn test_serialize_any_to_undefined() {
        assert_eq!(serde_json::to_string(&Any::Undefined).unwrap(), "null");
    }

    #[test]
    fn test_serialize_any_to_buffer() {
        assert_eq!(
            serde_json::to_string(&Any::Buffer("buffer".as_bytes().into())).unwrap(),
            "[98,117,102,102,101,114]"
        );
    }

    #[test]
    fn test_serialize_any_to_array() {
        assert_eq!(
            serde_json::to_string(&Any::from(vec![Any::from(true), Any::from(1)])).unwrap(),
            "[true,1.0]"
        );
    }

    #[test]
    fn test_serialize_any_to_map() {
        assert_eq!(
            serde_json::to_value(&Any::from(HashMap::from([
                ("key1".into(), Any::from(true)),
                ("key2".into(), Any::from(1))
            ])))
            .unwrap(),
            json!({"key1":true, "key2":1.0})
        );
    }

    #[test]
    fn test_serialize_any_to_complex_map() {
        assert_eq!(
            serde_json::to_value(&Any::from(HashMap::from([
                ("key1".into(), Any::from(true)),
                ("key2".into(), Any::from(1)),
                (
                    "key3".into(),
                    Any::from(HashMap::from([
                        ("key4".into(), Any::from(true)),
                        ("key5".into(), Any::from(1))
                    ]))
                ),
                (
                    "key6".into(),
                    Any::from(vec![Any::from(true), Any::from(1)])
                )
            ])))
            .unwrap(),
            json!({"key1": true, "key2":1.0, "key3":{"key4":true, "key5":1.0}, "key6": [true,1.0]})
        );
    }

    #[test]
    fn test_any_serializer_many_fields() {
        #[derive(Debug, Serialize, PartialEq)]
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
            nested: Nested,
            enum_a: StringEnum,
            enum_b: StringEnum,
        }

        #[derive(Debug, Serialize, PartialEq)]
        struct Nested {
            int: i16,
            other: f32,
        }

        #[derive(Debug, Serialize, PartialEq)]
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
            any,
            to_any(&Test {
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
                nested: Nested {
                    int: 100,
                    other: 100.0
                },
                enum_a: StringEnum::VariantA,
                enum_b: StringEnum::VariantB
            })
            .unwrap(),
        )
    }

    #[test]
    fn test_any_serializer_nested_array() {
        #[derive(Debug, Serialize, PartialEq)]
        struct Test {
            array: Vec<Test>,
        }

        let any: Any = HashMap::from([(
            "array".to_string(),
            Any::from(vec![
                Any::from(HashMap::from([(
                    "array".to_string(),
                    Any::Array(Arc::<[Any; 0]>::default()),
                )])),
                Any::from(HashMap::from([(
                    "array".to_string(),
                    Any::Array(Arc::<[Any; 0]>::default()),
                )])),
            ]),
        )])
        .into();

        assert_eq!(
            any,
            to_any(&Test {
                array: vec![Test { array: vec![] }, Test { array: vec![] }]
            },)
            .unwrap(),
        )
    }

    #[test]
    fn test_any_serializer_newtype_u64_within_bounds() {
        #[derive(Debug, Serialize, PartialEq)]
        struct Test(u64);

        assert_eq!(
            to_any(Test(i64::MAX as u64)).unwrap(),
            Any::BigInt(i64::MAX)
        )
    }

    #[test]
    fn test_any_serializer_u64_error() {
        #[derive(Debug, Serialize, PartialEq)]
        struct Test(u64);

        assert!(matches!(
            to_any(Test(u64::MAX)).unwrap_err(),
            AnySerializeError::UnrepresentableInt
        ))
    }

    #[test]
    fn test_any_serializer_non_string_keys_error() {
        #[derive(Debug, Serialize, PartialEq)]
        struct Test(HashMap<u32, u32>);

        assert!(matches!(
            to_any(Test(HashMap::from([(100, 1)]))).unwrap_err(),
            AnySerializeError::MapKeyNotString
        ))
    }

    #[test]
    fn test_serialize_any_to_any() {
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
                    ("int".to_string(), Any::from(100)),
                    ("other".to_string(), Any::from(100.0)),
                ])),
            ),
            ("enum_a".to_string(), "VariantA".into()),
            ("enum_b".to_string(), "VariantB".into()),
        ]));

        assert_eq!(any, to_any(&any.clone()).unwrap())
    }
}
