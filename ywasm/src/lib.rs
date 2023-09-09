use js_sys::{Object, Reflect, Uint8Array};
use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::__rt::{Ref, RefMut};
use wasm_bindgen::convert::{FromWasmAbi, IntoWasmAbi};
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::block::{ClientID, ItemContent, Prelim, Unused};
use yrs::types::array::ArrayEvent;
use yrs::types::map::MapEvent;
use yrs::types::text::{ChangeKind, Diff, TextEvent, YChange};
use yrs::types::xml::{XmlEvent, XmlTextEvent};
use yrs::types::{
    Attrs, Branch, BranchPtr, Change, DeepEventsSubscription, DeepObservable, Delta, EntryChange,
    Event, Events, Path, PathSegment, ToJson, TypeRef, Value
};
use yrs::undo::{EventKind, UndoEventSubscription};
use yrs::updates::decoder::{Decode, DecoderV1};
use yrs::updates::encoder::{Encode, Encoder, EncoderV1, EncoderV2};
use yrs::{
    Array, ArrayRef, Assoc, DeleteSet, DestroySubscription, Doc, GetString, IndexScope, Map,
    MapRef, Observable, Offset, OffsetKind, Options, Origin, ReadTxn, Snapshot, StateVector,
    StickyIndex, Store, SubdocsEvent, SubdocsEventIter, SubdocsSubscription, Subscription, Text,
    TextRef, Transact, Transaction, TransactionCleanupEvent, TransactionCleanupSubscription,
    TransactionMut, UndoManager, Update, UpdateSubscription, Xml, XmlElementPrelim, XmlElementRef,
    XmlFragment, XmlFragmentRef, XmlNode, XmlTextPrelim, XmlTextRef, ID, Any
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

impl AsRef<Doc> for YDoc {
    fn as_ref(&self) -> &Doc {
        &self.0
    }
}

impl From<Doc> for YDoc {
    fn from(doc: Doc) -> Self {
        YDoc(doc)
    }
}

#[wasm_bindgen]
impl YDoc {
    /// Creates a new ywasm document. If `id` parameter was passed it will be used as this document
    /// globally unique identifier (it's up to caller to ensure that requirement). Otherwise it will
    /// be assigned a randomly generated number.
    #[wasm_bindgen(constructor)]
    pub fn new(options: &JsValue) -> Self {
        let options = parse_options(options);
        Doc::with_options(options).into()
    }

    /// Returns a parent document of this document or null if current document is not sub-document.
    #[wasm_bindgen(getter, js_name = parentDoc)]
    pub fn parent_doc(&self) -> Option<YDoc> {
        let doc = self.0.parent_doc()?;
        Some(YDoc(doc))
    }

    /// Gets unique peer identifier of this `YDoc` instance.
    #[wasm_bindgen(getter)]
    pub fn id(&self) -> f64 {
        self.as_ref().client_id() as f64
    }

    /// Gets globally unique identifier of this `YDoc` instance.
    #[wasm_bindgen(getter)]
    pub fn guid(&self) -> String {
        self.as_ref().options().guid.to_string()
    }

    #[wasm_bindgen(getter, js_name = shouldLoad)]
    pub fn should_load(&self) -> bool {
        self.as_ref().options().should_load
    }

    #[wasm_bindgen(getter, js_name = autoLoad)]
    pub fn auto_load(&self) -> bool {
        self.as_ref().options().auto_load
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
    ///     const txn = this.readTransaction()
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
    #[wasm_bindgen(js_name = readTransaction)]
    pub fn read_transaction(&mut self) -> YTransaction {
        YTransaction::from(self.as_ref().transact())
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
    ///     const txn = this.writeTransaction()
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
    #[wasm_bindgen(js_name = writeTransaction)]
    pub fn write_transaction(&mut self, origin: JsValue) -> YTransaction {
        if origin.is_null() || origin.is_undefined() {
            YTransaction::from(self.as_ref().transact_mut())
        } else {
            let abi = origin.into_abi();
            YTransaction::from(self.as_ref().transact_mut_with(abi))
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
        self.as_ref().get_or_insert_text(name).into()
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
        self.as_ref().get_or_insert_array(name).into()
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
        self.as_ref().get_or_insert_map(name).into()
    }

    /// Returns a `YXmlFragment` shared data type, that's accessible for subsequent accesses using
    /// given `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YXmlFragment` instance.
    #[wasm_bindgen(js_name = getXmlFragment)]
    pub fn get_xml_fragment(&mut self, name: &str) -> YXmlFragment {
        YXmlFragment(self.as_ref().get_or_insert_xml_fragment(name))
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
        YXmlElement(self.as_ref().get_or_insert_xml_element(name))
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
        YXmlText(self.as_ref().get_or_insert_xml_text(name))
    }

    /// Subscribes given function to be called any time, a remote update is being applied to this
    /// document. Function takes an `Uint8Array` as a parameter which contains a lib0 v1 encoded
    /// update.
    ///
    /// Returns an observer, which can be freed in order to unsubscribe this callback.
    #[wasm_bindgen(js_name = onUpdate)]
    pub fn on_update(&mut self, f: js_sys::Function) -> YUpdateObserver {
        self.as_ref()
            .observe_update_v1(move |_, e| {
                let arg = Uint8Array::from(e.update.as_slice());
                f.call1(&JsValue::UNDEFINED, &arg).unwrap();
            })
            .unwrap()
            .into()
    }

    /// Subscribes given function to be called any time, a remote update is being applied to this
    /// document. Function takes an `Uint8Array` as a parameter which contains a lib0 v2 encoded
    /// update.
    ///
    /// Returns an observer, which can be freed in order to unsubscribe this callback.
    #[wasm_bindgen(js_name = onUpdateV2)]
    pub fn on_update_v2(&mut self, f: js_sys::Function) -> YUpdateObserver {
        self.as_ref()
            .observe_update_v2(move |_, e| {
                let arg = Uint8Array::from(e.update.as_slice());
                f.call1(&JsValue::UNDEFINED, &arg).unwrap();
            })
            .unwrap()
            .into()
    }

    /// Subscribes given function to be called, whenever a transaction created by this document is
    /// being committed.
    ///
    /// Returns an observer, which can be freed in order to unsubscribe this callback.
    #[wasm_bindgen(js_name = onAfterTransaction)]
    pub fn on_after_transaction(&mut self, f: js_sys::Function) -> YAfterTransactionObserver {
        self.as_ref()
            .observe_transaction_cleanup(move |_, e| {
                let arg: JsValue = YAfterTransactionEvent::new(e).into();
                f.call1(&JsValue::UNDEFINED, &arg).unwrap();
            })
            .unwrap()
            .into()
    }

    /// Subscribes given function to be called, whenever a subdocuments are being added, removed
    /// or loaded as children of a current document.
    ///
    /// Returns an observer, which can be freed in order to unsubscribe this callback.
    #[wasm_bindgen(js_name = onSubdocs)]
    pub fn on_subdocs(&mut self, f: js_sys::Function) -> YSubdocsObserver {
        self.as_ref()
            .observe_subdocs(move |_, e| {
                let arg: JsValue = YSubdocsEvent::new(e).into();
                f.call1(&JsValue::UNDEFINED, &arg).unwrap();
            })
            .unwrap()
            .into()
    }

    /// Subscribes given function to be called, whenever current document is being destroyed.
    ///
    /// Returns an observer, which can be freed in order to unsubscribe this callback.
    #[wasm_bindgen(js_name = onDestroy)]
    pub fn on_destroy(&mut self, f: js_sys::Function) -> YDestroyObserver {
        self.as_ref()
            .observe_destroy(move |_, e| {
                let arg: JsValue = YDoc::from(e.clone()).into();
                f.call1(&JsValue::UNDEFINED, &arg).unwrap();
            })
            .unwrap()
            .into()
    }

    /// Notify the parent document that you request to load data into this subdocument
    /// (if it is a subdocument).
    #[wasm_bindgen(js_name = load)]
    pub fn load(&self, parent_txn: &ImplicitTransaction) {
        if let Some(mut txn) = get_txn_mut(parent_txn) {
            self.0.load(txn.as_mut())
        } else {
            if let Some(parent) = self.0.parent_doc() {
                let mut txn = parent.transact_mut();
                self.0.load(&mut txn);
            }
        }
    }

    /// Emit `onDestroy` event and unregister all event handlers.
    #[wasm_bindgen(js_name = destroy)]
    pub fn destroy(&mut self, parent_txn: &ImplicitTransaction) {
        if let Some(mut txn) = get_txn_mut(parent_txn) {
            self.0.destroy(txn.as_mut())
        } else {
            if let Some(parent) = self.0.parent_doc() {
                let mut txn = parent.transact_mut();
                self.0.destroy(&mut txn);
            }
        }
    }

    /// Returns a list of sub-documents existings within the scope of this document.
    #[wasm_bindgen(js_name = getSubdocs)]
    pub fn subdocs(&self, txn: &ImplicitTransaction) -> js_sys::Array {
        let doc = self.as_ref();
        let buf = js_sys::Array::new();
        if let Some(txn) = get_txn(txn) {
            for doc in txn.subdocs() {
                let doc = YDoc::from(doc.clone());
                buf.push(&doc.into());
            }
        } else {
            let txn = doc.transact();
            for doc in txn.subdocs() {
                let doc = YDoc::from(doc.clone());
                buf.push(&doc.into());
            }
        }
        buf
    }

    /// Returns a list of unique identifiers of the sub-documents existings within the scope of
    /// this document.
    #[wasm_bindgen(js_name = getSubdocGuids)]
    pub fn subdoc_guids(&self, txn: &ImplicitTransaction) -> js_sys::Set {
        let doc = self.as_ref();
        let buf = js_sys::Set::new(&js_sys::Array::new());
        if let Some(txn) = get_txn(txn) {
            for uid in txn.subdoc_guids() {
                let str = uid.to_string();
                buf.add(&str.into());
            }
        } else {
            let txn = doc.transact();
            for uid in txn.subdoc_guids() {
                let str = uid.to_string();
                buf.add(&str.into());
            }
        }
        buf
    }
}

fn parse_options(js: &JsValue) -> Options {
    let mut options = Options::default();
    options.offset_kind = OffsetKind::Utf16;
    if js.is_object() {
        if let Some(client_id) = js_sys::Reflect::get(js, &JsValue::from_str("clientID"))
            .ok()
            .and_then(|v| v.as_f64())
        {
            options.client_id = client_id as u32 as ClientID;
        }

        if let Some(guid) = js_sys::Reflect::get(js, &JsValue::from_str("guid"))
            .ok()
            .and_then(|v| v.as_string())
        {
            options.guid = guid.into();
        }

        if let Some(collection_id) = js_sys::Reflect::get(js, &JsValue::from_str("collectionid"))
            .ok()
            .and_then(|v| v.as_string())
        {
            options.collection_id = Some(collection_id);
        }

        if let Some(gc) = js_sys::Reflect::get(js, &JsValue::from_str("gc"))
            .ok()
            .and_then(|v| v.as_bool())
        {
            options.skip_gc = !gc;
        }

        if let Some(auto_load) = js_sys::Reflect::get(js, &JsValue::from_str("autoLoad"))
            .ok()
            .and_then(|v| v.as_bool())
        {
            options.auto_load = auto_load;
        }

        if let Some(should_load) = js_sys::Reflect::get(js, &JsValue::from_str("shouldLoad"))
            .ok()
            .and_then(|v| v.as_bool())
        {
            options.should_load = should_load;
        }
    }

    options
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
    doc.read_transaction().state_vector_v1()
}

/// Returns a string dump representation of a given `update` encoded using lib0 v1 encoding.
#[wasm_bindgen(js_name = debugUpdateV1)]
pub fn debug_update_v1(update: Uint8Array) -> Result<String, JsValue> {
    let update: Vec<u8> = update.to_vec();
    let mut decoder = DecoderV1::from(update.as_slice());
    match Update::decode(&mut decoder) {
        Ok(update) => Ok(format!("{:#?}", update)),
        Err(e) => Err(JsValue::from(e.to_string())),
    }
}

/// Returns a string dump representation of a given `update` encoded using lib0 v2 encoding.
#[wasm_bindgen(js_name = debugUpdateV2)]
pub fn debug_update_v2(update: Uint8Array) -> Result<String, JsValue> {
    let mut update: Vec<u8> = update.to_vec();
    match Update::decode_v2(update.as_mut_slice()) {
        Ok(update) => Ok(format!("{:#?}", update)),
        Err(e) => Err(JsValue::from(e.to_string())),
    }
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
pub fn encode_state_as_update(
    doc: &mut YDoc,
    vector: Option<Uint8Array>,
) -> Result<Uint8Array, JsValue> {
    doc.read_transaction().diff_v1(vector)
}

/// Encodes all updates that have happened since a given version `vector` into a compact delta
/// representation using lib0 v2 encoding. If `vector` parameter has not been provided, generated
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
/// const remoteDelta = encodeStateAsUpdateV2(remoteDoc, localSV)
///
/// applyUpdate(localDoc, remoteDelta)
/// ```
#[wasm_bindgen(js_name = encodeStateAsUpdateV2)]
pub fn encode_state_as_update_v2(
    doc: &mut YDoc,
    vector: Option<Uint8Array>,
) -> Result<Uint8Array, JsValue> {
    doc.read_transaction().diff_v2(vector)
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
/// applyUpdateV2(localDoc, remoteDelta)
/// ```
#[wasm_bindgen(js_name = applyUpdate)]
pub fn apply_update(doc: &mut YDoc, diff: Uint8Array, origin: JsValue) -> Result<(), JsValue> {
    doc.write_transaction(origin).apply_v1(diff)
}

/// Applies delta update generated by the remote document replica to a current document. This
/// method assumes that a payload maintains lib0 v2 encoding format.
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
/// const remoteDelta = encodeStateAsUpdateV2(remoteDoc, localSV)
///
/// applyUpdateV2(localDoc, remoteDelta)
/// ```
#[wasm_bindgen(js_name = applyUpdateV2)]
pub fn apply_update_v2(doc: &mut YDoc, diff: Uint8Array, origin: JsValue) -> Result<(), JsValue> {
    doc.write_transaction(origin).apply_v2(diff)
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "YTransaction | null = null")]
    pub type ImplicitTransaction;
}

const WASM_REF_PTR: &str = "__wbg_ptr";

fn get_txn_mut<'a>(txn: &'a ImplicitTransaction) -> Option<RefMut<'static, YTransaction>> {
    use wasm_bindgen::convert::RefMutFromWasmAbi;

    let js: &JsValue = txn.as_ref();
    if js.is_undefined() || js.is_null() {
        return None;
    } else if let Ok(js_ptr) = Reflect::get(js, &JsValue::from_str(WASM_REF_PTR)) {
        if let Some(ptr) = js_ptr.as_f64() {
            unsafe {
                let txn = YTransaction::ref_mut_from_abi(ptr as u32);
                return Some(txn);
            }
        }
    }
    panic!("Provided object reference doesn't refer to wasm-bindgen object instance.")
}

fn get_txn<'a>(txn: &'a ImplicitTransaction) -> Option<Ref<'static, YTransaction>> {
    use wasm_bindgen::convert::RefFromWasmAbi;

    let js: &JsValue = txn.as_ref();
    if js.is_undefined() || js.is_null() {
        return None;
    } else if let Ok(js_ptr) = Reflect::get(js, &JsValue::from_str(WASM_REF_PTR)) {
        if let Some(ptr) = js_ptr.as_f64() {
            unsafe {
                let txn = YTransaction::ref_from_abi(ptr as u32);
                return Some(txn);
            }
        }
    }
    panic!("Provided object reference doesn't refer to wasm-bindgen object instance.")
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
pub struct YTransaction(InnerTxn);

enum InnerTxn {
    ReadOnly(ManuallyDrop<Transaction<'static>>),
    ReadWrite(ManuallyDrop<TransactionMut<'static>>),
}

impl YTransaction {
    fn try_mut(&mut self) -> Option<&mut TransactionMut<'static>> {
        match &mut self.0 {
            InnerTxn::ReadOnly(_) => None,
            InnerTxn::ReadWrite(txn) => Some(txn),
        }
    }

    fn as_mut(&mut self) -> &mut TransactionMut<'static> {
        self.try_mut()
            .expect("Read-write transaction required. Provided one is read-only.")
    }
}

impl Drop for InnerTxn {
    fn drop(&mut self) {
        match self {
            InnerTxn::ReadOnly(txn) => unsafe { ManuallyDrop::drop(txn) },
            InnerTxn::ReadWrite(txn) => unsafe { ManuallyDrop::drop(txn) },
        }
    }
}

impl ReadTxn for YTransaction {
    fn store(&self) -> &Store {
        match &self.0 {
            InnerTxn::ReadOnly(txn) => txn.store(),
            InnerTxn::ReadWrite(txn) => txn.store(),
        }
    }
}

impl<'doc> From<Transaction<'doc>> for YTransaction {
    fn from(txn: Transaction<'doc>) -> Self {
        let txn: Transaction<'static> = unsafe { std::mem::transmute(txn) };
        YTransaction(InnerTxn::ReadOnly(ManuallyDrop::new(txn)))
    }
}

impl<'doc> From<TransactionMut<'doc>> for YTransaction {
    fn from(txn: TransactionMut<'doc>) -> Self {
        let txn: TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        YTransaction(InnerTxn::ReadWrite(ManuallyDrop::new(txn)))
    }
}

#[wasm_bindgen]
impl YTransaction {
    /// Returns true if current transaction can be used only for read operations.
    #[wasm_bindgen(getter, js_name = isReadOnly)]
    pub fn is_readonly(&mut self) -> bool {
        match &self.0 {
            InnerTxn::ReadOnly(_) => true,
            InnerTxn::ReadWrite(_) => false,
        }
    }

    /// Returns true if current transaction can be used only for updating the document store.
    #[wasm_bindgen(getter, js_name = isWriteable)]
    pub fn is_writeable(&mut self) -> bool {
        !self.is_readonly()
    }

    /// Returns true if current transaction can be used only for updating the document store.
    #[wasm_bindgen(getter, js_name = origin)]
    pub fn origin(&mut self) -> JsValue {
        match &self.0 {
            InnerTxn::ReadOnly(_) => JsValue::NULL,
            InnerTxn::ReadWrite(t) => {
                if let Some(o) = t.origin() {
                    let be: [u8; 4] = o.as_ref().try_into().unwrap();
                    unsafe { JsValue::from_abi(u32::from_be_bytes(be)) }
                } else {
                    JsValue::NULL
                }
            }
        }
    }

    /// Triggers a post-update series of operations without `free`ing the transaction. This includes
    /// compaction and optimization of internal representation of updates, triggering events etc.
    /// ywasm transactions are auto-committed when they are `free`d.
    #[wasm_bindgen(js_name = commit)]
    pub fn commit(&mut self) {
        match &mut self.0 {
            InnerTxn::ReadOnly(_) => {}
            InnerTxn::ReadWrite(txn) => txn.commit(),
        }
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
        let sv = self.state_vector();
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
    pub fn diff_v1(&self, vector: Option<Uint8Array>) -> Result<Uint8Array, JsValue> {
        let mut encoder = EncoderV1::new();
        let sv = if let Some(vector) = vector {
            match StateVector::decode_v1(vector.to_vec().as_slice()) {
                Ok(sv) => sv,
                Err(e) => {
                    return Err(JsValue::from(e.to_string()));
                }
            }
        } else {
            StateVector::default()
        };
        self.encode_diff(&sv, &mut encoder);
        let payload = encoder.to_vec();
        Ok(Uint8Array::from(&payload[..payload.len()]))
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
    ///     const remoteDelta = remoteTxn.diffV2(localSv)
    ///     localTxn.applyV2(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    #[wasm_bindgen(js_name = diffV2)]
    pub fn diff_v2(&self, vector: Option<Uint8Array>) -> Result<Uint8Array, JsValue> {
        let mut encoder = EncoderV2::new();
        let sv = if let Some(vector) = vector {
            match StateVector::decode_v1(vector.to_vec().as_slice()) {
                Ok(sv) => sv,
                Err(e) => {
                    return Err(JsValue::from(e.to_string()));
                }
            }
        } else {
            StateVector::default()
        };
        self.encode_diff(&sv, &mut encoder);
        let payload = encoder.to_vec();
        Ok(Uint8Array::from(&payload[..payload.len()]))
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
    pub fn apply_v1(&mut self, diff: Uint8Array) -> Result<(), JsValue> {
        let diff: Vec<u8> = diff.to_vec();
        let mut decoder = DecoderV1::from(diff.as_slice());
        match Update::decode(&mut decoder) {
            Ok(update) => self.try_apply(update),
            Err(e) => Err(JsValue::from(e.to_string())),
        }
    }

    fn try_apply(&mut self, update: Update) -> Result<(), JsValue> {
        if let Some(txn) = self.try_mut() {
            txn.apply_update(update);
            Ok(())
        } else {
            Err(JsValue::from_str(
                "cannot apply an update using a read-only transaction",
            ))
        }
    }

    /// Applies delta update generated by the remote document replica to a current transaction's
    /// document. This method assumes that a payload maintains lib0 v2 encoding format.
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
    ///     const remoteDelta = remoteTxn.diffV2(localSv)
    ///     localTxn.applyV2(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    #[wasm_bindgen(js_name = applyV2)]
    pub fn apply_v2(&mut self, diff: Uint8Array) -> Result<(), JsValue> {
        let mut diff: Vec<u8> = diff.to_vec();
        match Update::decode_v2(&mut diff) {
            Ok(update) => self.try_apply(update),
            Err(e) => Err(JsValue::from(e.to_string())),
        }
    }

    #[wasm_bindgen(js_name = encodeUpdate)]
    pub fn encode_update(&mut self) -> Uint8Array {
        let out = match &self.0 {
            InnerTxn::ReadOnly(_) => vec![0u8, 0u8],
            InnerTxn::ReadWrite(txn) => txn.encode_update_v1(),
        };
        Uint8Array::from(&out[..out.len()])
    }

    #[wasm_bindgen(js_name = encodeUpdateV2)]
    pub fn encode_update_v2(&mut self) -> Uint8Array {
        let out = match &self.0 {
            InnerTxn::ReadOnly(_) => vec![0u8, 0u8],
            InnerTxn::ReadWrite(txn) => txn.encode_update_v2(),
        };
        Uint8Array::from(&out[..out.len()])
    }
}

/// Event generated by `YArray.observe` method. Emitted during transaction commit phase.
#[wasm_bindgen]
pub struct YArrayEvent {
    inner: *const ArrayEvent,
    txn: *const TransactionMut<'static>,
    target: Option<JsValue>,
    delta: Option<JsValue>,
    origin: JsValue,
}

#[wasm_bindgen]
impl YArrayEvent {
    fn new<'doc>(event: &ArrayEvent, txn: &TransactionMut<'doc>) -> Self {
        let origin = from_origin(txn.origin());
        let inner = event as *const ArrayEvent;
        let txn: &TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        let txn = txn as *const TransactionMut<'static>;
        YArrayEvent {
            inner,
            txn,
            origin,
            target: None,
            delta: None,
        }
    }

    fn inner(&self) -> &ArrayEvent {
        unsafe { self.inner.as_ref().unwrap() }
    }

    fn txn(&self) -> &TransactionMut {
        unsafe { self.txn.as_ref().unwrap() }
    }

    #[wasm_bindgen(getter, js_name = origin)]
    pub fn origin(&self) -> JsValue {
        self.origin.clone()
    }

    /// Returns a current shared type instance, that current event changes refer to.
    #[wasm_bindgen(getter)]
    pub fn target(&mut self) -> JsValue {
        if let Some(target) = self.target.as_ref() {
            target.clone()
        } else {
            let target: JsValue = YArray::from(self.inner().target().clone()).into();
            self.target = Some(target.clone());
            target
        }
    }

    /// Returns an array of keys and indexes creating a path from root type down to current instance
    /// of shared type (accessible via `target` getter).
    #[wasm_bindgen]
    pub fn path(&self) -> JsValue {
        path_into_js(self.inner().path())
    }

    /// Returns a list of text changes made over corresponding `YArray` collection within
    /// bounds of current transaction. These changes follow a format:
    ///
    /// - { insert: any[] }
    /// - { delete: number }
    /// - { retain: number }
    #[wasm_bindgen(getter)]
    pub fn delta(&mut self) -> JsValue {
        if let Some(delta) = &self.delta {
            delta.clone()
        } else {
            let delta = self
                .inner()
                .delta(self.txn())
                .into_iter()
                .map(change_into_js);
            let mut result = js_sys::Array::new();
            result.extend(delta);
            let delta: JsValue = result.into();
            self.delta = Some(delta.clone());
            delta
        }
    }
}

/// Event generated by `YMap.observe` method. Emitted during transaction commit phase.
#[wasm_bindgen]
pub struct YMapEvent {
    inner: *const MapEvent,
    txn: *const TransactionMut<'static>,
    target: Option<JsValue>,
    keys: Option<JsValue>,
    origin: JsValue,
}

#[wasm_bindgen]
impl YMapEvent {
    fn new<'doc>(event: &MapEvent, txn: &TransactionMut<'doc>) -> Self {
        let origin = from_origin(txn.origin());
        let inner = event as *const MapEvent;
        let txn: &TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        let txn = txn as *const TransactionMut<'static>;
        YMapEvent {
            inner,
            txn,
            origin,
            target: None,
            keys: None,
        }
    }

    fn inner(&self) -> &MapEvent {
        unsafe { self.inner.as_ref().unwrap() }
    }

    fn txn(&self) -> &TransactionMut {
        unsafe { self.txn.as_ref().unwrap() }
    }

    #[wasm_bindgen(getter, js_name = origin)]
    pub fn origin(&self) -> JsValue {
        self.origin.clone()
    }

    /// Returns a current shared type instance, that current event changes refer to.
    #[wasm_bindgen(getter)]
    pub fn target(&mut self) -> JsValue {
        if let Some(target) = self.target.as_ref() {
            target.clone()
        } else {
            let target: JsValue = YMap::from(self.inner().target().clone()).into();
            self.target = Some(target.clone());
            target
        }
    }

    /// Returns an array of keys and indexes creating a path from root type down to current instance
    /// of shared type (accessible via `target` getter).
    #[wasm_bindgen]
    pub fn path(&self) -> JsValue {
        path_into_js(self.inner().path())
    }

    /// Returns a list of key-value changes made over corresponding `YMap` collection within
    /// bounds of current transaction. These changes follow a format:
    ///
    /// - { action: 'add'|'update'|'delete', oldValue: any|undefined, newValue: any|undefined }
    #[wasm_bindgen(getter)]
    pub fn keys(&mut self) -> JsValue {
        if let Some(keys) = &self.keys {
            keys.clone()
        } else {
            let keys = self.inner().keys(self.txn());
            let result = js_sys::Object::new();
            for (key, value) in keys.iter() {
                let key = JsValue::from(key.as_ref());
                let value = entry_change_into_js(value);
                js_sys::Reflect::set(&result, &key, &value).unwrap();
            }
            let keys: JsValue = result.into();
            self.keys = Some(keys.clone());
            keys
        }
    }
}

/// Event generated by `YYText.observe` method. Emitted during transaction commit phase.
#[wasm_bindgen]
pub struct YTextEvent {
    inner: *const TextEvent,
    txn: *const TransactionMut<'static>,
    target: Option<JsValue>,
    delta: Option<JsValue>,
    origin: JsValue,
}

#[wasm_bindgen]
impl YTextEvent {
    fn new<'doc>(event: &TextEvent, txn: &TransactionMut<'doc>) -> Self {
        let origin = from_origin(txn.origin());
        let inner = event as *const TextEvent;
        let txn: &TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        let txn = txn as *const TransactionMut<'static>;
        YTextEvent {
            inner,
            txn,
            origin,
            target: None,
            delta: None,
        }
    }

    fn inner(&self) -> &TextEvent {
        unsafe { self.inner.as_ref().unwrap() }
    }

    fn txn(&self) -> &TransactionMut {
        unsafe { self.txn.as_ref().unwrap() }
    }

    #[wasm_bindgen(getter, js_name = origin)]
    pub fn origin(&self) -> JsValue {
        self.origin.clone()
    }

    /// Returns a current shared type instance, that current event changes refer to.
    #[wasm_bindgen(getter)]
    pub fn target(&mut self) -> JsValue {
        if let Some(target) = self.target.as_ref() {
            target.clone()
        } else {
            let target: JsValue = YText::from(self.inner().target().clone()).into();
            self.target = Some(target.clone());
            target
        }
    }

    /// Returns an array of keys and indexes creating a path from root type down to current instance
    /// of shared type (accessible via `target` getter).
    #[wasm_bindgen]
    pub fn path(&self) -> JsValue {
        path_into_js(self.inner().path())
    }

    /// Returns a list of text changes made over corresponding `YText` collection within
    /// bounds of current transaction. These changes follow a format:
    ///
    /// - { insert: string, attributes: any|undefined }
    /// - { delete: number }
    /// - { retain: number, attributes: any|undefined }
    #[wasm_bindgen(getter)]
    pub fn delta(&mut self) -> JsValue {
        if let Some(delta) = &self.delta {
            delta.clone()
        } else {
            let delta = self
                .inner()
                .delta(self.txn())
                .into_iter()
                .map(ytext_delta_into_js);
            let mut result = js_sys::Array::new();
            result.extend(delta);
            let delta: JsValue = result.into();
            self.delta = Some(delta.clone());
            delta
        }
    }
}

/// Event generated by `YXmlElement.observe` method. Emitted during transaction commit phase.
#[wasm_bindgen]
pub struct YXmlEvent {
    inner: *const XmlEvent,
    txn: *const TransactionMut<'static>,
    target: Option<JsValue>,
    keys: Option<JsValue>,
    delta: Option<JsValue>,
    origin: JsValue,
}

#[wasm_bindgen]
impl YXmlEvent {
    fn new<'doc>(event: &XmlEvent, txn: &TransactionMut<'doc>) -> Self {
        let origin = from_origin(txn.origin());
        let inner = event as *const XmlEvent;
        let txn: &TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        let txn = txn as *const TransactionMut<'static>;
        YXmlEvent {
            inner,
            txn,
            origin,
            target: None,
            delta: None,
            keys: None,
        }
    }

    fn inner(&self) -> &XmlEvent {
        unsafe { self.inner.as_ref().unwrap() }
    }

    fn txn(&self) -> &TransactionMut {
        unsafe { self.txn.as_ref().unwrap() }
    }

    #[wasm_bindgen(getter, js_name = origin)]
    pub fn origin(&self) -> JsValue {
        self.origin.clone()
    }

    /// Returns a current shared type instance, that current event changes refer to.
    #[wasm_bindgen(getter)]
    pub fn target(&mut self) -> JsValue {
        if let Some(target) = self.target.as_ref() {
            target.clone()
        } else {
            let node = self.inner().target().clone();
            let target: JsValue = xml_into_js(node);
            self.target = Some(target.clone());
            target
        }
    }

    /// Returns an array of keys and indexes creating a path from root type down to current instance
    /// of shared type (accessible via `target` getter).
    #[wasm_bindgen]
    pub fn path(&self) -> JsValue {
        path_into_js(self.inner().path())
    }

    /// Returns a list of attribute changes made over corresponding `YXmlText` collection within
    /// bounds of current transaction. These changes follow a format:
    ///
    /// - { action: 'add'|'update'|'delete', oldValue: string|undefined, newValue: string|undefined }
    #[wasm_bindgen(getter)]
    pub fn keys(&mut self) -> JsValue {
        if let Some(keys) = &self.keys {
            keys.clone()
        } else {
            let keys = self.inner().keys(self.txn());
            let result = js_sys::Object::new();
            for (key, value) in keys.iter() {
                let key = JsValue::from(key.as_ref());
                let value = entry_change_into_js(value);
                js_sys::Reflect::set(&result, &key, &value).unwrap();
            }
            let keys: JsValue = result.into();
            self.keys = Some(keys.clone());
            keys
        }
    }

    /// Returns a list of XML child node changes made over corresponding `YXmlElement` collection
    /// within bounds of current transaction. These changes follow a format:
    ///
    /// - { insert: (YXmlText|YXmlElement)[] }
    /// - { delete: number }
    /// - { retain: number }
    #[wasm_bindgen(getter)]
    pub fn delta(&mut self) -> JsValue {
        if let Some(delta) = &self.delta {
            delta.clone()
        } else {
            let delta = self
                .inner()
                .delta(self.txn())
                .into_iter()
                .map(change_into_js);
            let mut result = js_sys::Array::new();
            result.extend(delta);
            let delta: JsValue = result.into();
            self.delta = Some(delta.clone());
            delta
        }
    }
}

