use crate::collection::SharedCollection;
use crate::js::{Callback, Js};
use crate::transaction::Transaction as YTransaction;
use crate::Result;
use std::sync::Arc;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::branch::BranchPtr;
use yrs::types::weak::{LinkSource, WeakEvent};
use yrs::types::TYPE_REFS_WEAK;
use yrs::{ArrayRef, DeepObservable, Doc, GetString, Observable, SharedRef, WeakPrelim, WeakRef};

pub(crate) struct PrelimWrapper {
    prelim: WeakPrelim<BranchPtr>,
    doc: crate::Doc,
}

#[wasm_bindgen]
#[repr(transparent)]
pub struct YWeakLink(pub(crate) SharedCollection<PrelimWrapper, WeakRef<BranchPtr>>);

impl YWeakLink {
    pub(crate) fn from_prelim<S: SharedRef>(prelim: WeakPrelim<S>, doc: crate::Doc) -> Self {
        let prelim = prelim.upcast();
        YWeakLink(SharedCollection::Prelim(PrelimWrapper { prelim, doc }))
    }

    pub(crate) fn source(&self) -> Arc<LinkSource> {
        match &self.0 {
            SharedCollection::Integrated(c) => c
                .transact(|shared_ref, txn| Ok(shared_ref.source().clone()))
                .unwrap(),
            SharedCollection::Prelim(v) => v.prelim.source().clone(),
        }
    }

    fn doc(&self) -> &crate::Doc {
        match &self.0 {
            SharedCollection::Integrated(c) => &c.doc,
            SharedCollection::Prelim(v) => &v.doc,
        }
    }
}

#[wasm_bindgen]
impl YWeakLink {
    /// Returns true if this is a preliminary instance of `YWeakLink`.
    ///
    /// Preliminary instances can be nested into other shared data types such as `YArray` and `YMap`.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[wasm_bindgen(getter)]
    pub fn prelim(&self) -> bool {
        self.0.is_prelim()
    }

    #[wasm_bindgen(getter, js_name = type)]
    #[inline]
    pub fn get_type(&self) -> u8 {
        TYPE_REFS_WEAK
    }

    /// Gets unique logical identifier of this type, shared across peers collaborating on the same
    /// document.
    #[wasm_bindgen(getter, js_name = id)]
    #[inline]
    pub fn id(&self) -> crate::Result<JsValue> {
        self.0.id()
    }

    /// Checks if current YWeakLink reference is alive and has not been deleted by its parent collection.
    /// This method only works on already integrated shared types and will return false is current
    /// type is preliminary (has not been integrated into document).
    #[wasm_bindgen(js_name = alive)]
    pub fn alive(&self) -> bool {
        self.0.is_alive()
    }

    #[wasm_bindgen(js_name = deref)]
    pub fn deref(&self) -> Result<JsValue> {
        use yrs::MapRef;

        match &self.0 {
            SharedCollection::Prelim(c) => {
                let weak_ref: WeakPrelim<MapRef> = WeakPrelim::from(c.prelim.clone());
                let value = c
                    .doc
                    .transact(JsValue::UNDEFINED, |tx| weak_ref.try_deref_raw(tx));
                match value {
                    None => Ok(JsValue::UNDEFINED),
                    Some(value) => Ok(Js::from_value(&value, c.doc.clone()).into()),
                }
            }
            SharedCollection::Integrated(c) => {
                let doc = c.doc.clone();
                c.transact(|weak_ref, txn| {
                    let weak_ref: WeakRef<MapRef> = WeakRef::from(weak_ref.clone());
                    let value = weak_ref.try_deref_value(txn);
                    match value {
                        None => Ok(JsValue::UNDEFINED),
                        Some(value) => Ok(Js::from_value(&value, doc).into()),
                    }
                })
            }
        }
    }

