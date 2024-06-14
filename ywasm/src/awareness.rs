use gloo_utils::format::JsValueSerdeExt;
use js_sys::Uint8Array;
use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsValue, UnwrapThrowExt};

use yrs::sync::awareness::Event;
use yrs::sync::{Awareness as AwarenessInner, AwarenessUpdate, Timestamp};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;

use crate::doc::YDoc;
use crate::Observer;

#[wasm_bindgen]
pub struct Awareness {
    inner: AwarenessInner,
}

#[wasm_bindgen]
impl Awareness {
    #[wasm_bindgen(constructor)]
    pub fn new(doc: YDoc) -> Awareness {
        let inner = AwarenessInner::with_clock(doc.0.clone(), JsClock);
        Awareness { inner }
    }

    #[wasm_bindgen(getter, js_name = doc)]
    pub fn doc(&self) -> YDoc {
        YDoc(self.inner.doc().clone())
    }

    #[wasm_bindgen(js_name = destroy)]
    pub fn destroy(&mut self) {
        self.inner.clean_local_state();
    }

    #[wasm_bindgen(js_name = getLocalState)]
    pub fn local_state(&self) -> crate::Result<JsValue> {
        match self.inner.local_state() {
            None => Ok(JsValue::NULL),
            Some(str) => js_sys::JSON::parse(str),
        }
    }

    #[wasm_bindgen(js_name = setLocalState)]
    pub fn set_local_state(&mut self, state: JsValue) -> crate::Result<()> {
        if state.is_null() {
            self.inner.clean_local_state();
        } else {
            let state = js_sys::JSON::stringify(&state)?;
            self.inner.set_local_state(state);
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = setLocalStateField)]
    pub fn set_field(&mut self, key: &str, value: JsValue) -> crate::Result<()> {
        let state = self.local_state()?;
        js_sys::Reflect::set(&state, &JsValue::from_str(key), &value)?;
        self.set_local_state(state)
    }

    #[wasm_bindgen(js_name = getStates)]
    pub fn states(&self) -> crate::Result<js_sys::Map> {
        let result = js_sys::Map::new();
        for (&client_id, state) in self.inner.clients().iter() {
            let state = js_sys::JSON::parse(&state)?;
            result.set(&JsValue::from(client_id), &state);
        }
        Ok(result)
    }

    #[wasm_bindgen(js_name = onUpdate)]
    pub fn on_update(&self, callback: js_sys::Function) -> Observer {
        #[derive(Serialize)]
        struct AwarenessEvent {
            added: Vec<u64>,
            updated: Vec<u64>,
            removed: Vec<u64>,
        }

        impl<'a> From<&'a Event> for AwarenessEvent {
            fn from(event: &'a Event) -> Self {
                AwarenessEvent {
                    added: event.added().iter().copied().collect(),
                    updated: event.updated().iter().copied().collect(),
                    removed: event.removed().iter().copied().collect(),
                }
            }
        }

        let sub = self.inner.on_update(move |e| {
            let event = AwarenessEvent::from(e);
            let json = JsValue::from_serde(&event).unwrap_throw();
            callback.call1(&JsValue::NULL, &json).unwrap_throw();
        });
        Observer(sub)
    }
}

#[wasm_bindgen(js_name = removeAwarenessStates)]
pub fn remove_states(awareness: &mut Awareness, clients: Vec<u64>) -> crate::Result<()> {
    for client_id in clients {
        awareness.inner.remove_state(client_id);
    }
    Ok(())
}

#[wasm_bindgen(js_name = encodeAwarenessUpdate)]
pub fn encode_update(
    awareness: &Awareness,
    clients: Option<Vec<u64>>,
) -> crate::Result<Uint8Array> {
    let res = match clients {
        None => awareness.inner.update(),
        Some(clients) => awareness.inner.update_with_clients(clients),
    };

    let update = res.map_err(|e| JsValue::from_str(&e.to_string()))?;
    let bytes = update.encode_v1();
    Ok(Uint8Array::from(bytes.as_slice()))
}

#[wasm_bindgen(js_name = modifyAwarenessUpdate)]
pub fn modify_update(update: Uint8Array, modify: js_sys::Function) -> crate::Result<Uint8Array> {
    let mut update = AwarenessUpdate::decode_v1(&update.to_vec())
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    for (client_id, state) in update.clients.iter_mut() {
        let js = js_sys::JSON::parse(&state.json)?;
        let new_state = modify.call2(&JsValue::NULL, &js, &JsValue::from(*client_id))?;
        state.json = js_sys::JSON::stringify(&new_state)?.into();
    }
    Ok(Uint8Array::from(update.encode_v1().as_slice()))
}

#[wasm_bindgen(js_name = applyAwarenessUpdate)]
pub fn apply_update(awareness: &mut Awareness, update: Uint8Array) -> crate::Result<()> {
    let update = AwarenessUpdate::decode_v1(&update.to_vec())
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    awareness
        .inner
        .apply_update(update)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(())
}

struct JsClock;

impl yrs::sync::time::Clock for JsClock {
    fn now(&self) -> Timestamp {
        js_sys::Date::now() as u64
    }
}
