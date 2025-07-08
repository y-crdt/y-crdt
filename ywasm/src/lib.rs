use base64_light::base64_decode;
use gloo_utils::format::JsValueSerdeExt;
use js_sys::Uint8Array;
use serde::de::{Error, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::Formatter;
use std::ops::Deref;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::updates::decoder::{Decode, DecoderV1};
use yrs::updates::encoder::{Encode, Encoder};
use yrs::{Assoc, ReadTxn, StickyIndex, Transact, TransactionMut, Update};

mod array;
mod awareness;
mod collection;
mod doc;
mod js;
mod map;
mod text;
mod transaction;
mod undo;
mod weak;
mod xml_elem;
mod xml_frag;
mod xml_text;
mod xml;

type Result<T> = std::result::Result<T, JsValue>;

pub use crate::array::YArray as Array;
pub use crate::array::YArrayEvent as ArrayEvent;
pub use crate::doc::YDoc as Doc;
use crate::js::Shared;
pub use crate::map::YMap as Map;
pub use crate::map::YMapEvent as MapEvent;
pub use crate::text::YText as Text;
pub use crate::text::YTextEvent as TextEvent;
pub use crate::transaction::ImplicitTransaction;
pub use crate::transaction::YTransaction as Transaction;
use crate::transaction::YTransaction;
pub use crate::undo::YUndoEvent as UndoEvent;
pub use crate::undo::YUndoManager as UndoManager;
pub use crate::weak::YWeakLink as WeakLink;
pub use crate::weak::YWeakLinkEvent as WeakLinkEvent;

/// When called will call console log errors whenever internal panic is called from within
/// WebAssembly module.
#[wasm_bindgen(js_name = setPanicHook)]
pub fn set_panic_hook() {
    // When the `console_error_panic_hook` feature is enabled, we can call the
    // `set_panic_hook` function at least once during initialization, and then
    // we will get better error messages if our code ever panics.
    //
    // For more details see
    // https://github.com/rustwasm/console_error_panic_hook#readme
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
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
pub fn debug_update_v1(update: js_sys::Uint8Array) -> Result<String> {
    let update: Vec<u8> = update.to_vec();
    let mut decoder = DecoderV1::from(update.as_slice());
    match Update::decode(&mut decoder) {
        Ok(update) => Ok(format!("{:#?}", update)),
        Err(e) => Err(JsValue::from(e.to_string())),
    }
}

/// Returns a string dump representation of a given `update` encoded using lib0 v2 encoding.
#[wasm_bindgen(js_name = debugUpdateV2)]
pub fn debug_update_v2(update: js_sys::Uint8Array) -> Result<String> {
    let mut update: Vec<u8> = update.to_vec();
    match Update::decode_v2(update.as_mut_slice()) {
        Ok(update) => Ok(format!("{:#?}", update)),
        Err(e) => Err(JsValue::from(e.to_string())),
    }
}

/// Merges a sequence of updates (encoded using lib0 v1 encoding) together, producing another
/// update (also lib0 v1 encoded) in the result. Returned binary is a combination of all input
/// `updates`, compressed.
///
/// Returns an error whenever any of the input updates couldn't be decoded.
#[wasm_bindgen(js_name = mergeUpdatesV1)]
pub fn merge_updates_v1(updates: js_sys::Array) -> Result<js_sys::Uint8Array> {
    // Handle empty array case
    if updates.length() == 0 {
        return Ok(js_sys::Uint8Array::new_with_length(0));
    }

    let mut merged_updates = Vec::new();

    // Decode each update from the JavaScript array
    for update_js in updates.iter() {
        let update_bytes = js_sys::Uint8Array::from(update_js).to_vec();
        match Update::decode_v1(&update_bytes) {
            Ok(update) => merged_updates.push(update),
            Err(e) => return Err(JsValue::from(e.to_string())),
        }
    }

    // Merge all updates and encode as v1
    let merged = Update::merge_updates(merged_updates);
    let encoded = merged.encode_v1();
    Ok(js_sys::Uint8Array::from(encoded.as_slice()))
}

/// Merges a sequence of updates (encoded using lib0 v2 encoding) together, producing another
/// update (also lib0 v2 encoded) in the result. Returned binary is a combination of all input
/// `updates`, compressed.
///
/// Returns an error whenever any of the input updates couldn't be decoded.
#[wasm_bindgen(js_name = mergeUpdatesV2)]
pub fn merge_updates_v2(updates: js_sys::Array) -> Result<js_sys::Uint8Array> {
    // Handle empty array case
    if updates.length() == 0 {
        return Ok(js_sys::Uint8Array::new_with_length(0));
    }

    let mut merged_updates = Vec::new();

    // Decode each update from the JavaScript array
    for update_js in updates.iter() {
        let update_bytes = js_sys::Uint8Array::from(update_js).to_vec();
        match Update::decode_v2(&update_bytes) {
            Ok(update) => merged_updates.push(update),
            Err(e) => return Err(JsValue::from(e.to_string())),
        }
    }

    // Merge all updates and encode as v2
    let merged = Update::merge_updates(merged_updates);
    let encoded = merged.encode_v2();
    Ok(js_sys::Uint8Array::from(encoded.as_slice()))
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
    let txn = if !origin.is_undefined() {
        doc.0.try_transact_mut_with(js::Js::from(origin))
    } else {
        doc.0.try_transact_mut()
    };
    let mut txn = txn.map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_TX))?;
    let diff: Vec<u8> = update.to_vec();
    match Update::decode_v1(&diff) {
        Ok(update) => txn
            .apply_update(update)
            .map_err(|e| JsValue::from(e.to_string())),
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
pub fn apply_update_v2(doc: &Doc, update: js_sys::Uint8Array, origin: JsValue) -> Result<()> {
    let txn = if !origin.is_undefined() {
        doc.0.try_transact_mut_with(js::Js::from(origin))
    } else {
        doc.0.try_transact_mut()
    };
    let mut txn = txn.map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_TX))?;
    let diff: Vec<u8> = update.to_vec();
    match Update::decode_v2(&diff) {
        Ok(update) => txn
            .apply_update(update)
            .map_err(|e| JsValue::from(e.to_string())),
        Err(e) => Err(JsValue::from(e.to_string())),
    }
}

