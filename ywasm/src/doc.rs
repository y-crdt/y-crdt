use crate::array::YArray;
use crate::collection::SharedCollection;
use crate::js::{Callback, Js};
use crate::map::YMap;
use crate::text::YText;
use crate::xml_frag::YXmlFragment;
use crate::ImplicitTransaction;
use crate::Result;
use js_sys::Function;
use serde::Deserialize;
use std::iter::FromIterator;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use wasm_bindgen::__rt::{assert_not_null, IntoJsResult, RcRefMut, WasmRefCell};
use wasm_bindgen::convert::{FromWasmAbi, IntoWasmAbi, RefFromWasmAbi, RefMutFromWasmAbi};
use wasm_bindgen::describe::{inform, WasmDescribe, RUST_STRUCT};
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::types::TYPE_REFS_DOC;
use yrs::{OffsetKind, Options};

/// A ywasm document type. Documents are most important units of collaborative resources management.
/// All shared collections live within a scope of their corresponding documents. All updates are
/// generated on per-document basis (rather than individual shared type). All operations on shared
/// collections happen via [Transaction], which lifetime is also bound to a document.
///
/// Document manages so-called root types, which are top-level shared types definitions (as opposed
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
#[derive(Clone)]
pub struct Doc {
    instance: Rc<WasmRefCell<DocState>>,
}

pub(crate) struct DocState {
    inner: yrs::Doc,
    current_transaction: Option<crate::Transaction>,
    parent_doc: Option<Doc>,
}

impl From<yrs::Doc> for DocState {
    fn from(inner: yrs::Doc) -> Self {
        DocState {
            inner,
            current_transaction: None,
            parent_doc: None,
        }
    }
}

