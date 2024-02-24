use crate::doc::YDoc;
use crate::js::{Js, Shared};
use crate::transaction::YTransaction;
use crate::Result;
use js_sys::Reflect;
use std::rc::Rc;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::branch::BranchPtr;
use yrs::undo::{EventKind, UndoManager};
use yrs::{Doc, Transact};

#[wasm_bindgen]
#[repr(transparent)]
pub struct YUndoManager(UndoManager<JsValue>);

impl YUndoManager {
    fn get_scope(doc: &Doc, js: &JsValue) -> Result<BranchPtr> {
        let shared = Shared::from_ref(js)?;
        let branch_id = if let Some(id) = shared.branch_id() {
            id
        } else {
            return Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP));
        };
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
    pub fn new(doc: &YDoc, scope: JsValue, options: JsValue) -> Result<YUndoManager> {
        let doc = &doc.0;
        let scope = Self::get_scope(doc, &scope)?;
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
                        let v = Js::from(js);
                        o.tracked_origins.insert(v.into());
                    }
                }
            }
        }
        Ok(YUndoManager(UndoManager::with_options(doc, &scope, o)))
    }

    #[wasm_bindgen(js_name = addToScope)]
    pub fn add_to_scope(&mut self, ytypes: js_sys::Array) -> Result<()> {
        for js in ytypes.iter() {
            let scope = Self::get_scope(self.0.doc(), &js)?;
            self.0.expand_scope(&scope);
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = addTrackedOrigin)]
    pub fn add_tracked_origin(&mut self, origin: JsValue) {
        self.0.include_origin(Js::from(origin))
    }

    #[wasm_bindgen(js_name = removeTrackedOrigin)]
    pub fn remove_tracked_origin(&mut self, origin: JsValue) {
        self.0.exclude_origin(Js::from(origin))
    }

    #[wasm_bindgen(js_name = clear)]
    pub fn clear(&mut self) -> Result<()> {
        if let Err(_) = self.0.clear() {
            Err(JsValue::from_str(crate::js::errors::ANOTHER_TX))
        } else {
            Ok(())
        }
    }

    #[wasm_bindgen(js_name = stopCapturing)]
    pub fn stop_capturing(&mut self) {
        self.0.reset()
    }

    #[wasm_bindgen(js_name = undo)]
    pub fn undo(&mut self) -> Result<()> {
        if let Err(_) = self.0.undo() {
            Err(JsValue::from_str(crate::js::errors::ANOTHER_TX))
        } else {
            Ok(())
        }
    }

    #[wasm_bindgen(js_name = redo)]
    pub fn redo(&mut self) -> Result<()> {
        if let Err(_) = self.0.redo() {
            Err(JsValue::from_str(crate::js::errors::ANOTHER_TX))
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
    pub fn on_item_added(&mut self, callback: js_sys::Function) -> crate::Observer {
        self.0
            .observe_item_added(move |txn, e| {
                let event: JsValue = YUndoEvent::new(e).into();
                let txn: JsValue = YTransaction::from_ref(txn).into();
                callback.call2(&JsValue::UNDEFINED, &event, &txn).unwrap();
                let meta =
                    Reflect::get(&event, &JsValue::from_str("meta")).unwrap_or(JsValue::UNDEFINED);
                *e.meta_mut() = meta;
            })
            .into()
    }

    #[wasm_bindgen(js_name = onStackItemPopped)]
    pub fn on_item_popped(&mut self, callback: js_sys::Function) -> crate::Observer {
        self.0
            .observe_item_popped(move |txn, e| {
                let event: JsValue = YUndoEvent::new(e).into();
                let txn: JsValue = YTransaction::from_ref(txn).into();
                callback.call2(&JsValue::UNDEFINED, &event, &txn).unwrap();
                let meta =
                    Reflect::get(&event, &JsValue::from_str("meta")).unwrap_or(JsValue::UNDEFINED);
                *e.meta_mut() = meta;
            })
            .into()
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
