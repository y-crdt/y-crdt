use js_sys::Uint8Array;
use lib0::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use wasm_bindgen::__rt::Ref;
use wasm_bindgen::prelude::{wasm_bindgen, Closure};
use wasm_bindgen::JsValue;
use yrs::block::{ItemContent, Prelim};
use yrs::types::array::ArrayIter;
use yrs::types::map::MapIter;
use yrs::types::xml::{Attributes, TreeWalker};
use yrs::types::{
    Branch, BranchRef, TypePtr, TypeRefs, Value, TYPE_REFS_ARRAY, TYPE_REFS_MAP, TYPE_REFS_TEXT,
    TYPE_REFS_XML_ELEMENT, TYPE_REFS_XML_TEXT,
};
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

/// When called will call console log errors whenever internal panic is called from within
/// WebAssembly module.
#[wasm_bindgen(js_name = setPanicHook)]
pub fn set_panic_hook() {
    // When the `console_error_panic_hook` feature is enabled, we can call the
    // `set_panic_hook` function at least once during initialization, and then
    // we will get better error messages if our code ever panics.
    //
    // For more details see
    // https://github.com/rustwasm/console_error_panic_hook#readme
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// A ywasm document type. Documents are most important units of collaborative resources management.
/// All shared collections live within a scope of their corresponding documents. All updates are
/// generated on per document basis (rather than individual shared type). All operations on shared
/// collections happen via [YTransaction], which lifetime is also bound to a document.
///
/// Document manages so called root types, which are top-level shared types definitions (as opposed
/// to recursively nested types).
///
/// A basic workflow sample:
///
/// ```javascript
/// import YDoc from 'ywasm'
///
/// const doc = new YDoc()
/// const txn = doc.beginTransaction()
/// try {
///     const text = txn.getText('name')
///     text.push(txn, 'hello world')
///     const output = text.toString(txn)
///     console.log(output)
/// } finally {
///     txn.free()
/// }
/// ```
#[wasm_bindgen]
pub struct YDoc(Doc);

#[wasm_bindgen]
impl YDoc {
    /// Creates a new ywasm document. If `id` parameter was passed it will be used as this document
    /// globally unique identifier (it's up to caller to ensure that requirement). Otherwise it will
    /// be assigned a randomly generated number.
    #[wasm_bindgen(constructor)]
    pub fn new(id: Option<f64>) -> Self {
        if let Some(id) = id {
            YDoc(Doc::with_client_id(id as u64))
        } else {
            YDoc(Doc::new())
        }
    }

    /// Gets globally unique identifier of this `YDoc` instance.
    #[wasm_bindgen(method, getter)]
    pub fn id(&self) -> f64 {
        self.0.client_id as f64
    }

    /// Returns a new transaction for this document. Ywasm shared data types execute their
    /// operations in a context of a given transaction. Each document can have only one active
    /// transaction at the time - subsequent attempts will cause exception to be thrown.
    ///
    /// Transactions started with `doc.beginTransaction` can be released using `transaction.free`
    /// method.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// // helper function used to simplify transaction
    /// // create/release cycle
    /// YDoc.prototype.transact = callback => {
    ///     const txn = this.beginTransaction()
    ///     try {
    ///         return callback(txn)
    ///     } finally {
    ///         txn.free()
    ///     }
    /// }
    ///
    /// const doc = new YDoc()
    /// const text = doc.getText('name')
    /// doc.transact(txn => text.insert(txn, 0, 'hello world'))
    /// ```
    #[wasm_bindgen(js_name = beginTransaction)]
    pub fn begin_transaction(&mut self) -> YTransaction {
        unsafe {
            let doc: *mut Doc = &mut self.0;
            let static_txn: ManuallyDrop<Transaction<'static>> =
                ManuallyDrop::new((*doc).transact());
            YTransaction(static_txn)
        }
    }

    /// Returns a `YText` shared data type, that's accessible for subsequent accesses using given
    /// `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YText` instance.
    #[wasm_bindgen(js_name = getText)]
    pub fn get_text(&mut self, name: &str) -> YText {
        self.begin_transaction().get_text(name)
    }

    /// Returns a `YArray` shared data type, that's accessible for subsequent accesses using given
    /// `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YArray` instance.
    #[wasm_bindgen(js_name = getArray)]
    pub fn get_array(&mut self, name: &str) -> YArray {
        self.begin_transaction().get_array(name)
    }

    /// Returns a `YMap` shared data type, that's accessible for subsequent accesses using given
    /// `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YMap` instance.
    #[wasm_bindgen(js_name = getMap)]
    pub fn get_map(&mut self, name: &str) -> YMap {
        self.begin_transaction().get_map(name)
    }

    /// Returns a `YXmlElement` shared data type, that's accessible for subsequent accesses using
    /// given `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YXmlElement` instance.
    #[wasm_bindgen(js_name = getXmlElement)]
    pub fn get_xml_element(&mut self, name: &str) -> YXmlElement {
        self.begin_transaction().get_xml_element(name)
    }

    /// Returns a `YXmlText` shared data type, that's accessible for subsequent accesses using given
    /// `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YXmlText` instance.
    #[wasm_bindgen(js_name = getXmlText)]
    pub fn get_xml_text(&mut self, name: &str) -> YXmlText {
        self.begin_transaction().get_xml_text(name)
    }
}

/// Encodes a state vector of a given ywasm document into its binary representation using lib0 v1
/// encoding. State vector is a compact representation of updates performed on a given document and
/// can be used by `encode_state_as_update` on remote peer to generate a delta update payload to
/// synchronize changes between peers.
///
/// Example:
///
/// ```javascript
/// import {YDoc, encodeStateVector, encodeStateAsUpdate, applyUpdate} from 'ywasm'
///
/// /// document on machine A
/// const localDoc = new YDoc()
/// const localSV = encodeStateVector(localDoc)
///
/// // document on machine B
/// const remoteDoc = new YDoc()
/// const remoteDelta = encodeStateAsUpdate(remoteDoc, localSV)
///
/// applyUpdate(localDoc, remoteDelta)
/// ```
#[wasm_bindgen(js_name = encodeStateVector)]
pub fn encode_state_vector(doc: &mut YDoc) -> Uint8Array {
    doc.begin_transaction().state_vector_v1()
}

/// Encodes all updates that have happened since a given version `vector` into a compact delta
/// representation using lib0 v1 encoding. If `vector` parameter has not been provided, generated
/// delta payload will contain all changes of a current ywasm document, working effectivelly as its
/// state snapshot.
///
/// Example:
///
/// ```javascript
/// import {YDoc, encodeStateVector, encodeStateAsUpdate, applyUpdate} from 'ywasm'
///
/// /// document on machine A
/// const localDoc = new YDoc()
/// const localSV = encodeStateVector(localDoc)
///
/// // document on machine B
/// const remoteDoc = new YDoc()
/// const remoteDelta = encodeStateAsUpdate(remoteDoc, localSV)
///
/// applyUpdate(localDoc, remoteDelta)
/// ```
#[wasm_bindgen(js_name = encodeStateAsUpdate)]
pub fn encode_state_as_update(doc: &mut YDoc, vector: Option<Uint8Array>) -> Uint8Array {
    doc.begin_transaction().diff_v1(vector)
}

/// Applies delta update generated by the remote document replica to a current document. This
/// method assumes that a payload maintains lib0 v1 encoding format.
///
/// Example:
///
/// ```javascript
/// import {YDoc, encodeStateVector, encodeStateAsUpdate, applyUpdate} from 'ywasm'
///
/// /// document on machine A
/// const localDoc = new YDoc()
/// const localSV = encodeStateVector(localDoc)
///
/// // document on machine B
/// const remoteDoc = new YDoc()
/// const remoteDelta = encodeStateAsUpdate(remoteDoc, localSV)
///
/// applyUpdate(localDoc, remoteDelta)
/// ```
#[wasm_bindgen(js_name = applyUpdate)]
pub fn apply_update(doc: &mut YDoc, diff: Uint8Array) {
    doc.begin_transaction().apply_v1(diff);
}

/// A transaction that serves as a proxy to document block store. Ywasm shared data types execute
/// their operations in a context of a given transaction. Each document can have only one active
/// transaction at the time - subsequent attempts will cause exception to be thrown.
///
/// Transactions started with `doc.beginTransaction` can be released using `transaction.free`
/// method.
///
/// Example:
///
/// ```javascript
/// import YDoc from 'ywasm'
///
/// // helper function used to simplify transaction
/// // create/release cycle
/// YDoc.prototype.transact = callback => {
///     const txn = this.beginTransaction()
///     try {
///         return callback(txn)
///     } finally {
///         txn.free()
///     }
/// }
///
/// const doc = new YDoc()
/// const text = doc.getText('name')
/// doc.transact(txn => text.insert(txn, 0, 'hello world'))
/// ```
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
    /// Returns a `YText` shared data type, that's accessible for subsequent accesses using given
    /// `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YText` instance.
    #[wasm_bindgen(js_name = getText)]
    pub fn get_text(&mut self, name: &str) -> YText {
        self.0.get_text(name).into()
    }

    /// Returns a `YArray` shared data type, that's accessible for subsequent accesses using given
    /// `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YArray` instance.
    #[wasm_bindgen(js_name = getArray)]
    pub fn get_array(&mut self, name: &str) -> YArray {
        self.0.get_array(name).into()
    }

    /// Returns a `YMap` shared data type, that's accessible for subsequent accesses using given
    /// `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YMap` instance.
    #[wasm_bindgen(js_name = getMap)]
    pub fn get_map(&mut self, name: &str) -> YMap {
        self.0.get_map(name).into()
    }

    /// Returns a `YXmlElement` shared data type, that's accessible for subsequent accesses using
    /// given `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YXmlElement` instance.
    #[wasm_bindgen(js_name = getXmlElement)]
    pub fn get_xml_element(&mut self, name: &str) -> YXmlElement {
        YXmlElement(self.0.get_xml_element(name))
    }

    /// Returns a `YXmlText` shared data type, that's accessible for subsequent accesses using given
    /// `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YXmlText` instance.
    #[wasm_bindgen(js_name = getXmlText)]
    pub fn get_xml_text(&mut self, name: &str) -> YXmlText {
        YXmlText(self.0.get_xml_text(name))
    }

    /// Triggers a post-update series of operations without `free`ing the transaction. This includes
    /// compaction and optimization of internal representation of updates, triggering events etc.
    /// ywasm transactions are auto-committed when they are `free`d.
    #[wasm_bindgen(js_name = commit)]
    pub fn commit(&mut self) {
        self.0.commit()
    }

    /// Encodes a state vector of a given transaction document into its binary representation using
    /// lib0 v1 encoding. State vector is a compact representation of updates performed on a given
    /// document and can be used by `encode_state_as_update` on remote peer to generate a delta
    /// update payload to synchronize changes between peers.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const localDoc = new YDoc()
    /// const localTxn = localDoc.beginTransaction()
    ///
    /// // document on machine B
    /// const remoteDoc = new YDoc()
    /// const remoteTxn = localDoc.beginTransaction()
    ///
    /// try {
    ///     const localSV = localTxn.stateVectorV1()
    ///     const remoteDelta = remoteTxn.diffV1(localSv)
    ///     localTxn.applyV1(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    #[wasm_bindgen(js_name = stateVectorV1)]
    pub fn state_vector_v1(&self) -> Uint8Array {
        let sv = self.0.state_vector();
        let payload = sv.encode_v1();
        Uint8Array::from(&payload[..payload.len()])
    }

    /// Encodes all updates that have happened since a given version `vector` into a compact delta
    /// representation using lib0 v1 encoding. If `vector` parameter has not been provided, generated
    /// delta payload will contain all changes of a current ywasm document, working effectively as
    /// its state snapshot.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const localDoc = new YDoc()
    /// const localTxn = localDoc.beginTransaction()
    ///
    /// // document on machine B
    /// const remoteDoc = new YDoc()
    /// const remoteTxn = localDoc.beginTransaction()
    ///
    /// try {
    ///     const localSV = localTxn.stateVectorV1()
    ///     const remoteDelta = remoteTxn.diffV1(localSv)
    ///     localTxn.applyV1(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    #[wasm_bindgen(js_name = diffV1)]
    pub fn diff_v1(&self, vector: Option<Uint8Array>) -> Uint8Array {
        let mut encoder = EncoderV1::new();
        let sv = if let Some(vector) = vector {
            StateVector::decode_v1(vector.to_vec().as_slice())
        } else {
            StateVector::default()
        };
        self.0.encode_diff(&sv, &mut encoder);
        let payload = encoder.to_vec();
        Uint8Array::from(&payload[..payload.len()])
    }

    /// Applies delta update generated by the remote document replica to a current transaction's
    /// document. This method assumes that a payload maintains lib0 v1 encoding format.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const localDoc = new YDoc()
    /// const localTxn = localDoc.beginTransaction()
    ///
    /// // document on machine B
    /// const remoteDoc = new YDoc()
    /// const remoteTxn = localDoc.beginTransaction()
    ///
    /// try {
    ///     const localSV = localTxn.stateVectorV1()
    ///     const remoteDelta = remoteTxn.diffV1(localSv)
    ///     localTxn.applyV1(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    #[wasm_bindgen(js_name = applyV1)]
    pub fn apply_v1(&mut self, diff: Uint8Array) {
        let diff: Vec<u8> = diff.to_vec();
        let mut decoder = DecoderV1::from(diff.as_slice());
        let update = Update::decode(&mut decoder);
        self.0.apply_update(update)
    }

    #[wasm_bindgen(js_name = encodeUpdate)]
    pub fn encode_update(&mut self) -> Uint8Array {
        let diff = self.0.encode_update_v1();
        Uint8Array::from(&diff[..diff.len()])
    }
}

