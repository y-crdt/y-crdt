use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::updates::decoder::{Decode, DecoderV1};
use yrs::updates::encoder::Encode;
use yrs::{ReadTxn, Transact, Update};

mod array;
mod collection;
mod doc;
mod js;
mod map;
mod text;
mod transaction;
mod undo;
mod weak;

type Result<T> = std::result::Result<T, JsValue>;

pub use crate::array::YArray as Array;
pub use crate::array::YArrayEvent as ArrayEvent;
pub use crate::doc::YDoc as Doc;
pub use crate::map::YMap as Map;
pub use crate::map::YMapEvent as MapEvent;
pub use crate::text::YText as Text;
pub use crate::text::YTextEvent as TextEvent;
pub use crate::transaction::ImplicitTransaction;
pub use crate::transaction::YTransaction as Transaction;
pub use crate::undo::YUndoEvent as UndoEvent;
pub use crate::undo::YUndoManager as UndoManager;
pub use crate::weak::YWeakLink as WeakLink;
pub use crate::weak::YWeakLinkEvent as WeakLinkEvent;

#[wasm_bindgen]
#[repr(transparent)]
pub struct Observer(pub(crate) yrs::Subscription);

impl From<yrs::Subscription> for Observer {
    fn from(s: yrs::Subscription) -> Self {
        Observer(s)
    }
}

/// Encodes a state vector of a given ywasm document into its binary representation using lib0 v1
/// encoding. State vector is a compact representation of updates performed on a given document and
/// can be used by `encode_state_as_update` on remote peer to generate a delta update payload to
/// synchronize changes between peers.
///
/// Example:
///
/// ```javascript
/// import {YDoc, encodeStateVector, encodeStateAsUpdate, applyUpdate} from 'ywasm'
///
/// /// document on machine A
/// const localDoc = new YDoc()
/// const localSV = encodeStateVector(localDoc)
///
/// // document on machine B
/// const remoteDoc = new YDoc()
/// const remoteDelta = encodeStateAsUpdate(remoteDoc, localSV)
///
/// applyUpdate(localDoc, remoteDelta)
/// ```
#[wasm_bindgen(js_name = encodeStateVector)]
pub fn encode_state_vector(doc: &Doc) -> Result<js_sys::Uint8Array> {
    let txn = doc
        .0
        .try_transact()
        .map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_RW_TX))?;
    let bytes = txn.state_vector().encode_v1();
    Ok(js_sys::Uint8Array::from(bytes.as_slice()))
}

/// Returns a string dump representation of a given `update` encoded using lib0 v1 encoding.
#[wasm_bindgen(js_name = debugUpdateV1)]
pub fn debug_update_v1(update: js_sys::Uint8Array) -> std::result::Result<String, JsValue> {
    let update: Vec<u8> = update.to_vec();
    let mut decoder = DecoderV1::from(update.as_slice());
    match Update::decode(&mut decoder) {
        Ok(update) => Ok(format!("{:#?}", update)),
        Err(e) => Err(JsValue::from(e.to_string())),
    }
}

/// Returns a string dump representation of a given `update` encoded using lib0 v2 encoding.
#[wasm_bindgen(js_name = debugUpdateV2)]
pub fn debug_update_v2(update: js_sys::Uint8Array) -> std::result::Result<String, JsValue> {
    let mut update: Vec<u8> = update.to_vec();
    match Update::decode_v2(update.as_mut_slice()) {
        Ok(update) => Ok(format!("{:#?}", update)),
        Err(e) => Err(JsValue::from(e.to_string())),
    }
}

/// Encodes all updates that have happened since a given version `vector` into a compact delta
/// representation using lib0 v1 encoding. If `vector` parameter has not been provided, generated
/// delta payload will contain all changes of a current ywasm document, working effectivelly as its
/// state snapshot.
///
/// Example:
///
/// ```javascript
/// import {YDoc, encodeStateVector, encodeStateAsUpdate, applyUpdate} from 'ywasm'
///
/// /// document on machine A
/// const localDoc = new YDoc()
/// const localSV = encodeStateVector(localDoc)
///
/// // document on machine B
/// const remoteDoc = new YDoc()
/// const remoteDelta = encodeStateAsUpdate(remoteDoc, localSV)
///
/// applyUpdate(localDoc, remoteDelta)
/// ```
#[wasm_bindgen(js_name = encodeStateAsUpdate)]
pub fn encode_state_as_update(
    doc: &Doc,
    vector: Option<js_sys::Uint8Array>,
) -> Result<js_sys::Uint8Array> {
    let txn = doc
        .0
        .try_transact()
        .map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_RW_TX))?;
    let sv = crate::js::convert::state_vector_from_js(vector)?.unwrap_or_default();
    let bytes = txn.encode_state_as_update_v1(&sv);
    Ok(bytes.as_slice().into())
}

