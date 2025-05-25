use crate::array::{ArrayExt, YArray};
use crate::collection::{Integrated, SharedCollection};
use crate::doc::YDoc;
use crate::map::YMap;
use crate::text::YText;
use crate::weak::YWeakLink;
use crate::xml_elem::YXmlElement;
use crate::xml_frag::YXmlFragment;
use crate::xml_text::YXmlText;
use crate::Result;
use js_sys::Uint8Array;
use std::collections::{Bound, HashMap};
use std::convert::TryInto;
use std::ops::{Deref, RangeBounds};
use std::sync::Arc;
use wasm_bindgen::__rt::RcRefMut;
use wasm_bindgen::convert::{FromWasmAbi, IntoWasmAbi};
use wasm_bindgen::JsValue;
use yrs::block::{EmbedPrelim, ItemContent, Prelim, Unused};
use yrs::branch::{Branch, BranchPtr};
use yrs::types::xml::XmlPrelim;
use yrs::types::{
    TypeRef, TYPE_REFS_ARRAY, TYPE_REFS_DOC, TYPE_REFS_MAP, TYPE_REFS_TEXT, TYPE_REFS_WEAK,
    TYPE_REFS_XML_ELEMENT, TYPE_REFS_XML_FRAGMENT, TYPE_REFS_XML_TEXT,
};
use yrs::{
    Any, ArrayRef, BranchID, Doc, Map, MapRef, Origin, Out, Text, TextRef, TransactionMut, WeakRef,
    Xml, XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextRef,
};

#[repr(transparent)]
pub struct Js(JsValue);

impl Js {
    #[inline]
    pub fn new(js: JsValue) -> Self {
        Js(js)
    }

    pub fn assert_xml_prelim(xml_node: &JsValue) -> crate::Result<()> {
        match Js::get_type(&xml_node)? {
            TYPE_REFS_XML_ELEMENT | TYPE_REFS_XML_TEXT => { /* ok */ }
            _ => return Err(JsValue::from_str(crate::js::errors::NOT_XML_TYPE)),
        }
        let is_prelim = js_sys::Reflect::get(&xml_node, &JsValue::from_str("prelim"))?;
        if is_prelim.as_bool() != Some(true) {
            return Err(JsValue::from_str(crate::js::errors::NOT_PRELIM));
        }
        Ok(())
    }

