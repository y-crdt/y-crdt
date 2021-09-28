use js_sys::{Object, Uint8Array};
use lib0::any::Any;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fmt::Write;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use wasm_bindgen::convert::RefFromWasmAbi;
use wasm_bindgen::describe::WasmDescribe;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::block::{ItemContent, Prelim};
use yrs::types::array::ArrayIter;
use yrs::types::map::MapIter;
use yrs::types::xml::{Attributes, TreeWalker};
use yrs::types::{BranchRef, TypePtr, Value};
use yrs::updates::decoder::{Decode, DecoderV1};
use yrs::updates::encoder::{Encode, Encoder, EncoderV1};
use yrs::{
    Array, DeleteSet, Doc, Map, StateVector, Text, Transaction, Update, Xml, XmlElement, XmlText,
};

// When the `wee_alloc` feature is enabled, use `wee_alloc` as the global
// allocator.
#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[wasm_bindgen]
pub struct YDoc(Doc);

#[wasm_bindgen]
impl YDoc {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        YDoc(Doc::new())
    }

    #[wasm_bindgen(constructor)]
    pub fn new_with_id(id: u64) -> Self {
        YDoc(Doc::with_client_id(id))
    }

    #[wasm_bindgen(method, getter)]
    pub fn id(&self) -> u64 {
        self.0.client_id
    }

    #[wasm_bindgen]
    pub fn transact(&mut self) -> YTransaction {
        unsafe {
            let doc: *mut Doc = &mut self.0;
            let static_txn: ManuallyDrop<Transaction<'static>> =
                ManuallyDrop::new((*doc).transact());
            YTransaction(static_txn)
        }
    }
}

#[wasm_bindgen]
pub struct YTransaction(ManuallyDrop<Transaction<'static>>);

impl Deref for YTransaction {
    type Target = Transaction<'static>;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl DerefMut for YTransaction {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}

impl Drop for YTransaction {
    fn drop(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.0) }
    }
}

#[wasm_bindgen]
impl YTransaction {
    #[wasm_bindgen(js_name = getText)]
    pub fn get_text(&mut self, name: &str) -> YText {
        YText(SharedType::Integrated(self.0.get_text(name)))
    }

    #[wasm_bindgen(js_name = getArray)]
    pub fn get_array(&mut self, name: &str) -> YArray {
        YArray(SharedType::Integrated(self.0.get_array(name)))
    }

    #[wasm_bindgen(js_name = getMap)]
    pub fn get_map(&mut self, name: &str) -> YMap {
        YMap(SharedType::Integrated(self.0.get_map(name)))
    }

    #[wasm_bindgen(js_name = getXmlElement)]
    pub fn get_xml_element(&mut self, name: &str) -> YXmlElement {
        YXmlElement(self.0.get_xml_element(name))
    }

    #[wasm_bindgen(js_name = getXmlText)]
    pub fn get_xml_text(&mut self, name: &str) -> YXmlText {
        YXmlText(self.0.get_xml_text(name))
    }

    #[wasm_bindgen(js_name = commit)]
    pub fn commit(&mut self) {
        self.0.commit()
    }

    #[wasm_bindgen(js_name = stateVectorV1)]
    pub fn state_vector_v1(&self) -> Uint8Array {
        let sv = self.0.state_vector();
        let payload = sv.encode_v1();
        Uint8Array::from(&payload[..payload.len()])
    }

    #[wasm_bindgen(js_name = diffV1)]
    pub fn diff_v1(&self, vector: Uint8Array) -> Uint8Array {
        let mut encoder = EncoderV1::new();
        let sv = StateVector::decode_v1(vector.to_vec().as_slice());
        self.0.encode_diff(&sv, &mut encoder);
        let payload = encoder.to_vec();
        Uint8Array::from(&payload[..payload.len()])
    }

    #[wasm_bindgen(js_name = applyV1)]
    pub fn apply_v1(&mut self, diff: Uint8Array) {
        let diff: Vec<u8> = diff.to_vec();
        let mut decoder = DecoderV1::from(diff.as_slice());
        let update = Update::decode(&mut decoder);
        let ds = DeleteSet::decode(&mut decoder);
        self.0.apply_update(update, ds)
    }
}

enum SharedType<T, U> {
    Integrated(T),
    Prelim(U),
}

#[wasm_bindgen]
pub struct YText(SharedType<Text, String>);

impl From<Text> for YText {
    fn from(v: Text) -> Self {
        YText(SharedType::Integrated(v))
    }
}