struct Snapshot(yrs::Snapshot);

impl Deref for Snapshot {
    type Target = yrs::Snapshot;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Snapshot {
    #[inline]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SnapshotVisitor;
        impl<'de> Visitor<'de> for SnapshotVisitor {
            type Value = Snapshot;

            fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
                write!(formatter, "a valid snapshot")
            }

            #[inline]
            fn visit_str<E>(self, v: &str) -> std::result::Result<Self::Value, E>
            where
                E: Error,
            {
                let result = base64_light::base64_decode(v);
                let snap =
                    yrs::Snapshot::decode_v1(&result).map_err(|e| E::custom(e.to_string()))?;
                Ok(Snapshot(snap))
            }
        }

        deserializer.deserialize_str(SnapshotVisitor)
    }
}

impl Serialize for Snapshot {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let bytes = self.0.encode_v1();
        serializer.serialize_str(&base64_light::base64_encode_bytes(&bytes))
    }
}

#[wasm_bindgen(js_name = snapshot)]
pub fn snapshot(doc: &Doc) -> crate::Result<JsValue> {
    let snapshot = doc.0.transact().snapshot();
    JsValue::from_serde(&Snapshot(snapshot)).map_err(|e| JsValue::from_str(&e.to_string()))
}

#[wasm_bindgen(js_name = equalSnapshots)]
pub fn equal_snapshots(snap1: &JsValue, snap2: &JsValue) -> bool {
    let s1: Snapshot = snap1.into_serde().unwrap();
    let s2: Snapshot = snap2.into_serde().unwrap();
    s1.0 == s2.0
}

#[wasm_bindgen(js_name = encodeSnapshotV1)]
pub fn encode_snapshot_v1(snapshot: &JsValue) -> Vec<u8> {
    base64_decode(&snapshot.as_string().unwrap())
}

#[wasm_bindgen(js_name = encodeSnapshotV2)]
pub fn encode_snapshot_v2(snapshot: &JsValue) -> Vec<u8> {
    let s: Snapshot = snapshot.into_serde().unwrap();
    s.0.encode_v2()
}

#[wasm_bindgen(js_name = decodeSnapshotV2)]
pub fn decode_snapshot_v2(snapshot: &[u8]) -> Result<JsValue> {
    let s = yrs::Snapshot::decode_v2(snapshot)
        .map_err(|_| JsValue::from("failed to deserialize snapshot using lib0 v2 decoding"))?;
    JsValue::from_serde(&Snapshot(s)).map_err(|e| JsValue::from_str(&e.to_string()))
}

#[wasm_bindgen(js_name = decodeSnapshotV1)]
pub fn decode_snapshot_v1(snapshot: &[u8]) -> Result<JsValue> {
    Ok(JsValue::from_str(&base64_light::base64_encode_bytes(
        snapshot,
    )))
}

#[wasm_bindgen(js_name = encodeStateFromSnapshotV1)]
pub fn encode_state_from_snapshot_v1(doc: &Doc, snapshot: JsValue) -> Result<Vec<u8>> {
    let snapshot: Snapshot = snapshot
        .into_serde()
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let mut encoder = yrs::updates::encoder::EncoderV1::new();
    match doc
        .0
        .transact()
        .encode_state_from_snapshot(&*snapshot, &mut encoder)
    {
        Ok(_) => Ok(encoder.to_vec()),
        Err(e) => Err(JsValue::from(e.to_string())),
    }
}

