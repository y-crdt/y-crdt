use crate::collection::SharedCollection;
use crate::js::{Js, Shared};
use crate::transaction::YTransaction;
use crate::{ImplicitTransaction, Observer};
use std::iter::FromIterator;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::types::xml::XmlEvent;
use yrs::types::TYPE_REFS_XML_FRAGMENT;
use yrs::{DeepObservable, GetString, Observable, TransactionMut, XmlFragment, XmlFragmentRef};

/// Represents a list of `YXmlElement` and `YXmlText` types.
/// A `YXmlFragment` is similar to a `YXmlElement`, but it does not have a
/// nodeName and it does not have attributes. Though it can be bound to a DOM
/// element - in this case the attributes and the nodeName are not shared
#[wasm_bindgen]
pub struct YXmlFragment(pub(crate) SharedCollection<Vec<JsValue>, XmlFragmentRef>);

#[wasm_bindgen]
impl YXmlFragment {
    #[wasm_bindgen(constructor)]
    pub fn new(children: Vec<JsValue>) -> crate::Result<YXmlFragment> {
        let mut nodes = Vec::with_capacity(children.len());
        for xml_node in children {
            Js::assert_xml_prelim(&xml_node)?;
            nodes.push(xml_node);
        }
        Ok(YXmlFragment(SharedCollection::prelim(nodes)))
    }

    #[wasm_bindgen(getter, js_name = type)]
    #[inline]
    pub fn get_type(&self) -> u8 {
        TYPE_REFS_XML_FRAGMENT
    }

    /// Returns true if this is a preliminary instance of `YXmlFragment`.
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

    /// Returns a number of child XML nodes stored within this `YXMlElement` instance.
    #[wasm_bindgen(js_name = length)]
    pub fn length(&self, txn: &ImplicitTransaction) -> crate::Result<u32> {
        match &self.0 {
            SharedCollection::Prelim(c) => Ok(c.len() as u32),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.len(txn))),
        }
    }

    #[wasm_bindgen(js_name = insert)]
    pub fn insert(
        &mut self,
        index: u32,
        xml_node: JsValue,
        txn: ImplicitTransaction,
    ) -> crate::Result<()> {
        Js::assert_xml_prelim(&xml_node)?;
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                c.insert(index as usize, xml_node);
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.insert(txn, index, Js::new(xml_node));
                Ok(())
            }),
        }
    }

    #[wasm_bindgen(js_name = push)]
    pub fn push(&mut self, xml_node: JsValue, txn: ImplicitTransaction) -> crate::Result<()> {
        Js::assert_xml_prelim(&xml_node)?;
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                c.push(xml_node);
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.push_back(txn, Js::new(xml_node));
                Ok(())
            }),
        }
    }

    #[wasm_bindgen(method, js_name = delete)]
    pub fn delete(
        &mut self,
        index: u32,
        length: Option<u32>,
        txn: ImplicitTransaction,
    ) -> crate::Result<()> {
        let length = length.unwrap_or(1);
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

    /// Returns a first child of this XML node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node has not children.
    #[wasm_bindgen(js_name = firstChild)]
    pub fn first_child(&self, txn: &ImplicitTransaction) -> crate::Result<JsValue> {
        match &self.0 {
            SharedCollection::Prelim(c) => Ok(c.first().cloned().unwrap_or(JsValue::UNDEFINED)),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| match c.first_child() {
                None => Ok(JsValue::UNDEFINED),
                Some(xml) => Ok(Js::from_xml(xml, txn.doc().clone()).into()),
            }),
        }
    }

    /// Returns a string representation of this XML node.
    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self, txn: &ImplicitTransaction) -> crate::Result<String> {
        match &self.0 {
            SharedCollection::Prelim(c) => {
                let mut str = String::new();
                for js in c.iter() {
                    let res = match Shared::from_ref(js)? {
                        Shared::XmlText(c) => c.to_string(txn),
                        Shared::XmlElement(c) => c.to_string(txn),
                        Shared::XmlFragment(c) => c.to_string(txn),
                        _ => return Err(JsValue::from_str(crate::js::errors::NOT_XML_TYPE)),
                    };
                    str.push_str(&res?);
                }
                Ok(str)
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.get_string(txn))),
        }
    }

    /// Returns an iterator that enables a deep traversal of this XML node - starting from first
    /// child over this XML node successors using depth-first strategy.
    #[wasm_bindgen(js_name = treeWalker)]
    pub fn tree_walker(&self, txn: &ImplicitTransaction) -> crate::Result<js_sys::Array> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let doc = txn.doc();
                let walker = c.successors(txn).map(|n| {
                    let js: JsValue = Js::from_xml(n, doc.clone()).into();
                    js
                });
                let array = js_sys::Array::from_iter(walker);
                Ok(array.into())
            }),
        }
    }

    /// Subscribes to all operations happening over this instance of `YXmlFragment`. All changes are
    /// batched and eventually triggered during transaction commit phase.
    /// Returns an `YObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observe)]
    pub fn observe(&mut self, f: js_sys::Function) -> crate::Result<Observer> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => {
                let txn = c.transact()?;
                let array = c.resolve(&txn)?;
                Ok(Observer(array.observe(move |txn, e| {
                    let e = YXmlEvent::new(e, txn);
                    let txn = YTransaction::from_ref(txn);
                    f.call2(&JsValue::UNDEFINED, &e.into(), &txn.into())
                        .unwrap();
                })))
            }
        }
    }

    /// Subscribes to all operations happening over this Y shared type, as well as events in
    /// shared types stored within this one. All changes are batched and eventually triggered
    /// during transaction commit phase.
    /// Returns an `YEventObserver` which, when free'd, will unsubscribe current callback.
    #[wasm_bindgen(js_name = observeDeep)]
    pub fn observe_deep(&mut self, f: js_sys::Function) -> crate::Result<Observer> {
        match &self.0 {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => {
                let txn = c.transact()?;
                let array = c.resolve(&txn)?;
                Ok(Observer(array.observe_deep(move |txn, e| {
                    let e = crate::js::convert::events_into_js(txn, e);
                    let txn = YTransaction::from_ref(txn);
                    f.call2(&JsValue::UNDEFINED, &e, &txn.into()).unwrap();
                })))
            }
        }
    }
}

/// Event generated by `YXmlElement.observe` method. Emitted during transaction commit phase.
#[wasm_bindgen]
pub struct YXmlEvent {
    inner: &'static XmlEvent,
    txn: &'static TransactionMut<'static>,
    target: Option<JsValue>,
    keys: Option<JsValue>,
    delta: Option<JsValue>,
}

#[wasm_bindgen]
impl YXmlEvent {
    pub(crate) fn new<'doc>(event: &XmlEvent, txn: &TransactionMut<'doc>) -> Self {
        let inner: &'static XmlEvent = unsafe { std::mem::transmute(event) };
        let txn: &'static TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        YXmlEvent {
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
        let js = self
            .target
            .get_or_insert_with(|| Js::from_xml(target.clone(), doc.clone()).into());
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
            let inner = &self.inner;
            let txn = &self.txn;
            let delta = self.delta.get_or_insert_with(|| {
                let delta = inner
                    .delta(txn)
                    .into_iter()
                    .map(|change| crate::js::convert::change_into_js(change, txn.doc()));
                let mut result = js_sys::Array::new();
                result.extend(delta);
                result.into()
            });
            delta.clone()
        }
    }
}
