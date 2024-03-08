use crate::js::Js;
use crate::Result;
use js_sys::Uint8Array;
use std::ops::Deref;
use wasm_bindgen::__rt::{Ref, RefMut};
use wasm_bindgen::convert::{IntoWasmAbi, RefFromWasmAbi, RefMutFromWasmAbi};
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{ReadTxn, TransactionMut, Update};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "YTransaction | undefined")]
    pub type ImplicitTransaction;
}

enum Cell<'a, T> {
    Owned(T),
    Borrowed(&'a T),
}

#[wasm_bindgen]
pub struct YTransaction {
    inner: Cell<'static, TransactionMut<'static>>,
}

impl YTransaction {
    pub fn from_implicit(txn: &ImplicitTransaction) -> crate::Result<Option<Ref<Self>>> {
        let js_value: &JsValue = txn.as_ref();
        if js_value.is_undefined() {
            Ok(None)
        } else {
            match YTransaction::try_ref_from_js_value(js_value) {
                Ok(txn) => Ok(Some(txn)),
                Err(e) => Err(e),
            }
        }
    }

    pub fn from_implicit_mut(txn: &ImplicitTransaction) -> crate::Result<Option<RefMut<Self>>> {
        let js_value: &JsValue = txn.as_ref();
        if js_value.is_undefined() {
            Ok(None)
        } else {
            match YTransaction::try_mut_from_js_value(js_value) {
                Ok(txn) => Ok(Some(txn)),
                Err(e) => Err(e),
            }
        }
    }

    pub fn try_ref_from_js_value(value: &JsValue) -> Result<Ref<Self>> {
        let abi = value.into_abi();

        if abi == 0 {
            Err(JsValue::from_str(crate::js::errors::NON_TRANSACTION))
        } else {
            let ptr = js_sys::Reflect::get(&value, &JsValue::from_str(crate::js::JS_PTR))?;
            let ptr_u32 = ptr
                .as_f64()
                .ok_or(JsValue::from_str(crate::js::errors::NOT_WASM_OBJ))?
                as u32;
            let target = unsafe { YTransaction::ref_from_abi(ptr_u32) };
            Ok(target)
        }
    }

    pub fn try_mut_from_js_value(value: &JsValue) -> Result<RefMut<Self>> {
        let abi = value.into_abi();
        if abi == 0 {
            Err(JsValue::from_str(crate::js::errors::NON_TRANSACTION))
        } else {
            let ptr = js_sys::Reflect::get(&value, &JsValue::from_str(crate::js::JS_PTR))?;
            let ptr_u32 = ptr
                .as_f64()
                .ok_or(JsValue::from_str(crate::js::errors::NOT_WASM_OBJ))?
                as u32;
            let target = unsafe { YTransaction::ref_mut_from_abi(ptr_u32) };
            Ok(target)
        }
    }

    pub fn from_ref(txn: &TransactionMut) -> Self {
        let txn: &'static TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        YTransaction {
            inner: Cell::Borrowed(txn),
        }
    }

    pub fn as_mut(&mut self) -> Result<&mut TransactionMut<'static>> {
        match &mut self.inner {
            Cell::Owned(v) => Ok(v),
            Cell::Borrowed(_) => Err(JsValue::from_str(
                crate::js::errors::INVALID_TRANSACTION_CTX,
            )),
        }
    }
}

#[wasm_bindgen]
impl YTransaction {
    /// Returns state vector describing the state of the document
    /// at the moment when the transaction began.
    #[wasm_bindgen(getter, js_name = beforeState)]
    pub fn before_state(&self) -> js_sys::Map {
        let sv = self.deref().before_state();
        crate::js::convert::state_vector_to_js(&sv)
    }

    /// Returns state vector describing the current state of
    /// the document.
    #[wasm_bindgen(getter, js_name = afterState)]
    pub fn after_state(&self) -> js_sys::Map {
        let sv = self.deref().after_state();
        crate::js::convert::state_vector_to_js(&sv)
    }

    /// Returns a delete set containing information about
    /// all blocks removed as part of a current transaction.
    #[wasm_bindgen(getter, js_name = deleteSet)]
    pub fn delete_set(&self) -> js_sys::Map {
        let ds = self.deref().delete_set();
        crate::js::convert::delete_set_to_js(&ds)
    }

    #[wasm_bindgen(getter, js_name = origin)]
    pub fn origin(&self) -> JsValue {
        if let Some(origin) = self.deref().origin() {
            Js::from(origin).into()
        } else {
            JsValue::UNDEFINED
        }
    }

    /// Triggers a post-update series of operations without `free`ing the transaction. This includes
    /// compaction and optimization of internal representation of updates, triggering events etc.
    /// ywasm transactions are auto-committed when they are `free`d.
    #[wasm_bindgen(js_name = commit)]
    pub fn commit(&mut self) -> Result<()> {
        let txn = self
            .as_mut()
            .map_err(|_| crate::js::errors::INVALID_TRANSACTION_CTX)?;
        txn.commit();
        Ok(())
    }

    /// Encodes a state vector of a given transaction document into its binary representation using
    /// lib0 v1 encoding. State vector is a compact representation of updates performed on a given
    /// document and can be used by `encode_state_as_update` on remote peer to generate a delta
    /// update payload to synchronize changes between peers.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const localDoc = new YDoc()
    /// const localTxn = localDoc.beginTransaction()
    ///
    /// // document on machine B
    /// const remoteDoc = new YDoc()
    /// const remoteTxn = localDoc.beginTransaction()
    ///
    /// try {
    ///     const localSV = localTxn.stateVectorV1()
    ///     const remoteDelta = remoteTxn.diffV1(localSv)
    ///     localTxn.applyV1(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    #[wasm_bindgen(js_name = stateVectorV1)]
    pub fn state_vector_v1(&self) -> Uint8Array {
        let sv = self.state_vector();
        let payload = sv.encode_v1();
        Uint8Array::from(payload.as_slice())
    }