#[wasm_bindgen]
impl YText {
    #[wasm_bindgen(constructor)]
    pub fn new(str: &str) -> Self {
        YText(SharedType::Prelim(str.to_string()))
    }

    #[wasm_bindgen(method, getter)]
    pub fn length(&self) -> u32 {
        match &self.0 {
            SharedType::Integrated(v) => v.len(),
            SharedType::Prelim(v) => v.len() as u32,
        }
    }

    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &YTransaction) -> String {
        match &self.0 {
            SharedType::Integrated(v) => v.to_string(txn),
            SharedType::Prelim(v) => v.clone(),
        }
    }

    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &YTransaction) -> JsValue {
        match &self.0 {
            SharedType::Integrated(v) => {
                let str = v.to_string(txn);
                JsValue::from_str(str.as_str())
            }
            SharedType::Prelim(v) => JsValue::from_str(v.as_str()),
        }
    }

    #[wasm_bindgen(js_name = insert)]
    pub fn insert(&mut self, txn: &mut YTransaction, index: u32, chunk: &str) {
        match &mut self.0 {
            SharedType::Integrated(v) => v.insert(txn, index, chunk),
            SharedType::Prelim(v) => v.insert_str(index as usize, chunk),
        }
    }

    #[wasm_bindgen(js_name = append)]
    pub fn append(&mut self, txn: &mut YTransaction, chunk: &str) {
        match &mut self.0 {
            SharedType::Integrated(v) => v.push(txn, chunk),
            SharedType::Prelim(v) => v.push_str(chunk),
        }
    }

    #[wasm_bindgen(js_name = prepend)]
    pub fn prepend(&mut self, txn: &mut YTransaction, chunk: &str) {
        match &mut self.0 {
            SharedType::Integrated(v) => v.insert(txn, 0, chunk),
            SharedType::Prelim(v) => v.insert_str(0, chunk),
        }
    }

    #[wasm_bindgen(js_name = delete)]
    pub fn delete(&mut self, txn: &mut YTransaction, index: u32, length: u32) {
        match &mut self.0 {
            SharedType::Integrated(v) => v.remove_range(txn, index, length),
            SharedType::Prelim(v) => {
                let range = index as usize..(index + length) as usize;
                v.drain(range);
            }
        }
    }
}

#[wasm_bindgen]
pub struct YArray(SharedType<Array, Vec<JsValue>>);

#[wasm_bindgen]
impl YArray {
    #[wasm_bindgen(constructor)]
    pub fn new(items: Vec<JsValue>) -> Self {
        YArray(SharedType::Prelim(items))
    }

    #[wasm_bindgen(method, getter)]
    pub fn length(&self) -> u32 {
        match &self.0 {
            SharedType::Integrated(v) => v.len(),
            SharedType::Prelim(v) => v.len() as u32,
        }
    }

    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &YTransaction) -> JsValue {
        match &self.0 {
            SharedType::Integrated(v) => any_into_js(v.to_json(txn)),
            SharedType::Prelim(v) => JsValue::from(&v),
        }
    }

    #[wasm_bindgen(js_name = insert)]
    pub fn insert(&mut self, txn: &mut YTransaction, index: u32, items: Vec<JsValue>) {
        todo!()
    }

    #[wasm_bindgen(js_name = delete)]
    pub fn delete(&mut self, txn: &mut YTransaction, index: u32, length: u32) {
        match &mut self.0 {
            SharedType::Integrated(v) => v.remove_range(txn, index, length),
            SharedType::Prelim(v) => {
                let range = (index as usize..(index + length) as usize);
                v.drain(range);
            }
        }
    }

    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, txn: &YTransaction, index: u32) -> JsValue {
        match &self.0 {
            SharedType::Integrated(v) => {
                if let Some(value) = v.get(txn, index) {
                    value_into_js(value)
                } else {
                    JsValue::undefined()
                }
            }
            SharedType::Prelim(v) => {
                if let Some(value) = v.get(index as usize) {
                    value.clone()
                } else {
                    JsValue::undefined()
                }
            }
        }
    }

    #[wasm_bindgen(js_name = values)]
    pub fn values(&self, txn: &YTransaction) -> YArrayIterator {
        unsafe {
            let this: *const Array = &self.0;
            let tx: *const Transaction<'static> = txn.0.deref();
            let static_iter: ManuallyDrop<ArrayIter<'static, 'static>> =
                ManuallyDrop::new((*this).iter(tx.as_ref().unwrap()));
            YArrayIterator(static_iter)
        }
    }
}

#[wasm_bindgen]
pub struct IteratorNext {
    value: JsValue,
    done: bool,
}

