use crate::collection::SharedCollection;
use crate::js::{Js, YRange};
use crate::text::YText;
use crate::transaction::YTransaction;
use crate::weak::YWeakLink;
use crate::xml_elem::YXmlElement;
use crate::{ImplicitTransaction, YSnapshot};
use gloo_utils::format::JsValueSerdeExt;
use std::collections::HashMap;
use wasm_bindgen::convert::IntoWasmAbi;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::types::xml::XmlTextEvent;
use yrs::types::TYPE_REFS_XML_TEXT;
use yrs::{DeepObservable, GetString, Observable, Quotable, Text, TransactionMut, Xml, XmlTextRef};

pub(crate) struct PrelimXmlText {
    pub attributes: HashMap<String, String>,
    pub text: String,
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
pub struct YXmlText(pub(crate) SharedCollection<PrelimXmlText, XmlTextRef>);

#[wasm_bindgen]
impl YXmlText {
    #[wasm_bindgen(constructor)]
    pub fn new(text: Option<String>, attributes: JsValue) -> crate::Result<YXmlText> {
        let attributes = YXmlElement::parse_attrs(attributes)?;
        Ok(YXmlText(SharedCollection::prelim(PrelimXmlText {
            text: text.unwrap_or_default(),
            attributes,
        })))
    }

    #[wasm_bindgen(getter, js_name = type)]
    #[inline]
    pub fn get_type(&self) -> u8 {
        TYPE_REFS_XML_TEXT
    }

    /// Gets unique logical identifier of this type, shared across peers collaborating on the same
    /// document.
    #[wasm_bindgen(getter, js_name = id)]
    #[inline]
    pub fn id(&self) -> crate::Result<JsValue> {
        self.0.id()
    }

    /// Returns true if this is a preliminary instance of `YXmlText`.
    ///
    /// Preliminary instances can be nested into other shared data types.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[wasm_bindgen(getter)]
    #[inline]
    pub fn prelim(&self) -> bool {
        self.0.is_prelim()
    }

    /// Checks if current shared type reference is alive and has not been deleted by its parent collection.
    /// This method only works on already integrated shared types and will return false is current
    /// type is preliminary (has not been integrated into document).
    #[wasm_bindgen(js_name = alive)]
    #[inline]
    pub fn alive(&self, txn: &YTransaction) -> bool {
        self.0.is_alive(txn)
    }

