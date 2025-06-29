use std::collections::HashMap;
use std::sync::Arc;
use js_sys::Object;
use crate::js::{Js, ValueRef};
use wasm_bindgen::JsValue;
use yrs::Any;
use yrs::types::Attrs;

pub struct XmlAttrs;

impl XmlAttrs {
    pub(crate) fn from_attrs(attributes: HashMap<Arc<str>, Any>) -> Object {
        let map = js_sys::Object::new();
        for (name, value) in &attributes {
            js_sys::Reflect::set(
                &map,
                &JsValue::from_str(name),
                &Js::from_any(value).into(),
            ).unwrap();
        }

        map
    }

    pub(crate) fn parse_attrs_any(attributes: JsValue) -> crate::Result<Attrs> {
        if attributes.is_undefined() || attributes.is_null() {
            Ok(Attrs::new())
        } else if attributes.is_object() {
            let mut map = Attrs::new();
            let object = js_sys::Object::from(attributes);
            let entries = js_sys::Object::entries(&object);
            for tuple in entries.iter() {
                let tuple = js_sys::Array::from(&tuple);
                if let Some(key) = tuple.get(0).as_string() {
                    let value = Js::new(tuple.get(1));
                    if let Ok(ValueRef::Any(any)) = value.as_value() {
                        map.insert(key.into(), any);
                    } else {
                        return Err(JsValue::from_str(crate::js::errors::INVALID_XML_ATTRS));
                    }
                } else {
                    return Err(JsValue::from_str(crate::js::errors::INVALID_XML_ATTRS));
                }
            }
            Ok(map)
        } else {
            Err(JsValue::from_str(crate::js::errors::INVALID_XML_ATTRS))
        }
    }
}