#[wasm_bindgen(js_name = encodeStateFromSnapshotV2)]
pub fn encode_state_from_snapshot_v2(doc: &Doc, snapshot: JsValue) -> Result<Vec<u8>> {
    let snapshot: Snapshot = snapshot
        .into_serde()
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let mut encoder = yrs::updates::encoder::EncoderV2::new();
    match doc
        .0
        .transact()
        .encode_state_from_snapshot(&*snapshot, &mut encoder)
    {
        Ok(_) => Ok(encoder.to_vec()),
        Err(e) => Err(JsValue::from(e.to_string())),
    }
}

/// Retrieves a sticky index corresponding to a given human-readable `index` pointing into
/// the shared `ytype`. Unlike standard indexes sticky indexes enables to track
/// the location inside of a shared y-types, even in the face of concurrent updates.
///
/// If association is >= 0, the resulting position will point to location **after** the referenced index.
/// If association is < 0, the resulting position will point to location **before** the referenced index.
#[wasm_bindgen(js_name=createRelativePositionFromTypeIndex)]
pub fn create_sticky_index_from_type(
    ytype: &JsValue,
    index: u32,
    assoc: i32,
    txn: &ImplicitTransaction,
) -> Result<JsValue> {
    if let Ok(shared) = Shared::from_ref(ytype) {
        let assoc = if assoc >= 0 {
            Assoc::After
        } else {
            Assoc::Before
        };
        let (branch_id, doc) = shared.try_integrated()?;
        let index = match YTransaction::from_implicit(txn)? {
            Some(txn) => {
                let txn: &TransactionMut = (&*txn).deref();
                let ptr = match branch_id.get_branch(txn) {
                    None => return Err(JsValue::from_str(crate::js::errors::REF_DISPOSED)),
                    Some(ptr) => ptr,
                };
                StickyIndex::at(&*txn, ptr, index, assoc)
            }
            None => {
                let txn = doc
                    .try_transact()
                    .map_err(|_| JsValue::from_str(crate::js::errors::ANOTHER_RW_TX))?;
                let ptr = match branch_id.get_branch(&txn) {
                    None => return Err(JsValue::from_str(crate::js::errors::REF_DISPOSED)),
                    Some(ptr) => ptr,
                };
                StickyIndex::at(&txn, ptr, index, assoc)
            }
        };
        JsValue::from_serde(&index).map_err(|e| JsValue::from_str(&e.to_string()))
    } else {
        Err(JsValue::from_str(crate::js::errors::NOT_WASM_OBJ))
    }
}

/// Converts a sticky index (see: `createStickyIndexFromType`) into an object
/// containing human-readable index.
#[wasm_bindgen(js_name=createAbsolutePositionFromRelativePosition)]
pub fn create_offset_from_sticky_index(rpos: &JsValue, doc: &Doc) -> Result<JsValue> {
    let pos: StickyIndex =
        JsValue::into_serde(rpos).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let txn = doc.0.transact();
    if let Some(abs) = pos.get_offset(&txn) {
        #[derive(Serialize)]
        struct AbsolutePos {
            index: u32,
            assoc: Assoc,
        }
        let abs = AbsolutePos {
            index: abs.index,
            assoc: abs.assoc,
        };
        JsValue::from_serde(&abs).map_err(|e| JsValue::from_str(&e.to_string()))
    } else {
        Ok(JsValue::NULL)
    }
}

/// Serializes sticky index created by `createStickyIndexFromType` into a binary
/// payload.
#[wasm_bindgen(js_name=encodeRelativePosition)]
pub fn encode_sticky_index(rpos: &JsValue) -> Result<Uint8Array> {
    let pos: StickyIndex =
        JsValue::into_serde(rpos).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let bytes = Uint8Array::from(pos.encode_v1().as_slice());
    Ok(bytes)
}

/// Deserializes sticky index serialized previously by `encodeStickyIndex`.
#[wasm_bindgen(js_name=decodeRelativePosition)]
pub fn decode_sticky_index(bin: Uint8Array) -> Result<JsValue> {
    let data: Vec<u8> = bin.to_vec();
    match StickyIndex::decode_v1(&data) {
        Ok(index) => JsValue::from_serde(&index).map_err(|e| JsValue::from_str(&e.to_string())),
        Err(err) => Err(JsValue::from_str(&err.to_string())),
    }
}

#[wasm_bindgen(js_name=compareRelativePositions)]
pub fn sticky_index_cmp(a: JsValue, b: JsValue) -> bool {
    let a: Option<StickyIndex> = a.into_serde().unwrap();
    let b: Option<StickyIndex> = b.into_serde().unwrap();
    a == b
}