    /// Returns length of an underlying string stored in this `YXmlText` instance,
    /// understood as a number of UTF-8 encoded bytes.
    #[wasm_bindgen]
    pub fn length(&self, txn: &ImplicitTransaction) -> crate::Result<u32> {
        match &self.0 {
            SharedCollection::Prelim(c) => Ok(c.text.len() as u32),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.len(txn))),
        }
    }

    /// Inserts a given `chunk` of text into this `YXmlText` instance, starting at a given `index`.
    ///
    /// Optional object with defined `attributes` will be used to wrap provided text `chunk`
    /// with a formatting blocks.
    #[wasm_bindgen(js_name = insert)]
    pub fn insert(
        &mut self,
        index: u32,
        chunk: &str,
        attributes: JsValue,
        txn: ImplicitTransaction,
    ) -> crate::Result<()> {
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                if attributes.is_undefined() || attributes.is_null() {
                    c.text.insert_str(index as usize, chunk);
                    Ok(())
                } else {
                    Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
                }
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                if attributes.is_undefined() || attributes.is_null() {
                    c.insert(txn, index, chunk);
                    Ok(())
                } else if let Some(attrs) = YText::parse_fmt(attributes) {
                    c.insert_with_attributes(txn, index, chunk, attrs);
                    Ok(())
                } else {
                    Err(JsValue::from_str(crate::js::errors::INVALID_FMT))
                }
            }),
        }
    }

    /// Formats text within bounds specified by `index` and `len` with a given formatting
    /// attributes.
    #[wasm_bindgen(js_name = format)]
    pub fn format(
        &self,
        index: u32,
        length: u32,
        attributes: JsValue,
        txn: ImplicitTransaction,
    ) -> crate::Result<()> {
        let attrs = match YText::parse_fmt(attributes) {
            Some(attrs) => attrs,
            None => return Err(JsValue::from_str(crate::js::errors::INVALID_FMT)),
        };
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.format(txn, index, length, attrs);
                Ok(())
            }),
        }
    }

    #[wasm_bindgen(js_name = quote)]
    pub fn quote(
        &self,
        lower: u32,
        upper: u32,
        lower_open: Option<bool>,
        upper_open: Option<bool>,
        txn: &ImplicitTransaction,
    ) -> crate::Result<YWeakLink> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let range = YRange::new(lower, upper, lower_open, upper_open);
                let quote = c
                    .quote(txn, range)
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;
                Ok(YWeakLink::from_prelim(quote, txn.doc().clone()))
            }),
        }
    }

    /// Returns the Delta representation of this YXmlText type.
    #[wasm_bindgen(js_name = toDelta)]
    pub fn to_delta(
        &self,
        snapshot: Option<YSnapshot>,
        prev_snapshot: Option<YSnapshot>,
        compute_ychange: Option<js_sys::Function>,
        txn: ImplicitTransaction,
    ) -> crate::Result<js_sys::Array> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                let doc = txn.doc().clone();
                let hi = snapshot.map(|s| s.0);
                let lo = prev_snapshot.map(|s| s.0);
                let array = js_sys::Array::new();
                let delta = c.diff_range(txn, hi.as_ref(), lo.as_ref(), |change| {
                    crate::js::convert::ychange_to_js(change, &compute_ychange).unwrap()
                });
                for d in delta {
                    let d = crate::js::convert::diff_into_js(d, &doc)?;
                    array.push(&d);
                }
                Ok(array)
            }),
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
        txn: ImplicitTransaction,
    ) -> crate::Result<()> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                if attributes.is_undefined() || attributes.is_null() {
                    c.insert_embed(txn, index, Js::new(embed));
                    Ok(())
                } else if let Some(attrs) = YText::parse_fmt(attributes) {
                    c.insert_embed_with_attributes(txn, index, Js::new(embed), attrs);
                    Ok(())
                } else {
                    Err(JsValue::from_str(crate::js::errors::INVALID_FMT))
                }
            }),
        }
    }

    /// Appends a given `chunk` of text at the end of `YXmlText` instance.
    ///
    /// Optional object with defined `attributes` will be used to wrap provided text `chunk`
    /// with a formatting blocks.
    #[wasm_bindgen(js_name = push)]
    pub fn push(
        &mut self,
        chunk: &str,
        attributes: JsValue,
        txn: ImplicitTransaction,
    ) -> crate::Result<()> {
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                if attributes.is_undefined() || attributes.is_null() {
                    c.text.push_str(chunk);
                    Ok(())
                } else {
                    Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
                }
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                if attributes.is_undefined() || attributes.is_null() {
                    c.push(txn, chunk);
                    Ok(())
                } else if let Some(attrs) = YText::parse_fmt(attributes) {
                    let len = c.len(txn);
                    c.insert_with_attributes(txn, len, chunk, attrs);
                    Ok(())
                } else {
                    Err(JsValue::from_str(crate::js::errors::INVALID_FMT))
                }
            }),
        }
    }

    /// Deletes a specified range of of characters, starting at a given `index`.
    /// Both `index` and `length` are counted in terms of a number of UTF-8 character bytes.
    #[wasm_bindgen(method, js_name = delete)]
    pub fn delete(
        &mut self,
        index: u32,
        length: u32,
        txn: ImplicitTransaction,
    ) -> crate::Result<()> {
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                c.text.drain((index as usize)..((index + length) as usize));
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.remove_range(txn, index, length);
                Ok(())
            }),
        }
    }

    /// Returns a next XML sibling node of this XMl node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node is a last child of
    /// parent XML node.
    #[wasm_bindgen(js_name = nextSibling)]
    pub fn next_sibling(&self, txn: &ImplicitTransaction) -> crate::Result<JsValue> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let next = c.siblings(txn).next();
                match next {
                    Some(node) => Ok(Js::from_xml(node, txn.doc().clone()).into()),
                    None => Ok(JsValue::UNDEFINED),
                }
            }),
        }
    }

    /// Returns a previous XML sibling node of this XMl node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node is a first child
    /// of parent XML node.
    #[wasm_bindgen(js_name = prevSibling)]
    pub fn prev_sibling(&self, txn: &ImplicitTransaction) -> crate::Result<JsValue> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let next = c.siblings(txn).next_back();
                match next {
                    Some(node) => Ok(Js::from_xml(node, txn.doc().clone()).into()),
                    None => Ok(JsValue::UNDEFINED),
                }
            }),
        }
    }

    /// Returns a parent `YXmlElement` node or `undefined` if current node has no parent assigned.
    #[wasm_bindgen(js_name = parent)]
    pub fn parent(&self, txn: &ImplicitTransaction) -> crate::Result<JsValue> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| match c.parent() {
                None => Ok(JsValue::UNDEFINED),
                Some(node) => Ok(Js::from_xml(node, txn.doc().clone()).into()),
            }),
        }
    }

    /// Returns an underlying string stored in this `YXmlText` instance.
    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &ImplicitTransaction) -> crate::Result<String> {
        match &self.0 {
            SharedCollection::Prelim(c) => Ok(c.text.to_string()),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.get_string(txn))),
        }
    }

    /// Sets a `name` and `value` as new attribute for this XML node. If an attribute with the same
    /// `name` already existed on that node, its value with be overridden with a provided one.
    #[wasm_bindgen(js_name = setAttribute)]
    pub fn set_attribute(
        &mut self,
        name: &str,
        value: &str,
        txn: ImplicitTransaction,
    ) -> crate::Result<()> {
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                c.attributes.insert(name.to_string(), value.to_string());
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.insert_attribute(txn, name, value);
                Ok(())
            }),
        }
    }

    /// Returns a value of an attribute given its `name`. If no attribute with such name existed,
    /// `null` will be returned.
    #[wasm_bindgen(js_name = getAttribute)]
    pub fn get_attribute(&self, name: &str, txn: &ImplicitTransaction) -> crate::Result<JsValue> {
        let value = match &self.0 {
            SharedCollection::Integrated(c) => {
                c.readonly(txn, |c, txn| Ok(c.get_attribute(txn, name)))?
            }
            SharedCollection::Prelim(c) => c.attributes.get(name).cloned(),
        };
        match value {
            None => Ok(JsValue::UNDEFINED),
            Some(value) => Ok(JsValue::from_str(&value)),
        }
    }

    /// Removes an attribute from this XML node, given its `name`.
    #[wasm_bindgen(js_name = removeAttribute)]
    pub fn remove_attribute(
        &mut self,
        name: String,
        txn: ImplicitTransaction,
    ) -> crate::Result<()> {
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                c.attributes.remove(&name);
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.remove_attribute(txn, &name);
                Ok(())
            }),
        }
    }

    /// Returns an iterator that enables to traverse over all attributes of this XML node in
    /// unspecified order.
    #[wasm_bindgen(js_name = attributes)]
    pub fn attributes(&self, txn: &ImplicitTransaction) -> crate::Result<JsValue> {
        match &self.0 {
            SharedCollection::Prelim(c) => Ok(JsValue::from_serde(&c.attributes)
                .map_err(|_| JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))?),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let map = js_sys::Object::new();
                for (name, value) in c.attributes(txn) {
                    js_sys::Reflect::set(
                        &map,
                        &JsValue::from_str(name),
                        &JsValue::from_str(&value),
                    )?;
                }
                Ok(map.into())
            }),
        }
    }

    /// Subscribes to all operations happening over this instance of `YXmlText`. All changes are
    /// batched and eventually triggered during transaction commit phase.
    /// Returns an `YObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observe)]
    pub fn observe(&mut self, f: js_sys::Function) -> crate::Result<()> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => {
                let txn = c.transact()?;
                let array = c.resolve(&txn)?;
                let abi = f.clone().into_abi();
                array.observe_with(abi, move |txn, e| {
                    let e = YXmlTextEvent::new(e, txn);
                    let txn = YTransaction::from_ref(txn);
                    f.call2(&JsValue::UNDEFINED, &e.into(), &txn.into())
                        .unwrap();
                });
                Ok(())
            }
        }
    }

    /// Unsubscribes a callback previously subscribed with `observe` method.
    #[wasm_bindgen(js_name = unobserve)]
    pub fn unobserve(&mut self, callback: js_sys::Function) -> crate::Result<()> {
        if let SharedCollection::Integrated(c) = &self.0 {
            let txn = c.transact()?;
            let shared_ref = c.resolve(&txn)?;
            shared_ref.unobserve(callback.into_abi());
        }
        Ok(())
    }

    /// Subscribes to all operations happening over this Y shared type, as well as events in
    /// shared types stored within this one. All changes are batched and eventually triggered
    /// during transaction commit phase.
    /// Returns an `YEventObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observeDeep)]
    pub fn observe_deep(&mut self, f: js_sys::Function) -> crate::Result<()> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => {
                let txn = c.transact()?;
                let array = c.resolve(&txn)?;
                let abi = f.clone().into_abi();
                array.observe_deep_with(abi, move |txn, e| {
                    let e = crate::js::convert::events_into_js(txn, e);
                    let txn = YTransaction::from_ref(txn);
                    f.call2(&JsValue::UNDEFINED, &e, &txn.into()).unwrap();
                });
                Ok(())
            }
        }
    }

    /// Unsubscribes a callback previously subscribed with `observe` method.
    #[wasm_bindgen(js_name = unobserveDeep)]
    pub fn unobserve_deep(&mut self, callback: js_sys::Function) -> crate::Result<()> {
        if let SharedCollection::Integrated(c) = &self.0 {
            let txn = c.transact()?;
            let shared_ref = c.resolve(&txn)?;
            shared_ref.unobserve_deep(callback.into_abi());
        }
        Ok(())
    }
}

