use crate::collection::SharedCollection;
use crate::js::Js;
use crate::transaction::YTransaction;
use crate::weak::YWeakLink;
use crate::{js, ImplicitTransaction};
use gloo_utils::format::JsValueSerdeExt;
use std::collections::HashMap;
use wasm_bindgen::convert::IntoWasmAbi;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::types::map::MapEvent;
use yrs::types::{ToJson, TYPE_REFS_MAP};
use yrs::{DeepObservable, Map, MapRef, Observable, TransactionMut};

/// Collection used to store key-value entries in an unordered manner. Keys are always represented
/// as UTF-8 strings. Values can be any value type supported by Yrs: JSON-like primitives as well as
/// shared data types.
///
/// In terms of conflict resolution, [Map] uses logical last-write-wins principle, meaning the past
/// updates are automatically overridden and discarded by newer ones, while concurrent updates made
/// by different peers are resolved into a single value using document id seniority to establish
/// order.
#[wasm_bindgen]
pub struct YMap(pub(crate) SharedCollection<HashMap<String, JsValue>, MapRef>);

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
        YMap(SharedCollection::prelim(map))
    }

    #[wasm_bindgen(getter, js_name = type)]
    #[inline]
    pub fn get_type(&self) -> u8 {
        TYPE_REFS_MAP
    }

    /// Gets unique logical identifier of this type, shared across peers collaborating on the same
    /// document.
    #[wasm_bindgen(getter, js_name = id)]
    #[inline]
    pub fn id(&self) -> crate::Result<JsValue> {
        self.0.id()
    }

    /// Returns true if this is a preliminary instance of `YMap`.
    ///
    /// Preliminary instances can be nested into other shared data types such as `YArray` and `YMap`.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[wasm_bindgen(getter)]
    pub fn prelim(&self) -> bool {
        self.0.is_prelim()
    }

    /// Checks if current YMap reference is alive and has not been deleted by its parent collection.
    /// This method only works on already integrated shared types and will return false is current
    /// type is preliminary (has not been integrated into document).
    #[wasm_bindgen(js_name = alive)]
    pub fn alive(&self, txn: &YTransaction) -> bool {
        self.0.is_alive(txn)
    }

    /// Returns a number of entries stored within this instance of `YMap`.
    #[wasm_bindgen(js_name = length)]
    pub fn length(&self, txn: &ImplicitTransaction) -> crate::Result<u32> {
        match &self.0 {
            SharedCollection::Prelim(c) => Ok(c.len() as u32),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.len(txn))),
        }
    }

    /// Converts contents of this `YMap` instance into a JSON representation.
    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self, txn: &ImplicitTransaction) -> crate::Result<JsValue> {
        match &self.0 {
            SharedCollection::Prelim(c) => {
                let map = js_sys::Object::new();
                for (k, v) in c.iter() {
                    js_sys::Reflect::set(&map, &k.into(), v).unwrap();
                }
                Ok(map.into())
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let any = c.to_json(txn);
                JsValue::from_serde(&any).map_err(|e| JsValue::from_str(&e.to_string()))
            }),
        }
    }

    /// Sets a given `key`-`value` entry within this instance of `YMap`. If another entry was
    /// already stored under given `key`, it will be overridden with new `value`.
    #[wasm_bindgen(js_name = set)]
    pub fn set(
        &mut self,
        key: &str,
        value: JsValue,
        txn: ImplicitTransaction,
    ) -> crate::Result<()> {
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                c.insert(key.to_string(), value);
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.insert(txn, key.to_string(), Js::new(value));
                Ok(())
            }),
        }
    }

    /// Removes an entry identified by a given `key` from this instance of `YMap`, if such exists.
    #[wasm_bindgen(method, js_name = delete)]
    pub fn delete(&mut self, key: &str, txn: ImplicitTransaction) -> crate::Result<()> {
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                c.remove(key);
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.remove(txn, key);
                Ok(())
            }),
        }
    }

    /// Returns value of an entry stored under given `key` within this instance of `YMap`,
    /// or `undefined` if no such entry existed.
    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, key: &str, txn: &ImplicitTransaction) -> crate::Result<JsValue> {
        match &self.0 {
            SharedCollection::Prelim(c) => {
                let value = c.get(key);
                Ok(value.cloned().unwrap_or(JsValue::UNDEFINED))
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let value = c.get(txn, key);
                match value {
                    None => Ok(JsValue::UNDEFINED),
                    Some(value) => Ok(Js::from_value(&value, txn.doc()).into()),
                }
            }),
        }
    }

    #[wasm_bindgen(js_name = link)]
    pub fn link(&self, key: &str, txn: &ImplicitTransaction) -> crate::Result<JsValue> {
        match &self.0 {
            SharedCollection::Prelim(_) => Err(JsValue::from_str(js::errors::INVALID_PRELIM_OP)),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let link = c.link(txn, key);
                match link {
                    Some(link) => Ok(YWeakLink::from_prelim(link, txn.doc().clone()).into()),
                    None => Err(JsValue::from_str(js::errors::KEY_NOT_FOUND)),
                }
            }),
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
    pub fn entries(&self, txn: &ImplicitTransaction) -> crate::Result<JsValue> {
        match &self.0 {
            SharedCollection::Prelim(c) => {
                let map = js_sys::Object::new();
                for (k, v) in c.iter() {
                    js_sys::Reflect::set(&map, &k.into(), v)?;
                }
                Ok(map.into())
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let map = js_sys::Object::new();
                let doc = txn.doc();
                for (k, v) in c.iter(txn) {
                    let value = Js::from_value(&v, doc);
                    js_sys::Reflect::set(&map, &k.into(), &value.into())?;
                }
                Ok(map.into())
            }),
        }
    }

    /// Subscribes to all operations happening over this instance of `YMap`. All changes are
    /// batched and eventually triggered during transaction commit phase.
    /// Returns an `YObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observe)]
    pub fn observe(&mut self, callback: js_sys::Function) -> crate::Result<()> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => {
                let txn = c.transact()?;
                let array = c.resolve(&txn)?;
                let abi = callback.clone().into_abi();
                array.observe_with(abi, move |txn, e| {
                    let e = YMapEvent::new(e, txn);
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

    /// Unsubscribes a callback previously subscribed with `observeDeep` method.
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

/// Event generated by `YMap.observe` method. Emitted during transaction commit phase.
#[wasm_bindgen]
pub struct YMapEvent {
    inner: &'static MapEvent,
    txn: &'static TransactionMut<'static>,
    target: Option<JsValue>,
    keys: Option<JsValue>,
}

#[wasm_bindgen]
impl YMapEvent {
    pub(crate) fn new<'doc>(event: &MapEvent, txn: &TransactionMut<'doc>) -> Self {
        let inner: &'static MapEvent = unsafe { std::mem::transmute(event) };
        let txn: &'static TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        YMapEvent {
            inner,
            txn,
            target: None,
            keys: None,
        }
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
            YMap(SharedCollection::integrated(target.clone(), doc.clone())).into()
        });
        js.clone()
    }

    /// Returns a list of key-value changes made over corresponding `YMap` collection within
    /// bounds of current transaction. These changes follow a format:
    ///
    /// - { action: 'add'|'update'|'delete', oldValue: any|undefined, newValue: any|undefined }
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
