use crate::collection::SharedCollection;
use crate::js::{Callback, Js, ValueRef, YRange};
use crate::transaction::{ImplicitTransaction, YTransaction};
use crate::weak::YWeakLink;
use crate::Result;
use gloo_utils::format::JsValueSerdeExt;
use std::iter::FromIterator;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::types::array::ArrayEvent;
use yrs::types::{ToJson, TYPE_REFS_ARRAY};
use yrs::{Array, ArrayRef, DeepObservable, Observable, Quotable, SharedRef, TransactionMut};

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
#[repr(transparent)]
pub struct YArray(pub(crate) SharedCollection<Vec<JsValue>, ArrayRef>);

#[wasm_bindgen]
impl YArray {
    /// Creates a new preliminary instance of a `YArray` shared data type, with its state
    /// initialized to provided parameter.
    ///
    /// Preliminary instances can be nested into other shared data types such as `YArray` and `YMap`.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[wasm_bindgen(constructor)]
    pub fn new(items: Option<Vec<JsValue>>) -> Self {
        YArray(SharedCollection::prelim(items.unwrap_or_default()))
    }

    #[wasm_bindgen(getter, js_name = type)]
    #[inline]
    pub fn get_type(&self) -> u8 {
        TYPE_REFS_ARRAY
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

    /// Returns a number of elements stored within this instance of `YArray`.
    #[wasm_bindgen(js_name = length)]
    pub fn length(&self, txn: &ImplicitTransaction) -> Result<u32> {
        match &self.0 {
            SharedCollection::Prelim(c) => Ok(c.len() as u32),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.len(txn))),
        }
    }

    /// Converts an underlying contents of this `YArray` instance into their JSON representation.
    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &ImplicitTransaction) -> Result<JsValue> {
        match &self.0 {
            SharedCollection::Prelim(c) => {
                let a = js_sys::Array::new();
                for js in c.iter() {
                    a.push(js);
                }
                Ok(a.into())
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let any = c.to_json(txn);
                JsValue::from_serde(&any).map_err(|e| JsValue::from_str(&e.to_string()))
            }),
        }
    }

    /// Inserts a given range of `items` into this `YArray` instance, starting at given `index`.
    #[wasm_bindgen(js_name = insert)]
    pub fn insert(
        &mut self,
        index: u32,
        items: Vec<JsValue>,
        txn: ImplicitTransaction,
    ) -> Result<()> {
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                c.reserve(items.len());
                let mut i = index as usize;
                for item in items {
                    c.insert(i, item);
                    i += 1;
                }
                Ok(())
            }
            SharedCollection::Integrated(c) => {
                c.mutably(txn, |c, txn| c.insert_at(txn, index, items))
            }
        }
    }

    /// Appends a range of `items` at the end of this `YArray` instance.
    #[wasm_bindgen(js_name = push)]
    pub fn push(&mut self, items: Vec<JsValue>, txn: ImplicitTransaction) -> Result<()> {
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                c.extend_from_slice(&items);
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                let len = c.len(txn);
                c.insert_at(txn, len, items)
            }),
        }
    }

    /// Deletes a range of items of given `length` from current `YArray` instance,
    /// starting from given `index`.
    #[wasm_bindgen(js_name = "delete")]
    pub fn delete(
        &mut self,
        index: u32,
        length: Option<u32>,
        txn: ImplicitTransaction,
    ) -> Result<()> {
        let length = length.unwrap_or(1);
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                let start = index as usize;
                let end = start + length as usize;
                for _ in c.drain(start..end) { /* just drain */ }
                Ok(())
            }
            SharedCollection::Integrated(c) => {
                c.mutably(txn, |c, txn| Ok(c.remove_range(txn, index, length)))
            }
        }
    }

    /// Moves element found at `source` index into `target` index position.
    #[wasm_bindgen(js_name = move)]
    pub fn move_content(
        &mut self,
        source: u32,
        target: u32,
        txn: ImplicitTransaction,
    ) -> Result<()> {
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                let index = if target > source { target - 1 } else { target };
                let moved = c.remove(source as usize);
                c.insert(index as usize, moved);
                Ok(())
            }
            SharedCollection::Integrated(c) => {
                c.mutably(txn, |c, txn| Ok(c.move_to(txn, source, target)))
            }
        }
    }

    /// Returns an element stored under given `index`.
    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, index: u32, txn: &ImplicitTransaction) -> Result<JsValue> {
        match &self.0 {
            SharedCollection::Prelim(c) => match c.get(index as usize) {
                Some(item) => Ok(item.clone()),
                None => Err(JsValue::from_str(crate::js::errors::OUT_OF_BOUNDS)),
            },
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| match c.get(txn, index) {
                Some(item) => Ok(Js::from_value(&item, txn.doc()).into()),
                None => Err(JsValue::from_str(crate::js::errors::OUT_OF_BOUNDS)),
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
    ) -> Result<YWeakLink> {
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
    pub fn values(&self, txn: &ImplicitTransaction) -> Result<JsValue> {
        match &self.0 {
            SharedCollection::Prelim(c) => Ok(js_sys::Array::from_iter(c).into()),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let a = js_sys::Array::new();
                let doc = txn.doc();
                for item in c.iter(txn) {
                    a.push(&Js::from_value(&item, doc));
                }
                Ok(a.into())
            }),
        }
    }

    /// Subscribes to all operations happening over this instance of `YArray`. All changes are
    /// batched and eventually triggered during transaction commit phase.
    #[wasm_bindgen(js_name = observe)]
    pub fn observe(&self, callback: js_sys::Function) -> Result<()> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => {
                let txn = c.transact()?;
                let array = c.resolve(&txn)?;
                let abi = callback.subscription_key();
                array.observe_with(abi, move |txn, e| {
                    let e = YArrayEvent::new(e, txn);
                    let txn = YTransaction::from_ref(txn);
                    callback
                        .call2(&JsValue::UNDEFINED, &e.into(), &txn.into())
                        .unwrap();
                });
                Ok(())
            }
        }
    }

    #[wasm_bindgen(js_name = unobserve)]
    pub fn unobserve(&self, callback: js_sys::Function) -> Result<bool> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => {
                let txn = c.transact()?;
                let array = c.resolve(&txn)?;
                let abi = callback.subscription_key();
                Ok(array.unobserve(abi))
            }
        }
    }

    /// Subscribes to all operations happening over this Y shared type, as well as events in
    /// shared types stored within this one. All changes are batched and eventually triggered
    /// during transaction commit phase.
    #[wasm_bindgen(js_name = observeDeep)]
    pub fn observe_deep(&self, callback: js_sys::Function) -> Result<()> {
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

    #[wasm_bindgen(js_name = unobserveDeep)]
    pub fn unobserve_deep(&self, callback: js_sys::Function) -> Result<bool> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => {
                let txn = c.transact()?;
                let array = c.resolve(&txn)?;
                let abi = callback.subscription_key();
                Ok(array.unobserve_deep(abi))
            }
        }
    }
}

