use crate::array::YArray;
use crate::collection::SharedCollection;
use crate::doc::YDoc;
use crate::js::Js;
use crate::map::YMap;
use crate::text::YText;
use crate::weak::YWeakLink;
use crate::xml_elem::YXmlElement;
use crate::xml_frag::YXmlFragment;
use crate::xml_text::YXmlText;
use crate::Result;
use gloo_utils::format::JsValueSerdeExt;
use js_sys::Uint8Array;
use std::ops::Deref;
use wasm_bindgen::__rt::{RcRef, RcRefMut};
use wasm_bindgen::convert::{IntoWasmAbi, RefFromWasmAbi, RefMutFromWasmAbi};
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::types::TypeRef;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    ArrayRef, BranchID, JsonPath, JsonPathEval, MapRef, ReadTxn, TextRef, TransactionMut, Update,
    WeakRef, XmlElementRef, XmlFragmentRef, XmlTextRef,
};

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
    pub fn from_implicit(txn: &ImplicitTransaction) -> crate::Result<Option<RcRef<Self>>> {
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

    pub fn from_implicit_mut(txn: &ImplicitTransaction) -> crate::Result<Option<RcRefMut<Self>>> {
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

    pub fn try_ref_from_js_value(value: &JsValue) -> Result<RcRef<Self>> {
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

    pub fn try_mut_from_js_value(value: &JsValue) -> Result<RcRefMut<Self>> {
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

    pub fn as_ref(&self) -> &TransactionMut<'static> {
        match &self.inner {
            Cell::Owned(v) => v,
            Cell::Borrowed(v) => v,
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

    #[wasm_bindgen(getter, js_name = pendingStructs)]
    #[inline]
    pub fn pending_structs(&self) -> Result<JsValue> {
        let tx = self.deref();
        if let Some(update) = tx.store().pending_update() {
            let missing = crate::js::convert::state_vector_to_js(&update.missing);
            let update = js_sys::Uint8Array::from(update.update.encode_v1().as_slice());
            let obj: JsValue = js_sys::Object::new().into();
            js_sys::Reflect::set(&obj, &JsValue::from_str("update"), &update.into())?;
            js_sys::Reflect::set(&obj, &JsValue::from_str("missing"), &missing.into())?;
            Ok(obj.into())
        } else {
            Ok(JsValue::NULL)
        }
    }

    /// Returns a unapplied delete set, that was received in one of the previous remote updates.
    /// This DeleteSet is waiting for a missing updates to arrive in order to be applied.
    #[wasm_bindgen(getter, js_name = pendingDeleteSet)]
    #[inline]
    pub fn pending_ds(&self) -> Option<js_sys::Map> {
        let tx = self.deref();
        let ds = tx.store().pending_ds()?;
        Some(crate::js::convert::delete_set_to_js(&ds))
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

    /// Given a logical identifier of the collection (obtained via `YText.id`, `YArray.id` etc.),
    /// attempts to return an instance of that collection in the scope of current document.
    ///
    /// Returns `undefined` if an instance was not defined locally, haven't been integrated or
    /// has been deleted.
    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, id: JsValue) -> crate::Result<JsValue> {
        let branch_id: BranchID =
            JsValue::into_serde(&id).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let txn = self.as_ref();
        let doc = txn.doc().clone();
        Ok(match branch_id.get_branch(txn) {
            None => JsValue::UNDEFINED,
            Some(b) if b.is_deleted() => JsValue::UNDEFINED,
            Some(b) => match b.type_ref() {
                TypeRef::Array => {
                    YArray(SharedCollection::integrated(ArrayRef::from(b), doc)).into()
                }
                TypeRef::Map => YMap(SharedCollection::integrated(MapRef::from(b), doc)).into(),
                TypeRef::Text => YText(SharedCollection::integrated(TextRef::from(b), doc)).into(),
                TypeRef::XmlElement(_) => {
                    YXmlElement(SharedCollection::integrated(XmlElementRef::from(b), doc)).into()
                }
                TypeRef::XmlFragment => {
                    YXmlFragment(SharedCollection::integrated(XmlFragmentRef::from(b), doc)).into()
                }
                TypeRef::XmlText => {
                    YXmlText(SharedCollection::integrated(XmlTextRef::from(b), doc)).into()
                }
                TypeRef::WeakLink(_) => {
                    YWeakLink(SharedCollection::integrated(WeakRef::from(b), doc)).into()
                }
                TypeRef::SubDoc => match b.as_subdoc() {
                    None => JsValue::UNDEFINED,
                    Some(doc) => YDoc(doc).into(),
                },
                TypeRef::XmlHook | TypeRef::Undefined => JsValue::UNDEFINED,
            },
        })
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
        txn.apply_update(update)
            .map_err(|e| JsValue::from(e.to_string()))
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

    /// Force garbage collection of the deleted elements, regardless of a parent doc was created
    /// with `gc` option turned on or off.
    #[wasm_bindgen(js_name = gc)]
    pub fn gc(&mut self) -> Result<()> {
        let txn = self.as_mut()?;
        txn.gc(None);
        Ok(())
    }

    /// Evaluates a JSON path expression (see: https://en.wikipedia.org/wiki/JSONPath) on
    /// the document and returns an array of values matching that query.
    ///
    /// Currently, this method supports the following syntax:
    /// - `$` - root object
    /// - `@` - current object
    /// - `.field` or `['field']` - member accessor
    /// - `[1]` - array index (also supports negative indices)
    /// - `.*` or `[*]` - wildcard (matches all members of an object or array)
    /// - `..` - recursive descent (matches all descendants not only direct children)
    /// - `[start:end:step]` - array slice operator (requires positive integer arguments)
    /// - `['a', 'b', 'c']` - union operator (returns an array of values for each query)
    /// - `[1, -1, 3]` - multiple indices operator (returns an array of values for each index)
    ///
    /// At the moment, JSON Path does not support filter predicates.
    #[wasm_bindgen(js_name = selectAll)]
    pub fn select_all(&self, json_path: &str) -> Result<js_sys::Array> {
        let query = JsonPath::parse(json_path).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let txn = self.as_ref();
        let mut iter = txn.json_path(&query);
        let result = js_sys::Array::new();
        while let Some(value) = iter.next() {
            let value: JsValue = Js::from_value(&value, txn.doc()).into();
            result.push(&value);
        }
        Ok(result)
    }

    /// Evaluates a JSON path expression (see: https://en.wikipedia.org/wiki/JSONPath) on
    /// the document and returns first value matching that query.
    ///
    /// Currently, this method supports the following syntax:
    /// - `$` - root object
    /// - `@` - current object
    /// - `.field` or `['field']` - member accessor
    /// - `[1]` - array index (also supports negative indices)
    /// - `.*` or `[*]` - wildcard (matches all members of an object or array)
    /// - `..` - recursive descent (matches all descendants not only direct children)
    /// - `[start:end:step]` - array slice operator (requires positive integer arguments)
    /// - `['a', 'b', 'c']` - union operator (returns an array of values for each query)
    /// - `[1, -1, 3]` - multiple indices operator (returns an array of values for each index)
    ///
    /// At the moment, JSON Path does not support filter predicates.
    #[wasm_bindgen(js_name = selectOne)]
    pub fn select_one(&self, json_path: &str) -> Result<JsValue> {
        let query = JsonPath::parse(json_path).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let txn = self.as_ref();
        let mut iter = txn.json_path(&query);
        match iter.next() {
            None => Ok(JsValue::UNDEFINED),
            Some(value) => Ok(Js::from_value(&value, txn.doc()).into()),
        }
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