#[wasm_bindgen]
impl IteratorNext {
    fn new(value: JsValue) -> Self {
        IteratorNext { done: false, value }
    }

    fn finished() -> Self {
        IteratorNext {
            done: true,
            value: JsValue::undefined(),
        }
    }

    #[wasm_bindgen(method, getter)]
    pub fn value(&self) -> JsValue {
        self.value.clone()
    }

    #[wasm_bindgen(method, getter)]
    pub fn done(&self) -> bool {
        self.done
    }
}

impl From<Option<Value>> for IteratorNext {
    fn from(v: Option<Value>) -> Self {
        match v {
            None => IteratorNext::finished(),
            Some(v) => IteratorNext::new(value_into_js(v)),
        }
    }
}

#[wasm_bindgen]
pub struct YArrayIterator(ManuallyDrop<ArrayIter<'static, 'static>>);

impl Drop for YArrayIterator {
    fn drop(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.0) }
    }
}

#[wasm_bindgen]
impl YArrayIterator {
    #[wasm_bindgen]
    pub fn next(&mut self) -> IteratorNext {
        self.0.next().into()
    }
}

#[wasm_bindgen]
pub struct YMap(SharedType<Map, HashMap<String, JsValue>>);

#[wasm_bindgen]
impl YMap {
    #[wasm_bindgen(constructor, catch)]
    pub fn new(object: JsValue) -> Result<YMap, JsValue> {
        if object.is_object() {
            let mut map = HashMap::new();
            let o: Object = object.into();
            for js_val in Object::entries(&o).iter() {
                let tuple: js_sys::Array = js_sys::Array::from(&js_val);
                let key = tuple.get(0);
                let value = tuple.get(1);
                map.insert(key.as_string().unwrap(), value);
            }
            Ok(YMap(SharedType::Prelim(map)))
        } else {
            Err(JsValue::from_str(
                "YMap constructor must be initialized using object",
            ))
        }
    }
    #[wasm_bindgen(method, getter)]
    pub fn length(&self, txn: &YTransaction) -> u32 {
        match &self.0 {
            SharedType::Integrated(v) => v.len(txn),
            SharedType::Prelim(v) => v.len() as u32,
        }
    }

    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &YTransaction) -> JsValue {
        match &self.0 {
            SharedType::Integrated(v) => any_into_js(v.to_json(txn)),
            SharedType::Prelim(v) => {
                let mut result = js_sys::Map::new();
                for (k, v) in v.iter() {
                    result.set(&JsValue::from(k), v);
                }
                result.into()
            }
        }
    }

    #[wasm_bindgen(js_name = set)]
    pub fn set(&self, txn: &mut YTransaction, key: &str, value: JsValue) {
        todo!()
    }

    #[wasm_bindgen(js_name = delete)]
    pub fn delete(&mut self, txn: &mut YTransaction, key: &str) {
        match &mut self.0 {
            SharedType::Integrated(v) => {
                v.remove(txn, key);
            }
            SharedType::Prelim(v) => {
                v.remove(key);
            }
        }
    }

    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, txn: &mut YTransaction, key: &str) -> JsValue {
        match &self.0 {
            SharedType::Integrated(v) => {
                if let Some(v) = v.get(txn, key) {
                    value_into_js(v)
                } else {
                    JsValue::undefined()
                }
            }
            SharedType::Prelim(v) => {
                if let Some(v) = v.get(key) {
                    v.clone()
                } else {
                    JsValue::undefined()
                }
            }
        }
    }

    #[wasm_bindgen(js_name = get)]
    pub fn entries(&self, txn: &mut YTransaction) -> YMapIterator {
        unsafe {
            let this: *const Map = &self.0;
            let tx: *const Transaction<'static> = txn.0.deref();
            let static_iter: ManuallyDrop<MapIter<'static, 'static>> =
                ManuallyDrop::new((*this).iter(tx.as_ref().unwrap()));
            YMapIterator(static_iter)
        }
    }
}

#[wasm_bindgen]
pub struct YMapIterator(ManuallyDrop<MapIter<'static, 'static>>);

impl Drop for YMapIterator {
    fn drop(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.0) }
    }
}