impl Deref for DocState {
    type Target = yrs::Doc;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for DocState {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Doc {
    pub(crate) fn transact<F, T>(&self, origin: JsValue, f: F) -> Result<T>
    where
        F: FnOnce(&mut crate::Transaction) -> Result<T>,
    {
        let this = RcRefMut::new(self.instance.clone());
        match &mut this.current_transaction {
            None => {
                this.current_transaction = Some(crate::Transaction::new(this, origin));
                let tx = this.current_transaction.as_mut().unwrap();
                let result = f(tx);
                this.current_transaction = None;
                result
            }
            Some(t) => f(t),
        }
    }
}

#[wasm_bindgen]
impl Doc {
    /// Creates a new ywasm document. If `id` parameter was passed it will be used as this document
    /// globally unique identifier (it's up to caller to ensure that requirement). Otherwise it will
    /// be assigned a randomly generated number.
    #[wasm_bindgen(constructor)]
    pub fn new(options: &JsValue) -> Result<Doc> {
        use gloo_utils::format::JsValueSerdeExt;
        let js_options = options
            .into_serde::<Option<DocOptions>>()
            .map_err(|_| JsValue::from_str("invalid document options"))?;
        let mut options = Options::default();
        options.offset_kind = OffsetKind::Utf16;
        if let Some(o) = js_options {
            o.fill(&mut options);
        }

        Ok(yrs::Doc::with_options(options).into())
    }

    #[wasm_bindgen(getter, js_name = type)]
    #[inline]
    pub fn get_type(&self) -> u8 {
        TYPE_REFS_DOC
    }

    /// Checks if a document is a preliminary type. It returns false, if current document
    /// is already a sub-document of another document.
    #[wasm_bindgen(getter)]
    #[inline]
    pub fn prelim(&self) -> bool {
        let this = self.instance.borrow();
        this.parent_doc.is_none()
    }

    /// Returns a parent document of this document or null if current document is not sub-document.
    #[wasm_bindgen(getter, js_name = parentDoc)]
    pub fn parent_doc(&self) -> JsValue {
        let this = self.instance.borrow();
        match &this.parent_doc {
            None => JsValue::NULL,
            Some(parent) => parent.clone().into(),
        }
    }

    /// Gets unique peer identifier of this `YDoc` instance.
    #[wasm_bindgen(getter)]
    pub fn id(&self) -> f64 {
        self.instance.borrow().client_id() as f64
    }

    /// Gets globally unique identifier of this `YDoc` instance.
    #[wasm_bindgen(getter)]
    pub fn guid(&self) -> String {
        self.instance.borrow().guid().to_string()
    }

    #[wasm_bindgen(getter, js_name = shouldLoad)]
    pub fn should_load(&self) -> bool {
        self.instance.borrow().should_load()
    }

    #[wasm_bindgen(getter, js_name = autoLoad)]
    pub fn auto_load(&self) -> bool {
        self.instance.borrow().auto_load()
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
        let shared_ref = self.get_or_insert_text(name);
        let instance = self.instance.clone();
        YText(SharedCollection::integrated(shared_ref, instance))
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
        let shared_ref = self.get_or_insert_array(name);
        let instance = self.instance.clone();
        YArray(SharedCollection::integrated(shared_ref, instance))
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
        let shared_ref = self.get_or_insert_map(name);
        let instance = self.instance.clone();
        YMap(SharedCollection::integrated(shared_ref, instance))
    }

    /// Returns a `YXmlFragment` shared data type, that's accessible for subsequent accesses using
    /// given `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YXmlFragment` instance.
    #[wasm_bindgen(js_name = getXmlFragment)]
    pub fn get_xml_fragment(&self, name: &str) -> YXmlFragment {
        let shared_ref = self.get_or_insert_xml_fragment(name);
        let instance = self.instance.clone();
        YXmlFragment(SharedCollection::integrated(shared_ref, instance))
    }

    #[wasm_bindgen(js_name = on)]
    pub fn on(&self, event: &str, callback: js_sys::Function) -> Result<()> {
        let abi = callback.subscription_key();
        let result = match event {
            "update" => self.observe_update_v1_with(abi, move |txn, e| {
                let update = js_sys::Uint8Array::from(e.update.as_slice());
                let txn: JsValue = Transaction::from_ref(txn).into();
                callback.call2(&JsValue::UNDEFINED, &update, &txn).unwrap();
            }),
            "updateV2" => self.observe_update_v2_with(abi, move |txn, e| {
                let update = js_sys::Uint8Array::from(e.update.as_slice());
                let txn: JsValue = Transaction::from_ref(txn).into();
                callback.call2(&JsValue::UNDEFINED, &update, &txn).unwrap();
            }),
            "subdocs" => self.observe_subdocs_with(abi, move |txn, e| {
                let event: JsValue = YSubdocsEvent::new(e).into();
                let txn: JsValue = Transaction::from_ref(txn).into();
                callback.call2(&JsValue::UNDEFINED, &event, &txn).unwrap();
            }),
            "destroy" => self.observe_destroy_with(abi, move |txn, e| {
                let event: JsValue = Doc::from(e.clone()).into();
                let txn: JsValue = Transaction::from_ref(txn).into();
                callback.call2(&JsValue::UNDEFINED, &event, &txn).unwrap();
            }),
            "afterTransaction" => self.observe_after_transaction_with(abi, move |txn| {
                let txn: JsValue = Transaction::from_ref(txn).into();
                callback.call1(&JsValue::UNDEFINED, &txn).unwrap();
            }),
            "cleanup" => self.observe_transaction_cleanup_with(abi, move |txn, _| {
                let txn = Transaction::from_ref(txn).into();
                callback.call1(&JsValue::UNDEFINED, &txn).unwrap();
            }),
            other => {
                return Err(JsValue::from_str(&format!("unknown event: '{}'", other)).into());
            }
        };
        result.map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_TX))?;
        Ok(())
    }