/// Event generated by `YXmlText.observe` method. Emitted during transaction commit phase.
#[wasm_bindgen]
pub struct YXmlTextEvent {
    inner: *const XmlTextEvent,
    txn: *const TransactionMut<'static>,
    target: Option<JsValue>,
    delta: Option<JsValue>,
    keys: Option<JsValue>,
    origin: JsValue,
}

#[wasm_bindgen]
impl YXmlTextEvent {
    fn new<'doc>(event: &XmlTextEvent, txn: &TransactionMut<'doc>) -> Self {
        let origin = from_origin(txn.origin());
        let inner = event as *const XmlTextEvent;
        let txn: &TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        let txn = txn as *const TransactionMut<'static>;
        YXmlTextEvent {
            inner,
            txn,
            origin,
            target: None,
            delta: None,
            keys: None,
        }
    }

    fn inner(&self) -> &XmlTextEvent {
        unsafe { self.inner.as_ref().unwrap() }
    }

    fn txn(&self) -> &TransactionMut {
        unsafe { self.txn.as_ref().unwrap() }
    }

    #[wasm_bindgen(getter, js_name = origin)]
    pub fn origin(&self) -> JsValue {
        self.origin.clone()
    }

    /// Returns a current shared type instance, that current event changes refer to.
    #[wasm_bindgen(getter)]
    pub fn target(&mut self) -> JsValue {
        if let Some(target) = self.target.as_ref() {
            target.clone()
        } else {
            let target: JsValue = YXmlText(self.inner().target().clone()).into();
            self.target = Some(target.clone());
            target
        }
    }

    /// Returns an array of keys and indexes creating a path from root type down to current instance
    /// of shared type (accessible via `target` getter).
    #[wasm_bindgen]
    pub fn path(&self) -> JsValue {
        path_into_js(self.inner().path())
    }

    /// Returns a list of text changes made over corresponding `YXmlText` collection within
    /// bounds of current transaction. These changes follow a format:
    ///
    /// - { insert: string, attributes: any|undefined }
    /// - { delete: number }
    /// - { retain: number, attributes: any|undefined }
    #[wasm_bindgen(getter)]
    pub fn delta(&mut self) -> JsValue {
        if let Some(delta) = &self.delta {
            delta.clone()
        } else {
            let delta = self
                .inner()
                .delta(self.txn())
                .into_iter()
                .map(ytext_delta_into_js);
            let mut result = js_sys::Array::new();
            result.extend(delta);
            let delta: JsValue = result.into();
            self.delta = Some(delta.clone());
            delta
        }
    }

    /// Returns a list of attribute changes made over corresponding `YXmlText` collection within
    /// bounds of current transaction. These changes follow a format:
    ///
    /// - { action: 'add'|'update'|'delete', oldValue: string|undefined, newValue: string|undefined }
    #[wasm_bindgen(getter)]
    pub fn keys(&mut self) -> JsValue {
        if let Some(keys) = &self.keys {
            keys.clone()
        } else {
            let keys = self.inner().keys(self.txn());
            let result = js_sys::Object::new();
            for (key, value) in keys.iter() {
                let key = JsValue::from(key.as_ref());
                let value = entry_change_into_js(value);
                js_sys::Reflect::set(&result, &key, &value).unwrap();
            }
            let keys: JsValue = result.into();
            self.keys = Some(keys.clone());
            keys
        }
    }
}