impl<'a> From<Option<(&'a String, Value)>> for IteratorNext {
    fn from(entry: Option<(&'a String, Value)>) -> Self {
        match entry {
            None => IteratorNext::finished(),
            Some((k, v)) => {
                let mut tuple = js_sys::Array::new_with_length(2);
                tuple.set(0, JsValue::from(k));
                tuple.set(1, value_into_js(v));
                IteratorNext::new(tuple.into())
            }
        }
    }
}

#[wasm_bindgen]
impl YMapIterator {
    #[wasm_bindgen]
    pub fn next(&mut self) -> IteratorNext {
        self.0.next().into()
    }
}

#[wasm_bindgen]
pub struct YXmlElement(XmlElement);

#[wasm_bindgen]
impl YXmlElement {
    #[wasm_bindgen(method, getter)]
    pub fn name(&self) -> String {
        self.0.tag().to_string()
    }

    #[wasm_bindgen(method, getter)]
    pub fn length(&self, txn: &YTransaction) -> u32 {
        self.0.len(txn)
    }

    #[wasm_bindgen(js_name = insertXmlElement)]
    pub fn insert_xml_element(
        &self,
        txn: &mut YTransaction,
        index: u32,
        name: &str,
    ) -> YXmlElement {
        YXmlElement(self.0.insert_elem(txn, index, name))
    }

    #[wasm_bindgen(js_name = pushXmlElement)]
    pub fn push_xml_element(&self, txn: &mut YTransaction, name: &str) -> YXmlElement {
        YXmlElement(self.0.push_elem_back(txn, name))
    }

    #[wasm_bindgen(js_name = insertXmlText)]
    pub fn insert_xml_text(&self, txn: &mut YTransaction, index: u32) -> YXmlText {
        YXmlText(self.0.insert_text(txn, index))
    }

    #[wasm_bindgen(js_name = pushXmlText)]
    pub fn push_xml_text(&self, txn: &mut YTransaction) -> YXmlText {
        YXmlText(self.0.push_text_back(txn))
    }

    #[wasm_bindgen(js_name = firstChild)]
    pub fn first_child(&self, txn: &YTransaction) -> JsValue {
        if let Some(xml) = self.0.first_child(txn) {
            xml_into_js(xml)
        } else {
            JsValue::undefined()
        }
    }

    #[wasm_bindgen(js_name = nextSibling)]
    pub fn next_sibling(&self, txn: &YTransaction) -> JsValue {
        if let Some(xml) = self.0.next_sibling(txn) {
            xml_into_js(xml)
        } else {
            JsValue::undefined()
        }
    }

    #[wasm_bindgen(js_name = prevSibling)]
    pub fn prev_sibling(&self, txn: &YTransaction) -> JsValue {
        if let Some(xml) = self.0.prev_sibling(txn) {
            xml_into_js(xml)
        } else {
            JsValue::undefined()
        }
    }

    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &YTransaction) -> String {
        self.0.to_string(txn)
    }

    #[wasm_bindgen(js_name = setAttribute)]
    pub fn set_attribute(&self, txn: &mut YTransaction, name: &str, value: &str) {
        self.0.insert_attribute(txn, name, value)
    }

    #[wasm_bindgen(js_name = getAttribute)]
    pub fn get_attribute(&self, txn: &YTransaction, name: &str) -> Option<String> {
        self.0.get_attribute(txn, name)
    }

    #[wasm_bindgen(js_name = removeAttribute)]
    pub fn remove_attribute(&self, txn: &mut YTransaction, name: &str) {
        self.0.remove_attribute(txn, name);
    }

    #[wasm_bindgen(js_name = attributes)]
    pub fn attributes(&self, txn: &YTransaction) -> YXmlAttributes {
        unsafe {
            let this: *const XmlElement = &self.0;
            let tx: *const Transaction<'static> = txn.0.deref();
            let static_iter: ManuallyDrop<Attributes<'static, 'static>> =
                ManuallyDrop::new((*this).attributes(tx.as_ref().unwrap()));
            YXmlAttributes(static_iter)
        }
    }

    #[wasm_bindgen(js_name = treeWalker)]
    pub fn tree_walker(&self, txn: &YTransaction) -> YXmlTreeWalker {
        unsafe {
            let this: *const XmlElement = &self.0;
            let tx: *const Transaction<'static> = txn.0.deref();
            let static_iter: ManuallyDrop<TreeWalker<'static, 'static>> =
                ManuallyDrop::new((*this).successors(tx.as_ref().unwrap()));
            YXmlTreeWalker(static_iter)
        }
    }
}

#[wasm_bindgen]
pub struct YXmlAttributes(ManuallyDrop<Attributes<'static, 'static>>);

impl Drop for YXmlAttributes {
    fn drop(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.0) }
    }
}