    pub fn get_type(js: &JsValue) -> crate::Result<u8> {
        let tag = js_sys::Reflect::get(js, &JsValue::from_str("type"))?;
        if let Some(tag) = tag.as_f64() {
            Ok(tag as u8)
        } else {
            Err(js.clone())
        }
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

    pub fn from_xml(value: XmlOut, doc: Doc) -> Self {
        Js(match value {
            XmlOut::Element(v) => YXmlElement(SharedCollection::integrated(v, doc)).into(),
            XmlOut::Fragment(v) => YXmlFragment(SharedCollection::integrated(v, doc)).into(),
            XmlOut::Text(v) => YXmlText(SharedCollection::integrated(v, doc)).into(),
        })
    }

    pub fn from_value(value: &Out, doc: &Doc) -> Self {
        match value {
            Out::Any(any) => Self::from_any(any),
            Out::YText(c) => Js(YText(SharedCollection::integrated(c.clone(), doc.clone())).into()),
            Out::YMap(c) => Js(YMap(SharedCollection::integrated(c.clone(), doc.clone())).into()),
            Out::YArray(c) => {
                Js(YArray(SharedCollection::integrated(c.clone(), doc.clone())).into())
            }
            Out::YDoc(doc) => Js(YDoc(doc.clone()).into()),
            Out::YWeakLink(c) => {
                Js(YWeakLink(SharedCollection::integrated(c.clone(), doc.clone())).into())
            }
            Out::YXmlElement(c) => {
                Js(YXmlElement(SharedCollection::integrated(c.clone(), doc.clone())).into())
            }
            Out::YXmlFragment(c) => {
                Js(YXmlFragment(SharedCollection::integrated(c.clone(), doc.clone())).into())
            }
            Out::YXmlText(c) => {
                Js(YXmlText(SharedCollection::integrated(c.clone(), doc.clone())).into())
            }
            Out::UndefinedRef(_) => Js(JsValue::UNDEFINED),
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
    #[inline]
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

impl XmlPrelim for Js {}

impl Prelim for Js {
    type Return = Unused;

    fn into_content(self, txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        match self.as_value().unwrap() {
            ValueRef::Any(any) => (ItemContent::Any(vec![any]), None),
            ValueRef::Shared(shared) => {
                match &shared {
                    Shared::Weak(_) => { /* WeakRefs can always be integrated */ }
                    Shared::Doc(doc) if doc.prelim() => {
                        return (ItemContent::Doc(None, doc.0.clone()), None);
                    }
                    other if !other.prelim() => {
                        panic!("{}", crate::js::errors::NOT_PRELIM);
                    }
                    _ => { /* good to go */ }
                }
                let type_ref = shared.type_ref(txn);
                let branch = Branch::new(type_ref);
                (ItemContent::Type(branch), Some(self))
            }
        }
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        match self.as_value().unwrap() {
            ValueRef::Any(_) => { /* nothing to do */ }
            ValueRef::Shared(shared) => shared.integrate(txn, inner_ref),
        }
    }
}

impl Into<EmbedPrelim<Js>> for Js {
    fn into(self) -> EmbedPrelim<Js> {
        match self.as_value().unwrap() {
            ValueRef::Any(any) => EmbedPrelim::Primitive(any),
            ValueRef::Shared(_) => EmbedPrelim::Shared(self),
        }
    }
}

pub enum ValueRef {
    Any(Any),
    Shared(Shared),
}

pub enum Shared {
    Text(RcRefMut<YText>),
    Map(RcRefMut<YMap>),
    Array(RcRefMut<YArray>),
    Weak(RcRefMut<YWeakLink>),
    XmlText(RcRefMut<YXmlText>),
    XmlElement(RcRefMut<YXmlElement>),
    XmlFragment(RcRefMut<YXmlFragment>),
    Doc(RcRefMut<YDoc>),
}

impl Shared {
    pub fn from_ref(js: &JsValue) -> Result<Self> {
        let tag = Js::get_type(js)?;
        match tag as u8 {
            TYPE_REFS_TEXT => Ok(Shared::Text(convert::mut_from_js::<YText>(js)?)),
            TYPE_REFS_MAP => Ok(Shared::Map(convert::mut_from_js::<YMap>(js)?)),
            TYPE_REFS_ARRAY => Ok(Shared::Array(convert::mut_from_js::<YArray>(js)?)),
            TYPE_REFS_XML_TEXT => Ok(Shared::XmlText(convert::mut_from_js::<YXmlText>(js)?)),
            TYPE_REFS_XML_ELEMENT => {
                Ok(Shared::XmlElement(convert::mut_from_js::<YXmlElement>(js)?))
            }
            TYPE_REFS_XML_FRAGMENT => Ok(Shared::XmlFragment(
                convert::mut_from_js::<YXmlFragment>(js)?,
            )),
            TYPE_REFS_WEAK => Ok(Shared::Weak(convert::mut_from_js::<YWeakLink>(js)?)),
            TYPE_REFS_DOC => Ok(Shared::Doc(convert::mut_from_js::<YDoc>(js)?)),
            _ => Err(js.clone()),
        }
    }

    pub fn prelim(&self) -> bool {
        match self {
            Shared::Text(v) => v.prelim(),
            Shared::Map(v) => v.prelim(),
            Shared::Array(v) => v.prelim(),
            Shared::Weak(v) => v.prelim(),
            Shared::XmlText(v) => v.prelim(),
            Shared::XmlElement(v) => v.prelim(),
            Shared::XmlFragment(v) => v.prelim(),
            Shared::Doc(v) => v.prelim(),
        }
    }

    pub fn branch_id(&self) -> Option<&BranchID> {
        match self {
            Shared::Text(v) => v.0.branch_id(),
            Shared::Map(v) => v.0.branch_id(),
            Shared::Array(v) => v.0.branch_id(),
            Shared::Weak(v) => v.0.branch_id(),
            Shared::XmlText(v) => v.0.branch_id(),
            Shared::XmlElement(v) => v.0.branch_id(),
            Shared::XmlFragment(v) => v.0.branch_id(),
            Shared::Doc(_) => None,
        }
    }

    pub fn try_integrated(&self) -> Result<(&BranchID, &Doc)> {
        match self {
            Shared::Text(v) => v.0.try_integrated(),
            Shared::Map(v) => v.0.try_integrated(),
            Shared::Array(v) => v.0.try_integrated(),
            Shared::Weak(v) => v.0.try_integrated(),
            Shared::XmlText(v) => v.0.try_integrated(),
            Shared::XmlElement(v) => v.0.try_integrated(),
            Shared::XmlFragment(v) => v.0.try_integrated(),
            Shared::Doc(_) => Err(JsValue::from_str(crate::js::errors::NOT_WASM_OBJ)),
        }
    }

    fn type_ref(&self, txn: &TransactionMut) -> TypeRef {
        match self {
            Shared::Text(_) => TypeRef::Text,
            Shared::Map(_) => TypeRef::Map,
            Shared::Array(_) => TypeRef::Array,
            Shared::XmlText(_) => TypeRef::XmlText,
            Shared::XmlFragment(_) => TypeRef::XmlFragment,
            Shared::Doc(_) => TypeRef::SubDoc,
            Shared::Weak(v) => TypeRef::WeakLink(v.source(txn)),
            Shared::XmlElement(v) => {
                let name = match &v.0 {
                    SharedCollection::Integrated(_) => panic!("{}", crate::js::errors::NOT_PRELIM),
                    SharedCollection::Prelim(p) => Arc::from(p.name.as_str()),
                };
                TypeRef::XmlElement(name)
            }
        }
    }
}

impl Prelim for Shared {
    type Return = Unused;

    fn into_content(self, txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let type_ref = self.type_ref(txn);
        let branch = Branch::new(type_ref);
        (ItemContent::Type(branch), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        let doc = txn.doc().clone();
        match self {
            Shared::Text(mut cell) => {
                let text = TextRef::from(inner_ref);
                if let YText(SharedCollection::Prelim(raw)) = std::mem::replace(
                    &mut *cell,
                    YText(SharedCollection::Integrated(Integrated::new(
                        text.clone(),
                        doc,
                    ))),
                ) {
                    text.insert(txn, 0, &raw);
                }
            }
            Shared::Map(mut cell) => {
                let map = MapRef::from(inner_ref);
                if let YMap(SharedCollection::Prelim(raw)) = std::mem::replace(
                    &mut *cell,
                    YMap(SharedCollection::Integrated(Integrated::new(
                        map.clone(),
                        doc,
                    ))),
                ) {
                    for (key, js_val) in raw {
                        map.insert(txn, key, Js::new(js_val));
                    }
                }
            }
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
            Shared::XmlText(mut cell) => {
                let xml_text = XmlTextRef::from(inner_ref);
                if let YXmlText(SharedCollection::Prelim(raw)) = std::mem::replace(
                    &mut *cell,
                    YXmlText(SharedCollection::Integrated(Integrated::new(
                        xml_text.clone(),
                        doc,
                    ))),
                ) {
                    xml_text.insert(txn, 0, &raw.text);
                    for (name, value) in raw.attributes {
                        xml_text.insert_attribute(txn, name, value);
                    }
                }
            }
            Shared::XmlElement(mut cell) => {
                let xml_element = XmlElementRef::from(inner_ref);
                if let YXmlElement(SharedCollection::Prelim(raw)) = std::mem::replace(
                    &mut *cell,
                    YXmlElement(SharedCollection::Integrated(Integrated::new(
                        xml_element.clone(),
                        doc,
                    ))),
                ) {
                    for child in raw.children {
                        xml_element.push_back(txn, Js::new(child));
                    }
                    for (name, value) in raw.attributes {
                        xml_element.insert_attribute(txn, name, value);
                    }
                }
            }
            Shared::XmlFragment(mut cell) => {
                let xml_fragment = XmlFragmentRef::from(inner_ref);
                if let YXmlFragment(SharedCollection::Prelim(raw)) = std::mem::replace(
                    &mut *cell,
                    YXmlFragment(SharedCollection::Integrated(Integrated::new(
                        xml_fragment.clone(),
                        doc,
                    ))),
                ) {
                    for child in raw {
                        xml_fragment.push_back(txn, Js::new(child));
                    }
                }
            }
            Shared::Weak(mut cell) => {
                let weak_link: WeakRef<BranchPtr> = WeakRef::from(inner_ref);
                let _ = std::mem::replace(
                    &mut *cell,
                    YWeakLink(SharedCollection::Integrated(Integrated::new(
                        weak_link.clone(),
                        doc,
                    ))),
                );
            }
            Shared::Doc(_) => { /* do nothing */ }
        }
    }
}

pub(crate) struct YRange {
    lower: Option<u32>,
    upper: Option<u32>,
    lower_open: bool,
    upper_open: bool,
}

impl YRange {
    #[inline]
    pub fn new(
        lower: Option<u32>,
        upper: Option<u32>,
        lower_open: Option<bool>,
        upper_open: Option<bool>,
    ) -> Self {
        YRange {
            lower,
            upper,
            lower_open: lower_open.unwrap_or(false),
            upper_open: upper_open.unwrap_or(false),
        }
    }
}

impl RangeBounds<u32> for YRange {
    fn start_bound(&self) -> Bound<&u32> {
        match (&self.lower, self.lower_open) {
            (None, _) => Bound::Unbounded,
            (Some(i), true) => Bound::Excluded(i),
            (Some(i), false) => Bound::Included(i),
        }
    }

    fn end_bound(&self) -> Bound<&u32> {
        match (&self.upper, self.upper_open) {
            (None, _) => Bound::Unbounded,
            (Some(i), true) => Bound::Excluded(i),
            (Some(i), false) => Bound::Included(i),
        }
    }
}

pub(crate) const JS_ORIGIN: &'static str = "__subscription_key";
pub(crate) const JS_PTR: &'static str = "__wbg_ptr";

pub trait Callback: AsRef<JsValue> {
    fn subscription_key(&self) -> u32 {
        let js: &JsValue = self.as_ref();
        let origin_field = JsValue::from_str(JS_ORIGIN);
        let abi = js_sys::Reflect::get(js, &origin_field)
            .ok()
            .and_then(|v| v.as_f64());
        match abi {
            Some(abi) => abi as u32,
            None => {
                let abi = js.into_abi();
                js_sys::Reflect::set(js, &origin_field, &JsValue::from(abi)).unwrap();
                abi
            }
        }
    }
}

impl Callback for js_sys::Function {}

pub(crate) mod convert {
    use crate::array::YArrayEvent;
    use crate::js::errors::INVALID_DELTA;
    use crate::js::Js;
    use crate::map::YMapEvent;
    use crate::text::YTextEvent;
    use crate::weak::YWeakLinkEvent;
    use crate::xml_frag::YXmlEvent;
    use crate::xml_text::YXmlTextEvent;
    use crate::Text;
    use gloo_utils::format::JsValueSerdeExt;
    use std::iter::FromIterator;
    use wasm_bindgen::convert::RefMutFromWasmAbi;
    use wasm_bindgen::JsValue;
    use yrs::types::text::{ChangeKind, Diff, YChange};
    use yrs::types::{Change, Delta, EntryChange, Event, Events, Path, PathSegment};
    use yrs::updates::decoder::Decode;
    use yrs::{DeleteSet, Doc, StateVector, TransactionMut};

    pub fn js_into_delta(js: JsValue) -> crate::Result<Delta<Js>> {
        let attributes = js_sys::Reflect::get(&js, &JsValue::from("attributes"));
        if let Ok(insert) = js_sys::Reflect::get(&js, &JsValue::from("insert")) {
            if !insert.is_undefined() {
                let attrs = Text::parse_fmt(attributes.unwrap_or(JsValue::UNDEFINED)).map(Box::new);
                return Ok(Delta::Inserted(Js(insert), attrs));
            }
        }
        if let Ok(delete) = js_sys::Reflect::get(&js, &JsValue::from("delete")) {
            if let Some(len) = delete.as_f64() {
                return Ok(Delta::Deleted(len as u32));
            }
        }
        if let Ok(retain) = js_sys::Reflect::get(&js, &JsValue::from("retain")) {
            if let Some(len) = retain.as_f64() {
                let attrs = Text::parse_fmt(attributes.unwrap_or(JsValue::UNDEFINED)).map(Box::new);
                return Ok(Delta::Retain(len as u32, attrs));
            }
        }
        Err(JsValue::from_str(INVALID_DELTA))
    }

    pub fn mut_from_js<T>(js: &JsValue) -> crate::Result<T::Anchor>
    where
        T: RefMutFromWasmAbi<Abi = u32>,
    {
        let ptr = js_sys::Reflect::get(&js, &JsValue::from_str(crate::js::JS_PTR))?;
        let ptr_u32 =
            ptr.as_f64()
                .ok_or(JsValue::from_str(crate::js::errors::NOT_WASM_OBJ))? as u32;
        let target = unsafe { T::ref_mut_from_abi(ptr_u32) };
        Ok(target)
    }

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

    pub fn entry_change_into_js(change: &EntryChange, doc: &Doc) -> crate::Result<JsValue> {
        let result = js_sys::Object::new();
        let action = JsValue::from("action");
        match change {
            EntryChange::Inserted(new) => {
                let new_value = Js::from_value(new, doc).into();
                js_sys::Reflect::set(&result, &action, &JsValue::from("add"))?;
                js_sys::Reflect::set(&result, &JsValue::from("newValue"), &new_value)?;
            }
            EntryChange::Updated(old, new) => {
                let old_value = Js::from_value(old, doc).into();
                let new_value = Js::from_value(new, doc).into();
                js_sys::Reflect::set(&result, &action, &JsValue::from("update"))?;
                js_sys::Reflect::set(&result, &JsValue::from("oldValue"), &old_value)?;
                js_sys::Reflect::set(&result, &JsValue::from("newValue"), &new_value)?;
            }
            EntryChange::Removed(old) => {
                let old_value = Js::from_value(old, doc).into();
                js_sys::Reflect::set(&result, &action, &JsValue::from("delete"))?;
                js_sys::Reflect::set(&result, &JsValue::from("oldValue"), &old_value)?;
            }
        }
        Ok(result.into())
    }

    pub fn text_delta_into_js(delta: &Delta, doc: &Doc) -> crate::Result<JsValue> {
        let result = js_sys::Object::new();
        match delta {
            Delta::Inserted(value, attrs) => {
                js_sys::Reflect::set(
                    &result,
                    &JsValue::from("insert"),
                    &Js::from_value(value, doc).into(),
                )?;

                if let Some(attrs) = attrs {
                    let attrs = JsValue::from_serde(attrs)
                        .map_err(|e| JsValue::from_str(&e.to_string()))?;
                    js_sys::Reflect::set(&result, &JsValue::from("attributes"), &attrs)?;
                }
            }
            Delta::Retain(len, attrs) => {
                let value = JsValue::from(*len);
                js_sys::Reflect::set(&result, &JsValue::from("retain"), &value)?;

                if let Some(attrs) = attrs {
                    let attrs = JsValue::from_serde(&attrs)
                        .map_err(|e| JsValue::from_str(&e.to_string()))?;
                    js_sys::Reflect::set(&result, &JsValue::from("attributes"), &attrs)?;
                }
            }
            Delta::Deleted(len) => {
                let value = JsValue::from(*len);
                js_sys::Reflect::set(&result, &JsValue::from("delete"), &value)?;
            }
        }
        Ok(result.into())
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
                Event::Text(e) => YTextEvent::new(e, txn).into(),
                Event::Map(e) => YMapEvent::new(e, txn).into(),
                Event::Array(e) => YArrayEvent::new(e, txn).into(),
                Event::Weak(e) => YWeakLinkEvent::new(e, txn).into(),
                Event::XmlFragment(e) => YXmlEvent::new(e, txn).into(),
                Event::XmlText(e) => YXmlTextEvent::new(e, txn).into(),
            };
            js
        });
        array.extend(mapped);
        array.into()
    }

    pub fn state_vector_from_js(
        vector: Option<js_sys::Uint8Array>,
    ) -> crate::Result<Option<StateVector>> {
        if let Some(vector) = vector {
            match StateVector::decode_v1(vector.to_vec().as_slice()) {
                Ok(sv) => Ok(Some(sv)),
                Err(e) => {
                    return Err(JsValue::from(e.to_string()));
                }
            }
        } else {
            Ok(None)
        }
    }

    pub fn state_vector_to_js(sv: &StateVector) -> js_sys::Map {
        let map = js_sys::Map::new();
        for (&client_id, &clock) in sv.iter() {
            map.set(
                &JsValue::from_f64(client_id as f64),
                &JsValue::from_f64(clock as f64),
            );
        }
        map
    }

    pub fn delete_set_to_js(ds: &DeleteSet) -> js_sys::Map {
        let map = js_sys::Map::new();
        for (&client_id, range) in ds.iter() {
            let r = js_sys::Array::new();
            for segment in range.iter() {
                let start = JsValue::from_f64(segment.start as f64);
                let end = JsValue::from_f64((segment.end - segment.start) as f64);
                let segment = js_sys::Array::from_iter([start, end]);
                r.push(&segment.into());
            }
            map.set(&JsValue::from_f64(client_id as f64), &r.into());
        }
        map
    }

    pub fn ychange_to_js(
        change: YChange,
        compute_ychange: &Option<js_sys::Function>,
    ) -> crate::Result<JsValue> {
        let kind = match change.kind {
            ChangeKind::Added => JsValue::from("added"),
            ChangeKind::Removed => JsValue::from("removed"),
        };
        let result = if let Some(func) = compute_ychange {
            let id =
                JsValue::from_serde(&change.id).map_err(|e| JsValue::from_str(&e.to_string()))?;
            func.call2(&JsValue::UNDEFINED, &kind, &id).unwrap()
        } else {
            let js: JsValue = js_sys::Object::new().into();
            js_sys::Reflect::set(&js, &JsValue::from("type"), &kind).unwrap();
            js
        };
        Ok(result)
    }

    pub fn diff_into_js(diff: Diff<JsValue>, doc: &Doc) -> crate::Result<JsValue> {
        let delta = Delta::Inserted(diff.insert, diff.attributes);
        let js = text_delta_into_js(&delta, doc)?;
        if let Some(ychange) = diff.ychange {
            let attrs = match js_sys::Reflect::get(&js, &JsValue::from("attributes")) {
                Ok(attrs) if attrs.is_object() => attrs,
                _ => {
                    let attrs: JsValue = js_sys::Object::new().into();
                    js_sys::Reflect::set(&js, &JsValue::from("attributes"), &attrs).unwrap();
                    attrs
                }
            };
            js_sys::Reflect::set(&attrs, &JsValue::from("ychange"), &ychange).unwrap();
        }
        Ok(js)
    }
}

pub(crate) mod errors {
    pub const NON_TRANSACTION: &'static str = "provided argument was not a ywasm transaction";
    pub const INVALID_TRANSACTION_CTX: &'static str = "cannot modify transaction in this context";
    pub const REF_DISPOSED: &'static str = "shared collection has been destroyed";
    pub const ANOTHER_TX: &'static str = "another transaction is in progress";
    pub const ANOTHER_RW_TX: &'static str = "another read-write transaction is in progress";
    pub const OUT_OF_BOUNDS: &'static str = "index outside of the bounds of an array";
    pub const KEY_NOT_FOUND: &'static str = "key was not found in a map";
    pub const INVALID_PRELIM_OP: &'static str = "preliminary type doesn't support this operation";
    pub const INVALID_FMT: &'static str = "given object cannot be used as formatting attributes";
    pub const INVALID_XML_ATTRS: &'static str = "given object cannot be used as XML attributes";
    pub const NOT_XML_TYPE: &'static str = "provided object is not a valid XML shared type";
    pub const NOT_PRELIM: &'static str = "this operation only works on preliminary types";
    pub const NOT_WASM_OBJ: &'static str = "provided reference is not a WebAssembly object";
    pub const INVALID_DELTA: &'static str = "invalid delta format";
}