fn path_into_js(path: Path) -> JsValue {
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

fn entry_change_into_js(change: &EntryChange) -> JsValue {
    let result = js_sys::Object::new();
    let action = JsValue::from("action");
    match change {
        EntryChange::Inserted(new) => {
            let new_value = value_into_js(new.clone());
            js_sys::Reflect::set(&result, &action, &JsValue::from("add")).unwrap();
            js_sys::Reflect::set(&result, &JsValue::from("newValue"), &new_value).unwrap();
        }
        EntryChange::Updated(old, new) => {
            let old_value = value_into_js(old.clone());
            let new_value = value_into_js(new.clone());
            js_sys::Reflect::set(&result, &action, &JsValue::from("update")).unwrap();
            js_sys::Reflect::set(&result, &JsValue::from("oldValue"), &old_value).unwrap();
            js_sys::Reflect::set(&result, &JsValue::from("newValue"), &new_value).unwrap();
        }
        EntryChange::Removed(old) => {
            let old_value = value_into_js(old.clone());
            js_sys::Reflect::set(&result, &action, &JsValue::from("delete")).unwrap();
            js_sys::Reflect::set(&result, &JsValue::from("oldValue"), &old_value).unwrap();
        }
    }
    result.into()
}

fn ytext_change_into_js(change: Diff<JsValue>) -> JsValue {
    let delta = Delta::Inserted(change.insert, change.attributes);
    let js = ytext_delta_into_js(&delta);
    if let Some(ychange) = change.ychange {
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
    js
}

fn ytext_delta_into_js(delta: &Delta) -> JsValue {
    let result = js_sys::Object::new();
    match delta {
        Delta::Inserted(value, attrs) => {
            js_sys::Reflect::set(
                &result,
                &JsValue::from("insert"),
                &value_into_js(value.clone()),
            )
            .unwrap();

            if let Some(attrs) = attrs {
                let attrs = attrs_into_js(attrs);
                js_sys::Reflect::set(&result, &JsValue::from("attributes"), &attrs).unwrap();
            }
        }
        Delta::Retain(len, attrs) => {
            let value = JsValue::from(*len);
            js_sys::Reflect::set(&result, &JsValue::from("retain"), &value).unwrap();

            if let Some(attrs) = attrs {
                let attrs = attrs_into_js(attrs);
                js_sys::Reflect::set(&result, &JsValue::from("attributes"), &attrs).unwrap();
            }
        }
        Delta::Deleted(len) => {
            let value = JsValue::from(*len);
            js_sys::Reflect::set(&result, &JsValue::from("delete"), &value).unwrap();
        }
    }
    result.into()
}

fn attrs_into_js(attrs: &Attrs) -> JsValue {
    let o = js_sys::Object::new();
    for (key, value) in attrs.iter() {
        let key = JsValue::from_str(key.as_ref());
        let value = value_into_js(Value::Any(value.clone()));
        js_sys::Reflect::set(&o, &key, &value).unwrap();
    }

    o.into()
}

fn change_into_js(change: &Change) -> JsValue {
    let result = js_sys::Object::new();
    match change {
        Change::Added(values) => {
            let mut array = js_sys::Array::new();
            array.extend(values.iter().map(|v| value_into_js(v.clone())));
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

fn state_vector_into_map(sv: &StateVector) -> js_sys::Map {
    let m = js_sys::Map::new();
    for (&k, &v) in sv.iter() {
        let key: JsValue = k.into();
        let value: JsValue = v.into();
        m.set(&key, &value);
    }
    m
}

fn delete_set_into_map(ds: &DeleteSet) -> js_sys::Map {
    let m = js_sys::Map::new();
    for (&k, v) in ds.iter() {
        let key: JsValue = k.into();
        let iter = v.iter().map(|r| {
            let start = r.start;
            let len = r.end - r.start;
            let res: JsValue = js_sys::Array::of2(&start.into(), &len.into()).into();
            res
        });
        let value = js_sys::Array::new();
        for v in iter {
            value.push(&v);
        }
        m.set(&key, &value);
    }
    m
}

#[wasm_bindgen]
pub struct YSubdocsEvent {
    added: JsValue,
    removed: JsValue,
    loaded: JsValue,
}

#[wasm_bindgen]
impl YSubdocsEvent {
    fn new(e: &SubdocsEvent) -> Self {
        fn to_array(iter: SubdocsEventIter) -> JsValue {
            let mut buf = js_sys::Array::new();
            let values = iter.map(|d| {
                let doc = YDoc::from(d.clone());
                let js: JsValue = doc.into();
                js
            });
            buf.extend(values);
            buf.into()
        }

        let added = to_array(e.added());
        let removed = to_array(e.removed());
        let loaded = to_array(e.loaded());
        YSubdocsEvent {
            added,
            removed,
            loaded,
        }
    }

    #[wasm_bindgen(getter)]
    pub fn added(&mut self) -> JsValue {
        self.added.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn removed(&mut self) -> JsValue {
        self.removed.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn loaded(&mut self) -> JsValue {
        self.loaded.clone()
    }
}

#[wasm_bindgen]
pub struct YSubdocsObserver(SubdocsSubscription);

impl From<SubdocsSubscription> for YSubdocsObserver {
    fn from(o: SubdocsSubscription) -> Self {
        YSubdocsObserver(o)
    }
}

#[wasm_bindgen]
pub struct YDestroyObserver(DestroySubscription);

impl From<DestroySubscription> for YDestroyObserver {
    fn from(o: DestroySubscription) -> Self {
        YDestroyObserver(o)
    }
}

#[wasm_bindgen]
pub struct YAfterTransactionEvent {
    before_state: js_sys::Map,
    after_state: js_sys::Map,
    delete_set: js_sys::Map,
}

#[wasm_bindgen]
impl YAfterTransactionEvent {
    /// Returns a state vector - a map of entries (clientId, clock) - that represents logical
    /// time descriptor at the moment when transaction was originally created, prior to any changes
    /// made in scope of this transaction.
    #[wasm_bindgen(getter, js_name = beforeState)]
    pub fn before_state(&self) -> js_sys::Map {
        self.before_state.clone()
    }

    /// Returns a state vector - a map of entries (clientId, clock) - that represents logical
    /// time descriptor at the moment when transaction was comitted.
    #[wasm_bindgen(getter, js_name = afterState)]
    pub fn after_state(&self) -> js_sys::Map {
        self.after_state.clone()
    }

    /// Returns a delete set - a map of entries (clientId, (clock, len)[]) - that represents a range
    /// of all blocks deleted as part of current transaction.
    #[wasm_bindgen(getter, js_name = deleteSet)]
    pub fn delete_set(&self) -> js_sys::Map {
        self.delete_set.clone()
    }

    fn new(e: &TransactionCleanupEvent) -> Self {
        YAfterTransactionEvent {
            before_state: state_vector_into_map(&e.before_state),
            after_state: state_vector_into_map(&e.after_state),
            delete_set: delete_set_into_map(&e.delete_set),
        }
    }
}

#[wasm_bindgen]
pub struct YAfterTransactionObserver(TransactionCleanupSubscription);

impl From<TransactionCleanupSubscription> for YAfterTransactionObserver {
    fn from(o: TransactionCleanupSubscription) -> Self {
        YAfterTransactionObserver(o)
    }
}

#[wasm_bindgen]
pub struct YUpdateObserver(UpdateSubscription);

impl From<UpdateSubscription> for YUpdateObserver {
    fn from(o: UpdateSubscription) -> Self {
        YUpdateObserver(o)
    }
}

#[wasm_bindgen]
pub struct YArrayObserver(Subscription<Arc<dyn Fn(&TransactionMut, &ArrayEvent) -> ()>>);

#[wasm_bindgen]
pub struct YTextObserver(Subscription<Arc<dyn Fn(&TransactionMut, &TextEvent) -> ()>>);

#[wasm_bindgen]
pub struct YMapObserver(Subscription<Arc<dyn Fn(&TransactionMut, &MapEvent) -> ()>>);

#[wasm_bindgen]
pub struct YXmlObserver(Subscription<Arc<dyn Fn(&TransactionMut, &XmlEvent) -> ()>>);

#[wasm_bindgen]
pub struct YXmlTextObserver(Subscription<Arc<dyn Fn(&TransactionMut, &XmlTextEvent) -> ()>>);

#[wasm_bindgen]
pub struct YEventObserver(DeepEventsSubscription);

impl From<DeepEventsSubscription> for YEventObserver {
    fn from(o: DeepEventsSubscription) -> Self {
        YEventObserver(o)
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

    fn as_integrated(&self) -> Option<&T> {
        if let SharedType::Integrated(value) = self {
            Some(value)
        } else {
            None
        }
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
pub struct YText(RefCell<SharedType<TextRef, String>>);

impl From<TextRef> for YText {
    fn from(v: TextRef) -> Self {
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
    #[wasm_bindgen(getter)]
    pub fn prelim(&self) -> bool {
        if let SharedType::Prelim(_) = &*self.0.borrow() {
            true
        } else {
            false
        }
    }

    /// Returns length of an underlying string stored in this `YText` instance,
    /// understood as a number of UTF-8 encoded bytes.
    #[wasm_bindgen(js_name = length)]
    pub fn length(&self, txn: &ImplicitTransaction) -> u32 {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => {
                if let Some(txn) = get_txn(txn) {
                    v.len(&*txn)
                } else {
                    v.len(&v.transact())
                }
            }
            SharedType::Prelim(v) => v.len() as u32,
        }
    }

    /// Returns an underlying shared string stored in this data type.
    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &ImplicitTransaction) -> String {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => {
                if let Some(txn) = get_txn(txn) {
                    v.get_string(&*txn)
                } else {
                    v.get_string(&v.transact())
                }
            }
            SharedType::Prelim(v) => v.clone(),
        }
    }

    /// Returns an underlying shared string stored in this data type.
    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &ImplicitTransaction) -> JsValue {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => {
                if let Some(txn) = get_txn(txn) {
                    JsValue::from(&v.get_string(&*txn))
                } else {
                    JsValue::from(&v.get_string(&v.transact()))
                }
            }
            SharedType::Prelim(v) => JsValue::from(v),
        }
    }

    /// Inserts a given `chunk` of text into this `YText` instance, starting at a given `index`.
    ///
    /// Optional object with defined `attributes` will be used to wrap provided text `chunk`
    /// with a formatting blocks.`attributes` are only supported for a `YText` instance which
    /// already has been integrated into document store.
    #[wasm_bindgen(js_name = insert)]
    pub fn insert(&self, index: u32, chunk: &str, attributes: JsValue, txn: &ImplicitTransaction) {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                if let Some(mut txn) = get_txn_mut(txn) {
                    if let Some(attrs) = Self::parse_attrs(attributes) {
                        v.insert_with_attributes(txn.as_mut(), index, chunk, attrs)
                    } else {
                        v.insert(txn.as_mut(), index, chunk)
                    }
                } else {
                    let mut txn = v.transact_mut();
                    if let Some(attrs) = Self::parse_attrs(attributes) {
                        v.insert_with_attributes(&mut txn, index, chunk, attrs)
                    } else {
                        v.insert(&mut txn, index, chunk)
                    }
                }
            }
            SharedType::Prelim(v) => {
                if attributes.is_object() {
                    panic!("insert with attributes requires YText instance to be integrated first.")
                } else {
                    v.insert_str(index as usize, chunk)
                }
            }
        }
    }

    /// Inserts a given `embed` object into this `YText` instance, starting at a given `index`.
    ///
    /// Optional object with defined `attributes` will be used to wrap provided `embed`
    /// with a formatting blocks.`attributes` are only supported for a `YText` instance which
    /// already has been integrated into document store.
    #[wasm_bindgen(js_name = insertEmbed)]
    pub fn insert_embed(
        &self,
        index: u32,
        embed: JsValue,
        attributes: JsValue,
        txn: &ImplicitTransaction,
    ) {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                let content = js_into_any(&embed).unwrap();
                if let Some(mut txn) = get_txn_mut(txn) {
                    if let Some(attrs) = Self::parse_attrs(attributes) {
                        v.insert_embed_with_attributes(txn.as_mut(), index, content, attrs);
                    } else {
                        v.insert_embed(txn.as_mut(), index, content);
                    }
                } else {
                    let mut txn = v.transact_mut();

                    if let Some(attrs) = Self::parse_attrs(attributes) {
                        v.insert_embed_with_attributes(&mut txn, index, content, attrs);
                    } else {
                        v.insert_embed(&mut txn, index, content);
                    }
                }
            }
            SharedType::Prelim(_) => {
                panic!("insert embeds requires YText instance to be integrated first.")
            }
        }
    }

    /// Wraps an existing piece of text within a range described by `index`-`length` parameters with
    /// formatting blocks containing provided `attributes` metadata. This method only works for
    /// `YText` instances that already have been integrated into document store.
    #[wasm_bindgen(js_name = format)]
    pub fn format(&self, index: u32, length: u32, attributes: JsValue, txn: &ImplicitTransaction) {
        if let Some(attrs) = Self::parse_attrs(attributes) {
            match &mut *self.0.borrow_mut() {
                SharedType::Integrated(v) => {
                    if let Some(mut txn) = get_txn_mut(txn) {
                        v.format(txn.as_mut(), index, length, attrs);
                    } else {
                        let mut txn = v.transact_mut();
                        v.format(&mut txn, index, length, attrs);
                    }
                }
                SharedType::Prelim(_) => {
                    panic!("format with attributes requires YText instance to be integrated first.")
                }
            }
        }
    }

    fn parse_attrs(attrs: JsValue) -> Option<Attrs> {
        if attrs.is_object() {
            let mut map = Attrs::new();
            let object = js_sys::Object::from(attrs);
            let entries = js_sys::Object::entries(&object);
            for tuple in entries.iter() {
                let tuple = js_sys::Array::from(&tuple);
                let key: String = tuple.get(0).as_string()?;
                let value = js_into_any(&tuple.get(1))?;
                map.insert(key.into(), value);
            }
            Some(map)
        } else {
            None
        }
    }

    /// Appends a given `chunk` of text at the end of current `YText` instance.
    ///
    /// Optional object with defined `attributes` will be used to wrap provided text `chunk`
    /// with a formatting blocks.`attributes` are only supported for a `YText` instance which
    /// already has been integrated into document store.
    #[wasm_bindgen(js_name = push)]
    pub fn push(&self, chunk: &str, attributes: JsValue, txn: &ImplicitTransaction) {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                if let Some(mut txn) = get_txn_mut(txn) {
                    if let Some(attrs) = Self::parse_attrs(attributes) {
                        let len = v.len(&*txn);
                        v.insert_with_attributes(txn.as_mut(), len, chunk, attrs)
                    } else {
                        v.push(txn.as_mut(), chunk)
                    }
                } else {
                    let mut txn = v.transact_mut();
                    if let Some(attrs) = Self::parse_attrs(attributes) {
                        let index = v.len(&txn);
                        v.insert_with_attributes(&mut txn, index, chunk, attrs)
                    } else {
                        v.push(&mut txn, chunk)
                    }
                }
            }
            SharedType::Prelim(v) => {
                if attributes.is_object() {
                    panic!("push with attributes requires YText instance to be integrated first.")
                }
                v.push_str(chunk)
            }
        }
    }

    /// Deletes a specified range of of characters, starting at a given `index`.
    /// Both `index` and `length` are counted in terms of a number of UTF-8 character bytes.
    #[wasm_bindgen(method, js_name = delete)]
    pub fn delete(&mut self, index: u32, length: u32, txn: &ImplicitTransaction) {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                if let Some(mut txn) = get_txn_mut(txn) {
                    v.remove_range(txn.as_mut(), index, length);
                } else {
                    let mut txn = v.transact_mut();
                    v.remove_range(&mut txn, index, length);
                }
            }
            SharedType::Prelim(v) => {
                v.drain((index as usize)..(index + length) as usize);
            }
        }
    }

    /// Returns the Delta representation of this YText type.
    #[wasm_bindgen(js_name = toDelta)]
    pub fn to_delta(
        &self,
        snapshot: Option<YSnapshot>,
        prev_snapshot: Option<YSnapshot>,
        compute_ychange: Option<js_sys::Function>,
        txn: &ImplicitTransaction,
    ) -> JsValue {
        match &*self.0.borrow() {
            SharedType::Prelim(_) => JsValue::UNDEFINED,
            SharedType::Integrated(v) => {
                let hi = snapshot.map(|s| s.0);
                let lo = prev_snapshot.map(|s| s.0);

                fn changes(change: YChange, compute_ychange: &Option<js_sys::Function>) -> JsValue {
                    let kind = match change.kind {
                        ChangeKind::Added => JsValue::from("added"),
                        ChangeKind::Removed => JsValue::from("removed"),
                    };
                    let result = if let Some(func) = compute_ychange {
                        let id = change.id.into_js();
                        func.call2(&JsValue::UNDEFINED, &kind, &id).unwrap()
                    } else {
                        let js: JsValue = js_sys::Object::new().into();
                        js_sys::Reflect::set(&js, &JsValue::from("type"), &kind).unwrap();
                        js
                    };
                    result
                }

                let delta = if let Some(mut txn) = get_txn_mut(txn) {
                    v.diff_range(txn.as_mut(), hi.as_ref(), lo.as_ref(), |change| {
                        changes(change, &compute_ychange)
                    })
                    .into_iter()
                    .map(ytext_change_into_js)
                } else {
                    let mut txn = v.transact_mut();
                    v.diff_range(&mut txn, hi.as_ref(), lo.as_ref(), |change| {
                        changes(change, &compute_ychange)
                    })
                    .into_iter()
                    .map(ytext_change_into_js)
                };
                let mut result = js_sys::Array::new();
                result.extend(delta);
                let delta: JsValue = result.into();
                delta
            }
        }
    }

    /// Subscribes to all operations happening over this instance of `YText`. All changes are
    /// batched and eventually triggered during transaction commit phase.
    /// Returns an `YTextObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observe)]
    pub fn observe(&mut self, f: js_sys::Function) -> YTextObserver {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                let sub = v.observe(move |txn, e| {
                    let e = YTextEvent::new(e, txn);
                    let arg: JsValue = e.into();
                    f.call1(&JsValue::UNDEFINED, &arg).unwrap();
                });
                YTextObserver(sub)
            }
            SharedType::Prelim(_) => {
                panic!("YText.observe is not supported on preliminary type.")
            }
        }
    }

    /// Subscribes to all operations happening over this Y shared type, as well as events in
    /// shared types stored within this one. All changes are batched and eventually triggered
    /// during transaction commit phase.
    /// Returns an `YEventObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observeDeep)]
    pub fn observe_deep(&mut self, f: js_sys::Function) -> YEventObserver {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                let sub = v.observe_deep(move |txn, e| {
                    let arg = events_into_js(txn, e);
                    f.call1(&JsValue::UNDEFINED, &arg).unwrap();
                });
                YEventObserver(sub)
            }
            SharedType::Prelim(_) => {
                panic!("YText.observeDeep is not supported on preliminary type.")
            }
        }
    }
}

