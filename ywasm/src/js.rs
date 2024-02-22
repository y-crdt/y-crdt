use crate::array::{ArrayExt, YArray};
use crate::collection::{Integrated, SharedCollection};
use crate::Result;
use js_sys::Uint8Array;
use std::collections::HashMap;
use std::convert::TryInto;
use std::ops::Deref;
use std::sync::Arc;
use wasm_bindgen::__rt::RefMut;
use wasm_bindgen::convert::{FromWasmAbi, IntoWasmAbi, RefMutFromWasmAbi};
use wasm_bindgen::JsValue;
use yrs::block::{ItemContent, Prelim, Unused};
use yrs::branch::{Branch, BranchPtr};
use yrs::types::{
    TypeRef, TYPE_REFS_ARRAY, TYPE_REFS_MAP, TYPE_REFS_TEXT, TYPE_REFS_WEAK, TYPE_REFS_XML_ELEMENT,
    TYPE_REFS_XML_FRAGMENT, TYPE_REFS_XML_TEXT,
};
use yrs::{Any, ArrayRef, Doc, Origin, TransactionMut, Value};

#[repr(transparent)]
pub struct Js(JsValue);

impl Js {
    #[inline]
    pub fn new(js: JsValue) -> Self {
        Js(js)
    }

    pub fn from_any(any: &Any) -> Self {
        match any {
            Any::Null => Js(JsValue::NULL),
            Any::Undefined => Js(JsValue::UNDEFINED),
            Any::Bool(value) => Js(JsValue::from_bool(*value)),
            Any::Number(value) => Js(JsValue::from_f64(*value)),
            Any::BigInt(value) => Js(js_sys::BigInt::from(*value).into()),
            Any::String(str) => Js(JsValue::from_str(&*str)),
            Any::Buffer(binary) => Js(Uint8Array::from(binary.as_ref()).into()),
            Any::Array(array) => {
                let a = js_sys::Array::new();
                for any in array.iter() {
                    a.push(&Self::from_any(any).0);
                }
                Js(a.into())
            }
            Any::Map(map) => {
                let m = js_sys::Object::new();
                for (key, value) in map.iter() {
                    js_sys::Reflect::set(&m, &JsValue::from_str(&*key), &Self::from_any(value).0)
                        .unwrap();
                }
                Js(m.into())
            }
        }
    }

    pub fn from_value(value: &Value, doc: &Doc) -> Self {
        match value {
            Value::Any(any) => Self::from_any(any),
            Value::YArray(c) => {
                let c = YArray(SharedCollection::integrated(c.clone(), doc.clone()));
                Js(c.into())
            }
            _ => unreachable!(), //Value::YText(c) => {}
                                 //Value::YMap(c) => {}
                                 //Value::YXmlElement(c) => {}
                                 //Value::YXmlFragment(c) => {}
                                 //Value::YXmlText(c) => {}
                                 //Value::YDoc(c) => {}
                                 //Value::YWeakLink(c) => {}
                                 //Value::UndefinedRef(c) => {}
        }
    }

    pub fn as_value(&self) -> Result<ValueRef> {
        if let Some(str) = self.0.as_string() {
            Ok(ValueRef::Any(Any::from(str)))
        } else if self.0.is_null() {
            Ok(ValueRef::Any(Any::Null))
        } else if self.0.is_undefined() {
            Ok(ValueRef::Any(Any::Undefined))
        } else if let Some(f) = self.0.as_f64() {
            Ok(ValueRef::Any(Any::Number(f)))
        } else if let Some(b) = self.0.as_bool() {
            Ok(ValueRef::Any(Any::Bool(b)))
        } else if self.0.is_bigint() {
            let i = js_sys::BigInt::from(self.0.clone()).as_f64().unwrap();
            Ok(ValueRef::Any(Any::BigInt(i as i64)))
        } else if js_sys::Array::is_array(&self.0) {
            let array = js_sys::Array::from(&self.0);
            let mut result = Vec::with_capacity(array.length() as usize);
            for value in array.iter() {
                let js = Js::from(value);
                if let ValueRef::Any(any) = js.as_value()? {
                    result.push(any);
                } else {
                    return Err(js.0);
                }
            }
            Ok(ValueRef::Any(Any::Array(result.into())))
        } else if self.0.is_object() {
            if let Ok(shared) = Shared::from_ref(&self.0) {
                Ok(ValueRef::Shared(shared))
            } else {
                let mut map = HashMap::new();
                let object = js_sys::Object::from(self.0.clone());
                let entries = js_sys::Object::entries(&object);
                for tuple in entries.iter() {
                    let tuple = js_sys::Array::from(&tuple);
                    let key: String = if let Some(key) = tuple.get(0).as_string() {
                        key
                    } else {
                        return Err(JsValue::from_str("object field name is not string"));
                    };
                    let value = tuple.get(1);
                    let js = Js(value.clone());
                    if let ValueRef::Any(any) = js.as_value()? {
                        map.insert(key, any);
                    } else {
                        return Err(value);
                    }
                }
                Ok(ValueRef::Any(Any::Map(Arc::new(map))))
            }
        } else {
            Err(self.0.clone())
        }
    }
}