    /// Encodes all updates that have happened since a given version `vector` into a compact delta
    /// representation using lib0 v1 encoding. If `vector` parameter has not been provided, generated
    /// delta payload will contain all changes of a current ywasm document, working effectively as
    /// its state snapshot.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const localDoc = new YDoc()
    /// const localTxn = localDoc.beginTransaction()
    ///
    /// // document on machine B
    /// const remoteDoc = new YDoc()
    /// const remoteTxn = localDoc.beginTransaction()
    ///
    /// try {
    ///     const localSV = localTxn.stateVectorV1()
    ///     const remoteDelta = remoteTxn.diffV1(localSv)
    ///     localTxn.applyV1(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    #[wasm_bindgen(js_name = diffV1)]
    pub fn diff_v1(&self, vector: Option<Uint8Array>) -> Result<Uint8Array> {
        let sv = crate::js::convert::state_vector_from_js(vector)?.unwrap_or_default();
        let payload = self.encode_diff_v1(&sv);
        Ok(Uint8Array::from(payload.as_slice()))
    }

    /// Encodes all updates that have happened since a given version `vector` into a compact delta
    /// representation using lib0 v1 encoding. If `vector` parameter has not been provided, generated
    /// delta payload will contain all changes of a current ywasm document, working effectively as
    /// its state snapshot.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const localDoc = new YDoc()
    /// const localTxn = localDoc.beginTransaction()
    ///
    /// // document on machine B
    /// const remoteDoc = new YDoc()
    /// const remoteTxn = localDoc.beginTransaction()
    ///
    /// try {
    ///     const localSV = localTxn.stateVectorV1()
    ///     const remoteDelta = remoteTxn.diffV2(localSv)
    ///     localTxn.applyV2(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    #[wasm_bindgen(js_name = diffV2)]
    pub fn diff_v2(&self, vector: Option<Uint8Array>) -> Result<Uint8Array> {
        let sv = crate::js::convert::state_vector_from_js(vector)?.unwrap_or_default();
        let payload = self.encode_diff_v2(&sv);
        Ok(Uint8Array::from(payload.as_slice()))
    }

    /// Applies delta update generated by the remote document replica to a current transaction's
    /// document. This method assumes that a payload maintains lib0 v1 encoding format.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const localDoc = new YDoc()
    /// const localTxn = localDoc.beginTransaction()
    ///
    /// // document on machine B
    /// const remoteDoc = new YDoc()
    /// const remoteTxn = localDoc.beginTransaction()
    ///
    /// try {
    ///     const localSV = localTxn.stateVectorV1()
    ///     const remoteDelta = remoteTxn.diffV1(localSv)
    ///     localTxn.applyV1(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    #[wasm_bindgen(js_name = applyV1)]
    pub fn apply_v1(&mut self, diff: Uint8Array) -> Result<()> {
        let diff: Vec<u8> = diff.to_vec();
        match Update::decode_v1(&diff) {
            Ok(update) => self.try_apply(update),
            Err(e) => Err(JsValue::from(e.to_string())),
        }
    }

    fn try_apply(&mut self, update: Update) -> Result<()> {
        let txn = self.as_mut()?;
        txn.apply_update(update);
        Ok(())
    }

    /// Applies delta update generated by the remote document replica to a current transaction's
    /// document. This method assumes that a payload maintains lib0 v2 encoding format.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const localDoc = new YDoc()
    /// const localTxn = localDoc.beginTransaction()
    ///
    /// // document on machine B
    /// const remoteDoc = new YDoc()
    /// const remoteTxn = localDoc.beginTransaction()
    ///
    /// try {
    ///     const localSV = localTxn.stateVectorV1()
    ///     const remoteDelta = remoteTxn.diffV2(localSv)
    ///     localTxn.applyV2(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    #[wasm_bindgen(js_name = applyV2)]
    pub fn apply_v2(&mut self, diff: Uint8Array) -> Result<()> {
        let mut diff: Vec<u8> = diff.to_vec();
        match Update::decode_v2(&mut diff) {
            Ok(update) => self.try_apply(update),
            Err(e) => Err(JsValue::from(e.to_string())),
        }
    }

    #[wasm_bindgen(js_name = encodeUpdate)]
    pub fn encode_update(&self) -> Uint8Array {
        let payload = self.encode_update_v1();
        Uint8Array::from(payload.as_slice())
    }

    #[wasm_bindgen(js_name = encodeUpdateV2)]
    pub fn encode_update_v2(&self) -> Uint8Array {
        let txn: &TransactionMut = self.deref();
        let payload = txn.encode_update_v2();
        Uint8Array::from(payload.as_slice())
    }
}

impl<'doc> From<TransactionMut<'doc>> for YTransaction {
    fn from(value: TransactionMut<'doc>) -> Self {
        let txn: TransactionMut<'static> = unsafe { std::mem::transmute(value) };
        YTransaction {
            inner: Cell::Owned(txn),
        }
    }
}

impl Deref for YTransaction {
    type Target = TransactionMut<'static>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        match &self.inner {
            Cell::Owned(v) => v,
            Cell::Borrowed(v) => *v,
        }
    }
}
