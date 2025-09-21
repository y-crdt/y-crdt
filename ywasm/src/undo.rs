use std::collections::HashSet;
use std::sync::Arc;

use js_sys::Reflect;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;

use yrs::branch::BranchPtr;
use yrs::undo::EventKind;
use yrs::Doc as YDoc;

use crate::doc::Doc;
use crate::js::{Callback, Js, Shared};
use crate::transaction::Transaction;
use crate::Result;

#[wasm_bindgen]
pub struct YUndoManager {
    manager: yrs::undo::UndoManager<JsValue>,
    doc: crate::Doc,
}

impl YUndoManager {
    fn get_scope(doc: &crate::Doc, js: &JsValue) -> Result<BranchPtr> {
        let shared = Shared::from_ref(js)?;
        let branch_id = if let Some(id) = shared.branch_id() {
            id
        } else {
            return Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP));
        };
        let doc = doc.instance.borrow_mut();
        let txn = doc.transact();
        match branch_id.get_branch(&txn) {
            Some(branch) if !branch.is_deleted() => Ok(branch),
            _ => Err(JsValue::from_str(crate::js::errors::REF_DISPOSED)),
        }
    }
}

#[wasm_bindgen]
impl YUndoManager {
    #[wasm_bindgen(constructor)]
    pub fn new(doc: &crate::Doc, scope: JsValue, options: JsValue) -> Result<YUndoManager> {
        let scope = Self::get_scope(doc, &scope)?;
        let mut o = yrs::undo::Options {
            capture_timeout_millis: 500,
            tracked_origins: HashSet::new(),
            capture_transaction: None,
            timestamp: Arc::new(crate::awareness::JsClock),
            init_undo_stack: Vec::new(),
            init_redo_stack: Vec::new(),
        };
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
                        let v = Js::from(js);
                        o.tracked_origins.insert(v.into());
                    }
                }
            }
        }
        let doc = doc.clone();

        let manager = yrs::undo::UndoManager::with_scope_and_options(
            &mut doc.instance.borrow_mut(),
            &scope,
            o,
        );
        Ok(Self { manager, doc })
    }

    #[wasm_bindgen(js_name = addToScope)]
    pub fn add_to_scope(&mut self, ytypes: js_sys::Array) -> Result<()> {
        for js in ytypes.iter() {
            let scope = Self::get_scope(&self.doc, &js)?;
            self.manager.expand_scope(&scope);
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = addTrackedOrigin)]
    pub fn add_tracked_origin(&mut self, origin: JsValue) {
        self.manager.include_origin(Js::from(origin))
    }

    #[wasm_bindgen(js_name = removeTrackedOrigin)]
    pub fn remove_tracked_origin(&mut self, origin: JsValue) {
        self.manager.exclude_origin(Js::from(origin))
    }

    #[wasm_bindgen(js_name = clear)]
    pub fn clear(&mut self) {
        let doc = self.doc.instance.borrow();
        self.manager.clear(&doc);
    }

    #[wasm_bindgen(js_name = stopCapturing)]
    pub fn stop_capturing(&mut self) {
        self.manager.reset()
    }

    #[wasm_bindgen(js_name = undo)]
    pub fn undo(&mut self) -> bool {
        let mut doc = self.doc.instance.borrow_mut();
        self.manager.undo(&mut doc)
    }

    #[wasm_bindgen(js_name = redo)]
    pub fn redo(&mut self) -> bool {
        let mut doc = self.doc.instance.borrow_mut();
        self.manager.redo(&mut doc)
    }

    #[wasm_bindgen(getter, js_name = canUndo)]
    pub fn can_undo(&mut self) -> bool {
        self.manager.can_undo()
    }

    #[wasm_bindgen(getter, js_name = canRedo)]
    pub fn can_redo(&mut self) -> bool {
        self.manager.can_redo()
    }

    #[wasm_bindgen(js_name = on)]
    pub fn on(&mut self, event: &str, callback: js_sys::Function) -> crate::Result<()> {
        let abi = callback.subscription_key();
        match event {
            "stack-item-added" => self.manager.observe_item_added_with(abi, move |txn, e| {
                let event: JsValue = YUndoEvent::new(e).into();
                callback.call1(&JsValue::UNDEFINED, &event).unwrap();
                let meta =
                    Reflect::get(&event, &JsValue::from_str("meta")).unwrap_or(JsValue::UNDEFINED);
                *e.meta_mut() = meta;
            }),
            "stack-item-popped" => self.manager.observe_item_popped_with(abi, move |txn, e| {
                let event: JsValue = YUndoEvent::new(e).into();
                callback.call1(&JsValue::UNDEFINED, &event).unwrap();
                let meta =
                    Reflect::get(&event, &JsValue::from_str("meta")).unwrap_or(JsValue::UNDEFINED);
                *e.meta_mut() = meta;
            }),
            "stack-item-updated" => self.manager.observe_item_updated_with(abi, move |txn, e| {
                let event: JsValue = YUndoEvent::new(e).into();
                callback.call1(&JsValue::UNDEFINED, &event).unwrap();
                let meta =
                    Reflect::get(&event, &JsValue::from_str("meta")).unwrap_or(JsValue::UNDEFINED);
                *e.meta_mut() = meta;
            }),
            unknown => return Err(JsValue::from_str(&format!("Unknown event: {}", unknown))),
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = off)]
    pub fn off(&mut self, event: &str, callback: js_sys::Function) -> crate::Result<bool> {
        let abi = callback.subscription_key();
        match event {
            "stack-item-added" => Ok(self.manager.unobserve_item_added(abi)),
            "stack-item-popped" => Ok(self.manager.unobserve_item_popped(abi)),
            "stack-item-updated" => Ok(self.manager.unobserve_item_updated(abi)),
            unknown => Err(JsValue::from_str(&format!("Unknown event: {}", unknown))),
        }
    }
}

#[wasm_bindgen]
pub struct YUndoEvent {
    origin: JsValue,
    kind: JsValue,
    meta: JsValue,
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

    #[wasm_bindgen(getter, js_name = meta)]
    pub fn meta(&self) -> JsValue {
        self.meta.clone()
    }

    #[wasm_bindgen(setter, js_name = meta)]
    pub fn set_meta(&mut self, value: &JsValue) {
        self.meta = value.clone();
    }

    fn new(e: &yrs::undo::Event<JsValue>) -> Self {
        let origin = if let Some(origin) = e.origin() {
            Js::from(origin).into()
        } else {
            JsValue::UNDEFINED
        };
        YUndoEvent {
            meta: e.meta().clone(),
            origin,
            kind: match e.kind() {
                EventKind::Undo => JsValue::from_str("undo"),
                EventKind::Redo => JsValue::from_str("redo"),
            },
        }
    }
}