/// Event generated by `YXmlText.observe` method. Emitted during transaction commit phase.
#[wasm_bindgen]
pub struct YXmlTextEvent {
    inner: &'static XmlTextEvent,
    txn: &'static TransactionMut<'static>,
    target: Option<JsValue>,
    delta: Option<JsValue>,
    keys: Option<JsValue>,
}

#[wasm_bindgen]
impl YXmlTextEvent {
    pub(crate) fn new<'doc>(event: &XmlTextEvent, txn: &TransactionMut<'doc>) -> Self {
        let inner: &'static XmlTextEvent = unsafe { std::mem::transmute(event) };
        let txn: &'static TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        YXmlTextEvent {
            inner,
            txn,
            target: None,
            delta: None,
            keys: None,
        }
    }

    /// Returns an array of keys and indexes creating a path from root type down to current instance
    /// of shared type (accessible via `target` getter).
    #[wasm_bindgen]
    pub fn path(&self) -> JsValue {
        crate::js::convert::path_into_js(self.inner.path())
    }

    /// Returns a current shared type instance, that current event changes refer to.
    #[wasm_bindgen(getter)]
    pub fn target(&mut self) -> JsValue {
        let target = self.inner.target();
        let doc = self.txn.doc();
        let js = self.target.get_or_insert_with(|| {
            YXmlText(SharedCollection::integrated(target.clone(), doc.clone())).into()
        });
        js.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn origin(&mut self) -> JsValue {
        let origin = self.txn.origin();
        if let Some(origin) = origin {
            Js::from(origin).into()
        } else {
            JsValue::UNDEFINED
        }
    }

    /// Returns a list of text changes made over corresponding `YText` collection within
    /// bounds of current transaction. These changes follow a format:
    ///
    /// - { insert: string, attributes: any|undefined }
    /// - { delete: number }
    /// - { retain: number, attributes: any|undefined }
    #[wasm_bindgen(getter)]
    pub fn delta(&mut self) -> crate::Result<JsValue> {
        if let Some(delta) = &self.delta {
            Ok(delta.clone())
        } else {
            let result = js_sys::Array::new();
            let txn = self.txn;
            for d in self.inner.delta(txn) {
                let delta = crate::js::convert::text_delta_into_js(d, txn.doc())?;
                result.push(&delta);
            }
            let delta: JsValue = result.into();
            self.delta = Some(delta.clone());
            Ok(delta)
        }
    }

    /// Returns a list of attribute changes made over corresponding `YXmlText` collection within
    /// bounds of current transaction. These changes follow a format:
    ///
    /// - { action: 'add'|'update'|'delete', oldValue: string|undefined, newValue: string|undefined }
    #[wasm_bindgen(getter)]
    pub fn keys(&mut self) -> crate::Result<JsValue> {
        if let Some(keys) = &self.keys {
            Ok(keys.clone())
        } else {
            let txn = self.txn;
            let keys = self.inner.keys(txn);
            let result = js_sys::Object::new();
            for (key, value) in keys.iter() {
                let key = JsValue::from(key.as_ref());
                let value = crate::js::convert::entry_change_into_js(value, txn.doc())?;
                js_sys::Reflect::set(&result, &key, &value).unwrap();
            }
            let keys: JsValue = result.into();
            self.keys = Some(keys.clone());
            Ok(keys)
        }
    }
}