enum SharedType<T, P> {
    Integrated(T),
    Prelim(P),
}

impl<T, P> SharedType<T, P> {
    #[inline(always)]
    fn new(value: T) -> RefCell<Self> {
        RefCell::new(SharedType::Integrated(value))
    }

    #[inline(always)]
    fn prelim(prelim: P) -> RefCell<Self> {
        RefCell::new(SharedType::Prelim(prelim))
    }
}

/// A shared data type used for collaborative text editing. It enables multiple users to add and
/// remove chunks of text in efficient manner. This type is internally represented as a mutable
/// double-linked list of text chunks - an optimization occurs during `YTransaction.commit`, which
/// allows to squash multiple consecutively inserted characters together as a single chunk of text
/// even between transaction boundaries in order to preserve more efficient memory model.
///
/// `YText` structure internally uses UTF-8 encoding and its length is described in a number of
/// bytes rather than individual characters (a single UTF-8 code point can consist of many bytes).
///
/// Like all Yrs shared data types, `YText` is resistant to the problem of interleaving (situation
/// when characters inserted one after another may interleave with other peers concurrent inserts
/// after merging all updates together). In case of Yrs conflict resolution is solved by using
/// unique document id to determine correct and consistent ordering.
#[wasm_bindgen]
pub struct YText(RefCell<SharedType<Text, String>>);