impl<'a> From<Option<(&'a str, String)>> for IteratorNext {
    fn from(o: Option<(&'a str, String)>) -> Self {
        match o {
            None => IteratorNext::finished(),
            Some((name, value)) => {
                let mut tuple = js_sys::Array::new_with_length(2);
                tuple.set(0, JsValue::from_str(name));
                tuple.set(1, JsValue::from(&value));
                IteratorNext::new(tuple.into())
            }
        }
    }
}

#[wasm_bindgen]
impl YXmlAttributes {
    #[wasm_bindgen]
    pub fn next(&mut self) -> IteratorNext {
        self.0.next().into()
    }
}

#[wasm_bindgen]
pub struct YXmlTreeWalker(ManuallyDrop<TreeWalker<'static, 'static>>);

impl Drop for YXmlTreeWalker {
    fn drop(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.0) }
    }
}

#[wasm_bindgen]
impl YXmlTreeWalker {
    #[wasm_bindgen]
    pub fn next(&mut self) -> IteratorNext {
        if let Some(xml) = self.0.next() {
            let js_val = xml_into_js(xml);
            IteratorNext::new(js_val)
        } else {
            IteratorNext::finished()
        }
    }
}

#[wasm_bindgen]
pub struct YXmlText(XmlText);

#[wasm_bindgen]
impl YXmlText {
    #[wasm_bindgen(method, getter)]
    pub fn length(&self) -> u32 {
        self.0.len()
    }

    #[wasm_bindgen(js_name = nextSibling)]
    pub fn next_sibling(&self, txn: &YTransaction) -> JsValue {
        if let Some(xml) = self.0.next_sibling(txn) {
            xml_into_js(xml)
        } else {
            JsValue::undefined()
        }
    }

    #[wasm_bindgen(js_name = prevSibling)]
    pub fn prev_sibling(&self, txn: &YTransaction) -> JsValue {
        if let Some(xml) = self.0.prev_sibling(txn) {
            xml_into_js(xml)
        } else {
            JsValue::undefined()
        }
    }

    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &YTransaction) -> String {
        self.0.to_string(txn)
    }

    #[wasm_bindgen(js_name = setAttribute)]
    pub fn set_attribute(&self, txn: &mut YTransaction, name: &str, value: &str) {
        self.0.insert_attribute(txn, name, value);
    }

    #[wasm_bindgen(js_name = getAttribute)]
    pub fn get_attribute(&self, txn: &YTransaction, name: &str) -> Option<String> {
        self.0.get_attribute(txn, name)
    }

    #[wasm_bindgen(js_name = removeAttribute)]
    pub fn remove_attribute(&self, txn: &mut YTransaction, name: &str) {
        self.0.remove_attribute(txn, name);
    }

    #[wasm_bindgen(js_name = attributes)]
    pub fn attributes(&self, txn: &YTransaction) -> YXmlAttributes {
        unsafe {
            let this: *const XmlText = &self.0;
            let tx: *const Transaction<'static> = txn.0.deref();
            let static_iter: ManuallyDrop<Attributes<'static, 'static>> =
                ManuallyDrop::new((*this).attributes(tx.as_ref().unwrap()));
            YXmlAttributes(static_iter)
        }
    }
}

fn any_into_js(v: Any) -> JsValue {
    match v {
        Any::Null => JsValue::null(),
        Any::Undefined => JsValue::undefined(),
        Any::Bool(v) => JsValue::from_bool(v),
        Any::Number(v) => JsValue::from(v),
        Any::BigInt(v) => JsValue::from(v),
        Any::String(v) => JsValue::from(&v),
        Any::Buffer(v) => {
            let v = Uint8Array::from(v.as_ref());
            v.into()
        }
        Any::Array(v) => {
            let mut a = js_sys::Array::new();
            for value in v {
                a.push(&any_into_js(value));
            }
            a.into()
        }
        Any::Map(v) => {
            let mut m = js_sys::Map::new();
            for (k, v) in v {
                let key = JsValue::from(&k);
                let value = any_into_js(v);
                m.set(&key, &value);
            }
            m.into()
        }
    }
}

fn value_into_js(v: Value) -> JsValue {
    match v {
        Value::Any(v) => any_into_js(v),
        Value::YText(v) => YText::from(v).into(),
        Value::YArray(v) => YArray(v).into(),
        Value::YMap(v) => YMap(v).into(),
        Value::YXmlElement(v) => YXmlElement(v).into(),
        Value::YXmlText(v) => YXmlText(v).into(),
    }
}

fn xml_into_js(v: Xml) -> JsValue {
    match v {
        Xml::Element(v) => YXmlElement(v).into(),
        Xml::Text(v) => YXmlText(v).into(),
    }
}