#[wasm_bindgen]
pub struct YSnapshot(Snapshot);

#[wasm_bindgen]
impl YSnapshot {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        YSnapshot(Snapshot::default())
    }
}

#[wasm_bindgen(js_name = snapshot)]
pub fn snapshot(doc: &YDoc) -> YSnapshot {
    YSnapshot(doc.as_ref().transact().snapshot())
}

#[wasm_bindgen(js_name = equalSnapshots)]
pub fn equal_snapshots(snap1: &YSnapshot, snap2: &YSnapshot) -> bool {
    snap1.0 == snap2.0
}

#[wasm_bindgen(js_name = encodeSnapshotV1)]
pub fn encode_snapshot_v1(snapshot: &YSnapshot) -> Vec<u8> {
    snapshot.0.encode_v1()
}

#[wasm_bindgen(js_name = encodeSnapshotV2)]
pub fn encode_snapshot_v2(snapshot: &YSnapshot) -> Vec<u8> {
    snapshot.0.encode_v2()
}

#[wasm_bindgen(js_name = decodeSnapshotV2)]
pub fn decode_snapshot_v2(snapshot: &[u8]) -> Result<YSnapshot, JsValue> {
    let s = Snapshot::decode_v2(snapshot)
        .map_err(|_| JsValue::from("failed to deserialize snapshot using lib0 v2 decoding"))?;
    Ok(YSnapshot(s))
}

#[wasm_bindgen(js_name = decodeSnapshotV1)]
pub fn decode_snapshot_v1(snapshot: &[u8]) -> Result<YSnapshot, JsValue> {
    let s = Snapshot::decode_v1(snapshot)
        .map_err(|_| JsValue::from("failed to deserialize snapshot using lib0 v1 decoding"))?;
    Ok(YSnapshot(s))
}

#[wasm_bindgen(js_name = encodeStateFromSnapshotV1)]
pub fn encode_state_from_snapshot_v1(doc: &YDoc, snapshot: &YSnapshot) -> Result<Vec<u8>, JsValue> {
    let mut encoder = EncoderV1::new();
    match doc
        .as_ref()
        .transact()
        .encode_state_from_snapshot(&snapshot.0, &mut encoder)
    {
        Ok(_) => Ok(encoder.to_vec()),
        Err(e) => Err(JsValue::from(e.to_string())),
    }
}