    #[wasm_bindgen(js_name = off)]
    pub fn off(&self, event: &str, callback: js_sys::Function) -> Result<bool> {
        let abi = callback.subscription_key();
        let result = match event {
            "update" => self.unobserve_update_v1(abi),
            "updateV2" => self.unobserve_update_v2(abi),
            "subdocs" => self.unobserve_subdocs(abi),
            "destroy" => self.unobserve_destroy(abi),
            "afterTransaction" => self.unobserve_after_transaction(abi),
            "cleanup" => self.unobserve_transaction_cleanup(abi),
            other => {
                return Err(JsValue::from_str(&format!("unknown event: '{}'", other)).into());
            }
        };
        result.map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_TX))
    }

    /// Notify the parent document that you request to load data into this subdocument
    /// (if it is a subdocument).
    #[wasm_bindgen(js_name = load)]
    pub fn load(&self) -> Result<()> {
        let this = self.instance.borrow();
        match &this.parent_doc {
            Some(parent_doc) => parent_doc.transact(JsValue::UNDEFINED, |parent_txn| {
                let child_doc = &mut this.inner;
                child_doc.load(parent_txn.deref_mut().subdoc_scope());
                Ok(())
            }),
            None => Err(JsValue::from_str("not a subdocument").into()),
        }
    }

    /// Emit `onDestroy` event and unregister all event handlers.
    #[wasm_bindgen(js_name = destroy)]
    pub fn destroy(&self) -> Result<()> {
        let this = self.instance.borrow();
        match &this.parent_doc {
            Some(parent_doc) => parent_doc.transact(JsValue::UNDEFINED, |parent_txn| {
                let child_doc = &mut this.inner;
                child_doc.load(parent_txn.deref_mut().subdoc_scope());
                Ok(())
            }),
            None => Err(JsValue::from_str("not a subdocument").into()),
        }
    }

    /// Returns a list of sub-documents existings within the scope of this document.
    #[wasm_bindgen(js_name = getSubdocs)]
    pub fn subdocs(&self, txn: &ImplicitTransaction) -> Result<js_sys::Array> {
        match Transaction::from_implicit(&txn)? {
            Some(txn) => {
                let iter = txn.subdocs().map(|doc| {
                    let js: JsValue = Doc(doc.clone()).into();
                    js
                });
                Ok(js_sys::Array::from_iter(iter))
            }
            None => {
                let txn = self
                    .0
                    .try_transact()
                    .map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_RW_TX))?;
                let iter = txn.subdocs().map(|doc| {
                    let js: JsValue = Doc(doc.clone()).into();
                    js
                });
                Ok(js_sys::Array::from_iter(iter))
            }
        }
    }

    /// Returns a list of unique identifiers of the sub-documents existings within the scope of
    /// this document.
    #[wasm_bindgen(js_name = getSubdocGuids)]
    pub fn subdoc_guids(&self, txn: &ImplicitTransaction) -> Result<js_sys::Set> {
        let doc = &self.0;
        let guids = match Transaction::from_implicit(&txn)? {
            Some(txn) => {
                let values = txn.subdoc_guids().map(|id| JsValue::from_str(id.as_ref()));
                js_sys::Array::from_iter(values)
            }
            None => {
                let txn = doc
                    .try_transact()
                    .map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_RW_TX))?;
                let values = txn.subdoc_guids().map(|id| JsValue::from_str(id.as_ref()));
                js_sys::Array::from_iter(values)
            }
        };
        Ok(js_sys::Set::new(&guids))
    }

    /// Returns a list of all root-level replicated collections, together with their types.
    /// These collections can then be accessed via `getMap`/`getText` etc. methods.
    ///
    /// Example:
    /// ```js
    /// import * as Y from 'ywasm'
    ///
    /// const doc = new Y.YDoc()
    /// const ymap = doc.getMap('a')
    /// const yarray = doc.getArray('b')
    /// const ytext = doc.getText('c')
    /// const yxml = doc.getXmlFragment('d')
    ///
    /// const roots = doc.roots() // [['a',ymap], ['b',yarray], ['c',ytext], ['d',yxml]]
    /// ```
    #[wasm_bindgen(js_name = roots)]
    pub fn roots(&self, txn: &ImplicitTransaction) -> Result<JsValue> {
        let this = self.instance.borrow();
        let root_refs: Vec<_> = this.root_refs().collect();
    }

    /// Evaluates a JSON path expression (see: https://en.wikipedia.org/wiki/JSONPath) on
    /// the document and returns an array of values matching that query.
    ///
    /// Currently, this method supports the following syntax:
    /// - `$` - root object
    /// - `@` - current object
    /// - `.field` or `['field']` - member accessor
    /// - `[1]` - array index (also supports negative indices)
    /// - `.*` or `[*]` - wildcard (matches all members of an object or array)
    /// - `..` - recursive descent (matches all descendants not only direct children)
    /// - `[start:end:step]` - array slice operator (requires positive integer arguments)
    /// - `['a', 'b', 'c']` - union operator (returns an array of values for each query)
    /// - `[1, -1, 3]` - multiple indices operator (returns an array of values for each index)
    ///
    /// At the moment, JSON Path does not support filter predicates.
    #[wasm_bindgen(js_name = selectAll)]
    pub fn select_all(&self, json_path: &str) -> Result<js_sys::Array> {
        let txn = self.transaction(JsValue::UNDEFINED);
        txn.select_all(json_path)
    }

    /// Evaluates a JSON path expression (see: https://en.wikipedia.org/wiki/JSONPath) on
    /// the document and returns first value matching that query.
    ///
    /// Currently, this method supports the following syntax:
    /// - `$` - root object
    /// - `@` - current object
    /// - `.field` or `['field']` - member accessor
    /// - `[1]` - array index (also supports negative indices)
    /// - `.*` or `[*]` - wildcard (matches all members of an object or array)
    /// - `..` - recursive descent (matches all descendants not only direct children)
    /// - `[start:end:step]` - array slice operator (requires positive integer arguments)
    /// - `['a', 'b', 'c']` - union operator (returns an array of values for each query)
    /// - `[1, -1, 3]` - multiple indices operator (returns an array of values for each index)
    ///
    /// At the moment, JSON Path does not support filter predicates.
    #[wasm_bindgen(js_name = selectOne)]
    pub fn select_one(&self, json_path: &str) -> Result<JsValue> {
        let txn = self.transaction(JsValue::UNDEFINED);
        txn.select_one(json_path)
    }
}