impl From<Text> for YText {
    fn from(v: Text) -> Self {
        YText(SharedType::new(v))
    }
}

#[wasm_bindgen]
impl YText {
    /// Creates a new preliminary instance of a `YText` shared data type, with its state initialized
    /// to provided parameter.
    ///
    /// Preliminary instances can be nested into other shared data types such as `YArray` and `YMap`.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[wasm_bindgen(constructor)]
    pub fn new(init: Option<String>) -> Self {
        YText(SharedType::prelim(init.unwrap_or_default()))
    }

    /// Returns true if this is a preliminary instance of `YText`.
    ///
    /// Preliminary instances can be nested into other shared data types such as `YArray` and `YMap`.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[wasm_bindgen(method, getter)]
    pub fn prelim(&self) -> bool {
        if let SharedType::Prelim(_) = &*self.0.borrow() {
            true
        } else {
            false
        }
    }

    /// Returns length of an underlying string stored in this `YText` instance,
    /// understood as a number of UTF-8 encoded bytes.
    #[wasm_bindgen(method, getter)]
    pub fn length(&self) -> u32 {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => v.len(),
            SharedType::Prelim(v) => v.len() as u32,
        }
    }

    /// Returns an underlying shared string stored in this data type.
    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &YTransaction) -> String {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => v.to_string(txn),
            SharedType::Prelim(v) => v.clone(),
        }
    }

    /// Returns an underlying shared string stored in this data type.
    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &YTransaction) -> JsValue {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => JsValue::from(&v.to_string(txn)),
            SharedType::Prelim(v) => JsValue::from(v),
        }
    }

    /// Inserts a given `chunk` of text into this `YText` instance, starting at a given `index`.
    #[wasm_bindgen(js_name = insert)]
    pub fn insert(&self, txn: &mut YTransaction, index: u32, chunk: &str) {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => v.insert(txn, index, chunk),
            SharedType::Prelim(v) => v.insert_str(index as usize, chunk),
        }
    }

    /// Appends a given `chunk` of text at the end of current `YText` instance.
    #[wasm_bindgen(js_name = push)]
    pub fn push(&self, txn: &mut YTransaction, chunk: &str) {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => v.push(txn, chunk),
            SharedType::Prelim(v) => v.push_str(chunk),
        }
    }

    /// Deletes a specified range of of characters, starting at a given `index`.
    /// Both `index` and `length` are counted in terms of a number of UTF-8 character bytes.
    #[wasm_bindgen(js_name = delete)]
    pub fn delete(&mut self, txn: &mut YTransaction, index: u32, length: u32) {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => v.remove_range(txn, index, length),
            SharedType::Prelim(v) => {
                v.drain((index as usize)..(index + length) as usize);
            }
        }
    }
}

