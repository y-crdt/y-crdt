use wasm_bindgen::prelude::wasm_bindgen;
use yrs::{Doc, Transaction, Text, Array, Map, XmlElement, XmlText};
use yrs::types::map::MapIter;
use js_sys::IteratorNext;
use yrs::types::array::ArrayIter;
use yrs::types::xml::{Attributes, TreeWalker};
use wasm_bindgen::convert::RefFromWasmAbi;
use wasm_bindgen::describe::WasmDescribe;
use yrs::types::Value;
use serde::{Serialize,Deserialize};
use wasm_bindgen::JsValue;

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
        todo!()
    }

    #[wasm_bindgen(constructor)]
    pub fn new_with_id(id: u64) -> Self {
        todo!()
    }

    #[wasm_bindgen(method, getter)]
    pub fn id(&self) -> u64 {
        todo!()
    }

    #[wasm_bindgen]
    pub fn transact(&mut self) -> YTransaction {
        todo!()
    }
}

#[wasm_bindgen]
pub struct YTransaction(Transaction<'static>);

#[wasm_bindgen]
impl YTransaction {

    #[wasm_bindgen(js_name = getText)]
    pub fn get_text(&mut self, name: &str) -> YText {
        todo!()
    }

    #[wasm_bindgen(js_name = getArray)]
    pub fn get_array(&mut self, name: &str) -> YArray {
        todo!()
    }

    #[wasm_bindgen(js_name = getMap)]
    pub fn get_map(&mut self, name: &str) -> YMap {
        todo!()
    }

    #[wasm_bindgen(js_name = getXmlElement)]
    pub fn get_xml_element(&mut self, name: &str) -> YXmlElement {
        todo!()
    }

    #[wasm_bindgen(js_name = getXmlText)]
    pub fn get_xml_text(&mut self, name: &str) -> YXmlText {
        todo!()
    }

    #[wasm_bindgen(js_name = commit)]
    pub fn commit(&mut self) {
        todo!()
    }

    #[wasm_bindgen(js_name = stateVector)]
    pub fn state_vector(&self) -> js_sys::Int8Array {
        todo!()
    }

    #[wasm_bindgen(js_name = diff)]
    pub fn diff(&self, vector: js_sys::Int8Array) -> js_sys::Int8Array {
        todo!()
    }

    #[wasm_bindgen(js_name = apply)]
    pub fn apply(&mut self, diff: js_sys::Int8Array) {
        todo!()
    }
}

#[wasm_bindgen]
pub struct YText(Text);

#[wasm_bindgen]
impl YText {

    #[wasm_bindgen(method, getter)]
    pub fn length(&self) -> u32 {
        todo!()
    }

    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &YTransaction) -> String {
        todo!()
    }

    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &YTransaction) -> JsValue {
        todo!()
    }

    #[wasm_bindgen(js_name = insert)]
    pub fn insert(&self, txn: &mut YTransaction, index: u32, chunk: &str) {
        todo!()
    }

    #[wasm_bindgen(js_name = append)]
    pub fn append(&self, txn: &mut YTransaction, chunk: &str) {
        todo!()
    }

    #[wasm_bindgen(js_name = prepend)]
    pub fn prepend(&self, txn: &mut YTransaction, chunk: &str) {
        todo!()
    }

    #[wasm_bindgen(js_name = delete)]
    pub fn delete(&self, txn: &mut YTransaction, index: u32, length: u32) {
        todo!()
    }
}

#[wasm_bindgen]
pub struct YArray(Array);

#[wasm_bindgen]
impl YArray {

    #[wasm_bindgen(method, getter)]
    pub fn length(&self) -> u32 {
        todo!()
    }

    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &YTransaction) -> JsValue {
        todo!()
    }

    #[wasm_bindgen(js_name = insert)]
    pub fn insert(&self, txn: &mut YTransaction, index: u32, items: Vec<JsValue>) {
        todo!()
    }

    #[wasm_bindgen(js_name = delete)]
    pub fn delete(&self, txn: &mut YTransaction, index: u32, length: u32) {
        todo!()
    }

    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, txn: &mut YTransaction, index: u32) -> JsValue {
        todo!()
    }
}

#[wasm_bindgen]
pub struct YArrayIterator(ArrayIter<'static, 'static>);

#[wasm_bindgen]
impl YArrayIterator {

    #[wasm_bindgen]
    pub fn next(&mut self) -> JsValue {
        todo!()
    }
}

#[wasm_bindgen]
pub struct YMap(Map);

#[wasm_bindgen]
impl YMap {

