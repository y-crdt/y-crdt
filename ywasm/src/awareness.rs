use gloo_utils::format::JsValueSerdeExt;
use js_sys::Uint8Array;
use serde_json::Value;
use wasm_bindgen::convert::IntoWasmAbi;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;

use yrs::sync::{Awareness as YAwareness, AwarenessUpdate, Timestamp};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;

use crate::doc::YDoc;

#[wasm_bindgen]
pub struct Awareness {
    inner: YAwareness<Value>,
}

#[wasm_bindgen]
impl Awareness {
    #[wasm_bindgen(constructor)]
    pub fn new(doc: YDoc) -> Awareness {
        let inner = YAwareness::with_clock(doc.0.clone(), JsClock);
        Awareness { inner }
    }

    #[wasm_bindgen(getter, js_name = doc)]
    pub fn doc(&self) -> YDoc {
        YDoc(self.inner.doc().clone())
    }

    #[wasm_bindgen(getter, js_name = meta)]
    pub fn meta(&self) -> crate::Result<js_sys::Map> {
        let meta = self.inner.meta();
        let result = js_sys::Map::new();
        for (&client_id, info) in meta.iter() {
            let info = JsValue::from_serde(info).map_err(|e| JsValue::from_str(&e.to_string()))?;
            result.set(&JsValue::from(client_id), &info);
        }
        Ok(result)
    }

    #[wasm_bindgen(js_name = destroy)]
    pub fn destroy(&mut self) {
        self.inner.clean_local_state();
    }

    #[wasm_bindgen(js_name = getLocalState)]
    pub fn local_state(&self) -> crate::Result<JsValue> {
        match self.inner.local_state() {
            None => Ok(JsValue::NULL),
            Some(js) => JsValue::from_serde(js).map_err(|e| JsValue::from_str(&e.to_string())),
        }
    }

    #[wasm_bindgen(js_name = setLocalState)]
    pub fn set_local_state(&mut self, state: JsValue) -> crate::Result<()> {
        if state.is_null() {
            self.inner.clean_local_state();
        } else {
            self.inner.set_local_state(
                state
                    .into_serde()
                    .map_err(|e| JsValue::from_str(&e.to_string()))?,
            );
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
            let json = JsValue::from_serde(state).map_err(|e| JsValue::from_str(&e.to_string()))?;
            result.set(&JsValue::from(client_id), &json);
        }
        Ok(result)
    }

    #[wasm_bindgen(js_name = onUpdate)]
    pub fn on_update(&self, callback: js_sys::Function) {
        let abi = callback.clone().into_abi();
        self.inner.on_update_with(abi, move |_, e| {
            let json = JsValue::from_serde(e.summary()).unwrap();
            callback.call1(&JsValue::NULL, &json).unwrap();
        });
    }

    #[wasm_bindgen(js_name = offUpdate)]
    pub fn off_update(&self, callback: js_sys::Function) {
        let abi = callback.clone().into_abi();
        self.inner.unobserve_update(abi);
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
pub fn encode_update(awareness: &Awareness, clients: JsValue) -> crate::Result<Uint8Array> {
    let res = if clients.is_null() || clients.is_undefined() {
        awareness.inner.update()
    } else {
        let client_ids: Vec<u64> =
            JsValue::into_serde(&clients).map_err(|e| JsValue::from_str(&e.to_string()))?;
        awareness.inner.update_with_clients(client_ids)
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
pub fn apply_update(
    awareness: &mut Awareness,
    update: Uint8Array,
    _origin: JsValue, //TODO: use origin in Awareness::apply_update
) -> crate::Result<()> {
    let update = AwarenessUpdate::decode_v1(&update.to_vec())
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    awareness
        .inner
        .apply_update(update)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(())
}

pub struct JsClock;

impl yrs::sync::time::Clock for JsClock {
    fn now(&self) -> Timestamp {
        js_sys::Date::now() as u64
    }
}