impl Deref for Js {
    type Target = JsValue;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<JsValue> for Js {
    fn as_ref(&self) -> &JsValue {
        &self.0
    }
}

impl From<JsValue> for Js {
    #[inline]
    fn from(value: JsValue) -> Self {
        Js(value)
    }
}

impl Into<JsValue> for Js {
    #[inline]
    fn into(self) -> JsValue {
        self.0
    }
}

impl Into<Origin> for Js {
    fn into(self) -> Origin {
        if let Some(js_str) = self.0.as_string() {
            Origin::from(js_str)
        } else {
            let abi = self.0.into_abi();
            Origin::from(abi)
        }
    }
}

impl<'a> From<&'a Origin> for Js {
    fn from(value: &'a Origin) -> Self {
        let bytes = value.as_ref();
        match bytes.len() {
            0 => Js(JsValue::UNDEFINED),
            4 => {
                let abi = u32::from_be_bytes(bytes.try_into().unwrap());
                Js(unsafe { JsValue::from_abi(abi) })
            }
            _ => Js(JsValue::from_str(unsafe {
                std::str::from_utf8_unchecked(bytes)
            })),
        }
    }
}

impl Prelim for Js {
    type Return = Unused;

    fn into_content(self, txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        todo!()
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        todo!()
    }
}

pub enum ValueRef {
    Any(Any),
    Shared(Shared),
    Doc(Doc),
}

impl ValueRef {
    pub fn any(self) -> Option<Any> {
        if let ValueRef::Any(any) = self {
            Some(any)
        } else {
            None
        }
    }
}

pub enum Shared {
    Array(RefMut<'static, YArray>),
}

impl Shared {
    pub fn from_ref(js: &JsValue) -> Result<Self> {
        let tag = js_sys::Reflect::get(js, &JsValue::from_str("type"))?;
        if let Some(tag) = tag.as_f64() {
            match tag as u8 {
                TYPE_REFS_ARRAY => Ok(Shared::Array(unsafe {
                    YArray::ref_mut_from_abi(js.into_abi())
                })),
                TYPE_REFS_TEXT
                | TYPE_REFS_MAP
                | TYPE_REFS_XML_TEXT
                | TYPE_REFS_XML_ELEMENT
                | TYPE_REFS_XML_FRAGMENT
                | TYPE_REFS_WEAK
                | _ => Err(js.clone()),
            }
        } else {
            Err(js.clone())
        }
    }

    pub fn prelim(&self) -> bool {
        match self {
            Shared::Array(v) => v.prelim(),
        }
    }

    fn type_ref(&self) -> TypeRef {
        match self {
            Shared::Array(_) => TypeRef::Array,
        }
    }
}

impl Prelim for Shared {
    type Return = Unused;

    fn into_content(self, txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let type_ref = self.type_ref();
        let branch = Branch::new(type_ref);
        (ItemContent::Type(branch), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        let doc = txn.doc().clone();
        match self {
            Shared::Array(mut cell) => {
                let array = ArrayRef::from(inner_ref);
                if let YArray(SharedCollection::Prelim(raw)) = std::mem::replace(
                    &mut *cell,
                    YArray(SharedCollection::Integrated(Integrated::new(
                        array.clone(),
                        doc,
                    ))),
                ) {
                    array.insert_at(txn, 0, raw).unwrap();
                }
            }
        }
    }
}

pub(crate) mod convert {
    use crate::array::YArrayEvent;
    use crate::js::Js;
    use wasm_bindgen::JsValue;
    use yrs::types::{Change, Event, Events, Path, PathSegment};
    use yrs::{Doc, TransactionMut};

    pub fn change_into_js(change: &Change, doc: &Doc) -> JsValue {
        let result = js_sys::Object::new();
        match change {
            Change::Added(values) => {
                let mut array = js_sys::Array::new();
                array.extend(values.iter().map(|v| Js::from_value(v, doc)));
                js_sys::Reflect::set(&result, &JsValue::from("insert"), &array).unwrap();
            }
            Change::Removed(len) => {
                let value = JsValue::from(*len);
                js_sys::Reflect::set(&result, &JsValue::from("delete"), &value).unwrap();
            }
            Change::Retain(len) => {
                let value = JsValue::from(*len);
                js_sys::Reflect::set(&result, &JsValue::from("retain"), &value).unwrap();
            }
        }
        result.into()
    }

    pub fn path_into_js(path: Path) -> JsValue {
        let result = js_sys::Array::new();
        for segment in path {
            match segment {
                PathSegment::Key(key) => {
                    result.push(&JsValue::from(key.as_ref()));
                }
                PathSegment::Index(idx) => {
                    result.push(&JsValue::from(idx));
                }
            }
        }
        result.into()
    }

    pub fn events_into_js(txn: &TransactionMut, e: &Events) -> JsValue {
        let mut array = js_sys::Array::new();
        let mapped = e.iter().map(|e| {
            let js: JsValue = match e {
                Event::Array(e) => YArrayEvent::new(e, txn).into(),
                _ => todo!(), //Event::Text(e) => YTextEvent::new(e, txn).into(),
                              //Event::Map(e) => YMapEvent::new(e, txn).into(),
                              //Event::XmlText(e) => YXmlTextEvent::new(e, txn).into(),
                              //Event::XmlFragment(e) => YXmlEvent::new(e, txn).into(),
                              //Event::Weak(e) => YWeakLinkEvent::new(e, txn).into(),
            };
            js
        });
        array.extend(mapped);
        array.into()
    }
}