    #[wasm_bindgen(method, getter)]
    pub fn length(&self) -> u32 {
        todo!()
    }

    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &YTransaction) -> JsValue {
        todo!()
    }

    #[wasm_bindgen(js_name = set)]
    pub fn set(&self, txn: &mut YTransaction, key: &str, value: JsValue) {
        todo!()
    }

    #[wasm_bindgen(js_name = delete)]
    pub fn delete(&self, txn: &mut YTransaction, key: &str) {
        todo!()
    }

    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, txn: &mut YTransaction, key: &str) -> JsValue {
        todo!()
    }

    #[wasm_bindgen(js_name = get)]
    pub fn entries(&self, txn: &mut YTransaction) -> YMapIterator {
        todo!()
    }
}

#[wasm_bindgen]
pub struct YMapIterator(MapIter<'static, 'static>);

#[wasm_bindgen]
impl YMapIterator {
    #[wasm_bindgen]
    pub fn next(&mut self) -> JsValue {
        todo!()
    }
}

#[wasm_bindgen]
pub struct YXmlElement(XmlElement);

#[wasm_bindgen]
impl YXmlElement {

    #[wasm_bindgen(method, getter)]
    pub fn name(&self) -> String {
        todo!()
    }

    #[wasm_bindgen(method, getter)]
    pub fn length(&self) -> u32 {
        todo!()
    }

    #[wasm_bindgen(js_name = firstChild)]
    pub fn first_child(&self, txn: &YTransaction) -> JsValue {
        todo!()
    }

    #[wasm_bindgen(js_name = nextSibling)]
    pub fn next_sibling(&self, txn: &YTransaction) -> JsValue {
        todo!()
    }

    #[wasm_bindgen(js_name = prevSibling)]
    pub fn prev_sibling(&self, txn: &YTransaction) -> JsValue {
        todo!()
    }

    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &YTransaction) -> String {
        todo!()
    }

    #[wasm_bindgen(js_name = setAttribute)]
    pub fn set_attribute(&self, txn: &mut YTransaction, name: &str, value: &str) {
        todo!()
    }

    #[wasm_bindgen(js_name = getAttribute)]
    pub fn get_attribute(&self, txn: &YTransaction, name: &str) -> String {
        todo!()
    }

    #[wasm_bindgen(js_name = removeAttribute)]
    pub fn remove_attribute(&self, txn: &mut YTransaction, name: &str) {
        todo!()
    }

    #[wasm_bindgen(js_name = attributes)]
    pub fn attributes(&self, txn: &YTransaction) -> YXmlAttributes {
        todo!()
    }

    #[wasm_bindgen(js_name = treeWalker)]
    pub fn tree_walker(&self, txn: &YTransaction) -> YXmlTreeWalker {
        todo!()
    }
}

#[wasm_bindgen]
pub struct YXmlAttributes(Attributes<'static, 'static>);

#[wasm_bindgen]
impl YXmlAttributes {
    #[wasm_bindgen]
    pub fn next(&mut self) -> JsValue {
        todo!()
    }
}

#[wasm_bindgen]
pub struct YXmlTreeWalker(TreeWalker<'static, 'static>);

#[wasm_bindgen]
impl YXmlTreeWalker {
    #[wasm_bindgen]
    pub fn next(&mut self) -> JsValue {
        todo!()
    }
}

#[wasm_bindgen]
pub struct YXmlText(XmlText);

#[wasm_bindgen]
impl YXmlText {

    #[wasm_bindgen(method, getter)]
    pub fn length(&self) -> u32 {
        todo!()
    }

    #[wasm_bindgen(js_name = nextSibling)]
    pub fn next_sibling(&self, txn: &YTransaction) -> JsValue {
        todo!()
    }

    #[wasm_bindgen(js_name = prevSibling)]
    pub fn prev_sibling(&self, txn: &YTransaction) -> JsValue {
        todo!()
    }

    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &YTransaction) -> String {
        todo!()
    }

    #[wasm_bindgen(js_name = setAttribute)]
    pub fn set_attribute(&self, txn: &mut YTransaction, name: &str, value: &str) {
        todo!()
    }

    #[wasm_bindgen(js_name = getAttribute)]
    pub fn get_attribute(&self, txn: &YTransaction, name: &str) -> String {
        todo!()
    }

    #[wasm_bindgen(js_name = removeAttribute)]
    pub fn remove_attribute(&self, txn: &mut YTransaction, name: &str) {
        todo!()
    }

    #[wasm_bindgen(js_name = attributes)]
    pub fn attributes(&self, txn: &YTransaction) -> YXmlAttributes {
        todo!()
    }
}

#[derive(Serialize, Deserialize)]
pub enum JsOutput {

}

impl From<Value> for JsOutput {
    fn from(_: Value) -> Self {
        todo!()
    }
}