#[wasm_bindgen(js_name = encodeStateFromSnapshotV2)]
pub fn encode_state_from_snapshot_v2(doc: &YDoc, snapshot: &YSnapshot) -> Result<Vec<u8>, JsValue> {
    let mut encoder = EncoderV2::new();
    match doc
        .as_ref()
        .transact()
        .encode_state_from_snapshot(&snapshot.0, &mut encoder)
    {
        Ok(_) => Ok(encoder.to_vec()),
        Err(e) => Err(JsValue::from(e.to_string())),
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
pub struct YArray(RefCell<SharedType<ArrayRef, Vec<JsValue>>>);

impl From<ArrayRef> for YArray {
    fn from(v: ArrayRef) -> Self {
        YArray(SharedType::new(v))
    }
}

impl PartialEq for YArray {
    fn eq(&self, other: &Self) -> bool {
        match (&*self.0.borrow(), &*other.0.borrow()) {
            (SharedType::Integrated(v1), SharedType::Integrated(v2)) => v1 == v2,
            (SharedType::Prelim(v1), SharedType::Prelim(v2)) => v1 == v2,
            _ => false,
        }
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
    #[wasm_bindgen(getter)]
    pub fn prelim(&self) -> bool {
        if let SharedType::Prelim(_) = &*self.0.borrow() {
            true
        } else {
            false
        }
    }

    /// Returns a number of elements stored within this instance of `YArray`.
    #[wasm_bindgen(js_name = length)]
    pub fn length(&self, txn: &ImplicitTransaction) -> u32 {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => {
                if let Some(txn) = get_txn(txn) {
                    v.len(&*txn)
                } else {
                    v.len(&v.transact())
                }
            }
            SharedType::Prelim(v) => v.len() as u32,
        }
    }

    /// Converts an underlying contents of this `YArray` instance into their JSON representation.
    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &ImplicitTransaction) -> JsValue {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => {
                if let Some(txn) = get_txn(txn) {
                    any_into_js(&v.to_json(&*txn))
                } else {
                    let txn = v.transact();
                    any_into_js(&v.to_json(&txn))
                }
            }
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
    pub fn insert(&self, index: u32, items: Vec<JsValue>, txn: &ImplicitTransaction) {
        let mut j = index;
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                if let Some(mut txn) = get_txn_mut(txn) {
                    insert_at(v, txn.as_mut(), index, items);
                } else {
                    let mut txn = v.transact_mut();
                    insert_at(v, &mut txn, index, items);
                }
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
    pub fn push(&self, items: Vec<JsValue>, txn: &ImplicitTransaction) {
        let index = self.length(txn);
        self.insert(index, items, txn);
    }

    /// Deletes a range of items of given `length` from current `YArray` instance,
    /// starting from given `index`.
    #[wasm_bindgen(method, js_name = delete)]
    pub fn delete(&self, index: u32, length: u32, txn: &ImplicitTransaction) {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                if let Some(mut txn) = get_txn_mut(txn) {
                    v.remove_range(txn.as_mut(), index, length)
                } else {
                    let mut txn = v.transact_mut();
                    v.remove_range(&mut txn, index, length)
                }
            }
            SharedType::Prelim(v) => {
                v.drain((index as usize)..(index + length) as usize);
            }
        }
    }

    /// Moves element found at `source` index into `target` index position.
    #[wasm_bindgen(js_name = move)]
    pub fn move_content(&self, source: u32, target: u32, txn: &ImplicitTransaction) {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                if let Some(mut txn) = get_txn_mut(txn) {
                    v.move_to(txn.as_mut(), source, target)
                } else {
                    let mut txn = v.transact_mut();
                    v.move_to(&mut txn, source, target)
                }
            }
            SharedType::Prelim(v) => {
                let index = if target > source { target - 1 } else { target };
                let moved = v.remove(source as usize);
                v.insert(index as usize, moved);
            }
        }
    }

    /// Returns an element stored under given `index`.
    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, index: u32, txn: &ImplicitTransaction) -> Result<JsValue, JsValue> {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => {
                if let Some(txn) = get_txn(txn) {
                    if let Some(value) = v.get(&*txn, index) {
                        Ok(value_into_js(value))
                    } else {
                        Err(JsValue::from("Index outside the bounds of an YArray"))
                    }
                } else {
                    let txn = v.transact();
                    if let Some(value) = v.get(&txn, index) {
                        Ok(value_into_js(value))
                    } else {
                        Err(JsValue::from("Index outside the bounds of an YArray"))
                    }
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
    pub fn values(&self, txn: &ImplicitTransaction) -> JsValue {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => {
                if let Some(txn) = get_txn(txn) {
                    let values = v.iter(&*txn);
                    iter_to_array(values).into()
                } else {
                    let txn = v.transact();
                    let values = v.iter(&txn);
                    iter_to_array(values).into()
                }
            }
            SharedType::Prelim(v) => {
                let values = v.iter();
                iter_to_array(values).into()
            }
        }
    }

    /// Subscribes to all operations happening over this instance of `YArray`. All changes are
    /// batched and eventually triggered during transaction commit phase.
    /// Returns an `YObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observe)]
    pub fn observe(&mut self, f: js_sys::Function) -> YArrayObserver {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                let sub = v.observe(move |txn, e| {
                    let e = YArrayEvent::new(e, txn);
                    let arg: JsValue = e.into();
                    f.call1(&JsValue::UNDEFINED, &arg).unwrap();
                });
                YArrayObserver(sub)
            }
            SharedType::Prelim(_) => {
                panic!("YArray.observe is not supported on preliminary type.")
            }
        }
    }

    /// Subscribes to all operations happening over this Y shared type, as well as events in
    /// shared types stored within this one. All changes are batched and eventually triggered
    /// during transaction commit phase.
    /// Returns an `YEventObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observeDeep)]
    pub fn observe_deep(&mut self, f: js_sys::Function) -> YEventObserver {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => v
                .observe_deep(move |txn, e| {
                    let arg = events_into_js(txn, e);
                    f.call1(&JsValue::UNDEFINED, &arg).unwrap();
                })
                .into(),
            SharedType::Prelim(_) => {
                panic!("YText.observeDeep is not supported on preliminary type.")
            }
        }
    }
}

fn iter_to_array<I, T>(iter: I) -> js_sys::Array
where
    I: Iterator<Item = T>,
    T: IntoJs,
{
    let array = js_sys::Array::new();
    for value in iter {
        let js_value = value.into_js();
        array.push(&js_value);
    }
    array
}

fn iter_to_map<'a, I, T>(iter: I) -> js_sys::Object
where
    I: Iterator<Item = (&'a str, T)>,
    T: IntoJs,
{
    let obj = js_sys::Object::new();
    for (key, value) in iter {
        let key = JsValue::from_str(key);
        let value = value.into_js();
        js_sys::Reflect::set(&obj, &key, &value).unwrap();
    }
    obj
}

trait IntoJs {
    fn into_js(self) -> JsValue;
}

impl<'a> IntoJs for &'a JsValue {
    fn into_js(self) -> JsValue {
        self.clone()
    }
}

impl IntoJs for JsValue {
    fn into_js(self) -> JsValue {
        self
    }
}

impl IntoJs for Value {
    fn into_js(self) -> JsValue {
        value_into_js(self)
    }
}

impl IntoJs for String {
    fn into_js(self) -> JsValue {
        JsValue::from_str(&self)
    }
}

impl IntoJs for XmlNode {
    fn into_js(self) -> JsValue {
        xml_into_js(self)
    }
}
impl<'a> IntoJs for (&'a str, Value) {
    fn into_js(self) -> JsValue {
        let tuple = js_sys::Array::new_with_length(2);
        tuple.set(0, JsValue::from(self.0));
        tuple.set(1, value_into_js(self.1));
        tuple.into()
    }
}

impl<'a> IntoJs for (&'a str, String) {
    fn into_js(self) -> JsValue {
        let tuple = js_sys::Array::new_with_length(2);
        tuple.set(0, JsValue::from_str(self.0));
        tuple.set(1, JsValue::from(&self.1));
        tuple.into()
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
pub struct YMap(RefCell<SharedType<MapRef, HashMap<String, JsValue>>>);

impl From<MapRef> for YMap {
    fn from(v: MapRef) -> Self {
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
    #[wasm_bindgen(getter)]
    pub fn prelim(&self) -> bool {
        if let SharedType::Prelim(_) = &*self.0.borrow() {
            true
        } else {
            false
        }
    }

    /// Returns a number of entries stored within this instance of `YMap`.
    #[wasm_bindgen(js_name = length)]
    pub fn length(&self, txn: &ImplicitTransaction) -> u32 {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => {
                if let Some(txn) = get_txn(txn) {
                    v.len(&*txn)
                } else {
                    v.len(&v.transact())
                }
            }
            SharedType::Prelim(v) => v.len() as u32,
        }
    }

    /// Converts contents of this `YMap` instance into a JSON representation.
    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &ImplicitTransaction) -> JsValue {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => {
                if let Some(txn) = get_txn(txn) {
                    any_into_js(&v.to_json(&*txn))
                } else {
                    let txn = v.transact();
                    any_into_js(&v.to_json(&txn))
                }
            }
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
    pub fn set(&self, key: &str, value: JsValue, txn: &ImplicitTransaction) {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                if let Some(mut txn) = get_txn_mut(txn) {
                    v.insert(txn.as_mut(), key.to_string(), JsValueWrapper(value));
                } else {
                    let mut txn = v.transact_mut();
                    v.insert(&mut txn, key.to_string(), JsValueWrapper(value));
                }
            }
            SharedType::Prelim(v) => {
                v.insert(key.to_string(), value);
            }
        }
    }