#[wasm_bindgen]
pub struct YSubdocsEvent {
    added: js_sys::Array,
    removed: js_sys::Array,
    loaded: js_sys::Array,
}

#[wasm_bindgen]
impl YSubdocsEvent {
    fn new(e: &yrs::SubdocsEvent) -> Self {
        let added = js_sys::Array::from_iter(e.added().map(|doc| {
            let js: JsValue = Doc::from(doc.clone()).into();
            js
        }));
        let removed = js_sys::Array::from_iter(e.removed().map(|doc| {
            let js: JsValue = Doc::from(doc.clone()).into();
            js
        }));
        let loaded = js_sys::Array::from_iter(e.loaded().map(|doc| {
            let js: JsValue = Doc::from(doc.clone()).into();
            js
        }));
        YSubdocsEvent {
            added,
            removed,
            loaded,
        }
    }

    #[wasm_bindgen(getter)]
    pub fn added(&self) -> js_sys::Array {
        self.added.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn removed(&self) -> js_sys::Array {
        self.removed.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn loaded(&self) -> js_sys::Array {
        self.loaded.clone()
    }
}

#[derive(Deserialize)]
pub struct DocOptions {
    #[serde(alias = "clientID", default)]
    pub client_id: Option<u64>,

    #[serde(alias = "guid", default)]
    pub guid: Option<String>,

    #[serde(alias = "collectionid", default)]
    pub collection_id: Option<String>,

    #[serde(alias = "gc", default)]
    pub gc: Option<bool>,

    #[serde(alias = "autoLoad", default)]
    pub auto_load: Option<bool>,

    #[serde(alias = "shouldLoad", default)]
    pub should_load: Option<bool>,
}

impl DocOptions {
    fn fill(self, options: &mut Options) {
        if let Some(value) = self.client_id {
            options.client_id = value;
        }
        if let Some(value) = self.guid {
            options.guid = value.into();
        }
        if let Some(value) = self.collection_id {
            options.collection_id = Some(value.into());
        }
        if let Some(value) = self.gc {
            options.skip_gc = !value;
        }
        if let Some(value) = self.auto_load {
            options.auto_load = value;
        }
        if let Some(value) = self.should_load {
            options.should_load = value;
        }
    }
}