/// Encodes all updates that have happened since a given version `vector` into a compact delta
/// representation using lib0 v2 encoding. If `vector` parameter has not been provided, generated
/// delta payload will contain all changes of a current ywasm document, working effectivelly as its
/// state snapshot.
///
/// Example:
///
/// ```javascript
/// import {YDoc, encodeStateVector, encodeStateAsUpdate, applyUpdate} from 'ywasm'
///
/// /// document on machine A
/// const localDoc = new YDoc()
/// const localSV = encodeStateVector(localDoc)
///
/// // document on machine B
/// const remoteDoc = new YDoc()
/// const remoteDelta = encodeStateAsUpdateV2(remoteDoc, localSV)
///
/// applyUpdate(localDoc, remoteDelta)
/// ```
#[wasm_bindgen(js_name = encodeStateAsUpdateV2)]
pub fn encode_state_as_update_v2(
    doc: &Doc,
    vector: Option<js_sys::Uint8Array>,
) -> Result<js_sys::Uint8Array> {
    let txn = doc
        .0
        .try_transact()
        .map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_RW_TX))?;
    let sv = crate::js::convert::state_vector_from_js(vector)?.unwrap_or_default();
    let bytes = txn.encode_state_as_update_v2(&sv);
    Ok(bytes.as_slice().into())
}

/// Applies delta update generated by the remote document replica to a current document. This
/// method assumes that a payload maintains lib0 v1 encoding format.
///
/// Example:
///
/// ```javascript
/// import {YDoc, encodeStateVector, encodeStateAsUpdate, applyUpdate} from 'ywasm'
///
/// /// document on machine A
/// const localDoc = new YDoc()
/// const localSV = encodeStateVector(localDoc)
///
/// // document on machine B
/// const remoteDoc = new YDoc()
/// const remoteDelta = encodeStateAsUpdate(remoteDoc, localSV)
///
/// applyUpdateV2(localDoc, remoteDelta)
/// ```
#[wasm_bindgen(js_name = applyUpdate)]
pub fn apply_update(doc: &Doc, update: js_sys::Uint8Array, origin: JsValue) -> Result<()> {
    let txn = if origin.is_undefined() {
        doc.0.try_transact_mut_with(js::Js::from(origin))
    } else {
        doc.0.try_transact_mut()
    };
    let mut txn = txn.map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_TX))?;
    let diff: Vec<u8> = update.to_vec();
    match Update::decode_v1(&diff) {
        Ok(update) => Ok(txn.apply_update(update)),
        Err(e) => Err(JsValue::from(e.to_string())),
    }
}

/// Applies delta update generated by the remote document replica to a current document. This
/// method assumes that a payload maintains lib0 v2 encoding format.
///
/// Example:
///
/// ```javascript
/// import {YDoc, encodeStateVector, encodeStateAsUpdate, applyUpdate} from 'ywasm'
///
/// /// document on machine A
/// const localDoc = new YDoc()
/// const localSV = encodeStateVector(localDoc)
///
/// // document on machine B
/// const remoteDoc = new YDoc()
/// const remoteDelta = encodeStateAsUpdateV2(remoteDoc, localSV)
///
/// applyUpdateV2(localDoc, remoteDelta)
/// ```
#[wasm_bindgen(js_name = applyUpdateV2)]
pub fn apply_update_v2(
    doc: &Doc,
    update: js_sys::Uint8Array,
    origin: JsValue,
) -> std::result::Result<(), JsValue> {
    let txn = if origin.is_undefined() {
        doc.0.try_transact_mut_with(js::Js::from(origin))
    } else {
        doc.0.try_transact_mut()
    };
    let mut txn = txn.map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_TX))?;
    let diff: Vec<u8> = update.to_vec();
    match Update::decode_v2(&diff) {
        Ok(update) => Ok(txn.apply_update(update)),
        Err(e) => Err(JsValue::from(e.to_string())),
    }
}