/// A collection used to store data in an indexed sequence structure. This type is internally
/// implemented as a double linked list, which may squash values inserted directly one after another
/// into single list node upon transaction commit.
///
/// Reading a root-level type as an YArray means treating its sequence components as a list, where
/// every countable element becomes an individual entity:
///
/// - JSON-like primitives (booleans, numbers, strings, JSON maps, arrays etc.) are counted
///   individually.
/// - Text chunks inserted by [Text] data structure: each character becomes an element of an
///   array.
/// - Embedded and binary values: they count as a single element even though they correspond of
///   multiple bytes.
///
/// Like all Yrs shared data types, YArray is resistant to the problem of interleaving (situation
/// when elements inserted one after another may interleave with other peers concurrent inserts
/// after merging all updates together). In case of Yrs conflict resolution is solved by using
/// unique document id to determine correct and consistent ordering.
#[wasm_bindgen]
pub struct YArray(RefCell<SharedType<Array, Vec<JsValue>>>);

impl From<Array> for YArray {
    fn from(v: Array) -> Self {
        YArray(SharedType::new(v))
    }
}

#[wasm_bindgen]
impl YArray {
    /// Creates a new preliminary instance of a `YArray` shared data type, with its state
    /// initialized to provided parameter.
    ///
    /// Preliminary instances can be nested into other shared data types such as `YArray` and `YMap`.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[wasm_bindgen(constructor)]
    pub fn new(init: Option<Vec<JsValue>>) -> Self {
        YArray(SharedType::prelim(init.unwrap_or_default()))
    }

    /// Returns true if this is a preliminary instance of `YArray`.
    ///
    /// Preliminary instances can be nested into other shared data types such as `YArray` and `YMap`.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[wasm_bindgen(method, getter)]
    pub fn prelim(&self) -> bool {
        if let SharedType::Prelim(_) = &*self.0.borrow() {
            true
        } else {
            false
        }
    }

    /// Returns a number of elements stored within this instance of `YArray`.
    #[wasm_bindgen(method, getter)]
    pub fn length(&self) -> u32 {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => v.len(),
            SharedType::Prelim(v) => v.len() as u32,
        }
    }

    /// Converts an underlying contents of this `YArray` instance into their JSON representation.
    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &YTransaction) -> JsValue {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => any_into_js(v.to_json(txn)),
            SharedType::Prelim(v) => {
                let array = js_sys::Array::new();
                for js in v.iter() {
                    array.push(js);
                }
                array.into()
            }
        }
    }

    /// Inserts a given range of `items` into this `YArray` instance, starting at given `index`.
    #[wasm_bindgen(js_name = insert)]
    pub fn insert(&self, txn: &mut YTransaction, index: u32, items: Vec<JsValue>) {
        let mut j = index;
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(array) => {
                insert_at(array, txn, index, items);
            }
            SharedType::Prelim(vec) => {
                for js in items {
                    vec.insert(j as usize, js);
                    j += 1;
                }
            }
        }
    }

    /// Appends a range of `items` at the end of this `YArray` instance.
    #[wasm_bindgen(js_name = push)]
    pub fn push(&self, txn: &mut YTransaction, items: Vec<JsValue>) {
        let index = self.length();
        self.insert(txn, index, items);
    }

    /// Deletes a range of items of given `length` from current `YArray` instance,
    /// starting from given `index`.
    #[wasm_bindgen(js_name = delete)]
    pub fn delete(&self, txn: &mut YTransaction, index: u32, length: u32) {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => v.remove_range(txn, index, length),
            SharedType::Prelim(v) => {
                v.drain((index as usize)..(index + length) as usize);
            }
        }
    }

    /// Returns an element stored under given `index`.
    #[wasm_bindgen(catch, js_name = get)]
    pub fn get(&self, txn: &YTransaction, index: u32) -> Result<JsValue, JsValue> {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => {
                if let Some(value) = v.get(txn, index) {
                    Ok(value_into_js(value))
                } else {
                    Err(JsValue::from("Index outside the bounds of an YArray"))
                }
            }
            SharedType::Prelim(v) => {
                if let Some(value) = v.get(index as usize) {
                    Ok(value.clone())
                } else {
                    Err(JsValue::from("Index outside the bounds of an YArray"))
                }
            }
        }
    }

    /// Returns an iterator that can be used to traverse over the values stored withing this
    /// instance of `YArray`.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const doc = new YDoc()
    /// const array = doc.getArray('name')
    /// const txn = doc.beginTransaction()
    /// try {
    ///     array.push(txn, ['hello', 'world'])
    ///     for (let item of array.values(txn)) {
    ///         console.log(item)
    ///     }
    /// } finally {
    ///     txn.free()
    /// }
    /// ```
    #[wasm_bindgen(js_name = values)]
    pub fn values(&self, txn: &YTransaction) -> JsValue {
        to_iter(match &*self.0.borrow() {
            SharedType::Integrated(v) => unsafe {
                let this: *const Array = v;
                let tx: *const Transaction<'static> = txn.0.deref();
                let static_iter: ManuallyDrop<ArrayIter<'static, 'static>> =
                    ManuallyDrop::new((*this).iter(tx.as_ref().unwrap()));
                YArrayIterator(static_iter).into()
            },
            SharedType::Prelim(v) => unsafe {
                let this: *const Vec<JsValue> = v;
                let static_iter: ManuallyDrop<std::slice::Iter<'static, JsValue>> =
                    ManuallyDrop::new((*this).iter());
                PrelimArrayIterator(static_iter).into()
            },
        })
    }
}

fn to_iter(iterator: JsValue) -> JsValue {
    let iter = js_sys::Object::new();
    let symbol_iter = js_sys::Symbol::iterator();
    let cb = Closure::once_into_js(move || iterator);
    js_sys::Reflect::set(&iter, &symbol_iter.into(), &cb).unwrap();
    iter.into()
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
pub struct PrelimArrayIterator(ManuallyDrop<std::slice::Iter<'static, JsValue>>);

impl Drop for PrelimArrayIterator {
    fn drop(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.0) }
    }
}

#[wasm_bindgen]
impl PrelimArrayIterator {
    #[wasm_bindgen]
    pub fn next(&mut self) -> IteratorNext {
        if let Some(js) = self.0.next() {
            IteratorNext::new(js.clone())
        } else {
            IteratorNext::finished()
        }
    }
}

