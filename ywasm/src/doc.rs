use crate::array::YArray;
use crate::collection::SharedCollection;
use crate::js::Js;
use crate::map::YMap;
use crate::text::YText;
use crate::transaction::YTransaction;
use crate::xml_frag::YXmlFragment;
use crate::ImplicitTransaction;
use crate::Result;
use serde::Deserialize;
use std::iter::FromIterator;
use std::ops::Deref;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::types::TYPE_REFS_DOC;
use yrs::{Doc, OffsetKind, Options, ReadTxn, Transact};

/// A ywasm document type. Documents are most important units of collaborative resources management.
/// All shared collections live within a scope of their corresponding documents. All updates are
/// generated on per-document basis (rather than individual shared type). All operations on shared
/// collections happen via [YTransaction], which lifetime is also bound to a document.
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
#[repr(transparent)]
pub struct YDoc(pub(crate) Doc);

impl Deref for YDoc {
    type Target = Doc;

    #[inline]
    fn deref(&self) -> &Self::Target {
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
    pub fn new(options: &JsValue) -> Result<YDoc> {
        use gloo_utils::format::JsValueSerdeExt;
        let js_options = options
            .into_serde::<Option<DocOptions>>()
            .map_err(|_| JsValue::from_str("invalid document options"))?;
        let mut options = Options::default();
        options.offset_kind = OffsetKind::Utf16;
        if let Some(o) = js_options {
            o.fill(&mut options);
        }

        Ok(Doc::with_options(options).into())
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
        self.0.parent_doc().is_none()
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
        self.client_id() as f64
    }

    /// Gets globally unique identifier of this `YDoc` instance.
    #[wasm_bindgen(getter)]
    pub fn guid(&self) -> String {
        self.options().guid.to_string()
    }

    #[wasm_bindgen(getter, js_name = shouldLoad)]
    pub fn should_load(&self) -> bool {
        self.options().should_load
    }

    #[wasm_bindgen(getter, js_name = autoLoad)]
    pub fn auto_load(&self) -> bool {
        self.options().auto_load
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
    ///     const txn = this.transaction()
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
    pub fn transaction(&self, origin: JsValue) -> YTransaction {
        if origin.is_undefined() {
            YTransaction::from(self.transact_mut())
        } else {
            YTransaction::from(self.transact_mut_with(Js::from(origin)))
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
    pub fn get_text(&self, name: &str) -> YText {
        let shared_ref = self.get_or_insert_text(name);
        YText(SharedCollection::integrated(shared_ref, self.0.clone()))
    }

    /// Returns a `YArray` shared data type, that's accessible for subsequent accesses using given
    /// `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YArray` instance.
    #[wasm_bindgen(js_name = getArray)]
    pub fn get_array(&self, name: &str) -> YArray {
        let shared_ref = self.get_or_insert_array(name);
        YArray(SharedCollection::integrated(shared_ref, self.0.clone()))
    }

    /// Returns a `YMap` shared data type, that's accessible for subsequent accesses using given
    /// `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YMap` instance.
    #[wasm_bindgen(js_name = getMap)]
    pub fn get_map(&self, name: &str) -> YMap {
        let shared_ref = self.get_or_insert_map(name);
        YMap(SharedCollection::integrated(shared_ref, self.0.clone()))
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
        YXmlFragment(SharedCollection::integrated(shared_ref, self.0.clone()))
    }

    /// Subscribes given function to be called any time, a remote update is being applied to this
    /// document. Function takes an `Uint8Array` as a parameter which contains a lib0 v1 encoded
    /// update.
    ///
    /// Returns an observer, which can be freed in order to unsubscribe this callback.
    #[wasm_bindgen(js_name = onUpdate)]
    pub fn on_update(&self, f: js_sys::Function) -> Result<crate::Observer> {
        let subscription = self
            .observe_update_v1(move |txn, e| {
                let update = js_sys::Uint8Array::from(e.update.as_slice());
                let txn: JsValue = YTransaction::from_ref(txn).into();
                f.call2(&JsValue::UNDEFINED, &update, &txn).unwrap();
            })
            .map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_TX))?;
        Ok(subscription.into())
    }

    /// Subscribes given function to be called any time, a remote update is being applied to this
    /// document. Function takes an `Uint8Array` as a parameter which contains a lib0 v2 encoded
    /// update.
    ///
    /// Returns an observer, which can be freed in order to unsubscribe this callback.
    #[wasm_bindgen(js_name = onUpdateV2)]
    pub fn on_update_v2(&self, f: js_sys::Function) -> Result<crate::Observer> {
        let subscription = self
            .observe_update_v2(move |txn, e| {
                let update = js_sys::Uint8Array::from(e.update.as_slice());
                let txn: JsValue = YTransaction::from_ref(txn).into();
                f.call2(&JsValue::UNDEFINED, &update, &txn).unwrap();
            })
            .map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_TX))?;
        Ok(subscription.into())
    }

