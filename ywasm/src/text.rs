use crate::collection::SharedCollection;
use crate::js::{Callback, Js, ValueRef, YRange};
use crate::transaction::YTransaction;
use crate::weak::YWeakLink;
use crate::{ImplicitTransaction, Snapshot};
use gloo_utils::format::JsValueSerdeExt;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::types::text::TextEvent;
use yrs::types::{Attrs, TYPE_REFS_TEXT};
use yrs::{DeepObservable, GetString, Observable, Quotable, Text, TextRef, TransactionMut};

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
pub struct YText(pub(crate) SharedCollection<String, TextRef>);

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
        YText(SharedCollection::prelim(init.unwrap_or_default()))
    }

    #[wasm_bindgen(getter, js_name = type)]
    #[inline]
    pub fn get_type(&self) -> u8 {
        TYPE_REFS_TEXT
    }

    /// Gets unique logical identifier of this type, shared across peers collaborating on the same
    /// document.
    #[wasm_bindgen(getter, js_name = id)]
    #[inline]
    pub fn id(&self) -> crate::Result<JsValue> {
        self.0.id()
    }

    /// Returns true if this is a preliminary instance of `YArray`.
    ///
    /// Preliminary instances can be nested into other shared data types such as `YArray` and `YMap`.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[wasm_bindgen(getter)]
    #[inline]
    pub fn prelim(&self) -> bool {
        self.0.is_prelim()
    }

    /// Checks if current YArray reference is alive and has not been deleted by its parent collection.
    /// This method only works on already integrated shared types and will return false is current
    /// type is preliminary (has not been integrated into document).
    #[wasm_bindgen(js_name = alive)]
    #[inline]
    pub fn alive(&self, txn: &YTransaction) -> bool {
        self.0.is_alive(txn)
    }

    /// Returns length of an underlying string stored in this `YText` instance,
    /// understood as a number of UTF-8 encoded bytes.
    #[wasm_bindgen(js_name = length)]
    pub fn length(&self, txn: &ImplicitTransaction) -> crate::Result<u32> {
        match &self.0 {
            SharedCollection::Prelim(c) => Ok(c.len() as u32),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.len(txn))),
        }
    }

    /// Returns an underlying shared string stored in this data type.
    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &ImplicitTransaction) -> crate::Result<String> {
        match &self.0 {
            SharedCollection::Prelim(c) => Ok(c.clone()),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.get_string(txn))),
        }
    }

    /// Returns an underlying shared string stored in this data type.
    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &ImplicitTransaction) -> crate::Result<JsValue> {
        match &self.0 {
            SharedCollection::Prelim(c) => Ok(c.clone().into()),
            SharedCollection::Integrated(c) => {
                c.readonly(txn, |c, txn| Ok(c.get_string(txn).into()))
            }
        }
    }

    /// Inserts a given `chunk` of text into this `YText` instance, starting at a given `index`.
    ///
    /// Optional object with defined `attributes` will be used to wrap provided text `chunk`
    /// with a formatting blocks.`attributes` are only supported for a `YText` instance which
    /// already has been integrated into document store.
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
                    c.insert_str(index as usize, chunk);
                    Ok(())
                } else {
                    Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
                }
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                if attributes.is_undefined() || attributes.is_null() {
                    c.insert(txn, index, chunk);
                    Ok(())
                } else if let Some(attrs) = Self::parse_fmt(attributes) {
                    c.insert_with_attributes(txn, index, chunk, attrs);
                    Ok(())
                } else {
                    Err(JsValue::from_str(crate::js::errors::INVALID_FMT))
                }
            }),
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
                } else if let Some(attrs) = Self::parse_fmt(attributes) {
                    c.insert_embed_with_attributes(txn, index, Js::new(embed), attrs);
                    Ok(())
                } else {
                    Err(JsValue::from_str(crate::js::errors::INVALID_FMT))
                }
            }),
        }
    }

    /// Wraps an existing piece of text within a range described by `index`-`length` parameters with
    /// formatting blocks containing provided `attributes` metadata. This method only works for
    /// `YText` instances that already have been integrated into document store.
    #[wasm_bindgen(js_name = format)]
    pub fn format(
        &self,
        index: u32,
        length: u32,
        attributes: JsValue,
        txn: ImplicitTransaction,
    ) -> crate::Result<()> {
        let attrs = match Self::parse_fmt(attributes) {
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

    pub(crate) fn parse_fmt(attrs: JsValue) -> Option<Attrs> {
        if attrs.is_object() {
            let mut map = Attrs::new();
            let object = js_sys::Object::from(attrs);
            let entries = js_sys::Object::entries(&object);
            for tuple in entries.iter() {
                let tuple = js_sys::Array::from(&tuple);
                let key: String = tuple.get(0).as_string()?;
                let value = Js::new(tuple.get(1));
                if let Ok(ValueRef::Any(any)) = value.as_value() {
                    map.insert(key.into(), any);
                } else {
                    return None;
                }
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
    pub fn push(
        &mut self,
        chunk: &str,
        attributes: JsValue,
        txn: ImplicitTransaction,
    ) -> crate::Result<()> {
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                if attributes.is_undefined() || attributes.is_null() {
                    c.push_str(chunk);
                    Ok(())
                } else {
                    Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
                }
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                if attributes.is_undefined() || attributes.is_null() {
                    c.push(txn, chunk);
                    Ok(())
                } else if let Some(attrs) = Self::parse_fmt(attributes) {
                    let len = c.len(txn);
                    c.insert_with_attributes(txn, len, chunk, attrs);
                    Ok(())
                } else {
                    Err(JsValue::from_str(crate::js::errors::INVALID_FMT))
                }
            }),
        }
    }

    /// Deletes a specified range of characters, starting at a given `index`.
    /// Both `index` and `length` are counted in terms of a number of UTF-8 character bytes.
    #[wasm_bindgen(js_name = delete)]
    pub fn delete(
        &mut self,
        index: u32,
        length: u32,
        txn: ImplicitTransaction,
    ) -> crate::Result<()> {
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                c.drain((index as usize)..((index + length) as usize));
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.remove_range(txn, index, length);
                Ok(())
            }),
        }
    }

    #[wasm_bindgen(js_name = quote)]
    pub fn quote(
        &self,
        lower: Option<u32>,
        upper: Option<u32>,
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

    /// Returns the Delta representation of this YText type.
    #[wasm_bindgen(js_name = toDelta)]
    pub fn to_delta(
        &self,
        snapshot: JsValue,
        prev_snapshot: JsValue,
        compute_ychange: Option<js_sys::Function>,
        txn: ImplicitTransaction,
    ) -> crate::Result<js_sys::Array> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                let doc = txn.doc().clone();
                let hi: Option<Snapshot> = snapshot
                    .into_serde()
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;
                let lo: Option<Snapshot> = prev_snapshot
                    .into_serde()
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;
                let array = js_sys::Array::new();
                let delta = c.diff_range(txn, hi.as_deref(), lo.as_deref(), |change| {
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

    #[wasm_bindgen(js_name = applyDelta)]
    pub fn apply_delta(&self, delta: js_sys::Array, txn: ImplicitTransaction) -> crate::Result<()> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                let mut result = Vec::new();
                for js in delta.iter() {
                    let d = crate::js::convert::js_into_delta(js)?;
                    result.push(d);
                }
                c.apply_delta(txn, result);
                Ok(())
            }),
        }
    }

    /// Subscribes to all operations happening over this instance of `YText`. All changes are
    /// batched and eventually triggered during transaction commit phase.
    #[wasm_bindgen(js_name = observe)]
    pub fn observe(&self, callback: js_sys::Function) -> crate::Result<()> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => {
                let txn = c.transact()?;
                let array = c.resolve(&txn)?;
                let abi = callback.subscription_key();
                array.observe_with(abi, move |txn, e| {
                    let e = YTextEvent::new(e, txn);
                    let txn = YTransaction::from_ref(txn);
                    callback
                        .call2(&JsValue::UNDEFINED, &e.into(), &txn.into())
                        .unwrap();
                });
                Ok(())
            }
        }
    }

    /// Unsubscribes a callback previously subscribed with `observe` method.
    #[wasm_bindgen(js_name = unobserve)]
    pub fn unobserve(&mut self, callback: js_sys::Function) -> crate::Result<bool> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => {
                let txn = c.transact()?;
                let shared_ref = c.resolve(&txn)?;
                let abi = callback.subscription_key();
                Ok(shared_ref.unobserve(abi))
            }
        }
    }

    /// Subscribes to all operations happening over this Y shared type, as well as events in
    /// shared types stored within this one. All changes are batched and eventually triggered
    /// during transaction commit phase.
    #[wasm_bindgen(js_name = observeDeep)]
    pub fn observe_deep(&self, callback: js_sys::Function) -> crate::Result<()> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => {
                let txn = c.transact()?;
                let array = c.resolve(&txn)?;
                let abi = callback.subscription_key();
                array.observe_deep_with(abi, move |txn, e| {
                    let e = crate::js::convert::events_into_js(txn, e);
                    let txn = YTransaction::from_ref(txn);
                    callback
                        .call2(&JsValue::UNDEFINED, &e, &txn.into())
                        .unwrap();
                });
                Ok(())
            }
        }
    }

    /// Unsubscribes a callback previously subscribed with `observeDeep` method.
    #[wasm_bindgen(js_name = unobserveDeep)]
    pub fn unobserve_deep(&mut self, callback: js_sys::Function) -> crate::Result<bool> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => {
                let txn = c.transact()?;
                let shared_ref = c.resolve(&txn)?;
                let abi = callback.subscription_key();
                Ok(shared_ref.unobserve_deep(abi))
            }
        }
    }
}

/// Event generated by `YYText.observe` method. Emitted during transaction commit phase.
#[wasm_bindgen]
pub struct YTextEvent {
    inner: &'static TextEvent,
    txn: &'static TransactionMut<'static>,
    target: Option<JsValue>,
    delta: Option<JsValue>,
}

#[wasm_bindgen]
impl YTextEvent {
    pub(crate) fn new<'doc>(event: &TextEvent, txn: &TransactionMut<'doc>) -> Self {
        let inner: &'static TextEvent = unsafe { std::mem::transmute(event) };
        let txn: &'static TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        YTextEvent {
            inner,
            txn,
            target: None,
            delta: None,
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
            YText(SharedCollection::integrated(target.clone(), doc.clone())).into()
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
}