/// Collection used to store key-value entries in an unordered manner. Keys are always represented
/// as UTF-8 strings. Values can be any value type supported by Yrs: JSON-like primitives as well as
/// shared data types.
///
/// In terms of conflict resolution, [Map] uses logical last-write-wins principle, meaning the past
/// updates are automatically overridden and discarded by newer ones, while concurrent updates made
/// by different peers are resolved into a single value using document id seniority to establish
/// order.
#[wasm_bindgen]
pub struct YMap(RefCell<SharedType<Map, HashMap<String, JsValue>>>);

impl From<Map> for YMap {
    fn from(v: Map) -> Self {
        YMap(SharedType::new(v))
    }
}

#[wasm_bindgen]
impl YMap {
    /// Creates a new preliminary instance of a `YMap` shared data type, with its state
    /// initialized to provided parameter.
    ///
    /// Preliminary instances can be nested into other shared data types such as `YArray` and `YMap`.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[wasm_bindgen(constructor)]
    pub fn new(init: Option<js_sys::Object>) -> Self {
        let map = if let Some(object) = init {
            let mut map = HashMap::new();
            let entries = js_sys::Object::entries(&object);
            for tuple in entries.iter() {
                let tuple = js_sys::Array::from(&tuple);
                let key = tuple.get(0).as_string().unwrap();
                let value = tuple.get(1);
                map.insert(key, value);
            }
            map
        } else {
            HashMap::new()
        };
        YMap(SharedType::prelim(map))
    }

    /// Returns true if this is a preliminary instance of `YMap`.
    ///
    /// Preliminary instances can be nested into other shared data types such as `YArray` and `YMap`.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[wasm_bindgen(method, getter)]
    pub fn prelim(&self) -> bool {
        if let SharedType::Prelim(_) = &*self.0.borrow() {
            true
        } else {
            false
        }
    }

    /// Returns a number of entries stored within this instance of `YMap`.
    #[wasm_bindgen(method)]
    pub fn length(&self, txn: &YTransaction) -> u32 {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => v.len(txn),
            SharedType::Prelim(v) => v.len() as u32,
        }
    }

    /// Converts contents of this `YMap` instance into a JSON representation.
    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &YTransaction) -> JsValue {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => any_into_js(v.to_json(txn)),
            SharedType::Prelim(v) => {
                let map = js_sys::Object::new();
                for (k, v) in v.iter() {
                    js_sys::Reflect::set(&map, &k.into(), v).unwrap();
                }
                map.into()
            }
        }
    }

    /// Sets a given `key`-`value` entry within this instance of `YMap`. If another entry was
    /// already stored under given `key`, it will be overridden with new `value`.
    #[wasm_bindgen(js_name = set)]
    pub fn set(&self, txn: &mut YTransaction, key: &str, value: JsValue) {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                v.insert(txn, key.to_string(), JsValueWrapper(value));
            }
            SharedType::Prelim(v) => {
                v.insert(key.to_string(), value);
            }
        }
    }

    /// Removes an entry identified by a given `key` from this instance of `YMap`, if such exists.
    #[wasm_bindgen(js_name = delete)]
    pub fn delete(&mut self, txn: &mut YTransaction, key: &str) {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                v.remove(txn, key);
            }
            SharedType::Prelim(v) => {
                v.remove(key);
            }
        }
    }

    /// Returns value of an entry stored under given `key` within this instance of `YMap`,
    /// or `undefined` if no such entry existed.
    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, txn: &mut YTransaction, key: &str) -> JsValue {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => {
                if let Some(value) = v.get(txn, key) {
                    value_into_js(value)
                } else {
                    JsValue::undefined()
                }
            }
            SharedType::Prelim(v) => {
                if let Some(value) = v.get(key) {
                    value.clone()
                } else {
                    JsValue::undefined()
                }
            }
        }
    }

    /// Returns an iterator that can be used to traverse over all entries stored within this
    /// instance of `YMap`. Order of entry is not specified.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const doc = new YDoc()
    /// const map = doc.getMap('name')
    /// const txn = doc.beginTransaction()
    /// try {
    ///     map.set(txn, 'key1', 'value1')
    ///     map.set(txn, 'key2', true)
    ///
    ///     for (let [key, value] of map.entries(txn)) {
    ///         console.log(key, value)
    ///     }
    /// } finally {
    ///     txn.free()
    /// }
    /// ```
    #[wasm_bindgen(js_name = entries)]
    pub fn entries(&self, txn: &mut YTransaction) -> JsValue {
        to_iter(match &*self.0.borrow() {
            SharedType::Integrated(v) => unsafe {
                let this: *const Map = v;
                let tx: *const Transaction<'static> = txn.0.deref();
                let static_iter: ManuallyDrop<MapIter<'static, 'static>> =
                    ManuallyDrop::new((*this).iter(tx.as_ref().unwrap()));
                YMapIterator(static_iter).into()
            },
            SharedType::Prelim(v) => unsafe {
                let this: *const HashMap<String, JsValue> = v;
                let static_iter: ManuallyDrop<
                    std::collections::hash_map::Iter<'static, String, JsValue>,
                > = ManuallyDrop::new((*this).iter());
                PrelimMapIterator(static_iter).into()
            },
        })
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
                let tuple = js_sys::Array::new_with_length(2);
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
pub struct PrelimMapIterator(
    ManuallyDrop<std::collections::hash_map::Iter<'static, String, JsValue>>,
);

impl Drop for PrelimMapIterator {
    fn drop(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.0) }
    }
}

#[wasm_bindgen]
impl PrelimMapIterator {
    #[wasm_bindgen]
    pub fn next(&mut self) -> IteratorNext {
        if let Some((key, value)) = self.0.next() {
            let array = js_sys::Array::new_with_length(2);
            array.push(&JsValue::from(key));
            array.push(value);
            IteratorNext::new(array.into())
        } else {
            IteratorNext::finished()
        }
    }
}