    /// Subscribes given function to be called, whenever a transaction created by this document is
    /// being committed.
    ///
    /// Returns an observer, which can be freed in order to unsubscribe this callback.
    #[wasm_bindgen(js_name = onAfterTransaction)]
    pub fn on_after_transaction(&self, f: js_sys::Function) -> Result<crate::Observer> {
        let subscription = self
            .observe_transaction_cleanup(move |txn, _| {
                let txn = YTransaction::from_ref(txn).into();
                f.call1(&JsValue::UNDEFINED, &txn).unwrap();
            })
            .map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_TX))?;
        Ok(subscription.into())
    }

    /// Subscribes given function to be called, whenever a subdocuments are being added, removed
    /// or loaded as children of a current document.
    ///
    /// Returns an observer, which can be freed in order to unsubscribe this callback.
    #[wasm_bindgen(js_name = onSubdocs)]
    pub fn on_subdocs(&self, f: js_sys::Function) -> Result<crate::Observer> {
        let subscription = self
            .observe_subdocs(move |txn, e| {
                let event: JsValue = YSubdocsEvent::new(e).into();
                let txn: JsValue = YTransaction::from_ref(txn).into();
                f.call2(&JsValue::UNDEFINED, &event, &txn).unwrap();
            })
            .map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_TX))?;
        Ok(subscription.into())
    }

    /// Subscribes given function to be called, whenever current document is being destroyed.
    ///
    /// Returns an observer, which can be freed in order to unsubscribe this callback.
    #[wasm_bindgen(js_name = onDestroy)]
    pub fn on_destroy(&self, f: js_sys::Function) -> Result<crate::Observer> {
        let subscription = self
            .observe_destroy(move |txn, e| {
                let event: JsValue = YDoc::from(e.clone()).into();
                let txn: JsValue = YTransaction::from_ref(txn).into();
                f.call2(&JsValue::UNDEFINED, &event, &txn).unwrap();
            })
            .map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_TX))?;
        Ok(subscription.into())
    }

    /// Notify the parent document that you request to load data into this subdocument
    /// (if it is a subdocument).
    #[wasm_bindgen(js_name = load)]
    pub fn load(&self, parent_txn: &ImplicitTransaction) -> Result<()> {
        match YTransaction::from_implicit_mut(parent_txn)? {
            Some(mut parent_txn) => {
                self.0.load(parent_txn.as_mut()?);
            }
            None => {
                let parent_doc = if let Some(parent_doc) = self.0.parent_doc() {
                    parent_doc
                } else {
                    return Ok(());
                };
                let mut parent_txn = parent_doc.transact_mut();
                self.0.load(&mut parent_txn);
            }
        }
        Ok(())
    }

    /// Emit `onDestroy` event and unregister all event handlers.
    #[wasm_bindgen(js_name = destroy)]
    pub fn destroy(&self, parent_txn: &ImplicitTransaction) -> Result<()> {
        match YTransaction::from_implicit_mut(parent_txn)? {
            Some(mut parent_txn) => {
                self.0.destroy(parent_txn.as_mut()?);
            }
            None => {
                let parent_doc = if let Some(parent_doc) = self.0.parent_doc() {
                    parent_doc
                } else {
                    return Ok(());
                };
                let mut parent_txn = parent_doc.transact_mut();
                self.0.destroy(&mut parent_txn);
            }
        }
        Ok(())
    }

    /// Returns a list of sub-documents existings within the scope of this document.
    #[wasm_bindgen(js_name = getSubdocs)]
    pub fn subdocs(&self, txn: &ImplicitTransaction) -> Result<js_sys::Array> {
        match YTransaction::from_implicit(&txn)? {
            Some(txn) => {
                let iter = txn.subdocs().map(|doc| {
                    let js: JsValue = YDoc(doc.clone()).into();
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
                    let js: JsValue = YDoc(doc.clone()).into();
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
        let guids = match YTransaction::from_implicit(&txn)? {
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
    pub fn roots(&self, txn: &ImplicitTransaction) -> Result<js_sys::Array> {
        let doc = &self.0;
        match YTransaction::from_implicit(&txn)? {
            Some(txn) => {
                let values = txn.root_refs().map(|(k, v)| {
                    js_sys::Array::from_iter([JsValue::from_str(k), Js::from_value(&v, doc).into()])
                });
                Ok(js_sys::Array::from_iter(values))
            }
            None => {
                let txn = doc
                    .try_transact()
                    .map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_RW_TX))?;
                let values = txn.root_refs().map(|(k, v)| {
                    js_sys::Array::from_iter([JsValue::from_str(k), Js::from_value(&v, doc).into()])
                });
                Ok(js_sys::Array::from_iter(values))
            }
        }
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
            let js: JsValue = YDoc::from(doc.clone()).into();
            js
        }));
        let removed = js_sys::Array::from_iter(e.removed().map(|doc| {
            let js: JsValue = YDoc::from(doc.clone()).into();
            js
        }));
        let loaded = js_sys::Array::from_iter(e.loaded().map(|doc| {
            let js: JsValue = YDoc::from(doc.clone()).into();
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
            options.collection_id = Some(value);
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