    #[wasm_bindgen(js_name = unquote)]
    pub fn unquote(&self) -> Result<js_sys::Array> {
        use std::iter::FromIterator;
        use yrs::ArrayRef;

        match &self.0 {
            SharedCollection::Prelim(c) => {
                let doc = c.doc.clone();
                let weak_ref: WeakPrelim<ArrayRef> = WeakPrelim::from(c.prelim.clone());
                c.doc.transact(JsValue::UNDEFINED, |tx| {
                    let values = weak_ref
                        .unquote(tx)
                        .map(|value| Js::from_value(&value, doc.clone()));
                    Ok(js_sys::Array::from_iter(values))
                })
            }
            SharedCollection::Integrated(c) => {
                let doc = c.doc.clone();
                c.transact(|weak_ref, txn| {
                    let weak_ref: WeakRef<ArrayRef> = WeakRef::from(weak_ref.clone());
                    let iter = weak_ref
                        .unquote(txn)
                        .map(|value| Js::from_value(&value, doc.clone()));
                    Ok(js_sys::Array::from_iter(iter))
                })
            }
        }
    }

    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self) -> Result<String> {
        use yrs::XmlTextRef;

        match &self.0 {
            SharedCollection::Prelim(c) => {
                let weak_ref: WeakPrelim<XmlTextRef> = WeakPrelim::from(c.prelim.clone());
                c.doc
                    .transact(JsValue::UNDEFINED, |tx| Ok(weak_ref.get_string(tx)))
            }
            SharedCollection::Integrated(c) => c.transact(|c, txn| {
                let weak_ref: WeakRef<XmlTextRef> = WeakRef::from(c.clone());
                let string = weak_ref.get_string(txn);
                Ok(string)
            }),
        }
    }

    /// Subscribes to all operations happening over this instance of `YWeakLink`. All changes are
    /// batched and eventually triggered during transaction commit phase.
    #[wasm_bindgen(js_name = observe)]
    pub fn observe(&self, callback: js_sys::Function) -> crate::Result<()> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => {
                let abi = callback.subscription_key();
                let doc = c.doc.clone();
                c.transact(|array, txn| {
                    array.observe_with(abi, move |_, e| {
                        let e = YWeakLinkEvent::new(e, doc.clone());
                        callback.call1(&JsValue::UNDEFINED, &e.into()).unwrap();
                    });
                    Ok(())
                })
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
                let abi = callback.subscription_key();
                c.transact(|array, _| Ok(array.unobserve(abi)))
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
                let abi = callback.subscription_key();
                let doc = c.doc.clone();
                c.transact(|array, _| {
                    array.observe_deep_with(abi, move |_, e| {
                        let e = crate::js::convert::events_into_js(doc.clone(), e);
                        callback.call1(&JsValue::UNDEFINED, &e).unwrap();
                    });
                    Ok(())
                })
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
                let abi = callback.subscription_key();
                c.transact(|array, _| Ok(array.unobserve_deep(abi)))
            }
        }
    }
}

/// Event generated by `YXmlElement.observe` method. Emitted during transaction commit phase.
#[wasm_bindgen]
pub struct YWeakLinkEvent {
    inner: &'static WeakEvent,
    doc: crate::Doc,
    target: Option<JsValue>,
    origin: JsValue,
}

#[wasm_bindgen]
impl YWeakLinkEvent {
    pub(crate) fn new<'doc>(event: &WeakEvent, doc: crate::Doc) -> Self {
        let inner: &'static WeakEvent = unsafe { std::mem::transmute(event) };
        let origin = doc.transaction_origin().unwrap_or(JsValue::UNDEFINED);
        YWeakLinkEvent {
            inner,
            doc,
            origin,
            target: None,
        }
    }

    #[wasm_bindgen(getter, js_name = origin)]
    pub fn origin(&self) -> JsValue {
        self.origin.clone()
    }

    /// Returns a current shared type instance, that current event changes refer to.
    #[wasm_bindgen(getter)]
    pub fn target(&mut self) -> JsValue {
        let target: WeakRef<BranchPtr> = self.inner.as_target();
        let doc = self.doc.clone();
        let js = self.target.get_or_insert_with(|| {
            let target = target.clone();
            YWeakLink(SharedCollection::integrated(target, doc)).into()
        });
        js.clone()
    }

    /// Returns an array of keys and indexes creating a path from root type down to current instance
    /// of shared type (accessible via `target` getter).
    #[wasm_bindgen]
    pub fn path(&self) -> JsValue {
        crate::js::convert::path_into_js(self.inner.path())
    }
}