/// XML element data type. It represents an XML node, which can contain key-value attributes
/// (interpreted as strings) as well as other nested XML elements or rich text (represented by
/// `YXmlText` type).
///
/// In terms of conflict resolution, `YXmlElement` uses following rules:
///
/// - Attribute updates use logical last-write-wins principle, meaning the past updates are
///   automatically overridden and discarded by newer ones, while concurrent updates made by
///   different peers are resolved into a single value using document id seniority to establish
///   an order.
/// - Child node insertion uses sequencing rules from other Yrs collections - elements are inserted
///   using interleave-resistant algorithm, where order of concurrent inserts at the same index
///   is established using peer's document id seniority.
#[wasm_bindgen]
pub struct YXmlElement(XmlElement);

#[wasm_bindgen]
impl YXmlElement {
    /// Returns a tag name of this XML node.
    #[wasm_bindgen(method, getter)]
    pub fn name(&self) -> String {
        self.0.tag().to_string()
    }

    /// Returns a number of child XML nodes stored within this `YXMlElement` instance.
    #[wasm_bindgen(js_name = length)]
    pub fn length(&self, txn: &YTransaction) -> u32 {
        self.0.len(txn)
    }

    /// Inserts a new instance of `YXmlElement` as a child of this XML node and returns it.
    #[wasm_bindgen(js_name = insertXmlElement)]
    pub fn insert_xml_element(
        &self,
        txn: &mut YTransaction,
        index: u32,
        name: &str,
    ) -> YXmlElement {
        YXmlElement(self.0.insert_elem(txn, index, name))
    }

    /// Inserts a new instance of `YXmlText` as a child of this XML node and returns it.
    #[wasm_bindgen(js_name = insertXmlText)]
    pub fn insert_xml_text(&self, txn: &mut YTransaction, index: u32) -> YXmlText {
        YXmlText(self.0.insert_text(txn, index))
    }

    /// Removes a range of children XML nodes from this `YXmlElement` instance,
    /// starting at given `index`.
    #[wasm_bindgen(js_name = delete)]
    pub fn delete(&self, txn: &mut YTransaction, index: u32, length: u32) {
        self.0.remove_range(txn, index, length)
    }

    /// Appends a new instance of `YXmlElement` as the last child of this XML node and returns it.
    #[wasm_bindgen(js_name = pushXmlElement)]
    pub fn push_xml_element(&self, txn: &mut YTransaction, name: &str) -> YXmlElement {
        YXmlElement(self.0.push_elem_back(txn, name))
    }

    /// Appends a new instance of `YXmlText` as the last child of this XML node and returns it.
    #[wasm_bindgen(js_name = pushXmlText)]
    pub fn push_xml_text(&self, txn: &mut YTransaction) -> YXmlText {
        YXmlText(self.0.push_text_back(txn))
    }

    /// Returns a first child of this XML node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node has not children.
    #[wasm_bindgen(js_name = firstChild)]
    pub fn first_child(&self, txn: &YTransaction) -> JsValue {
        if let Some(xml) = self.0.first_child(txn) {
            xml_into_js(xml)
        } else {
            JsValue::undefined()
        }
    }

    /// Returns a next XML sibling node of this XMl node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node is a last child of
    /// parent XML node.
    #[wasm_bindgen(js_name = nextSibling)]
    pub fn next_sibling(&self, txn: &YTransaction) -> JsValue {
        if let Some(xml) = self.0.next_sibling(txn) {
            xml_into_js(xml)
        } else {
            JsValue::undefined()
        }
    }

    /// Returns a previous XML sibling node of this XMl node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node is a first child
    /// of parent XML node.
    #[wasm_bindgen(js_name = prevSibling)]
    pub fn prev_sibling(&self, txn: &YTransaction) -> JsValue {
        if let Some(xml) = self.0.prev_sibling(txn) {
            xml_into_js(xml)
        } else {
            JsValue::undefined()
        }
    }

    /// Returns a parent `YXmlElement` node or `undefined` if current node has no parent assigned.
    #[wasm_bindgen(js_name = parent)]
    pub fn parent(&self, txn: &YTransaction) -> JsValue {
        if let Some(xml) = self.0.parent(txn) {
            xml_into_js(Xml::Element(xml))
        } else {
            JsValue::undefined()
        }
    }