    /// Removes an entry identified by a given `key` from this instance of `YMap`, if such exists.
    #[wasm_bindgen(method, js_name = delete)]
    pub fn delete(&mut self, key: &str, txn: &ImplicitTransaction) {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                if let Some(mut txn) = get_txn_mut(txn) {
                    v.remove(txn.as_mut(), key);
                } else {
                    let mut txn = v.transact_mut();
                    v.remove(&mut txn, key);
                }
            }
            SharedType::Prelim(v) => {
                v.remove(key);
            }
        }
    }

    /// Returns value of an entry stored under given `key` within this instance of `YMap`,
    /// or `undefined` if no such entry existed.
    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, key: &str, txn: &ImplicitTransaction) -> JsValue {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => {
                let value = if let Some(txn) = get_txn(txn) {
                    v.get(&*txn, key)
                } else {
                    v.get(&v.transact(), key)
                };

                if let Some(value) = value {
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
    pub fn entries(&self, txn: &ImplicitTransaction) -> JsValue {
        match &*self.0.borrow() {
            SharedType::Integrated(v) => {
                if let Some(txn) = get_txn(txn) {
                    let entries = v.iter(&*txn);
                    iter_to_map(entries).into()
                } else {
                    let txn = v.transact();
                    let entries = v.iter(&txn);
                    iter_to_map(entries).into()
                }
            }
            SharedType::Prelim(v) => {
                let obj = js_sys::Object::new();
                for (key, value) in v.iter() {
                    let key = JsValue::from_str(key.as_str());
                    let value = value.into_js();
                    js_sys::Reflect::set(&obj, &key, &value).unwrap();
                }
                obj.into()
            }
        }
    }

    /// Subscribes to all operations happening over this instance of `YMap`. All changes are
    /// batched and eventually triggered during transaction commit phase.
    /// Returns an `YObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observe)]
    pub fn observe(&mut self, f: js_sys::Function) -> YMapObserver {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => {
                let sub = v.observe(move |txn, e| {
                    let e = YMapEvent::new(e, txn);
                    let arg: JsValue = e.into();
                    f.call1(&JsValue::UNDEFINED, &arg).unwrap();
                });
                YMapObserver(sub)
            }
            SharedType::Prelim(_) => {
                panic!("YMap.observe is not supported on preliminary type.")
            }
        }
    }

    /// Subscribes to all operations happening over this Y shared type, as well as events in
    /// shared types stored within this one. All changes are batched and eventually triggered
    /// during transaction commit phase.
    /// Returns an `YEventObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observeDeep)]
    pub fn observe_deep(&mut self, f: js_sys::Function) -> YEventObserver {
        match &mut *self.0.borrow_mut() {
            SharedType::Integrated(v) => v
                .observe_deep(move |txn, e| {
                    let arg = events_into_js(txn, e);
                    f.call1(&JsValue::UNDEFINED, &arg).unwrap();
                })
                .into(),
            SharedType::Prelim(_) => {
                panic!("YText.observeDeep is not supported on preliminary type.")
            }
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
pub struct YXmlElement(XmlElementRef);

#[wasm_bindgen]
impl YXmlElement {
    /// Returns a tag name of this XML node.
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> JsValue {
        if let Some(name) = self.0.try_tag() {
            JsValue::from_str(name.deref())
        } else {
            JsValue::NULL
        }
    }

    /// Returns a number of child XML nodes stored within this `YXMlElement` instance.
    #[wasm_bindgen(js_name = length)]
    pub fn length(&self, txn: &ImplicitTransaction) -> u32 {
        if let Some(txn) = get_txn(txn) {
            self.0.len(&*txn)
        } else {
            let txn = self.0.transact();
            self.0.len(&txn)
        }
    }

    /// Inserts a new instance of `YXmlElement` as a child of this XML node and returns it.
    #[wasm_bindgen(js_name = insertXmlElement)]
    pub fn insert_xml_element(
        &self,
        index: u32,
        name: &str,
        txn: &ImplicitTransaction,
    ) -> YXmlElement {
        if let Some(mut txn) = get_txn_mut(txn) {
            YXmlElement(
                self.0
                    .insert(txn.as_mut(), index, XmlElementPrelim::empty(name)),
            )
        } else {
            let mut txn = self.0.transact_mut();
            YXmlElement(
                self.0
                    .insert(&mut txn, index, XmlElementPrelim::empty(name)),
            )
        }
    }

    /// Inserts a new instance of `YXmlText` as a child of this XML node and returns it.
    #[wasm_bindgen(js_name = insertXmlText)]
    pub fn insert_xml_text(&self, index: u32, txn: &ImplicitTransaction) -> YXmlText {
        if let Some(mut txn) = get_txn_mut(txn) {
            YXmlText(self.0.insert(txn.as_mut(), index, XmlTextPrelim::new("")))
        } else {
            let mut txn = self.0.transact_mut();
            YXmlText(self.0.insert(&mut txn, index, XmlTextPrelim::new("")))
        }
    }

    /// Removes a range of children XML nodes from this `YXmlElement` instance,
    /// starting at given `index`.
    #[wasm_bindgen(method, js_name = delete)]
    pub fn delete(&self, index: u32, length: u32, txn: &ImplicitTransaction) {
        if let Some(mut txn) = get_txn_mut(txn) {
            self.0.remove_range(txn.as_mut(), index, length)
        } else {
            let mut txn = self.0.transact_mut();
            self.0.remove_range(&mut txn, index, length)
        }
    }

    /// Appends a new instance of `YXmlElement` as the last child of this XML node and returns it.
    #[wasm_bindgen(js_name = pushXmlElement)]
    pub fn push_xml_element(&self, name: &str, txn: &ImplicitTransaction) -> YXmlElement {
        if let Some(mut txn) = get_txn_mut(txn) {
            YXmlElement(
                self.0
                    .push_back(txn.as_mut(), XmlElementPrelim::empty(name)),
            )
        } else {
            let mut txn = self.0.transact_mut();
            YXmlElement(self.0.push_back(&mut txn, XmlElementPrelim::empty(name)))
        }
    }

    /// Appends a new instance of `YXmlText` as the last child of this XML node and returns it.
    #[wasm_bindgen(js_name = pushXmlText)]
    pub fn push_xml_text(&self, txn: &ImplicitTransaction) -> YXmlText {
        if let Some(mut txn) = get_txn_mut(txn) {
            YXmlText(self.0.push_back(txn.as_mut(), XmlTextPrelim::new("")))
        } else {
            let mut txn = self.0.transact_mut();
            YXmlText(self.0.push_back(&mut txn, XmlTextPrelim::new("")))
        }
    }

    /// Returns a first child of this XML node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node has not children.
    #[wasm_bindgen(js_name = firstChild)]
    pub fn first_child(&self) -> JsValue {
        if let Some(xml) = self.0.first_child() {
            xml_into_js(xml)
        } else {
            JsValue::undefined()
        }
    }

    /// Returns a next XML sibling node of this XMl node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node is a last child of
    /// parent XML node.
    #[wasm_bindgen(js_name = nextSibling)]
    pub fn next_sibling(&self, txn: &ImplicitTransaction) -> JsValue {
        if let Some(txn) = get_txn(txn) {
            let mut siblings = self.0.siblings(&*txn);
            siblings
                .next()
                .map(xml_into_js)
                .unwrap_or(JsValue::UNDEFINED)
        } else {
            let txn = self.0.transact_mut();
            let mut siblings = self.0.siblings(&txn);
            siblings
                .next()
                .map(xml_into_js)
                .unwrap_or(JsValue::UNDEFINED)
        }
    }

    /// Returns a previous XML sibling node of this XMl node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node is a first child
    /// of parent XML node.
    #[wasm_bindgen(js_name = prevSibling)]
    pub fn prev_sibling(&self, txn: &ImplicitTransaction) -> JsValue {
        if let Some(txn) = get_txn(txn) {
            let mut siblings = self.0.siblings(&*txn);
            siblings
                .next_back()
                .map(xml_into_js)
                .unwrap_or(JsValue::UNDEFINED)
        } else {
            let txn = self.0.transact_mut();
            let mut siblings = self.0.siblings(&txn);
            siblings
                .next_back()
                .map(xml_into_js)
                .unwrap_or(JsValue::UNDEFINED)
        }
    }

    /// Returns a parent `YXmlElement` node or `undefined` if current node has no parent assigned.
    #[wasm_bindgen(getter, js_name = parent)]
    pub fn parent(&self) -> JsValue {
        if let Some(xml) = self.0.parent() {
            xml_into_js(xml)
        } else {
            JsValue::undefined()
        }
    }

    /// Returns a string representation of this XML node.
    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &ImplicitTransaction) -> String {
        if let Some(txn) = get_txn(txn) {
            self.0.get_string(&*txn)
        } else {
            self.0.get_string(&self.0.transact())
        }
    }

    /// Sets a `name` and `value` as new attribute for this XML node. If an attribute with the same
    /// `name` already existed on that node, its value with be overridden with a provided one.
    #[wasm_bindgen(js_name = setAttribute)]
    pub fn set_attribute(&self, name: &str, value: &str, txn: &ImplicitTransaction) {
        if let Some(mut txn) = get_txn_mut(txn) {
            self.0.insert_attribute(txn.as_mut(), name, value)
        } else {
            let mut txn = self.0.transact_mut();
            self.0.insert_attribute(&mut txn, name, value)
        }
    }

    /// Returns a value of an attribute given its `name`. If no attribute with such name existed,
    /// `null` will be returned.
    #[wasm_bindgen(js_name = getAttribute)]
    pub fn get_attribute(&self, name: &str, txn: &ImplicitTransaction) -> Option<String> {
        if let Some(txn) = get_txn(txn) {
            self.0.get_attribute(&*txn, name)
        } else {
            let txn = self.0.transact();
            self.0.get_attribute(&txn, name)
        }
    }

    /// Removes an attribute from this XML node, given its `name`.
    #[wasm_bindgen(js_name = removeAttribute)]
    pub fn remove_attribute(&self, name: &str, txn: &ImplicitTransaction) {
        if let Some(mut txn) = get_txn_mut(txn) {
            self.0.remove_attribute(txn.as_mut(), &name);
        } else {
            let mut txn = self.0.transact_mut();
            self.0.remove_attribute(&mut txn, &name);
        }
    }

    /// Returns an iterator that enables to traverse over all attributes of this XML node in
    /// unspecified order.
    #[wasm_bindgen(js_name = attributes)]
    pub fn attributes(&self, txn: &ImplicitTransaction) -> JsValue {
        if let Some(txn) = get_txn(txn) {
            let attrs = self.0.attributes(&*txn);
            iter_to_map(attrs).into()
        } else {
            let txn = self.0.transact();
            let attrs = self.0.attributes(&txn);
            iter_to_map(attrs).into()
        }
    }

    /// Returns an iterator that enables a deep traversal of this XML node - starting from first
    /// child over this XML node successors using depth-first strategy.
    #[wasm_bindgen(js_name = treeWalker)]
    pub fn tree_walker(&self, txn: &ImplicitTransaction) -> JsValue {
        if let Some(txn) = get_txn(txn) {
            let tree_walker = self.0.successors(&*txn);
            iter_to_array(tree_walker).into()
        } else {
            let txn = self.0.transact();
            let tree_walker = self.0.successors(&txn);
            iter_to_array(tree_walker).into()
        }
    }

    /// Subscribes to all operations happening over this instance of `YXmlElement`. All changes are
    /// batched and eventually triggered during transaction commit phase.
    /// Returns an `YObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observe)]
    pub fn observe(&mut self, f: js_sys::Function) -> YXmlObserver {
        let sub = self.0.observe(move |txn, e| {
            let e = YXmlEvent::new(e, txn);
            let arg: JsValue = e.into();
            f.call1(&JsValue::UNDEFINED, &arg).unwrap();
        });
        YXmlObserver(sub)
    }

    /// Subscribes to all operations happening over this Y shared type, as well as events in
    /// shared types stored within this one. All changes are batched and eventually triggered
    /// during transaction commit phase.
    /// Returns an `YEventObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observeDeep)]
    pub fn observe_deep(&mut self, f: js_sys::Function) -> YEventObserver {
        let sub = self.0.observe_deep(move |txn, e| {
            let arg = events_into_js(txn, e);
            f.call1(&JsValue::UNDEFINED, &arg).unwrap();
        });
        YEventObserver(sub)
    }
}

/// Represents a list of `YXmlElement` and `YXmlText` types.
/// A `YXmlFragment` is similar to a `YXmlElement`, but it does not have a
/// nodeName and it does not have attributes. Though it can be bound to a DOM
/// element - in this case the attributes and the nodeName are not shared
#[wasm_bindgen]
pub struct YXmlFragment(XmlFragmentRef);

#[wasm_bindgen]
impl YXmlFragment {
    /// Returns a number of child XML nodes stored within this `YXMlElement` instance.
    #[wasm_bindgen(js_name = length)]
    pub fn length(&self, txn: &ImplicitTransaction) -> u32 {
        if let Some(txn) = get_txn(txn) {
            self.0.len(&*txn)
        } else {
            let txn = self.0.transact();
            self.0.len(&txn)
        }
    }

    /// Inserts a new instance of `YXmlElement` as a child of this XML node and returns it.
    #[wasm_bindgen(js_name = insertXmlElement)]
    pub fn insert_xml_element(
        &self,
        index: u32,
        name: &str,
        txn: &ImplicitTransaction,
    ) -> YXmlElement {
        if let Some(mut txn) = get_txn_mut(txn) {
            YXmlElement(
                self.0
                    .insert(txn.as_mut(), index, XmlElementPrelim::empty(name)),
            )
        } else {
            let mut txn = self.0.transact_mut();
            YXmlElement(
                self.0
                    .insert(&mut txn, index, XmlElementPrelim::empty(name)),
            )
        }
    }

    /// Inserts a new instance of `YXmlText` as a child of this XML node and returns it.
    #[wasm_bindgen(js_name = insertXmlText)]
    pub fn insert_xml_text(&self, index: u32, txn: &ImplicitTransaction) -> YXmlText {
        if let Some(mut txn) = get_txn_mut(txn) {
            YXmlText(self.0.insert(txn.as_mut(), index, XmlTextPrelim::new("")))
        } else {
            let mut txn = self.0.transact_mut();
            YXmlText(self.0.insert(&mut txn, index, XmlTextPrelim::new("")))
        }
    }

    /// Removes a range of children XML nodes from this `YXmlElement` instance,
    /// starting at given `index`.
    #[wasm_bindgen(method, js_name = delete)]
    pub fn delete(&self, index: u32, length: u32, txn: &ImplicitTransaction) {
        if let Some(mut txn) = get_txn_mut(txn) {
            self.0.remove_range(txn.as_mut(), index, length)
        } else {
            let mut txn = self.0.transact_mut();
            self.0.remove_range(&mut txn, index, length)
        }
    }

    /// Appends a new instance of `YXmlElement` as the last child of this XML node and returns it.
    #[wasm_bindgen(js_name = pushXmlElement)]
    pub fn push_xml_element(&self, name: &str, txn: &ImplicitTransaction) -> YXmlElement {
        if let Some(mut txn) = get_txn_mut(txn) {
            YXmlElement(
                self.0
                    .push_back(txn.as_mut(), XmlElementPrelim::empty(name)),
            )
        } else {
            let mut txn = self.0.transact_mut();
            YXmlElement(self.0.push_back(&mut txn, XmlElementPrelim::empty(name)))
        }
    }

    /// Appends a new instance of `YXmlText` as the last child of this XML node and returns it.
    #[wasm_bindgen(js_name = pushXmlText)]
    pub fn push_xml_text(&self, txn: &ImplicitTransaction) -> YXmlText {
        if let Some(mut txn) = get_txn_mut(txn) {
            YXmlText(self.0.push_back(txn.as_mut(), XmlTextPrelim::new("")))
        } else {
            let mut txn = self.0.transact_mut();
            YXmlText(self.0.push_back(&mut txn, XmlTextPrelim::new("")))
        }
    }

    /// Returns a first child of this XML node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node has not children.
    #[wasm_bindgen(js_name = firstChild)]
    pub fn first_child(&self) -> JsValue {
        if let Some(xml) = self.0.first_child() {
            xml_into_js(xml)
        } else {
            JsValue::undefined()
        }
    }

    /// Returns a string representation of this XML node.
    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &ImplicitTransaction) -> String {
        if let Some(txn) = get_txn(txn) {
            self.0.get_string(&*txn)
        } else {
            self.0.get_string(&self.0.transact())
        }
    }

    /// Returns an iterator that enables a deep traversal of this XML node - starting from first
    /// child over this XML node successors using depth-first strategy.
    #[wasm_bindgen(js_name = treeWalker)]
    pub fn tree_walker(&self, txn: &ImplicitTransaction) -> JsValue {
        if let Some(txn) = get_txn(txn) {
            let tree_walker = self.0.successors(&*txn);
            iter_to_array(tree_walker).into()
        } else {
            let txn = self.0.transact();
            let tree_walker = self.0.successors(&txn);
            iter_to_array(tree_walker).into()
        }
    }

    /// Subscribes to all operations happening over this instance of `YXmlElement`. All changes are
    /// batched and eventually triggered during transaction commit phase.
    /// Returns an `YObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observe)]
    pub fn observe(&mut self, f: js_sys::Function) -> YXmlObserver {
        let sub = self.0.observe(move |txn, e| {
            let e = YXmlEvent::new(e, txn);
            let arg: JsValue = e.into();
            f.call1(&JsValue::UNDEFINED, &arg).unwrap();
        });
        YXmlObserver(sub)
    }

    /// Subscribes to all operations happening over this Y shared type, as well as events in
    /// shared types stored within this one. All changes are batched and eventually triggered
    /// during transaction commit phase.
    /// Returns an `YEventObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observeDeep)]
    pub fn observe_deep(&mut self, f: js_sys::Function) -> YEventObserver {
        let sub = self.0.observe_deep(move |txn, e| {
            let arg = events_into_js(txn, e);
            f.call1(&JsValue::UNDEFINED, &arg).unwrap();
        });
        YEventObserver(sub)
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
pub struct YXmlText(XmlTextRef);

#[wasm_bindgen]
impl YXmlText {
    /// Returns length of an underlying string stored in this `YXmlText` instance,
    /// understood as a number of UTF-8 encoded bytes.
    #[wasm_bindgen]
    pub fn length(&self, txn: &ImplicitTransaction) -> u32 {
        if let Some(txn) = get_txn(txn) {
            self.0.len(&*txn)
        } else {
            let txn = self.0.transact();
            self.0.len(&txn)
        }
    }

    /// Inserts a given `chunk` of text into this `YXmlText` instance, starting at a given `index`.
    ///
    /// Optional object with defined `attributes` will be used to wrap provided text `chunk`
    /// with a formatting blocks.
    #[wasm_bindgen(js_name = insert)]
    pub fn insert(&self, index: i32, chunk: &str, attrs: JsValue, txn: &ImplicitTransaction) {
        if let Some(mut txn) = get_txn_mut(txn) {
            if let Some(attrs) = YText::parse_attrs(attrs) {
                self.0
                    .insert_with_attributes(txn.as_mut(), index as u32, chunk, attrs)
            } else {
                self.0.insert(txn.as_mut(), index as u32, chunk)
            }
        } else {
            let mut txn = self.0.transact_mut();
            if let Some(attrs) = YText::parse_attrs(attrs) {
                self.0
                    .insert_with_attributes(&mut txn, index as u32, chunk, attrs)
            } else {
                self.0.insert(&mut txn, index as u32, chunk)
            }
        }
    }

    /// Formats text within bounds specified by `index` and `len` with a given formatting
    /// attributes.
    #[wasm_bindgen(js_name = format)]
    pub fn format(&self, index: i32, len: i32, attrs: JsValue, txn: &ImplicitTransaction) {
        if let Some(attrs) = YText::parse_attrs(attrs) {
            if let Some(mut txn) = get_txn_mut(txn) {
                self.0.format(txn.as_mut(), index as u32, len as u32, attrs)
            } else {
                let mut txn = self.0.transact_mut();
                self.0.format(&mut txn, index as u32, len as u32, attrs)
            }
        } else {
            panic!("couldn't parse format attributes")
        }
    }

    /// Inserts a given `embed` object into this `YXmlText` instance, starting at a given `index`.
    ///
    /// Optional object with defined `attributes` will be used to wrap provided `embed`
    /// with a formatting blocks.`attributes` are only supported for a `YXmlText` instance which
    /// already has been integrated into document store.
    #[wasm_bindgen(js_name = insertEmbed)]
    pub fn insert_embed(
        &self,
        index: u32,
        embed: JsValue,
        attributes: JsValue,
        txn: &ImplicitTransaction,
    ) {
        let content = js_into_any(&embed).unwrap();
        if let Some(mut txn) = get_txn_mut(txn) {
            if let Some(attrs) = YText::parse_attrs(attributes) {
                self.0
                    .insert_embed_with_attributes(txn.as_mut(), index, content, attrs);
            } else {
                self.0.insert_embed(txn.as_mut(), index, content);
            }
        } else {
            let mut txn = self.0.transact_mut();
            if let Some(attrs) = YText::parse_attrs(attributes) {
                self.0
                    .insert_embed_with_attributes(&mut txn, index, content, attrs);
            } else {
                self.0.insert_embed(&mut txn, index, content);
            }
        }
    }

    /// Appends a given `chunk` of text at the end of `YXmlText` instance.
    ///
    /// Optional object with defined `attributes` will be used to wrap provided text `chunk`
    /// with a formatting blocks.
    #[wasm_bindgen(js_name = push)]
    pub fn push(&self, chunk: &str, attrs: JsValue, txn: &ImplicitTransaction) {
        let index = if let Some(txn) = get_txn(txn) {
            self.0.len(&*txn)
        } else {
            let txn = self.0.transact();
            self.0.len(&txn)
        };
        self.insert(index as i32, chunk, attrs, txn)
    }

    /// Deletes a specified range of of characters, starting at a given `index`.
    /// Both `index` and `length` are counted in terms of a number of UTF-8 character bytes.
    #[wasm_bindgen(method, js_name = delete)]
    pub fn delete(&self, index: u32, length: u32, txn: &ImplicitTransaction) {
        if let Some(mut txn) = get_txn_mut(txn) {
            self.0.remove_range(txn.as_mut(), index, length)
        } else {
            let mut txn = self.0.transact_mut();
            self.0.remove_range(&mut txn, index, length)
        }
    }

    /// Returns a next XML sibling node of this XMl node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node is a last child of
    /// parent XML node.
    #[wasm_bindgen(js_name = nextSibling)]
    pub fn next_sibling(&self, txn: &ImplicitTransaction) -> JsValue {
        if let Some(txn) = get_txn(txn) {
            let mut siblings = self.0.siblings(&*txn);
            siblings
                .next()
                .map(xml_into_js)
                .unwrap_or(JsValue::UNDEFINED)
        } else {
            let txn = self.0.transact_mut();
            let mut siblings = self.0.siblings(&txn);
            siblings
                .next()
                .map(xml_into_js)
                .unwrap_or(JsValue::UNDEFINED)
        }
    }

    /// Returns a previous XML sibling node of this XMl node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node is a first child
    /// of parent XML node.
    #[wasm_bindgen(js_name = prevSibling)]
    pub fn prev_sibling(&self, txn: &ImplicitTransaction) -> JsValue {
        if let Some(txn) = get_txn(txn) {
            let mut siblings = self.0.siblings(&*txn);
            siblings
                .next_back()
                .map(xml_into_js)
                .unwrap_or(JsValue::UNDEFINED)
        } else {
            let txn = self.0.transact_mut();
            let mut siblings = self.0.siblings(&txn);
            siblings
                .next_back()
                .map(xml_into_js)
                .unwrap_or(JsValue::UNDEFINED)
        }
    }

    /// Returns a parent `YXmlElement` node or `undefined` if current node has no parent assigned.
    #[wasm_bindgen(getter, js_name = parent)]
    pub fn parent(&self) -> JsValue {
        if let Some(xml) = self.0.parent() {
            xml_into_js(xml)
        } else {
            JsValue::undefined()
        }
    }

    /// Returns an underlying string stored in this `YXmlText` instance.
    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &ImplicitTransaction) -> String {
        if let Some(txn) = get_txn(txn) {
            self.0.get_string(&*txn)
        } else {
            let txn = self.0.transact();
            self.0.get_string(&txn)
        }
    }

    /// Sets a `name` and `value` as new attribute for this XML node. If an attribute with the same
    /// `name` already existed on that node, its value with be overridden with a provided one.
    #[wasm_bindgen(js_name = setAttribute)]
    pub fn set_attribute(&self, name: &str, value: &str, txn: &ImplicitTransaction) {
        if let Some(mut txn) = get_txn_mut(txn) {
            self.0.insert_attribute(txn.as_mut(), name, value);
        } else {
            let mut txn = self.0.transact_mut();
            self.0.insert_attribute(&mut txn, name, value);
        }
    }

    /// Returns a value of an attribute given its `name`. If no attribute with such name existed,
    /// `null` will be returned.
    #[wasm_bindgen(js_name = getAttribute)]
    pub fn get_attribute(&self, name: &str, txn: &ImplicitTransaction) -> Option<String> {
        if let Some(txn) = get_txn(txn) {
            self.0.get_attribute(&*txn, name)
        } else {
            let txn = self.0.transact();
            self.0.get_attribute(&txn, name)
        }
    }

    /// Removes an attribute from this XML node, given its `name`.
    #[wasm_bindgen(js_name = removeAttribute)]
    pub fn remove_attribute(&self, name: &str, txn: &ImplicitTransaction) {
        if let Some(mut txn) = get_txn_mut(txn) {
            self.0.remove_attribute(txn.as_mut(), &name);
        } else {
            let mut txn = self.0.transact_mut();
            self.0.remove_attribute(&mut txn, &name);
        }
    }

    /// Returns an iterator that enables to traverse over all attributes of this XML node in
    /// unspecified order.
    #[wasm_bindgen(js_name = attributes)]
    pub fn attributes(&self, txn: &ImplicitTransaction) -> JsValue {
        if let Some(txn) = get_txn(txn) {
            let attrs = self.0.attributes(&*txn);
            iter_to_map(attrs).into()
        } else {
            let txn = self.0.transact();
            let attrs = self.0.attributes(&txn);
            iter_to_map(attrs).into()
        }
    }

    /// Subscribes to all operations happening over this instance of `YXmlText`. All changes are
    /// batched and eventually triggered during transaction commit phase.
    /// Returns an `YObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observe)]
    pub fn observe(&mut self, f: js_sys::Function) -> YXmlTextObserver {
        let sub = self.0.observe(move |txn, e| {
            let e = YXmlTextEvent::new(e, txn);
            let arg: JsValue = e.into();
            f.call1(&JsValue::UNDEFINED, &arg).unwrap();
        });
        YXmlTextObserver(sub)
    }

    /// Subscribes to all operations happening over this Y shared type, as well as events in
    /// shared types stored within this one. All changes are batched and eventually triggered
    /// during transaction commit phase.
    /// Returns an `YEventObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observeDeep)]
    pub fn observe_deep(&mut self, f: js_sys::Function) -> YEventObserver {
        let sub = self.0.observe_deep(move |txn, e| {
            let arg = events_into_js(txn, e);
            f.call1(&JsValue::UNDEFINED, &arg).unwrap();
        });
        YEventObserver(sub)
    }
}

#[wasm_bindgen]
#[repr(transparent)]
pub struct YUndoManager(UndoManager);

#[wasm_bindgen]
impl YUndoManager {
    #[wasm_bindgen(constructor)]
    pub fn new(doc: &YDoc, scope: JsValue, options: JsValue) -> Self {
        let doc = &doc.0;
        let scope = JsValueWrapper(scope);
        let mut o = yrs::undo::Options::default();
        o.timestamp = Rc::new(|| js_sys::Date::now() as u64);
        if options.is_object() {
            if let Ok(js) = Reflect::get(&options, &JsValue::from_str("captureTimeout")) {
                if let Some(millis) = js.as_f64() {
                    o.capture_timeout_millis = millis as u64;
                }
            }
            if let Ok(js) = Reflect::get(&options, &JsValue::from_str("trackedOrigins")) {
                if js_sys::Array::is_array(&js) {
                    let array = js_sys::Array::from(&js);
                    for js in array.iter() {
                        let v = JsValueWrapper(js);
                        o.tracked_origins.insert(v.into());
                    }
                }
            }
        }
        YUndoManager(UndoManager::with_options(doc, &scope, o))
    }

    #[wasm_bindgen(js_name = addToScope)]
    pub fn add_to_scope(&mut self, ytypes: js_sys::Array) {
        for js in ytypes.iter() {
            let scope = JsValueWrapper(js);
            self.0.expand_scope(&scope);
        }
    }

    #[wasm_bindgen(js_name = addTrackedOrigin)]
    pub fn add_tracked_origin(&mut self, origin: JsValue) {
        self.0.include_origin(JsValueWrapper(origin))
    }

    #[wasm_bindgen(js_name = removeTrackedOrigin)]
    pub fn remove_tracked_origin(&mut self, origin: JsValue) {
        self.0.exclude_origin(JsValueWrapper(origin))
    }

    #[wasm_bindgen(js_name = clear)]
    pub fn clear(&mut self) -> Result<(), JsValue> {
        if let Err(err) = self.0.clear() {
            Err(JsValue::from_str(&err.to_string()))
        } else {
            Ok(())
        }
    }

    #[wasm_bindgen(js_name = stopCapturing)]
    pub fn stop_capturing(&mut self) {
        self.0.reset()
    }

    #[wasm_bindgen(js_name = undo)]
    pub fn undo(&mut self) -> Result<(), JsValue> {
        if let Err(err) = self.0.undo() {
            Err(JsValue::from_str(&err.to_string()))
        } else {
            Ok(())
        }
    }

    #[wasm_bindgen(js_name = redo)]
    pub fn redo(&mut self) -> Result<(), JsValue> {
        if let Err(err) = self.0.redo() {
            Err(JsValue::from_str(&err.to_string()))
        } else {
            Ok(())
        }
    }

    #[wasm_bindgen(getter, js_name = canUndo)]
    pub fn can_undo(&mut self) -> bool {
        self.0.can_undo()
    }

    #[wasm_bindgen(getter, js_name = canRedo)]
    pub fn can_redo(&mut self) -> bool {
        self.0.can_redo()
    }

    #[wasm_bindgen(js_name = onStackItemAdded)]
    pub fn on_item_added(&mut self, callback: js_sys::Function) -> YUndoObserver {
        YUndoObserver(self.0.observe_item_added(move |_, e| {
            let arg: JsValue = YUndoEvent::new(e).into();
            callback.call1(&JsValue::UNDEFINED, &arg).unwrap();
        }))
    }

    #[wasm_bindgen(js_name = onStackItemPopped)]
    pub fn on_item_popped(&mut self, callback: js_sys::Function) -> YUndoObserver {
        YUndoObserver(self.0.observe_item_popped(move |_, e| {
            let arg: JsValue = YUndoEvent::new(e).into();
            callback.call1(&JsValue::UNDEFINED, &arg).unwrap();
        }))
    }
}

#[wasm_bindgen]
pub struct YUndoEvent {
    origin: JsValue,
    kind: JsValue,
    stack_item: JsValue,
}

#[wasm_bindgen]
impl YUndoEvent {
    #[wasm_bindgen(getter, js_name = origin)]
    pub fn origin(&self) -> JsValue {
        self.origin.clone()
    }
    #[wasm_bindgen(getter, js_name = kind)]
    pub fn kind(&self) -> JsValue {
        self.kind.clone()
    }
    #[wasm_bindgen(getter, js_name = stackItem)]
    pub fn stack_item(&self) -> JsValue {
        self.stack_item.clone()
    }

    fn new(e: &yrs::undo::Event) -> Self {
        let stack_item: JsValue = Object::new().into();
        Reflect::set(
            &stack_item,
            &JsValue::from_str("deletions"),
            &delete_set_into_map(e.item.deletions()),
        )
        .unwrap();
        Reflect::set(
            &stack_item,
            &JsValue::from_str("insertions"),
            &delete_set_into_map(e.item.insertions()),
        )
        .unwrap();
        YUndoEvent {
            stack_item,
            origin: from_origin(e.origin.as_ref()),
            kind: match e.kind {
                EventKind::Undo => JsValue::from_str("undo"),
                EventKind::Redo => JsValue::from_str("redo"),
            },
        }
    }
}

#[wasm_bindgen]
pub struct YUndoObserver(UndoEventSubscription);

#[repr(transparent)]
struct JsValueWrapper(JsValue);

impl Prelim for JsValueWrapper {
    type Return = Unused;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let content = if let Some(any) = js_into_any(&self.0) {
            ItemContent::Any(vec![any])
        } else if let Ok(shared) = Shared::try_from(&self.0) {
            if shared.is_prelim() {
                let branch = shared.as_branch();
                ItemContent::Type(branch)
            } else if let Shared::Doc(doc) = shared {
                if doc.0.parent_doc().is_some() {
                    panic!("Cannot integrate document, that has been already integrated elsewhere")
                } else {
                    ItemContent::Doc(None, doc.0.clone())
                }
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

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        if let Ok(shared) = Shared::try_from(&self.0) {
            if shared.is_prelim() {
                match shared {
                    Shared::Text(v) => {
                        let text = TextRef::from(inner_ref);
                        if let SharedType::Prelim(v) =
                            v.0.replace(SharedType::Integrated(text.clone()))
                        {
                            text.push(txn, v.as_str());
                        }
                    }
                    Shared::Array(v) => {
                        let array = ArrayRef::from(inner_ref);
                        if let SharedType::Prelim(items) =
                            v.0.replace(SharedType::Integrated(array.clone()))
                        {
                            let len = array.len(txn);
                            insert_at(&array, txn, len, items);
                        }
                    }
                    Shared::Map(v) => {
                        let map = MapRef::from(inner_ref);
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

impl Into<Origin> for JsValueWrapper {
    fn into(self) -> Origin {
        if let Ok(branch) = self.as_branch_ptr() {
            BranchPtr::from(branch).into()
        } else {
            let ptr = self.0.into_abi();
            let bytes = ptr.to_be_bytes();
            Origin::from(bytes.as_ref())
        }
    }
}

impl AsRef<Branch> for JsValueWrapper {
    fn as_ref(&self) -> &Branch {
        let ptr = self.as_branch_ptr().unwrap();
        let branch = ptr.deref();
        unsafe { std::mem::transmute(branch) }
    }
}

impl JsValueWrapper {
    fn as_branch_ptr<'a>(&'a self) -> Result<BranchPtr, JsValue> {
        let s = Shared::<'a>::try_from(&self.0)?;
        match s {
            Shared::Text(v) => {
                if let SharedType::Integrated(x) = v.0.borrow().deref() {
                    Ok(BranchPtr::from(x.as_ref()))
                } else {
                    Err(JsValue::from_str(
                        "Shared type must be integrated first to be used in this context",
                    ))
                }
            }
            Shared::Array(v) => {
                if let SharedType::Integrated(x) = v.0.borrow().deref() {
                    Ok(BranchPtr::from(x.as_ref()))
                } else {
                    Err(JsValue::from_str(
                        "Shared type must be integrated first to be used in this context",
                    ))
                }
            }
            Shared::Map(v) => {
                if let SharedType::Integrated(x) = v.0.borrow().deref() {
                    Ok(BranchPtr::from(x.as_ref()))
                } else {
                    Err(JsValue::from_str(
                        "Shared type must be integrated first to be used in this context",
                    ))
                }
            }
            Shared::XmlElement(v) => Ok(BranchPtr::from(v.deref().0.as_ref())),
            Shared::XmlText(v) => Ok(BranchPtr::from(v.deref().0.as_ref())),
            Shared::XmlFragment(v) => Ok(BranchPtr::from(v.deref().0.as_ref())),
            Shared::Doc(_) => Err(JsValue::from_str("Doc is not a shared type")),
        }
    }
}

fn insert_at(dst: &ArrayRef, txn: &mut TransactionMut, index: u32, src: Vec<JsValue>) {
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
        Some(Any::from(v.as_string()?))
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
        Some(Any::from(result))
    } else if v.is_object() {
        if let Ok(_) = Shared::try_from(v) {
            None
        } else {
            let mut map = HashMap::new();
            let object = js_sys::Object::from(v.clone());
            let entries = js_sys::Object::entries(&object);
            for tuple in entries.iter() {
                let tuple = js_sys::Array::from(&tuple);
                let key: String = tuple.get(0).as_string()?;
                let value = js_into_any(&tuple.get(1))?;
                map.insert(key, value);
            }
            Some(Any::from(map))
        }
    } else {
        None
    }
}

fn any_into_js(v: &Any) -> JsValue {
    match v {
        Any::Null => JsValue::NULL,
        Any::Undefined => JsValue::UNDEFINED,
        Any::Bool(v) => JsValue::from_bool(*v),
        Any::Number(v) => JsValue::from(*v),
        Any::BigInt(v) => JsValue::from(*v),
        Any::String(v) => JsValue::from(v.as_ref()),
        Any::Buffer(v) => {
            let v = Uint8Array::from(v.as_ref());
            v.into()
        }
        Any::Array(v) => {
            let a = js_sys::Array::new();
            for value in v.as_ref() {
                a.push(&any_into_js(value));
            }
            a.into()
        }
        Any::Map(v) => {
            let m = js_sys::Object::new();
            for (k, v) in v.as_ref() {
                let key = JsValue::from(k);
                let value = any_into_js(v);
                js_sys::Reflect::set(&m, &key, &value).unwrap();
            }
            m.into()
        }
    }
}

fn from_origin(origin: Option<&Origin>) -> JsValue {
    if let Some(o) = origin {
        let be: [u8; 4] = o.as_ref().try_into().unwrap();
        unsafe { JsValue::from_abi(u32::from_be_bytes(be)) }
    } else {
        JsValue::UNDEFINED
    }
}

fn value_into_js(v: Value) -> JsValue {
    match v {
        Value::Any(v) => any_into_js(&v),
        Value::YText(v) => YText::from(v).into(),
        Value::YArray(v) => YArray::from(v).into(),
        Value::YMap(v) => YMap::from(v).into(),
        Value::YXmlElement(v) => YXmlElement(v).into(),
        Value::YXmlText(v) => YXmlText(v).into(),
        Value::YXmlFragment(v) => YXmlFragment(v).into(),
        Value::YDoc(doc) => YDoc::from(doc).into(),
    }
}

fn xml_into_js(v: XmlNode) -> JsValue {
    match v {
        XmlNode::Element(v) => YXmlElement(v).into(),
        XmlNode::Text(v) => YXmlText(v).into(),
        XmlNode::Fragment(v) => YXmlFragment(v).into(),
    }
}

fn events_into_js(txn: &TransactionMut, e: &Events) -> JsValue {
    let mut array = js_sys::Array::new();
    let mapped = e.iter().map(|e| {
        let js: JsValue = match e {
            Event::Text(e) => YTextEvent::new(e, txn).into(),
            Event::Array(e) => YArrayEvent::new(e, txn).into(),
            Event::Map(e) => YMapEvent::new(e, txn).into(),
            Event::XmlText(e) => YXmlTextEvent::new(e, txn).into(),
            Event::XmlFragment(e) => YXmlEvent::new(e, txn).into(),
        };
        js
    });
    array.extend(mapped);
    array.into()
}

enum Shared<'a> {
    Text(Ref<'a, YText>),
    Array(Ref<'a, YArray>),
    Map(Ref<'a, YMap>),
    XmlElement(Ref<'a, YXmlElement>),
    XmlText(Ref<'a, YXmlText>),
    XmlFragment(Ref<'a, YXmlFragment>),
    Doc(Ref<'a, YDoc>),
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
        let ctor_name = Object::get_prototype_of(js).constructor().name();
        let ptr = Reflect::get(js, &JsValue::from_str(WASM_REF_PTR))?;
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
        } else if ctor_name == "YXmlFragment" {
            Ok(Shared::XmlFragment(as_ref(ptr_u32)))
        } else if ctor_name == "YDoc" {
            Ok(Shared::Doc(as_ref(ptr_u32)))
        } else {
            Err(ctor_name.into())
        }
    }
}

impl<'a> Shared<'a> {
    fn is_prelim(&self) -> bool {
        match self {
            Shared::Text(v) => v.prelim(),
            Shared::Array(v) => v.prelim(),
            Shared::Map(v) => v.prelim(),
            Shared::Doc(_)
            | Shared::XmlElement(_)
            | Shared::XmlText(_)
            | Shared::XmlFragment(_) => false,
        }
    }

    fn as_branch(&self) -> Box<Branch> {
        let type_ref = match self {
            Shared::Text(_) => TypeRef::Text,
            Shared::Array(_) => TypeRef::Array,
            Shared::Map(_) => TypeRef::Map,
            Shared::XmlElement(elem) => TypeRef::XmlElement(elem.0.tag().clone()),
            Shared::XmlText(_) => TypeRef::XmlText,
            Shared::XmlFragment(_) => TypeRef::XmlFragment,
            Shared::Doc(_) => TypeRef::SubDoc,
        };

        Branch::new(type_ref)
    }

    fn branch(&self) -> Option<BranchPtr> {
        match self {
            Shared::Text(v) => {
                let inner = v.0.borrow();
                let integrated = inner.as_integrated()?;
                Some(BranchPtr::from(integrated.as_ref()))
            }
            Shared::Array(v) => {
                let inner = v.0.borrow();
                let integrated = inner.as_integrated()?;
                Some(BranchPtr::from(integrated.as_ref()))
            }
            Shared::Map(v) => {
                let inner = v.0.borrow();
                let integrated = inner.as_integrated()?;
                Some(BranchPtr::from(integrated.as_ref()))
            }
            Shared::XmlElement(v) => Some(BranchPtr::from(v.0.as_ref())),
            Shared::XmlText(v) => Some(BranchPtr::from(v.0.as_ref())),
            Shared::XmlFragment(v) => Some(BranchPtr::from(v.0.as_ref())),
            Shared::Doc(_) => None,
        }
    }
}

/// Retrieves a sticky index corresponding to a given human-readable `index` pointing into
/// the shared `ytype`. Unlike standard indexes sticky indexes enables to track
/// the location inside of a shared y-types, even in the face of concurrent updates.
///
/// If association is >= 0, the resulting position will point to location **after** the referenced index.
/// If association is < 0, the resulting position will point to location **before** the referenced index.
#[wasm_bindgen(js_name=createStickyIndexFromType)]
pub fn create_sticky_index_from_type(
    ytype: &JsValue,
    index: u32,
    assoc: i32,
    txn: &ImplicitTransaction,
) -> Result<JsValue, JsValue> {
    if let Ok(shared) = Shared::try_from(ytype) {
        if shared.is_prelim() {
            return Err(JsValue::from_str(
                "cannot build sticky index if shared type was not integrated",
            ));
        }
        let assoc = if assoc >= 0 {
            Assoc::After
        } else {
            Assoc::Before
        };
        if let Some(branch) = shared.branch() {
            let pos = if let Some(mut txn) = get_txn_mut(txn) {
                StickyIndex::at(txn.as_mut(), branch, index, assoc)
            } else {
                let mut txn = branch.transact_mut();
                StickyIndex::at(&mut txn, branch, index, assoc)
            };
            let result = if let Some(pos) = pos {
                Ok(pos.into_js())
            } else {
                Ok(JsValue::NULL)
            };
            return result;
        }
    }
    Err(JsValue::from_str("shared type parameter is not indexable"))
}

/// Converts a sticky index (see: `createStickyIndexFromType`) into an object
/// containing human-readable index.
#[wasm_bindgen(js_name=createOffsetFromStickyIndex)]
pub fn create_offset_from_sticky_index(rpos: &JsValue, doc: &YDoc) -> Result<JsValue, JsValue> {
    let pos = sticky_index_from_js(rpos)?;
    let txn = doc.0.transact();
    if let Some(abs) = pos.get_offset(&txn) {
        Ok(abs.into_js())
    } else {
        Ok(JsValue::NULL)
    }
}

/// Serializes sticky index created by `createStickyIndexFromType` into a binary
/// payload.
#[wasm_bindgen(js_name=encodeStickyIndex)]
pub fn encode_sticky_index(rpos: &JsValue) -> Result<Uint8Array, JsValue> {
    if let Ok(pos) = sticky_index_from_js(rpos) {
        let bytes = Uint8Array::from(pos.encode_v1().as_slice());
        Ok(bytes)
    } else {
        Err(JsValue::from_str("passed parameter is not StickyIndex"))
    }
}

/// Deserializes sticky index serialized previously by `encodeStickyIndex`.
#[wasm_bindgen(js_name=decodeStickyIndex)]
pub fn decode_sticky_index(bin: Uint8Array) -> Result<JsValue, JsValue> {
    let data: Vec<u8> = bin.to_vec();
    match StickyIndex::decode_v1(&data) {
        Ok(value) => Ok(value.into_js()),
        Err(err) => Err(JsValue::from_str(&err.to_string())),
    }
}

fn sticky_index_from_js(js: &JsValue) -> Result<StickyIndex, JsValue> {
    let value = Reflect::get(js, &JsValue::from_str("item"))?;
    let context = if value.is_undefined() || value.is_null() {
        let value = Reflect::get(js, &JsValue::from_str("tname"))?;
        if value.is_undefined() || value.is_null() {
            let value = Reflect::get(js, &JsValue::from_str("type"))?;
            let id = id_from_js(&value)?;
            IndexScope::Nested(id)
        } else {
            if let Some(tname) = value.as_string() {
                IndexScope::Root(tname.into())
            } else {
                return Err(value);
            }
        }
    } else {
        let id = id_from_js(&value)?;
        IndexScope::Relative(id)
    };
    let assoc = Reflect::get(js, &JsValue::from_str("assoc"))?;
    let assoc = if let Some(a) = assoc.as_f64() {
        if a >= 0.0 {
            Assoc::After
        } else {
            Assoc::Before
        }
    } else {
        return Err(assoc);
    };

    Ok(StickyIndex::new(context, assoc))
}

fn id_from_js(js: &JsValue) -> Result<ID, JsValue> {
    let value = Reflect::get(js, &JsValue::from_str("client"))?;
    let client = if let Ok(client) = u64::try_from(value) {
        client as ClientID
    } else {
        return Err(JsValue::from_str("ID.client was not a number"));
    };
    let value = Reflect::get(js, &JsValue::from_str("clock"))?;
    let clock = if let Some(clock) = value.as_f64() {
        clock as u32
    } else {
        return Err(JsValue::from_str("ID.clock was not a number"));
    };
    Ok(ID::new(client, clock))
}

impl IntoJs for ID {
    fn into_js(self) -> JsValue {
        let js: JsValue = js_sys::Object::new().into();
        Reflect::set(
            &js,
            &JsValue::from_str("client"),
            &JsValue::from(self.client),
        )
        .unwrap();
        Reflect::set(&js, &JsValue::from_str("clock"), &JsValue::from(self.clock)).unwrap();
        js
    }
}

impl IntoJs for StickyIndex {
    fn into_js(self) -> JsValue {
        let js: JsValue = js_sys::Object::new().into();

        match self.scope() {
            IndexScope::Relative(id) => {
                Reflect::set(&js, &JsValue::from_str("item"), &id.into_js()).unwrap();
            }
            IndexScope::Nested(id) => {
                Reflect::set(&js, &JsValue::from_str("type"), &id.into_js()).unwrap();
            }
            IndexScope::Root(tname) => {
                Reflect::set(&js, &JsValue::from_str("tname"), &JsValue::from_str(&tname)).unwrap();
            }
        }

        let assoc = match self.assoc {
            Assoc::After => 0,
            Assoc::Before => -1,
        };
        Reflect::set(&js, &JsValue::from_str("assoc"), &JsValue::from(assoc)).unwrap();
        js
    }
}

impl IntoJs for Offset {
    fn into_js(self) -> JsValue {
        let js: JsValue = js_sys::Object::new().into();
        Reflect::set(&js, &JsValue::from_str("index"), &JsValue::from(self.index)).unwrap();
        let assoc = match self.assoc {
            Assoc::After => 0,
            Assoc::Before => -1,
        };
        Reflect::set(&js, &JsValue::from_str("assoc"), &JsValue::from(assoc)).unwrap();
        js
    }
}