pub(crate) trait ArrayExt: Array + SharedRef {
    fn insert_at<I>(&self, txn: &mut TransactionMut, index: u32, src: I) -> Result<()>
    where
        I: IntoIterator<Item = JsValue>,
    {
        let mut primitive = Vec::default();
        let mut i = 0;
        let mut j = index;
        for value in src {
            let js = Js::from(value);
            match js.as_value()? {
                ValueRef::Any(any) => primitive.push(any),
                ValueRef::Shared(shared) => {
                    if shared.prelim() {
                        let len = primitive.len() as u32;
                        if len > 0 {
                            self.insert_range(txn, j, std::mem::take(&mut primitive));
                            j += len;
                        }
                        self.insert(txn, j, shared);
                        j += 1;
                    } else {
                        let err = format!("cannot insert item at index {}: shared collection is not a preliminary type", i);
                        return Err(JsValue::from(&err));
                    }
                }
            }
            i += 1;
        }
        if !primitive.is_empty() {
            self.insert_range(txn, j, primitive);
        }
        Ok(())
    }
}

impl ArrayExt for ArrayRef {}

/// Event generated by `YArray.observe` method. Emitted during transaction commit phase.
#[wasm_bindgen]
pub struct YArrayEvent {
    inner: &'static ArrayEvent,
    txn: &'static TransactionMut<'static>,
    target: Option<JsValue>,
    delta: Option<JsValue>,
}

#[wasm_bindgen]
impl YArrayEvent {
    pub(crate) fn new<'doc>(event: &ArrayEvent, txn: &TransactionMut<'doc>) -> Self {
        let inner: &'static ArrayEvent = unsafe { std::mem::transmute(event) };
        let txn: &'static TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        YArrayEvent {
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
            YArray(SharedCollection::integrated(target.clone(), doc.clone())).into()
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

    /// Returns a list of text changes made over corresponding `YArray` collection within
    /// bounds of current transaction. These changes follow a format:
    ///
    /// - { insert: any[] }
    /// - { delete: number }
    /// - { retain: number }
    #[wasm_bindgen(getter)]
    pub fn delta(&mut self) -> JsValue {
        let inner = &self.inner;
        let txn = &self.txn;
        let js = self.delta.get_or_insert_with(|| {
            let delta = inner
                .delta(txn)
                .into_iter()
                .map(|change| crate::js::convert::change_into_js(change, txn.doc()));
            let mut result = js_sys::Array::new();
            result.extend(delta);
            result.into()
        });
        js.clone()
    }
}