    /// Returns a string representation of this XML node.
    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &YTransaction) -> String {
        self.0.to_string(txn)
    }

    /// Sets a `name` and `value` as new attribute for this XML node. If an attribute with the same
    /// `name` already existed on that node, its value with be overridden with a provided one.
    #[wasm_bindgen(js_name = setAttribute)]
    pub fn set_attribute(&self, txn: &mut YTransaction, name: &str, value: &str) {
        self.0.insert_attribute(txn, name, value)
    }

    /// Returns a value of an attribute given its `name`. If no attribute with such name existed,
    /// `null` will be returned.
    #[wasm_bindgen(js_name = getAttribute)]
    pub fn get_attribute(&self, txn: &YTransaction, name: &str) -> Option<String> {
        self.0.get_attribute(txn, name)
    }

    /// Removes an attribute from this XML node, given its `name`.
    #[wasm_bindgen(js_name = removeAttribute)]
    pub fn remove_attribute(&self, txn: &mut YTransaction, name: &str) {
        self.0.remove_attribute(txn, name);
    }

    /// Returns an iterator that enables to traverse over all attributes of this XML node in
    /// unspecified order.
    #[wasm_bindgen(js_name = attributes)]
    pub fn attributes(&self, txn: &YTransaction) -> JsValue {
        to_iter(unsafe {
            let this: *const XmlElement = &self.0;
            let tx: *const Transaction<'static> = txn.0.deref();
            let static_iter: ManuallyDrop<Attributes<'static, 'static>> =
                ManuallyDrop::new((*this).attributes(tx.as_ref().unwrap()));
            YXmlAttributes(static_iter).into()
        })
    }

    /// Returns an iterator that enables a deep traversal of this XML node - starting from first
    /// child over this XML node successors using depth-first strategy.
    #[wasm_bindgen(js_name = treeWalker)]
    pub fn tree_walker(&self, txn: &YTransaction) -> JsValue {
        to_iter(unsafe {
            let this: *const XmlElement = &self.0;
            let tx: *const Transaction<'static> = txn.0.deref();
            let static_iter: ManuallyDrop<TreeWalker<'static, 'static>> =
                ManuallyDrop::new((*this).successors(tx.as_ref().unwrap()));
            YXmlTreeWalker(static_iter).into()
        })
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
                let tuple = js_sys::Array::new_with_length(2);
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

/// A shared data type used for collaborative text editing, that can be used in a context of
/// `YXmlElement` nodee. It enables multiple users to add and remove chunks of text in efficient
/// manner. This type is internally represented as a mutable double-linked list of text chunks
/// - an optimization occurs during `YTransaction.commit`, which allows to squash multiple
/// consecutively inserted characters together as a single chunk of text even between transaction
/// boundaries in order to preserve more efficient memory model.
///
/// Just like `YXmlElement`, `YXmlText` can be marked with extra metadata in form of attributes.
///
/// `YXmlText` structure internally uses UTF-8 encoding and its length is described in a number of
/// bytes rather than individual characters (a single UTF-8 code point can consist of many bytes).
///
/// Like all Yrs shared data types, `YXmlText` is resistant to the problem of interleaving (situation
/// when characters inserted one after another may interleave with other peers concurrent inserts
/// after merging all updates together). In case of Yrs conflict resolution is solved by using
/// unique document id to determine correct and consistent ordering.
#[wasm_bindgen]
pub struct YXmlText(XmlText);

#[wasm_bindgen]
impl YXmlText {
    /// Returns length of an underlying string stored in this `YXmlText` instance,
    /// understood as a number of UTF-8 encoded bytes.
    #[wasm_bindgen(method, getter)]
    pub fn length(&self) -> u32 {
        self.0.len()
    }

    /// Inserts a given `chunk` of text into this `YXmlText` instance, starting at a given `index`.
    #[wasm_bindgen(js_name = insert)]
    pub fn insert(&self, txn: &mut YTransaction, index: i32, chunk: &str) {
        self.0.insert(txn, index as u32, chunk)
    }

    /// Appends a given `chunk` of text at the end of `YXmlText` instance.
    #[wasm_bindgen(js_name = push)]
    pub fn push(&self, txn: &mut YTransaction, chunk: &str) {
        self.0.push(txn, chunk)
    }

    /// Deletes a specified range of of characters, starting at a given `index`.
    /// Both `index` and `length` are counted in terms of a number of UTF-8 character bytes.
    #[wasm_bindgen(js_name = delete)]
    pub fn delete(&self, txn: &mut YTransaction, index: u32, length: u32) {
        self.0.remove_range(txn, index, length)
    }

    /// Returns a next XML sibling node of this XMl node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node is a last child of
    /// parent XML node.
    #[wasm_bindgen(js_name = nextSibling)]
    pub fn next_sibling(&self, txn: &YTransaction) -> JsValue {
        if let Some(xml) = self.0.next_sibling(txn) {
            xml_into_js(xml)
        } else {
            JsValue::undefined()
        }
    }

    /// Returns a previous XML sibling node of this XMl node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node is a first child
    /// of parent XML node.
    #[wasm_bindgen(js_name = prevSibling)]
    pub fn prev_sibling(&self, txn: &YTransaction) -> JsValue {
        if let Some(xml) = self.0.prev_sibling(txn) {
            xml_into_js(xml)
        } else {
            JsValue::undefined()
        }
    }

    /// Returns a parent `YXmlElement` node or `undefined` if current node has no parent assigned.
    #[wasm_bindgen(js_name = parent)]
    pub fn parent(&self, txn: &YTransaction) -> JsValue {
        if let Some(xml) = self.0.parent(txn) {
            xml_into_js(Xml::Element(xml))
        } else {
            JsValue::undefined()
        }
    }

    /// Returns an underlying string stored in this `YXmlText` instance.
    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &YTransaction) -> String {
        self.0.to_string(txn)
    }

    /// Sets a `name` and `value` as new attribute for this XML node. If an attribute with the same
    /// `name` already existed on that node, its value with be overridden with a provided one.
    #[wasm_bindgen(js_name = setAttribute)]
    pub fn set_attribute(&self, txn: &mut YTransaction, name: &str, value: &str) {
        self.0.insert_attribute(txn, name, value);
    }

    /// Returns a value of an attribute given its `name`. If no attribute with such name existed,
    /// `null` will be returned.
    #[wasm_bindgen(js_name = getAttribute)]
    pub fn get_attribute(&self, txn: &YTransaction, name: &str) -> Option<String> {
        self.0.get_attribute(txn, name)
    }

    /// Removes an attribute from this XML node, given its `name`.
    #[wasm_bindgen(js_name = removeAttribute)]
    pub fn remove_attribute(&self, txn: &mut YTransaction, name: &str) {
        self.0.remove_attribute(txn, name);
    }

    /// Returns an iterator that enables to traverse over all attributes of this XML node in
    /// unspecified order.
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

#[repr(transparent)]
struct JsValueWrapper(JsValue);

impl Prelim for JsValueWrapper {
    fn into_content(self, _txn: &mut Transaction, ptr: TypePtr) -> (ItemContent, Option<Self>) {
        let content = if let Some(any) = js_into_any(&self.0) {
            ItemContent::Any(vec![any])
        } else if let Ok(shared) = Shared::try_from(&self.0) {
            if shared.is_prelim() {
                let branch = BranchRef::new(Branch::new(ptr, shared.type_ref(), None));
                ItemContent::Type(branch)
            } else {
                panic!("Cannot integrate this type")
            }
        } else {
            panic!("Cannot integrate this type")
        };

        let this = if let ItemContent::Type(_) = &content {
            Some(self)
        } else {
            None
        };

        (content, this)
    }

    fn integrate(self, txn: &mut Transaction, inner_ref: BranchRef) {
        if let Ok(shared) = Shared::try_from(&self.0) {
            if shared.is_prelim() {
                match shared {
                    Shared::Text(v) => {
                        let text = Text::from(inner_ref);
                        if let SharedType::Prelim(v) =
                            v.0.replace(SharedType::Integrated(text.clone()))
                        {
                            text.push(txn, v.as_str());
                        }
                    }
                    Shared::Array(v) => {
                        let array = Array::from(inner_ref);
                        if let SharedType::Prelim(items) =
                            v.0.replace(SharedType::Integrated(array.clone()))
                        {
                            let len = array.len();
                            insert_at(&array, txn, len, items);
                        }
                    }
                    Shared::Map(v) => {
                        let map = Map::from(inner_ref);
                        if let SharedType::Prelim(entries) =
                            v.0.replace(SharedType::Integrated(map.clone()))
                        {
                            for (k, v) in entries {
                                map.insert(txn, k, JsValueWrapper(v));
                            }
                        }
                    }
                    _ => panic!("Cannot integrate this type"),
                }
            }
        }
    }
}

fn insert_at(dst: &Array, txn: &mut Transaction, index: u32, src: Vec<JsValue>) {
    let mut j = index;
    let mut i = 0;
    while i < src.len() {
        let mut anys = Vec::default();
        while i < src.len() {
            let js = &src[i];
            if let Some(any) = js_into_any(js) {
                anys.push(any);
                i += 1;
            } else {
                break;
            }
        }

        if !anys.is_empty() {
            let len = anys.len() as u32;
            dst.insert_range(txn, j, anys);
            j += len;
        } else {
            let js = &src[i];
            let wrapper = JsValueWrapper(js.clone());
            dst.insert(txn, j, wrapper);
            i += 1;
            j += 1;
        }
    }
}

fn js_into_any(v: &JsValue) -> Option<Any> {
    if v.is_string() {
        Some(Any::String(v.as_string()?))
    } else if v.is_bigint() {
        let i = js_sys::BigInt::from(v.clone()).as_f64()?;
        Some(Any::BigInt(i as i64))
    } else if v.is_null() {
        Some(Any::Null)
    } else if v.is_undefined() {
        Some(Any::Undefined)
    } else if let Some(f) = v.as_f64() {
        Some(Any::Number(f))
    } else if let Some(b) = v.as_bool() {
        Some(Any::Bool(b))
    } else if js_sys::Array::is_array(v) {
        let array = js_sys::Array::from(v);
        let mut result = Vec::with_capacity(array.length() as usize);
        for value in array.iter() {
            result.push(js_into_any(&value)?);
        }
        Some(Any::Array(result))
    } else if v.is_object() {
        if let Ok(_) = Shared::try_from(v) {
            None
        } else {
            let object = js_sys::Object::from(v.clone());
            let mut result = HashMap::new();
            let entries = js_sys::Object::entries(&object);
            for tuple in entries.iter() {
                let tuple = js_sys::Array::from(&tuple);
                let key = tuple.get(0).as_string()?;
                let value = js_into_any(&tuple.get(1))?;
                result.insert(key, value);
            }
            Some(Any::Map(result))
        }
    } else {
        None
    }
}

fn any_into_js(v: Any) -> JsValue {
    match v {
        Any::Null => JsValue::NULL,
        Any::Undefined => JsValue::UNDEFINED,
        Any::Bool(v) => JsValue::from_bool(v),
        Any::Number(v) => JsValue::from(v),
        Any::BigInt(v) => JsValue::from(v),
        Any::String(v) => JsValue::from(&v),
        Any::Buffer(v) => {
            let v = Uint8Array::from(v.as_ref());
            v.into()
        }
        Any::Array(v) => {
            let a = js_sys::Array::new();
            for value in v {
                a.push(&any_into_js(value));
            }
            a.into()
        }
        Any::Map(v) => {
            let m = js_sys::Object::new();
            for (k, v) in v {
                let key = JsValue::from(&k);
                let value = any_into_js(v);
                js_sys::Reflect::set(&m, &key, &value).unwrap();
            }
            m.into()
        }
    }
}

fn value_into_js(v: Value) -> JsValue {
    match v {
        Value::Any(v) => any_into_js(v),
        Value::YText(v) => YText::from(v).into(),
        Value::YArray(v) => YArray::from(v).into(),
        Value::YMap(v) => YMap::from(v).into(),
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

enum Shared<'a> {
    Text(Ref<'a, YText>),
    Array(Ref<'a, YArray>),
    Map(Ref<'a, YMap>),
    XmlElement(Ref<'a, YXmlElement>),
    XmlText(Ref<'a, YXmlText>),
}

fn as_ref<'a, T>(js: u32) -> Ref<'a, T> {
    unsafe {
        let js = js as *mut wasm_bindgen::__rt::WasmRefCell<T>;
        (*js).borrow()
    }
}

impl<'a> TryFrom<&'a JsValue> for Shared<'a> {
    type Error = JsValue;

    fn try_from(js: &'a JsValue) -> Result<Self, Self::Error> {
        use js_sys::{Object, Reflect};
        let ctor_name = Object::get_prototype_of(js).constructor().name();
        let ptr = Reflect::get(js, &JsValue::from_str("ptr"))?;
        let ptr_u32: u32 = ptr.as_f64().ok_or(JsValue::NULL)? as u32;
        if ctor_name == "YText" {
            Ok(Shared::Text(as_ref(ptr_u32)))
        } else if ctor_name == "YArray" {
            Ok(Shared::Array(as_ref(ptr_u32)))
        } else if ctor_name == "YMap" {
            Ok(Shared::Map(as_ref(ptr_u32)))
        } else if ctor_name == "YXmlElement" {
            Ok(Shared::XmlElement(as_ref(ptr_u32)))
        } else if ctor_name == "YXmlText" {
            Ok(Shared::XmlText(as_ref(ptr_u32)))
        } else {
            Err(JsValue::NULL)
        }
    }
}

impl<'a> Shared<'a> {
    fn is_prelim(&self) -> bool {
        match self {
            Shared::Text(v) => v.prelim(),
            Shared::Array(v) => v.prelim(),
            Shared::Map(v) => v.prelim(),
            Shared::XmlElement(_) | Shared::XmlText(_) => false,
        }
    }

    fn type_ref(&self) -> TypeRefs {
        match self {
            Shared::Text(_) => TYPE_REFS_TEXT,
            Shared::Array(_) => TYPE_REFS_ARRAY,
            Shared::Map(_) => TYPE_REFS_MAP,
            Shared::XmlElement(_) => TYPE_REFS_XML_ELEMENT,
            Shared::XmlText(_) => TYPE_REFS_XML_TEXT,
        }
    }
}
