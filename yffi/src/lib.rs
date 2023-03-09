use lib0::any::Any;
use lib0::error::Error;
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::mem::{forget, ManuallyDrop, MaybeUninit};
use std::ops::Deref;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use yrs::block::{ClientID, ItemContent, Prelim, Unused};
use yrs::types::array::ArrayEvent;
use yrs::types::array::ArrayIter as NativeArrayIter;
use yrs::types::map::MapEvent;
use yrs::types::map::MapIter as NativeMapIter;
use yrs::types::text::{Diff, TextEvent, YChange};
use yrs::types::xml::{Attributes as NativeAttributes, XmlNode};
use yrs::types::xml::{TreeWalker as NativeTreeWalker, XmlFragment};
use yrs::types::xml::{XmlEvent, XmlTextEvent};
use yrs::types::{
    Attrs, BranchPtr, Change, Delta, EntryChange, Event, PathSegment, Value, TYPE_REFS_ARRAY,
    TYPE_REFS_DOC, TYPE_REFS_MAP, TYPE_REFS_TEXT, TYPE_REFS_XML_ELEMENT, TYPE_REFS_XML_FRAGMENT,
    TYPE_REFS_XML_TEXT,
};
use yrs::undo::EventKind;
use yrs::updates::decoder::{Decode, DecoderV1};
use yrs::updates::encoder::{Encode, Encoder, EncoderV1, EncoderV2};
use yrs::{
    uuid_v4, Array, ArrayRef, Assoc, DeleteSet, GetString, Map, MapRef, Observable, OffsetKind,
    Options, Origin, ReadTxn, Snapshot, StateVector, StickyIndex, Store, SubdocsEvent,
    SubdocsEventIter, SubscriptionId, Text, TextRef, Transact, TransactionCleanupEvent,
    UndoManager, Update, Xml, XmlElementPrelim, XmlElementRef, XmlFragmentRef, XmlTextPrelim,
    XmlTextRef,
};

/// Flag used by `YInput` and `YOutput` to tag boolean values.
pub const Y_JSON_BOOL: i8 = -8;

/// Flag used by `YInput` and `YOutput` to tag floating point numbers.
pub const Y_JSON_NUM: i8 = -7;

/// Flag used by `YInput` and `YOutput` to tag 64-bit integer numbers.
pub const Y_JSON_INT: i8 = -6;

/// Flag used by `YInput` and `YOutput` to tag strings.
pub const Y_JSON_STR: i8 = -5;

/// Flag used by `YInput` and `YOutput` to tag binary content.
pub const Y_JSON_BUF: i8 = -4;

/// Flag used by `YInput` and `YOutput` to tag embedded JSON-like arrays of values,
/// which themselves are `YInput` and `YOutput` instances respectively.
pub const Y_JSON_ARR: i8 = -3;

/// Flag used by `YInput` and `YOutput` to tag embedded JSON-like maps of key-value pairs,
/// where keys are strings and v
pub const Y_JSON_MAP: i8 = -2;

/// Flag used by `YInput` and `YOutput` to tag JSON-like null values.
pub const Y_JSON_NULL: i8 = -1;

/// Flag used by `YInput` and `YOutput` to tag JSON-like undefined values.
pub const Y_JSON_UNDEF: i8 = 0;

/// Flag used by `YInput` and `YOutput` to tag content, which is an `YArray` shared type.
pub const Y_ARRAY: i8 = 1;

/// Flag used by `YInput` and `YOutput` to tag content, which is an `YMap` shared type.
pub const Y_MAP: i8 = 2;

/// Flag used by `YInput` and `YOutput` to tag content, which is an `YText` shared type.
pub const Y_TEXT: i8 = 3;

/// Flag used by `YInput` and `YOutput` to tag content, which is an `YXmlElement` shared type.
pub const Y_XML_ELEM: i8 = 4;

/// Flag used by `YInput` and `YOutput` to tag content, which is an `YXmlText` shared type.
pub const Y_XML_TEXT: i8 = 5;

/// Flag used by `YInput` and `YOutput` to tag content, which is an `YXmlFragment` shared type.
pub const Y_XML_FRAG: i8 = 6;

/// Flag used by `YInput` and `YOutput` to tag content, which is an `YDoc` shared type.
pub const Y_DOC: i8 = 7;

/// Flag used to mark a truthy boolean numbers.
pub const Y_TRUE: u8 = 1;

/// Flag used to mark a falsy boolean numbers.
pub const Y_FALSE: u8 = 0;

/// Flag used by `YOptions` to determine, that text operations offsets and length will be counted by
/// the byte number of UTF8-encoded string.
pub const Y_OFFSET_BYTES: u8 = 0;

/// Flag used by `YOptions` to determine, that text operations offsets and length will be counted by
/// UTF-16 chars of encoded string.
pub const Y_OFFSET_UTF16: u8 = 1;

/// Flag used by `YOptions` to determine, that text operations offsets and length will be counted by
/// by UTF-32 chars of encoded string.
pub const Y_OFFSET_UTF32: u8 = 2;

/* pub types below are used by cbindgen for c header generation */

/// A Yrs document type. Documents are most important units of collaborative resources management.
/// All shared collections live within a scope of their corresponding documents. All updates are
/// generated on per document basis (rather than individual shared type). All operations on shared
/// collections happen via `YTransaction`, which lifetime is also bound to a document.
///
/// Document manages so called root types, which are top-level shared types definitions (as opposed
/// to recursively nested types).
pub type Doc = yrs::Doc;

/// A common shared data type. All Yrs instances can be refered to using this data type (use
/// `ytype_kind` function if a specific type needs to be determined). Branch pointers are passed
/// over type-specific functions like `ytext_insert`, `yarray_insert` or `ymap_insert` to perform
/// a specific shared type operations.
///
/// Using write methods of different shared types (eg. `ytext_insert` and `yarray_insert`) over
/// the same branch may result in undefined behavior.
pub type Branch = yrs::types::Branch;

/// Iterator structure used by shared array data type.
#[repr(transparent)]
pub struct ArrayIter(NativeArrayIter<&'static Transaction, Transaction>);

/// Iterator structure used by shared map data type. Map iterators are unordered - there's no
/// specific order in which map entries will be returned during consecutive iterator calls.
#[repr(transparent)]
pub struct MapIter(NativeMapIter<'static, &'static Transaction, Transaction>);

/// Iterator structure used by XML nodes (elements and text) to iterate over node's attributes.
/// Attribute iterators are unordered - there's no specific order in which map entries will be
/// returned during consecutive iterator calls.
#[repr(transparent)]
pub struct Attributes(NativeAttributes<'static, &'static Transaction, Transaction>);

/// Iterator used to traverse over the complex nested tree structure of a XML node. XML node
/// iterator walks only over `YXmlElement` and `YXmlText` nodes. It does so in ordered manner (using
/// the order in which children are ordered within their parent nodes) and using **depth-first**
/// traverse.
#[repr(transparent)]
pub struct TreeWalker(NativeTreeWalker<'static, &'static Transaction, Transaction>);

/// Transaction is one of the core types in Yrs. All operations that need to touch or
/// modify a document's contents (a.k.a. block store), need to be executed in scope of a
/// transaction.
#[repr(transparent)]
pub struct Transaction(TransactionInner);

enum TransactionInner {
    ReadOnly(yrs::Transaction<'static>),
    ReadWrite(yrs::TransactionMut<'static>),
}

impl Transaction {
    fn read_only(txn: yrs::Transaction) -> Self {
        Transaction(TransactionInner::ReadOnly(unsafe {
            std::mem::transmute(txn)
        }))
    }

    fn read_write(txn: yrs::TransactionMut) -> Self {
        Transaction(TransactionInner::ReadWrite(unsafe {
            std::mem::transmute(txn)
        }))
    }

    fn is_writeable(&self) -> bool {
        match &self.0 {
            TransactionInner::ReadOnly(_) => false,
            TransactionInner::ReadWrite(_) => true,
        }
    }

    fn as_mut(&mut self) -> Option<&mut yrs::TransactionMut<'static>> {
        match &mut self.0 {
            TransactionInner::ReadOnly(_) => None,
            TransactionInner::ReadWrite(txn) => Some(txn),
        }
    }
}

impl ReadTxn for Transaction {
    fn store(&self) -> &Store {
        match &self.0 {
            TransactionInner::ReadOnly(txn) => txn.store(),
            TransactionInner::ReadWrite(txn) => txn.store(),
        }
    }
}

/// A structure representing single key-value entry of a map output (used by either
/// embedded JSON-like maps or YMaps).
#[repr(C)]
pub struct YMapEntry {
    /// Null-terminated string representing an entry's key component. Encoded as UTF-8.
    pub key: *const c_char,
    /// A `YOutput` value representing containing variadic content that can be stored withing map's
    /// entry.
    pub value: YOutput,
}

impl YMapEntry {
    fn new(key: &str, value: Value) -> Self {
        let value = YOutput::from(value);
        YMapEntry {
            key: CString::new(key).unwrap().into_raw(),
            value,
        }
    }
}

impl Drop for YMapEntry {
    fn drop(&mut self) {
        unsafe {
            drop(CString::from_raw(self.key as *mut c_char));
            //self.value.drop();
        }
    }
}

/// A structure representing single attribute of an either `YXmlElement` or `YXmlText` instance.
/// It consists of attribute name and string, both of which are null-terminated UTF-8 strings.
#[repr(C)]
pub struct YXmlAttr {
    pub name: *const c_char,
    pub value: *const c_char,
}

impl Drop for YXmlAttr {
    fn drop(&mut self) {
        unsafe {
            drop(CString::from_raw(self.name as *mut _));
            drop(CString::from_raw(self.value as *mut _));
        }
    }
}

/// Configuration object used by `YDoc`.
#[repr(C)]
pub struct YOptions {
    /// Globally unique 53-bit integer assigned to corresponding document replica as its identifier.
    ///
    /// If two clients share the same `id` and will perform any updates, it will result in
    /// unrecoverable document state corruption. The same thing may happen if the client restored
    /// document state from snapshot, that didn't contain all of that clients updates that were sent
    /// to other peers.
    pub id: u64,

    /// A NULL-able globally unique Uuid v4 compatible null-terminated string identifier
    /// of this document. If passed as NULL, a random Uuid will be generated instead.
    pub guid: *const c_char,

    /// A NULL-able, UTF-8 encoded, null-terminated string of a collection that this document
    /// belongs to. It's used only by providers.
    pub collection_id: *const c_char,

    /// Encoding used by text editing operations on this document. It's used to compute
    /// `YText`/`YXmlText` insertion offsets and text lengths. Either:
    ///
    /// - `Y_ENCODING_BYTES`
    /// - `Y_ENCODING_UTF16`
    /// - `Y_ENCODING_UTF32`
    pub encoding: u8,

    /// Boolean flag used to determine if deleted blocks should be garbage collected or not
    /// during the transaction commits. Setting this value to 0 means GC will be performed.
    pub skip_gc: u8,

    /// Boolean flag used to determine if subdocument should be loaded automatically.
    /// If this is a subdocument, remote peers will load the document as well automatically.
    pub auto_load: u8,

    /// Boolean flag used to determine whether the document should be synced by the provider now.
    pub should_load: u8,
}

impl Into<Options> for YOptions {
    fn into(self) -> Options {
        let encoding = match self.encoding {
            Y_OFFSET_BYTES => OffsetKind::Bytes,
            Y_OFFSET_UTF16 => OffsetKind::Utf16,
            Y_OFFSET_UTF32 => OffsetKind::Utf32,
            _ => panic!("Unrecognized YOptions.encoding type"),
        };
        let guid = if self.guid.is_null() {
            uuid_v4(&mut rand::thread_rng())
        } else {
            let c_str = unsafe { CStr::from_ptr(self.guid) };
            let str = c_str.to_str().unwrap();
            str.into()
        };
        let collection_id = if self.collection_id.is_null() {
            None
        } else {
            let c_str = unsafe { CStr::from_ptr(self.collection_id) };
            let str = c_str.to_str().unwrap().to_string();
            Some(str)
        };
        Options {
            client_id: self.id as ClientID,
            guid,
            collection_id,
            skip_gc: if self.skip_gc == 0 { false } else { true },
            auto_load: if self.auto_load == 0 { false } else { true },
            should_load: if self.should_load == 0 { false } else { true },
            offset_kind: encoding,
        }
    }
}

impl From<Options> for YOptions {
    fn from(o: Options) -> Self {
        YOptions {
            id: o.client_id,
            guid: CString::new(o.guid.as_ref()).unwrap().into_raw(),
            collection_id: if let Some(collection_id) = o.collection_id {
                CString::new(collection_id).unwrap().into_raw()
            } else {
                null_mut()
            },
            encoding: match o.offset_kind {
                OffsetKind::Bytes => Y_OFFSET_BYTES,
                OffsetKind::Utf16 => Y_OFFSET_UTF16,
                OffsetKind::Utf32 => Y_OFFSET_UTF32,
            },
            skip_gc: if o.skip_gc { 1 } else { 0 },
            auto_load: if o.auto_load { 1 } else { 0 },
            should_load: if o.should_load { 1 } else { 0 },
        }
    }
}

/// Returns default ceonfiguration for `YOptions`.
#[no_mangle]
pub unsafe extern "C" fn yoptions() -> YOptions {
    Options::default().into()
}

/// Releases all memory-allocated resources bound to given document.
#[no_mangle]
pub unsafe extern "C" fn ydoc_destroy(value: *mut Doc) {
    if !value.is_null() {
        drop(Box::from_raw(value));
    }
}

/// Frees all memory-allocated resources bound to a given [YMapEntry].
#[no_mangle]
pub unsafe extern "C" fn ymap_entry_destroy(value: *mut YMapEntry) {
    if !value.is_null() {
        drop(Box::from_raw(value));
    }
}

/// Frees all memory-allocated resources bound to a given [YXmlAttr].
#[no_mangle]
pub unsafe extern "C" fn yxmlattr_destroy(attr: *mut YXmlAttr) {
    if !attr.is_null() {
        drop(Box::from_raw(attr));
    }
}

/// Frees all memory-allocated resources bound to a given UTF-8 null-terminated string returned from
/// Yrs document API. Yrs strings don't use libc malloc, so calling `free()` on them will fault.
#[no_mangle]
pub unsafe extern "C" fn ystring_destroy(str: *mut c_char) {
    if !str.is_null() {
        drop(CString::from_raw(str));
    }
}

/// Frees all memory-allocated resources bound to a given binary returned from Yrs document API.
/// Unlike strings binaries are not null-terminated and can contain null characters inside,
/// therefore a size of memory to be released must be explicitly provided.
/// Yrs binaries don't use libc malloc, so calling `free()` on them will fault.
#[no_mangle]
pub unsafe extern "C" fn ybinary_destroy(ptr: *mut c_char, len: u32) {
    if !ptr.is_null() {
        drop(Vec::from_raw_parts(ptr, len as usize, len as usize));
    }
}

/// Creates a new [Doc] instance with a randomized unique client identifier.
///
/// Use [ydoc_destroy] in order to release created [Doc] resources.
#[no_mangle]
pub extern "C" fn ydoc_new() -> *mut Doc {
    Box::into_raw(Box::new(Doc::new()))
}

/// Creates a shallow clone of a provided `doc` - it's realized by increasing the ref-count
/// value of the document. In result both input and output documents point to the same instance.
///
/// Documents created this way can be destroyed via [ydoc_destroy] - keep in mind, that the memory
/// will still be persisted until all strong references are dropped.
#[no_mangle]
pub unsafe extern "C" fn ydoc_clone(doc: *mut Doc) -> *mut Doc {
    let doc = doc.as_mut().unwrap();
    Box::into_raw(Box::new(doc.clone()))
}

/// Creates a new [Doc] instance with a specified `options`.
///
/// Use [ydoc_destroy] in order to release created [Doc] resources.
#[no_mangle]
pub extern "C" fn ydoc_new_with_options(options: YOptions) -> *mut Doc {
    Box::into_raw(Box::new(Doc::with_options(options.into())))
}

/// Returns a unique client identifier of this [Doc] instance.
#[no_mangle]
pub unsafe extern "C" fn ydoc_id(doc: *mut Doc) -> u64 {
    let doc = doc.as_ref().unwrap();
    doc.client_id()
}

/// Returns a unique document identifier of this [Doc] instance.
#[no_mangle]
pub unsafe extern "C" fn ydoc_guid(doc: *mut Doc) -> *mut c_char {
    let doc = doc.as_ref().unwrap();
    let uid = &doc.options().guid;
    CString::new(uid.as_ref()).unwrap().into_raw()
}

/// Returns a collection identifier of this [Doc] instance.
/// If none was defined, a `NULL` will be returned.
#[no_mangle]
pub unsafe extern "C" fn ydoc_collection_id(doc: *mut Doc) -> *mut c_char {
    let doc = doc.as_ref().unwrap();
    if let Some(cid) = doc.options().collection_id.as_ref() {
        CString::new(cid.as_str()).unwrap().into_raw()
    } else {
        null_mut()
    }
}

/// Returns status of should_load flag of this [Doc] instance, informing parent [Doc] if this
/// document instance requested a data load.
#[no_mangle]
pub unsafe extern "C" fn ydoc_should_load(doc: *mut Doc) -> u8 {
    let doc = doc.as_ref().unwrap();
    doc.options().should_load as u8
}

/// Returns status of auto_load flag of this [Doc] instance. Auto loaded sub-documents automatically
/// send a load request to their parent documents.
#[no_mangle]
pub unsafe extern "C" fn ydoc_auto_load(doc: *mut Doc) -> u8 {
    let doc = doc.as_ref().unwrap();
    doc.options().auto_load as u8
}

#[no_mangle]
pub unsafe extern "C" fn ydoc_observe_updates_v1(
    doc: *mut Doc,
    state: *mut c_void,
    cb: extern "C" fn(*mut c_void, u32, *const c_char),
) -> u32 {
    let doc = doc.as_ref().unwrap();
    let observer = doc
        .observe_update_v1(move |_, e| {
            let bytes = &e.update;
            let len = bytes.len() as u32;
            cb(state, len, bytes.as_ptr() as *const c_char)
        })
        .unwrap();
    let subscription_id: u32 = observer.into();
    subscription_id
}

#[no_mangle]
pub unsafe extern "C" fn ydoc_observe_updates_v2(
    doc: *mut Doc,
    state: *mut c_void,
    cb: extern "C" fn(*mut c_void, u32, *const c_char),
) -> u32 {
    let doc = doc.as_ref().unwrap();
    let observer = doc
        .observe_update_v2(move |_, e| {
            let bytes = &e.update;
            let len = bytes.len() as u32;
            cb(state, len, bytes.as_ptr() as *const c_char)
        })
        .unwrap();
    let subscription_id: u32 = observer.into();
    subscription_id
}

#[no_mangle]
pub unsafe extern "C" fn ydoc_unobserve_updates_v1(doc: *mut Doc, subscription_id: u32) {
    let doc = doc.as_ref().unwrap();
    doc.unobserve_update_v1(subscription_id as SubscriptionId);
}

#[no_mangle]
pub unsafe extern "C" fn ydoc_unobserve_updates_v2(doc: *mut Doc, subscription_id: u32) {
    let doc = doc.as_ref().unwrap();
    doc.unobserve_update_v2(subscription_id as SubscriptionId);
}

#[no_mangle]
pub unsafe extern "C" fn ydoc_observe_after_transaction(
    doc: *mut Doc,
    state: *mut c_void,
    cb: extern "C" fn(*mut c_void, *mut YAfterTransactionEvent),
) -> u32 {
    let doc = doc.as_ref().unwrap();
    let observer = doc
        .observe_transaction_cleanup(move |_, e| {
            let mut event = YAfterTransactionEvent::new(e);
            cb(state, (&mut event) as *mut _);
        })
        .unwrap();
    let subscription_id: u32 = observer.into();
    subscription_id
}

#[no_mangle]
pub unsafe extern "C" fn ydoc_unobserve_after_transaction(doc: *mut Doc, subscription_id: u32) {
    let doc = doc.as_ref().unwrap();
    doc.unobserve_transaction_cleanup(subscription_id as SubscriptionId);
}

#[no_mangle]
pub unsafe extern "C" fn ydoc_observe_subdocs(
    doc: *mut Doc,
    state: *mut c_void,
    cb: extern "C" fn(*mut c_void, *mut YSubdocsEvent),
) -> u32 {
    let doc = doc.as_mut().unwrap();
    let observer = doc
        .observe_subdocs(move |_, e| {
            let mut event = YSubdocsEvent::new(e);
            cb(state, (&mut event) as *mut _);
        })
        .unwrap();
    let subscription_id: u32 = observer.into();
    subscription_id
}

#[no_mangle]
pub unsafe extern "C" fn ydoc_unobserve_subdocs(doc: *mut Doc, subscription_id: u32) {
    let doc = doc.as_ref().unwrap();
    doc.unobserve_subdocs(subscription_id as SubscriptionId);
}

#[no_mangle]
pub unsafe extern "C" fn ydoc_observe_clear(
    doc: *mut Doc,
    state: *mut c_void,
    cb: extern "C" fn(*mut c_void, *mut Doc),
) -> u32 {
    let doc = doc.as_mut().unwrap();
    let observer = doc
        .observe_destroy(move |_, e| cb(state, e as *const Doc as *mut _))
        .unwrap();
    let subscription_id: u32 = observer.into();
    subscription_id
}

#[no_mangle]
pub unsafe extern "C" fn ydoc_unobserve_clear(doc: *mut Doc, subscription_id: u32) {
    let doc = doc.as_ref().unwrap();
    doc.unobserve_destroy(subscription_id as SubscriptionId);
}

/// Manually send a load request to a parent document of this subdoc.
#[no_mangle]
pub unsafe extern "C" fn ydoc_load(doc: *mut Doc, parent_txn: *mut Transaction) {
    let doc = doc.as_ref().unwrap();
    let txn = parent_txn.as_mut().unwrap();
    if let Some(txn) = txn.as_mut() {
        doc.load(txn)
    } else {
        panic!("ydoc_load: passed read-only parent transaction, where read-write one was expected")
    }
}

/// Destroys current document, sending a 'destroy' event and clearing up all the event callbacks
/// registered.
#[no_mangle]
pub unsafe extern "C" fn ydoc_clear(doc: *mut Doc, parent_txn: *mut Transaction) {
    let doc = doc.as_mut().unwrap();
    let txn = parent_txn.as_mut().unwrap();
    if let Some(txn) = txn.as_mut() {
        doc.destroy(txn)
    } else {
        panic!("ydoc_clear: passed read-only parent transaction, where read-write one was expected")
    }
}

/// Starts a new read-only transaction on a given document. All other operations happen in context
/// of a transaction. Yrs transactions do not follow ACID rules. Once a set of operations is
/// complete, a transaction can be finished using `ytransaction_commit` function.
///
/// Returns `NULL` if read-only transaction couldn't be created, i.e. when another read-write
/// transaction is already opened.
#[no_mangle]
pub unsafe extern "C" fn ydoc_read_transaction(doc: *mut Doc) -> *mut Transaction {
    assert!(!doc.is_null());

    let doc = doc.as_mut().unwrap();
    if let Ok(txn) = doc.try_transact() {
        Box::into_raw(Box::new(Transaction::read_only(txn)))
    } else {
        null_mut()
    }
}

/// Starts a new read-write transaction on a given document. All other operations happen in context
/// of a transaction. Yrs transactions do not follow ACID rules. Once a set of operations is
/// complete, a transaction can be finished using `ytransaction_commit` function.
///
/// `origin_len` and `origin` are optional parameters to specify a byte sequence used to mark
/// the origin of this transaction (eg. you may decide to give different origins for transaction
/// applying remote updates). These can be used by event handlers or `UndoManager` to perform
/// specific actions. If origin should not be set, call `ydoc_write_transaction(doc, 0, NULL)`.
///
/// Returns `NULL` if read-write transaction couldn't be created, i.e. when another transaction is
/// already opened.
#[no_mangle]
pub unsafe extern "C" fn ydoc_write_transaction(
    doc: *mut Doc,
    origin_len: u32,
    origin: *const c_char,
) -> *mut Transaction {
    assert!(!doc.is_null());

    let doc = doc.as_mut().unwrap();
    if origin_len == 0 {
        if let Ok(txn) = doc.try_transact_mut() {
            Box::into_raw(Box::new(Transaction::read_write(txn)))
        } else {
            null_mut()
        }
    } else {
        let origin = std::slice::from_raw_parts(origin as *const u8, origin_len as usize);
        if let Ok(txn) = doc.try_transact_mut_with(origin) {
            Box::into_raw(Box::new(Transaction::read_write(txn)))
        } else {
            null_mut()
        }
    }
}

/// Starts a new read-write transaction on a given branches document. All other operations happen in
/// context of a transaction. Yrs transactions do not follow ACID rules. Once a set of operations is
/// complete, a transaction can be finished using `ytransaction_commit` function.
///
/// Returns `NULL` if read-write transaction couldn't be created, i.e. when another transaction is
/// already opened.
#[no_mangle]
pub unsafe extern "C" fn ybranch_write_transaction(branch: *mut Branch) -> *mut Transaction {
    assert!(!branch.is_null());

    let branch = branch.as_mut().unwrap();
    if let Ok(txn) = branch.try_transact_mut() {
        Box::into_raw(Box::new(Transaction::read_write(txn)))
    } else {
        null_mut()
    }
}

/// Starts a new read-only transaction on a given branches document. All other operations happen in
/// context of a transaction. Yrs transactions do not follow ACID rules. Once a set of operations is
/// complete, a transaction can be finished using `ytransaction_commit` function.
///
/// Returns `NULL` if read-only transaction couldn't be created, i.e. when another read-write
/// transaction is already opened.
#[no_mangle]
pub unsafe extern "C" fn ybranch_read_transaction(branch: *mut Branch) -> *mut Transaction {
    assert!(!branch.is_null());

    let doc = branch.as_mut().unwrap();
    if let Ok(txn) = doc.try_transact() {
        Box::into_raw(Box::new(Transaction::read_only(txn)))
    } else {
        null_mut()
    }
}

/// Returns a list of subdocs existing within current document.
#[no_mangle]
pub unsafe extern "C" fn ytransaction_subdocs(
    txn: *mut Transaction,
    len: *mut u32,
) -> *mut *mut Doc {
    let txn = txn.as_ref().unwrap();
    let subdocs: Vec<_> = txn
        .subdocs()
        .map(|doc| doc as *const Doc as *mut Doc)
        .collect();
    let out = subdocs.into_boxed_slice();
    *len = out.len() as u32;
    Box::into_raw(out) as *mut _
}

/// Commit and dispose provided read-write transaction. This operation releases allocated resources,
/// triggers update events and performs a storage compression over all operations executed in scope
/// of a current transaction.
#[no_mangle]
pub unsafe extern "C" fn ytransaction_commit(txn: *mut Transaction) {
    assert!(!txn.is_null());
    drop(Box::from_raw(txn)); // transaction is auto-committed when dropped
}

/// Returns `1` if current transaction is of read-write type.
/// Returns `0` if transaction is read-only.
#[no_mangle]
pub unsafe extern "C" fn ytransaction_writeable(txn: *mut Transaction) -> u8 {
    assert!(!txn.is_null());
    if txn.as_ref().unwrap().is_writeable() {
        1
    } else {
        0
    }
}

/// Gets a reference to shared data type instance at the document root-level,
/// identified by its `name`, which must be a null-terminated UTF-8 compatible string.
///
/// Returns `NULL` if no such structure was defined in the document before.
#[no_mangle]
pub unsafe extern "C" fn ytype_get(txn: *mut Transaction, name: *const c_char) -> *mut Branch {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    //NOTE: we're retrieving this as a text, but ultimatelly it doesn't matter as we don't define
    // nor redefine the underlying branch type
    if let Some(txt) = txn.as_mut().unwrap().get_text(name) {
        txt.into_raw_branch()
    } else {
        null_mut()
    }
}

/// Gets or creates a new shared `YText` data type instance as a root-level type of a given document.
/// This structure can later be accessed using its `name`, which must be a null-terminated UTF-8
/// compatible string.
#[no_mangle]
pub unsafe extern "C" fn ytext(doc: *mut Doc, name: *const c_char) -> *mut Branch {
    assert!(!doc.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let txt = doc.as_mut().unwrap().get_or_insert_text(name);
    txt.into_raw_branch()
}

/// Gets or creates a new shared `YArray` data type instance as a root-level type of a given document.
/// This structure can later be accessed using its `name`, which must be a null-terminated UTF-8
/// compatible string.
///
/// Use [yarray_destroy] in order to release pointer returned that way - keep in mind that this will
/// not remove `YArray` instance from the document itself (once created it'll last for the entire
/// lifecycle of a document).
#[no_mangle]
pub unsafe extern "C" fn yarray(doc: *mut Doc, name: *const c_char) -> *mut Branch {
    assert!(!doc.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    doc.as_mut()
        .unwrap()
        .get_or_insert_array(name)
        .into_raw_branch()
}

/// Gets or creates a new shared `YMap` data type instance as a root-level type of a given document.
/// This structure can later be accessed using its `name`, which must be a null-terminated UTF-8
/// compatible string.
///
/// Use [ymap_destroy] in order to release pointer returned that way - keep in mind that this will
/// not remove `YMap` instance from the document itself (once created it'll last for the entire
/// lifecycle of a document).
#[no_mangle]
pub unsafe extern "C" fn ymap(doc: *mut Doc, name: *const c_char) -> *mut Branch {
    assert!(!doc.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    doc.as_mut()
        .unwrap()
        .get_or_insert_map(name)
        .into_raw_branch()
}

/// Gets or creates a new shared `YXmlElement` data type instance as a root-level type of a given
/// document. This structure can later be accessed using its `name`, which must be a null-terminated
/// UTF-8 compatible string.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem(doc: *mut Doc, name: *const c_char) -> *mut Branch {
    assert!(!doc.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    doc.as_mut()
        .unwrap()
        .get_or_insert_xml_element(name)
        .into_raw_branch()
}

/// Gets or creates a new shared `YXmlElement` data type instance as a root-level type of a given
/// document. This structure can later be accessed using its `name`, which must be a null-terminated
/// UTF-8 compatible string.
#[no_mangle]
pub unsafe extern "C" fn yxmlfragment(doc: *mut Doc, name: *const c_char) -> *mut Branch {
    assert!(!doc.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    doc.as_mut()
        .unwrap()
        .get_or_insert_xml_fragment(name)
        .into_raw_branch()
}

/// Gets or creates a new shared `YXmlText` data type instance as a root-level type of a given
/// document. This structure can later be accessed using its `name`, which must be a null-terminated
/// UTF-8 compatible string.
#[no_mangle]
pub unsafe extern "C" fn yxmltext(doc: *mut Doc, name: *const c_char) -> *mut Branch {
    assert!(!doc.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    doc.as_mut()
        .unwrap()
        .get_or_insert_xml_text(name)
        .into_raw_branch()
}

/// Returns a state vector of a current transaction's document, serialized using lib0 version 1
/// encoding. Payload created by this function can then be send over the network to a remote peer,
/// where it can be used as a parameter of [ytransaction_state_diff_v1] in order to produce a delta
/// update payload, that can be send back and applied locally in order to efficiently propagate
/// updates from one peer to another.
///
/// The length of a generated binary will be passed within a `len` out parameter.
///
/// Once no longer needed, a returned binary can be disposed using [ybinary_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn ytransaction_state_vector_v1(
    txn: *const Transaction,
    len: *mut u32,
) -> *mut c_char {
    assert!(!txn.is_null());

    let txn = txn.as_ref().unwrap();
    let state_vector = txn.state_vector();
    let binary = state_vector.encode_v1().into_boxed_slice();

    *len = binary.len() as u32;
    Box::into_raw(binary) as *mut c_char
}

/// Returns a delta difference between current state of a transaction's document and a state vector
/// `sv` encoded as a binary payload using lib0 version 1 encoding (which could be generated using
/// [ytransaction_state_vector_v1]). Such delta can be send back to the state vector's sender in
/// order to propagate and apply (using [ytransaction_apply]) all updates known to a current
/// document, which remote peer was not aware of.
///
/// If passed `sv` pointer is null, the generated diff will be a snapshot containing entire state of
/// the document.
///
/// A length of an encoded state vector payload must be passed as `sv_len` parameter.
///
/// A length of generated delta diff binary will be passed within a `len` out parameter.
///
/// Once no longer needed, a returned binary can be disposed using [ybinary_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn ytransaction_state_diff_v1(
    txn: *const Transaction,
    sv: *const c_char,
    sv_len: u32,
    len: *mut u32,
) -> *mut c_char {
    assert!(!txn.is_null());

    let txn = txn.as_ref().unwrap();
    let sv = {
        if sv.is_null() {
            StateVector::default()
        } else {
            let sv_slice = std::slice::from_raw_parts(sv as *const u8, sv_len as usize);
            if let Ok(sv) = StateVector::decode_v1(sv_slice) {
                sv
            } else {
                return null_mut();
            }
        }
    };

    let mut encoder = EncoderV1::new();
    txn.encode_diff(&sv, &mut encoder);
    let binary = encoder.to_vec().into_boxed_slice();
    *len = binary.len() as u32;
    Box::into_raw(binary) as *mut c_char
}

/// Returns a delta difference between current state of a transaction's document and a state vector
/// `sv` encoded as a binary payload using lib0 version 1 encoding (which could be generated using
/// [ytransaction_state_vector_v1]). Such delta can be send back to the state vector's sender in
/// order to propagate and apply (using [ytransaction_apply_v2]) all updates known to a current
/// document, which remote peer was not aware of.
///
/// If passed `sv` pointer is null, the generated diff will be a snapshot containing entire state of
/// the document.
///
/// A length of an encoded state vector payload must be passed as `sv_len` parameter.
///
/// A length of generated delta diff binary will be passed within a `len` out parameter.
///
/// Once no longer needed, a returned binary can be disposed using [ybinary_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn ytransaction_state_diff_v2(
    txn: *const Transaction,
    sv: *const c_char,
    sv_len: u32,
    len: *mut u32,
) -> *mut c_char {
    assert!(!txn.is_null());

    let txn = txn.as_ref().unwrap();
    let sv = {
        if sv.is_null() {
            StateVector::default()
        } else {
            let sv_slice = std::slice::from_raw_parts(sv as *const u8, sv_len as usize);
            if let Ok(sv) = StateVector::decode_v1(sv_slice) {
                sv
            } else {
                return null_mut();
            }
        }
    };

    let mut encoder = EncoderV2::new();
    txn.encode_diff(&sv, &mut encoder);
    let binary = encoder.to_vec().into_boxed_slice();
    *len = binary.len() as u32;
    Box::into_raw(binary) as *mut c_char
}

/// Returns a snapshot descriptor of a current state of the document. This snapshot information
/// can be then used to encode document data at a particular point in time
/// (see: `ytransaction_encode_state_from_snapshot`).
#[no_mangle]
pub unsafe extern "C" fn ytransaction_snapshot(
    txn: *const Transaction,
    len: *mut u32,
) -> *mut c_char {
    assert!(!txn.is_null());
    let txn = txn.as_ref().unwrap();
    let binary = txn.snapshot().encode_v1().into_boxed_slice();

    *len = binary.len() as u32;
    Box::into_raw(binary) as *mut c_char
}

/// Encodes a state of the document at a point in time specified by the provided `snapshot`
/// (generated by: `ytransaction_snapshot`). This is useful to generate a past view of the document.
///
/// The returned update is binary compatible with Yrs update lib0 v1 encoding, and can be processed
/// with functions dedicated to work on it, like `ytransaction_apply`.
///
/// This function requires document with a GC option flag turned off (otherwise "time travel" would
/// not be a safe operation). If this is not a case, the NULL pointer will be returned.
#[no_mangle]
pub unsafe extern "C" fn ytransaction_encode_state_from_snapshot_v1(
    txn: *const Transaction,
    snapshot: *const c_char,
    snapshot_len: u32,
    len: *mut u32,
) -> *mut c_char {
    assert!(!txn.is_null());
    let txn = txn.as_ref().unwrap();
    let snapshot = {
        let len = snapshot_len as usize;
        let data = std::slice::from_raw_parts(snapshot as *mut u8, len);
        Snapshot::decode_v1(&data).unwrap()
    };
    let mut encoder = EncoderV1::new();
    match txn.encode_state_from_snapshot(&snapshot, &mut encoder) {
        Err(_) => null_mut(),
        Ok(_) => {
            let binary = encoder.to_vec().into_boxed_slice();
            *len = binary.len() as u32;
            Box::into_raw(binary) as *mut c_char
        }
    }
}

/// Encodes a state of the document at a point in time specified by the provided `snapshot`
/// (generated by: `ytransaction_snapshot`). This is useful to generate a past view of the document.
///
/// The returned update is binary compatible with Yrs update lib0 v2 encoding, and can be processed
/// with functions dedicated to work on it, like `ytransaction_apply_v2`.
///
/// This function requires document with a GC option flag turned off (otherwise "time travel" would
/// not be a safe operation). If this is not a case, the NULL pointer will be returned.
#[no_mangle]
pub unsafe extern "C" fn ytransaction_encode_state_from_snapshot_v2(
    txn: *const Transaction,
    snapshot: *const c_char,
    snapshot_len: u32,
    len: *mut u32,
) -> *mut c_char {
    assert!(!txn.is_null());
    let txn = txn.as_ref().unwrap();
    let snapshot = {
        let len = snapshot_len as usize;
        let data = std::slice::from_raw_parts(snapshot as *mut u8, len);
        Snapshot::decode_v1(&data).unwrap()
    };
    let mut encoder = EncoderV2::new();
    match txn.encode_state_from_snapshot(&snapshot, &mut encoder) {
        Err(_) => null_mut(),
        Ok(_) => {
            let binary = encoder.to_vec().into_boxed_slice();
            *len = binary.len() as u32;
            Box::into_raw(binary) as *mut c_char
        }
    }
}

/// Returns a null-terminated UTF-8 encoded string representation of an `update` binary payload,
/// encoded using lib0 v1 encoding.
/// Returns null if update couldn't be parsed into a lib0 v1 formatting.
#[no_mangle]
pub unsafe extern "C" fn yupdate_debug_v1(update: *const c_char, update_len: u32) -> *mut c_char {
    assert!(!update.is_null());

    let data = std::slice::from_raw_parts(update as *const u8, update_len as usize);
    if let Ok(u) = Update::decode_v1(data) {
        let str = format!("{:#?}", u);
        CString::new(str).unwrap().into_raw()
    } else {
        null_mut()
    }
}

/// Returns a null-terminated UTF-8 encoded string representation of an `update` binary payload,
/// encoded using lib0 v2 encoding.
/// Returns null if update couldn't be parsed into a lib0 v2 formatting.
#[no_mangle]
pub unsafe extern "C" fn yupdate_debug_v2(update: *const c_char, update_len: u32) -> *mut c_char {
    assert!(!update.is_null());

    let data = std::slice::from_raw_parts(update as *const u8, update_len as usize);
    if let Ok(u) = Update::decode_v2(data) {
        let str = format!("{:#?}", u);
        CString::new(str).unwrap().into_raw()
    } else {
        null_mut()
    }
}

/// Applies an diff update (generated by `ytransaction_state_diff_v1`) to a local transaction's
/// document.
///
/// A length of generated `diff` binary must be passed within a `diff_len` out parameter.
///
/// Returns an error code in case if transaction succeeded failed:
/// - **0**: success
/// - `ERR_CODE_IO` (**1**): couldn't read data from input stream.
/// - `ERR_CODE_VAR_INT` (**2**): decoded variable integer outside of the expected integer size bounds.
/// - `ERR_CODE_EOS` (**3**): end of stream found when more data was expected.
/// - `ERR_CODE_UNEXPECTED_VALUE` (**4**): decoded enum tag value was not among known cases.
/// - `ERR_CODE_INVALID_JSON` (**5**): failure when trying to decode JSON content.
/// - `ERR_CODE_OTHER` (**6**): other error type than the one specified.
#[no_mangle]
pub unsafe extern "C" fn ytransaction_apply(
    txn: *mut Transaction,
    diff: *const c_char,
    diff_len: u32,
) -> u8 {
    assert!(!txn.is_null());
    assert!(!diff.is_null());

    let update = std::slice::from_raw_parts(diff as *const u8, diff_len as usize);
    let mut decoder = DecoderV1::from(update);
    match Update::decode(&mut decoder) {
        Ok(update) => {
            let txn = txn.as_mut().unwrap();
            let txn = txn
                .as_mut()
                .expect("provided transaction was not writeable");
            txn.apply_update(update);
            0
        }
        Err(e) => err_code(e),
    }
}

/// Applies an diff update (generated by [ytransaction_state_diff_v2]) to a local transaction's
/// document.
///
/// A length of generated `diff` binary must be passed within a `diff_len` out parameter.
///
/// Returns an error code in case if transaction succeeded failed:
/// - **0**: success
/// - `ERR_CODE_IO` (**1**): couldn't read data from input stream.
/// - `ERR_CODE_VAR_INT` (**2**): decoded variable integer outside of the expected integer size bounds.
/// - `ERR_CODE_EOS` (**3**): end of stream found when more data was expected.
/// - `ERR_CODE_UNEXPECTED_VALUE` (**4**): decoded enum tag value was not among known cases.
/// - `ERR_CODE_INVALID_JSON` (**5**): failure when trying to decode JSON content.
/// - `ERR_CODE_OTHER` (**6**): other error type than the one specified.
#[no_mangle]
pub unsafe extern "C" fn ytransaction_apply_v2(
    txn: *mut Transaction,
    diff: *const c_char,
    diff_len: u32,
) -> u8 {
    assert!(!txn.is_null());
    assert!(!diff.is_null());

    let mut update = std::slice::from_raw_parts(diff as *const u8, diff_len as usize);
    match Update::decode_v2(&mut update) {
        Ok(update) => {
            let txn = txn.as_mut().unwrap();
            let txn = txn
                .as_mut()
                .expect("provided transaction was not writeable");
            txn.apply_update(update);
            0
        }
        Err(e) => err_code(e),
    }
}

/// Error code: couldn't read data from input stream.
pub const ERR_CODE_IO: u8 = 1;

/// Error code: decoded variable integer outside of the expected integer size bounds.
pub const ERR_CODE_VAR_INT: u8 = 2;

/// Error code: end of stream found when more data was expected.
pub const ERR_CODE_EOS: u8 = 3;

/// Error code: decoded enum tag value was not among known cases.
pub const ERR_CODE_UNEXPECTED_VALUE: u8 = 4;

/// Error code: failure when trying to decode JSON content.
pub const ERR_CODE_INVALID_JSON: u8 = 5;

/// Error code: other error type than the one specified.
pub const ERR_CODE_OTHER: u8 = 6;

fn err_code(e: Error) -> u8 {
    match e {
        Error::IO(_) => ERR_CODE_IO,
        Error::VarIntSizeExceeded(_) => ERR_CODE_VAR_INT,
        Error::EndOfBuffer(_) => ERR_CODE_EOS,
        Error::UnexpectedValue => ERR_CODE_UNEXPECTED_VALUE,
        Error::Other(_) => ERR_CODE_OTHER,
        Error::InvalidJSON(_) => ERR_CODE_INVALID_JSON,
    }
}

/// Returns the length of the `YText` string content in bytes (without the null terminator character)
#[no_mangle]
pub unsafe extern "C" fn ytext_len(txt: *const Branch, txn: *const Transaction) -> u32 {
    assert!(!txt.is_null());
    let txn = txn.as_ref().unwrap();
    let txt = TextRef::from_raw_branch(txt);
    txt.len(txn)
}

/// Returns a null-terminated UTF-8 encoded string content of a current `YText` shared data type.
///
/// Generated string resources should be released using [ystring_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn ytext_string(txt: *const Branch, txn: *const Transaction) -> *mut c_char {
    assert!(!txt.is_null());

    let txn = txn.as_ref().unwrap();
    let txt = TextRef::from_raw_branch(txt);
    let str = txt.get_string(txn);
    CString::new(str).unwrap().into_raw()
}

/// Inserts a null-terminated UTF-8 encoded string a given `index`. `index` value must be between
/// 0 and a length of a `YText` (inclusive, accordingly to [ytext_len] return value), otherwise this
/// function will panic.
///
/// A `str` parameter must be a null-terminated UTF-8 encoded string. This function doesn't take
/// ownership over a passed value - it will be copied and therefore a string parameter must be
/// released by the caller.
///
/// A nullable pointer with defined `attrs` will be used to wrap provided text with
/// a formatting blocks. `attrs` must be a map-like type.
#[no_mangle]
pub unsafe extern "C" fn ytext_insert(
    txt: *const Branch,
    txn: *mut Transaction,
    index: u32,
    value: *const c_char,
    attrs: *const YInput,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!value.is_null());

    let chunk = CStr::from_ptr(value).to_str().unwrap();
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");
    let txt = TextRef::from_raw_branch(txt);
    let index = index as u32;
    if attrs.is_null() {
        txt.insert(txn, index, chunk)
    } else {
        if let Some(attrs) = map_attrs(attrs.read().into()) {
            txt.insert_with_attributes(txn, index, chunk, attrs)
        } else {
            panic!("ytext_insert: passed attributes are not of map type")
        }
    }
}

/// Wraps an existing piece of text within a range described by `index`-`len` parameters with
/// formatting blocks containing provided `attrs` metadata. `attrs` must be a map-like type.
#[no_mangle]
pub unsafe extern "C" fn ytext_format(
    txt: *const Branch,
    txn: *mut Transaction,
    index: u32,
    len: u32,
    attrs: *const YInput,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!attrs.is_null());

    if let Some(attrs) = map_attrs(attrs.read().into()) {
        let txt = TextRef::from_raw_branch(txt);
        let txn = txn.as_mut().unwrap();
        let txn = txn
            .as_mut()
            .expect("provided transaction was not writeable");
        let index = index as u32;
        let len = len as u32;
        txt.format(txn, index, len, attrs);
    } else {
        panic!("ytext_format: passed attributes are not of map type")
    }
}

/// Inserts an embed content given `index`. `index` value must be between 0 and a length of a
/// `YText` (inclusive, accordingly to [ytext_len] return value), otherwise this
/// function will panic.
///
/// A `str` parameter must be a null-terminated UTF-8 encoded string. This function doesn't take
/// ownership over a passed value - it will be copied and therefore a string parameter must be
/// released by the caller.
///
/// A nullable pointer with defined `attrs` will be used to wrap provided text with
/// a formatting blocks. `attrs` must be a map-like type.
#[no_mangle]
pub unsafe extern "C" fn ytext_insert_embed(
    txt: *const Branch,
    txn: *mut Transaction,
    index: u32,
    content: *const YInput,
    attrs: *const YInput,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!content.is_null());

    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");
    let txt = TextRef::from_raw_branch(txt);
    let index = index as u32;
    let content: Any = content.read().into();
    if attrs.is_null() {
        txt.insert_embed(txn, index, content);
    } else {
        if let Some(attrs) = map_attrs(attrs.read().into()) {
            txt.insert_embed_with_attributes(txn, index, content, attrs);
        } else {
            panic!("ytext_insert_embed: passed attributes are not of map type")
        }
    }
}

fn map_attrs(attrs: Any) -> Option<Attrs> {
    if let Any::Map(attrs) = attrs {
        let attrs = attrs.into_iter().map(|(k, v)| (k.into(), v)).collect();
        Some(attrs)
    } else {
        None
    }
}

/// Removes a range of characters, starting a a given `index`. This range must fit within the bounds
/// of a current `YText`, otherwise this function call will fail.
///
/// An `index` value must be between 0 and the length of a `YText` (exclusive, accordingly to
/// [ytext_len] return value).
///
/// A `length` must be lower or equal number of characters (counted as UTF chars depending on the
/// encoding configured by `YDoc`) from `index` position to the end of of the string.
#[no_mangle]
pub unsafe extern "C" fn ytext_remove_range(
    txt: *const Branch,
    txn: *mut Transaction,
    index: u32,
    length: u32,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");
    let txt = TextRef::from_raw_branch(txt);
    txt.remove_range(txn, index as u32, length as u32)
}

/// Returns a number of elements stored within current instance of `YArray`.
#[no_mangle]
pub unsafe extern "C" fn yarray_len(array: *const Branch) -> u32 {
    assert!(!array.is_null());

    let array = array.as_ref().unwrap();
    array.len() as u32
}

/// Returns a pointer to a `YOutput` value stored at a given `index` of a current `YArray`.
/// If `index` is outside of the bounds of an array, a null pointer will be returned.
///
/// A value returned should be eventually released using [youtput_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yarray_get(
    array: *const Branch,
    txn: *const Transaction,
    index: u32,
) -> *mut YOutput {
    assert!(!array.is_null());

    let array = ArrayRef::from_raw_branch(array);
    let txn = txn.as_ref().unwrap();

    if let Some(val) = array.get(txn, index as u32) {
        Box::into_raw(Box::new(YOutput::from(val)))
    } else {
        std::ptr::null_mut()
    }
}

/// Inserts a range of `items` into current `YArray`, starting at given `index`. An `items_len`
/// parameter is used to determine the size of `items` array - it can also be used to insert
/// a single element given its pointer.
///
/// An `index` value must be between 0 and (inclusive) length of a current array (use [yarray_len]
/// to determine its length), otherwise it will panic at runtime.
///
/// `YArray` doesn't take ownership over the inserted `items` data - their contents are being copied
/// into array structure - therefore caller is responsible for freeing all memory associated with
/// input params.
#[no_mangle]
pub unsafe extern "C" fn yarray_insert_range(
    array: *const Branch,
    txn: *mut Transaction,
    index: u32,
    items: *const YInput,
    items_len: u32,
) {
    assert!(!array.is_null());
    assert!(!txn.is_null());
    assert!(!items.is_null());

    let array = ArrayRef::from_raw_branch(array);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");

    let ptr = items;
    let mut i = 0;
    let mut j = index as u32;
    let len = items_len as isize;
    while i < len {
        let mut vec: Vec<Any> = Vec::default();

        // try read as many values a JSON-like primitives and insert them at once
        while i < len {
            let val = ptr.offset(i).read();
            if val.tag <= 0 {
                let any = val.into();
                vec.push(any);
            } else {
                break;
            }
            i += 1;
        }

        if !vec.is_empty() {
            let len = vec.len() as u32;
            array.insert_range(txn, j, vec);
            j += len;
        } else {
            let val = ptr.offset(i).read();
            array.insert(txn, j, val);
            i += 1;
            j += 1;
        }
    }
}

/// Removes a `len` of consecutive range of elements from current `array` instance, starting at
/// a given `index`. Range determined by `index` and `len` must fit into boundaries of an array,
/// otherwise it will panic at runtime.
#[no_mangle]
pub unsafe extern "C" fn yarray_remove_range(
    array: *const Branch,
    txn: *mut Transaction,
    index: u32,
    len: u32,
) {
    assert!(!array.is_null());
    assert!(!txn.is_null());

    let array = ArrayRef::from_raw_branch(array);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");

    array.remove_range(txn, index as u32, len as u32)
}

#[no_mangle]
pub unsafe extern "C" fn yarray_move(
    array: *const Branch,
    txn: *mut Transaction,
    source: u32,
    target: u32,
) {
    assert!(!array.is_null());
    assert!(!txn.is_null());

    let array = ArrayRef::from_raw_branch(array);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");

    array.move_to(txn, source as u32, target as u32)
}

/// Returns an iterator, which can be used to traverse over all elements of an `array` (`array`'s
/// length can be determined using [yarray_len] function).
///
/// Use [yarray_iter_next] function in order to retrieve a consecutive array elements.
/// Use [yarray_iter_destroy] function in order to close the iterator and release its resources.
#[no_mangle]
pub unsafe extern "C" fn yarray_iter(
    array: *const Branch,
    txn: *mut Transaction,
) -> *mut ArrayIter {
    assert!(!array.is_null());
    assert!(!txn.is_null());

    let txn = txn.as_ref().unwrap();
    let array = &ArrayRef::from_raw_branch(array) as *const ArrayRef;
    Box::into_raw(Box::new(ArrayIter(array.as_ref().unwrap().iter(txn))))
}

/// Releases all of an `YArray` iterator resources created by calling [yarray_iter].
#[no_mangle]
pub unsafe extern "C" fn yarray_iter_destroy(iter: *mut ArrayIter) {
    if !iter.is_null() {
        drop(Box::from_raw(iter))
    }
}

/// Moves current `YArray` iterator over to a next element, returning a pointer to it. If an iterator
/// comes to an end of an array, a null pointer will be returned.
///
/// Returned values should be eventually released using [youtput_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yarray_iter_next(iterator: *mut ArrayIter) -> *mut YOutput {
    assert!(!iterator.is_null());

    let iter = iterator.as_mut().unwrap();
    if let Some(v) = iter.0.next() {
        let out = YOutput::from(v);
        Box::into_raw(Box::new(out))
    } else {
        std::ptr::null_mut()
    }
}

/// Returns an iterator, which can be used to traverse over all key-value pairs of a `map`.
///
/// Use [ymap_iter_next] function in order to retrieve a consecutive (**unordered**) map entries.
/// Use [ymap_iter_destroy] function in order to close the iterator and release its resources.
#[no_mangle]
pub unsafe extern "C" fn ymap_iter(map: *const Branch, txn: *const Transaction) -> *mut MapIter {
    assert!(!map.is_null());

    let txn = txn.as_ref().unwrap();
    let map = &MapRef::from_raw_branch(map) as *const MapRef;
    Box::into_raw(Box::new(MapIter(map.as_ref().unwrap().iter(txn))))
}

/// Releases all of an `YMap` iterator resources created by calling [ymap_iter].
#[no_mangle]
pub unsafe extern "C" fn ymap_iter_destroy(iter: *mut MapIter) {
    if !iter.is_null() {
        drop(Box::from_raw(iter))
    }
}

/// Moves current `YMap` iterator over to a next entry, returning a pointer to it. If an iterator
/// comes to an end of a map, a null pointer will be returned. Yrs maps are unordered and so are
/// their iterators.
///
/// Returned values should be eventually released using [ymap_entry_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn ymap_iter_next(iter: *mut MapIter) -> *mut YMapEntry {
    assert!(!iter.is_null());

    let iter = iter.as_mut().unwrap();
    if let Some((key, value)) = iter.0.next() {
        Box::into_raw(Box::new(YMapEntry::new(key, value)))
    } else {
        std::ptr::null_mut()
    }
}

/// Returns a number of entries stored within a `map`.
#[no_mangle]
pub unsafe extern "C" fn ymap_len(map: *const Branch, txn: *const Transaction) -> u32 {
    assert!(!map.is_null());

    let txn = txn.as_ref().unwrap();
    let map = MapRef::from_raw_branch(map);

    map.len(txn) as u32
}

/// Inserts a new entry (specified as `key`-`value` pair) into a current `map`. If entry under such
/// given `key` already existed, its corresponding value will be replaced.
///
/// A `key` must be a null-terminated UTF-8 encoded string, which contents will be copied into
/// a `map` (therefore it must be freed by the function caller).
///
/// A `value` content is being copied into a `map`, therefore any of its content must be freed by
/// the function caller.
#[no_mangle]
pub unsafe extern "C" fn ymap_insert(
    map: *const Branch,
    txn: *mut Transaction,
    key: *const c_char,
    value: *const YInput,
) {
    assert!(!map.is_null());
    assert!(!txn.is_null());
    assert!(!key.is_null());
    assert!(!value.is_null());

    let cstr = CStr::from_ptr(key);
    let key = cstr.to_str().unwrap().to_string();

    let map = MapRef::from_raw_branch(map);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");

    map.insert(txn, key, value.read());
}

/// Removes a `map` entry, given its `key`. Returns `1` if the corresponding entry was successfully
/// removed or `0` if no entry with a provided `key` has been found inside of a `map`.
///
/// A `key` must be a null-terminated UTF-8 encoded string.
#[no_mangle]
pub unsafe extern "C" fn ymap_remove(
    map: *const Branch,
    txn: *mut Transaction,
    key: *const c_char,
) -> u8 {
    assert!(!map.is_null());
    assert!(!txn.is_null());
    assert!(!key.is_null());

    let key = CStr::from_ptr(key).to_str().unwrap();

    let map = MapRef::from_raw_branch(map);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");

    if let Some(_) = map.remove(txn, key) {
        Y_TRUE
    } else {
        Y_FALSE
    }
}

/// Returns a value stored under the provided `key`, or a null pointer if no entry with such `key`
/// has been found in a current `map`. A returned value is allocated by this function and therefore
/// should be eventually released using [youtput_destroy] function.
///
/// A `key` must be a null-terminated UTF-8 encoded string.
#[no_mangle]
pub unsafe extern "C" fn ymap_get(
    map: *const Branch,
    txn: *const Transaction,
    key: *const c_char,
) -> *mut YOutput {
    assert!(!map.is_null());
    assert!(!key.is_null());
    assert!(!txn.is_null());

    let txn = txn.as_ref().unwrap();
    let key = CStr::from_ptr(key).to_str().unwrap();

    let map = MapRef::from_raw_branch(map);

    if let Some(value) = map.get(txn, key) {
        Box::into_raw(Box::new(YOutput::from(value)))
    } else {
        std::ptr::null_mut()
    }
}

/// Removes all entries from a current `map`.
#[no_mangle]
pub unsafe extern "C" fn ymap_remove_all(map: *const Branch, txn: *mut Transaction) {
    assert!(!map.is_null());
    assert!(!txn.is_null());

    let map = MapRef::from_raw_branch(map);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");

    map.clear(txn);
}

/// Return a name (or an XML tag) of a current `YXmlElement`. Root-level XML nodes use "UNDEFINED" as
/// their tag names.
///
/// Returned value is a null-terminated UTF-8 string, which must be released using [ystring_destroy]
/// function.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_tag(xml: *const Branch) -> *mut c_char {
    assert!(!xml.is_null());
    let xml = XmlElementRef::from_raw_branch(xml);
    let tag = xml.tag();
    CString::new(tag).unwrap().into_raw()
}

/// Converts current `YXmlElement` together with its children and attributes into a flat string
/// representation (no padding) eg. `<UNDEFINED><title key="value">sample text</title></UNDEFINED>`.
///
/// Returned value is a null-terminated UTF-8 string, which must be released using [ystring_destroy]
/// function.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_string(
    xml: *const Branch,
    txn: *const Transaction,
) -> *mut c_char {
    assert!(!xml.is_null());

    let txn = txn.as_ref().unwrap();
    let xml = XmlElementRef::from_raw_branch(xml);

    let str = xml.get_string(txn);
    CString::new(str).unwrap().into_raw()
}

/// Inserts an XML attribute described using `attr_name` and `attr_value`. If another attribute with
/// the same name already existed, its value will be replaced with a provided one.
///
/// Both `attr_name` and `attr_value` must be a null-terminated UTF-8 encoded strings. Their
/// contents are being copied, therefore it's up to a function caller to properly release them.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_insert_attr(
    xml: *const Branch,
    txn: *mut Transaction,
    attr_name: *const c_char,
    attr_value: *const c_char,
) {
    assert!(!xml.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());
    assert!(!attr_value.is_null());

    let xml = XmlElementRef::from_raw_branch(xml);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");

    let key = CStr::from_ptr(attr_name).to_str().unwrap();
    let value = CStr::from_ptr(attr_value).to_str().unwrap();

    xml.insert_attribute(txn, key, value);
}

/// Removes an attribute from a current `YXmlElement`, given its name.
///
/// An `attr_name`must be a null-terminated UTF-8 encoded string.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_remove_attr(
    xml: *const Branch,
    txn: *mut Transaction,
    attr_name: *const c_char,
) {
    assert!(!xml.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());

    let xml = XmlElementRef::from_raw_branch(xml);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");

    let key = CStr::from_ptr(attr_name).to_str().unwrap();
    xml.remove_attribute(txn, &key);
}

/// Returns the value of a current `YXmlElement`, given its name, or a null pointer if not attribute
/// with such name has been found. Returned pointer is a null-terminated UTF-8 encoded string, which
/// should be released using [ystring_destroy] function.
///
/// An `attr_name` must be a null-terminated UTF-8 encoded string.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_get_attr(
    xml: *const Branch,
    txn: *const Transaction,
    attr_name: *const c_char,
) -> *mut c_char {
    assert!(!xml.is_null());
    assert!(!attr_name.is_null());
    assert!(!txn.is_null());

    let xml = XmlElementRef::from_raw_branch(xml);

    let key = CStr::from_ptr(attr_name).to_str().unwrap();
    let txn = txn.as_ref().unwrap();
    if let Some(value) = xml.get_attribute(txn, key) {
        CString::new(value).unwrap().into_raw()
    } else {
        std::ptr::null_mut()
    }
}

/// Returns an iterator over the `YXmlElement` attributes.
///
/// Use [yxmlattr_iter_next] function in order to retrieve a consecutive (**unordered**) attributes.
/// Use [yxmlattr_iter_destroy] function in order to close the iterator and release its resources.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_attr_iter(
    xml: *const Branch,
    txn: *const Transaction,
) -> *mut Attributes {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = &XmlElementRef::from_raw_branch(xml) as *const XmlElementRef;
    let txn = txn.as_ref().unwrap();
    Box::into_raw(Box::new(Attributes(xml.as_ref().unwrap().attributes(txn))))
}

/// Returns an iterator over the `YXmlText` attributes.
///
/// Use [yxmlattr_iter_next] function in order to retrieve a consecutive (**unordered**) attributes.
/// Use [yxmlattr_iter_destroy] function in order to close the iterator and release its resources.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_attr_iter(
    xml: *const Branch,
    txn: *const Transaction,
) -> *mut Attributes {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = &XmlTextRef::from_raw_branch(xml) as *const XmlTextRef;
    let txn = txn.as_ref().unwrap();
    Box::into_raw(Box::new(Attributes(xml.as_ref().unwrap().attributes(txn))))
}

/// Releases all of attributes iterator resources created by calling [yxmlelem_attr_iter]
/// or [yxmltext_attr_iter].
#[no_mangle]
pub unsafe extern "C" fn yxmlattr_iter_destroy(iterator: *mut Attributes) {
    if !iterator.is_null() {
        drop(Box::from_raw(iterator))
    }
}

/// Returns a next XML attribute from an `iterator`. Attributes are returned in an unordered
/// manner. Once `iterator` reaches the end of attributes collection, a null pointer will be
/// returned.
///
/// Returned value should be eventually released using [yxmlattr_destroy].
#[no_mangle]
pub unsafe extern "C" fn yxmlattr_iter_next(iterator: *mut Attributes) -> *mut YXmlAttr {
    assert!(!iterator.is_null());

    let iter = iterator.as_mut().unwrap();

    if let Some((name, value)) = iter.0.next() {
        Box::into_raw(Box::new(YXmlAttr {
            name: CString::new(name).unwrap().into_raw(),
            value: CString::new(value).unwrap().into_raw(),
        }))
    } else {
        std::ptr::null_mut()
    }
}

/// Returns a next sibling of a current XML node, which can be either another `YXmlElement`
/// or a `YXmlText`. Together with [yxmlelem_first_child] it may be used to iterate over the direct
/// children of an XML node (in order to iterate over the nested XML structure use
/// [yxmlelem_tree_walker]).
///
/// If current `YXmlElement` is the last child, this function returns a null pointer.
/// A returned value should be eventually released using [youtput_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yxml_next_sibling(
    xml: *const Branch,
    txn: *const Transaction,
) -> *mut YOutput {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = XmlElementRef::from_raw_branch(xml);
    let txn = txn.as_ref().unwrap();

    let mut siblings = xml.siblings(txn);
    if let Some(next) = siblings.next() {
        match next {
            XmlNode::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            XmlNode::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
            XmlNode::Fragment(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlFragment(v)))),
        }
    } else {
        null_mut()
    }
}

/// Returns a previous sibling of a current XML node, which can be either another `YXmlElement`
/// or a `YXmlText`.
///
/// If current `YXmlElement` is the first child, this function returns a null pointer.
/// A returned value should be eventually released using [youtput_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yxml_prev_sibling(
    xml: *const Branch,
    txn: *const Transaction,
) -> *mut YOutput {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = XmlElementRef::from_raw_branch(xml);
    let txn = txn.as_ref().unwrap();

    let mut siblings = xml.siblings(txn);
    if let Some(next) = siblings.next_back() {
        match next {
            XmlNode::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            XmlNode::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
            XmlNode::Fragment(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlFragment(v)))),
        }
    } else {
        null_mut()
    }
}

/// Returns a parent `YXmlElement` of a current node, or null pointer when current `YXmlElement` is
/// a root-level shared data type.
///
/// A returned value should be eventually released using [youtput_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_parent(xml: *const Branch) -> *mut Branch {
    assert!(!xml.is_null());

    let xml = XmlElementRef::from_raw_branch(xml);

    if let Some(parent) = xml.parent() {
        let branch = parent.as_ptr();
        branch.deref() as *const Branch as *mut Branch
    } else {
        std::ptr::null_mut()
    }
}

/// Returns a number of child nodes (both `YXmlElement` and `YXmlText`) living under a current XML
/// element. This function doesn't count a recursive nodes, only direct children of a current node.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_child_len(xml: *const Branch, txn: *const Transaction) -> u32 {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let txn = txn.as_ref().unwrap();
    let xml = XmlElementRef::from_raw_branch(xml);

    xml.len(txn) as u32
}

/// Returns a first child node of a current `YXmlElement`, or null pointer if current XML node is
/// empty. Returned value could be either another `YXmlElement` or `YXmlText`.
///
/// A returned value should be eventually released using [youtput_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_first_child(xml: *const Branch) -> *mut YOutput {
    assert!(!xml.is_null());

    let xml = XmlElementRef::from_raw_branch(xml);

    if let Some(value) = xml.first_child() {
        match value {
            XmlNode::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            XmlNode::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
            XmlNode::Fragment(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlFragment(v)))),
        }
    } else {
        std::ptr::null_mut()
    }
}

/// Returns an iterator over a nested recursive structure of a current `YXmlElement`, starting from
/// first of its children. Returned values can be either `YXmlElement` or `YXmlText` nodes.
///
/// Use [yxmlelem_tree_walker_next] function in order to iterate over to a next node.
/// Use [yxmlelem_tree_walker_destroy] function to release resources used by the iterator.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_tree_walker(
    xml: *const Branch,
    txn: *const Transaction,
) -> *mut TreeWalker {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let txn = txn.as_ref().unwrap();
    let xml = &XmlElementRef::from_raw_branch(xml) as *const XmlElementRef;
    Box::into_raw(Box::new(TreeWalker(xml.as_ref().unwrap().successors(txn))))
}

/// Releases resources associated with a current XML tree walker iterator.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_tree_walker_destroy(iter: *mut TreeWalker) {
    if !iter.is_null() {
        drop(Box::from_raw(iter))
    }
}

/// Moves current `iterator` to a next value (either `YXmlElement` or `YXmlText`), returning its
/// pointer or a null, if an `iterator` already reached the last successor node.
///
/// Values returned by this function should be eventually released using [youtput_destroy].
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_tree_walker_next(iterator: *mut TreeWalker) -> *mut YOutput {
    assert!(!iterator.is_null());

    let iter = iterator.as_mut().unwrap();

    if let Some(next) = iter.0.next() {
        match next {
            XmlNode::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            XmlNode::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
            XmlNode::Fragment(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlFragment(v)))),
        }
    } else {
        std::ptr::null_mut()
    }
}

/// Inserts an `YXmlElement` as a child of a current node at the given `index` and returns its
/// pointer. Node created this way will have a given `name` as its tag (eg. `p` for `<p></p>` node).
///
/// An `index` value must be between 0 and (inclusive) length of a current XML element (use
/// [yxmlelem_child_len] function to determine its length).
///
/// A `name` must be a null-terminated UTF-8 encoded string, which will be copied into current
/// document. Therefore `name` should be freed by the function caller.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_insert_elem(
    xml: *const Branch,
    txn: *mut Transaction,
    index: u32,
    name: *const c_char,
) -> *mut Branch {
    assert!(!xml.is_null());
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let xml = XmlElementRef::from_raw_branch(xml);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");

    let name = CStr::from_ptr(name).to_str().unwrap();
    xml.insert(txn, index as u32, XmlElementPrelim::empty(name))
        .into_raw_branch()
}

/// Inserts an `YXmlText` as a child of a current node at the given `index` and returns its
/// pointer.
///
/// An `index` value must be between 0 and (inclusive) length of a current XML element (use
/// [yxmlelem_child_len] function to determine its length).
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_insert_text(
    xml: *const Branch,
    txn: *mut Transaction,
    index: u32,
) -> *mut Branch {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = XmlElementRef::from_raw_branch(xml);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");
    xml.insert(txn, index as u32, XmlTextPrelim::new(""))
        .into_raw_branch()
}

/// Removes a consecutive range of child elements (of specified length) from the current
/// `YXmlElement`, starting at the given `index`. Specified range must fit into boundaries of current
/// XML node children, otherwise this function will panic at runtime.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_remove_range(
    xml: *const Branch,
    txn: *mut Transaction,
    index: u32,
    len: u32,
) {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = XmlElementRef::from_raw_branch(xml);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");

    xml.remove_range(txn, index as u32, len as u32)
}

/// Returns an XML child node (either a `YXmlElement` or `YXmlText`) stored at a given `index` of
/// a current `YXmlElement`. Returns null pointer if `index` was outside of the bound of current XML
/// node children.
///
/// Returned value should be eventually released using [youtput_destroy].
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_get(
    xml: *const Branch,
    txn: *const Transaction,
    index: u32,
) -> *const YOutput {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = XmlElementRef::from_raw_branch(xml);
    let txn = txn.as_ref().unwrap();

    if let Some(child) = xml.get(txn, index as u32) {
        match child {
            XmlNode::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            XmlNode::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
            XmlNode::Fragment(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlFragment(v)))),
        }
    } else {
        std::ptr::null()
    }
}

/// Returns the length of the `YXmlText` string content in bytes (without the null terminator
/// character)
#[no_mangle]
pub unsafe extern "C" fn yxmltext_len(txt: *const Branch, txn: *const Transaction) -> u32 {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let txn = txn.as_ref().unwrap();
    let txt = XmlTextRef::from_raw_branch(txt);

    txt.len(txn) as u32
}

/// Returns a null-terminated UTF-8 encoded string content of a current `YXmlText` shared data type.
///
/// Generated string resources should be released using [ystring_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_string(
    txt: *const Branch,
    txn: *const Transaction,
) -> *mut c_char {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let txn = txn.as_ref().unwrap();
    let txt = XmlTextRef::from_raw_branch(txt);

    let str = txt.get_string(txn);
    CString::new(str).unwrap().into_raw()
}

/// Inserts a null-terminated UTF-8 encoded string a a given `index`. `index` value must be between
/// 0 and a length of a `YXmlText` (inclusive, accordingly to [yxmltext_len] return value), otherwise
/// this function will panic.
///
/// A `str` parameter must be a null-terminated UTF-8 encoded string. This function doesn't take
/// ownership over a passed value - it will be copied and therefore a string parameter must be
/// released by the caller.
///
/// A nullable pointer with defined `attrs` will be used to wrap provided text with
/// a formatting blocks. `attrs` must be a map-like type.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_insert(
    txt: *const Branch,
    txn: *mut Transaction,
    index: u32,
    str: *const c_char,
    attrs: *const YInput,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!str.is_null());

    let txt = XmlTextRef::from_raw_branch(txt);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");
    let chunk = CStr::from_ptr(str).to_str().unwrap();

    if attrs.is_null() {
        txt.insert(txn, index as u32, chunk)
    } else {
        if let Some(attrs) = map_attrs(attrs.read().into()) {
            txt.insert_with_attributes(txn, index as u32, chunk, attrs)
        } else {
            panic!("yxmltext_insert: passed attributes are not of map type")
        }
    }
}

/// Inserts an embed content given `index`. `index` value must be between 0 and a length of a
/// `YXmlText` (inclusive, accordingly to [ytext_len] return value), otherwise this
/// function will panic.
///
/// A `str` parameter must be a null-terminated UTF-8 encoded string. This function doesn't take
/// ownership over a passed value - it will be copied and therefore a string parameter must be
/// released by the caller.
///
/// A nullable pointer with defined `attrs` will be used to wrap provided text with
/// a formatting blocks. `attrs` must be a map-like type.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_insert_embed(
    txt: *const Branch,
    txn: *mut Transaction,
    index: u32,
    content: *const YInput,
    attrs: *const YInput,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!content.is_null());

    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");
    let txt = XmlTextRef::from_raw_branch(txt);
    let index = index as u32;
    let content: Any = content.read().into();
    if attrs.is_null() {
        txt.insert_embed(txn, index, content);
    } else {
        if let Some(attrs) = map_attrs(attrs.read().into()) {
            txt.insert_embed_with_attributes(txn, index, content, attrs);
        } else {
            panic!("yxmltext_insert_embed: passed attributes are not of map type")
        }
    }
}

/// Wraps an existing piece of text within a range described by `index`-`len` parameters with
/// formatting blocks containing provided `attrs` metadata. `attrs` must be a map-like type.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_format(
    txt: *const Branch,
    txn: *mut Transaction,
    index: u32,
    len: u32,
    attrs: *const YInput,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!attrs.is_null());

    if let Some(attrs) = map_attrs(attrs.read().into()) {
        let txt = XmlTextRef::from_raw_branch(txt);
        let txn = txn.as_mut().unwrap();
        let txn = txn
            .as_mut()
            .expect("provided transaction was not writeable");
        let index = index as u32;
        let len = len as u32;
        txt.format(txn, index, len, attrs);
    } else {
        panic!("yxmltext_format: passed attributes are not of map type")
    }
}

/// Removes a range of characters, starting a a given `index`. This range must fit within the bounds
/// of a current `YXmlText`, otherwise this function call will fail.
///
/// An `index` value must be between 0 and the length of a `YXmlText` (exclusive, accordingly to
/// [yxmltext_len] return value).
///
/// A `length` must be lower or equal number of characters (counted as UTF chars depending on the
/// encoding configured by `YDoc`) from `index` position to the end of of the string.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_remove_range(
    txt: *const Branch,
    txn: *mut Transaction,
    idx: u32,
    len: u32,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let txt = XmlTextRef::from_raw_branch(txt);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");
    txt.remove_range(txn, idx as u32, len as u32)
}

/// Inserts an XML attribute described using `attr_name` and `attr_value`. If another attribute with
/// the same name already existed, its value will be replaced with a provided one.
///
/// Both `attr_name` and `attr_value` must be a null-terminated UTF-8 encoded strings. Their
/// contents are being copied, therefore it's up to a function caller to properly release them.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_insert_attr(
    txt: *const Branch,
    txn: *mut Transaction,
    attr_name: *const c_char,
    attr_value: *const c_char,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());
    assert!(!attr_value.is_null());

    let txt = XmlTextRef::from_raw_branch(txt);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");

    let name = CStr::from_ptr(attr_name).to_str().unwrap();
    let value = CStr::from_ptr(attr_value).to_str().unwrap();

    txt.insert_attribute(txn, name, value)
}

/// Removes an attribute from a current `YXmlText`, given its name.
///
/// An `attr_name`must be a null-terminated UTF-8 encoded string.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_remove_attr(
    txt: *const Branch,
    txn: *mut Transaction,
    attr_name: *const c_char,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());

    let txt = XmlTextRef::from_raw_branch(txt);
    let txn = txn.as_mut().unwrap();
    let txn = txn
        .as_mut()
        .expect("provided transaction was not writeable");
    let name = CStr::from_ptr(attr_name).to_str().unwrap();

    txt.remove_attribute(txn, &name)
}

/// Returns the value of a current `YXmlText`, given its name, or a null pointer if not attribute
/// with such name has been found. Returned pointer is a null-terminated UTF-8 encoded string, which
/// should be released using [ystring_destroy] function.
///
/// An `attr_name` must be a null-terminated UTF-8 encoded string.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_get_attr(
    txt: *const Branch,
    txn: *const Transaction,
    attr_name: *const c_char,
) -> *mut c_char {
    assert!(!txt.is_null());
    assert!(!attr_name.is_null());
    assert!(!txn.is_null());

    let txn = txn.as_ref().unwrap();
    let txt = XmlTextRef::from_raw_branch(txt);
    let name = CStr::from_ptr(attr_name).to_str().unwrap();

    if let Some(value) = txt.get_attribute(txn, name) {
        CString::new(value).unwrap().into_raw()
    } else {
        std::ptr::null_mut()
    }
}

/// Returns a collection of chunks representing pieces of `YText` rich text string grouped together
/// by the same formatting rules and type. `chunks_len` is used to inform about a number of chunks
/// generated this way.
///
/// Returned array needs to be eventually deallocated using `ychunks_destroy`.
#[no_mangle]
pub unsafe extern "C" fn ytext_chunks(
    txt: *const Branch,
    txn: *const Transaction,
    chunks_len: *mut u32,
) -> *mut YChunk {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let txt = TextRef::from_raw_branch(txt);
    let txn = txn.as_ref().unwrap();

    let diffs = txt.diff(txn, YChange::identity);
    let mut chunks: Vec<_> = diffs.into_iter().map(YChunk::from).collect();
    let out = chunks.into_boxed_slice();
    *chunks_len = out.len() as u32;
    Box::into_raw(out) as *mut _
}

/// Deallocates result of `ytext_chunks` method.
#[no_mangle]
pub unsafe extern "C" fn ychunks_destroy(chunks: *mut YChunk, len: u32) {
    drop(Vec::from_raw_parts(chunks, len as usize, len as usize));
}

pub const YCHANGE_ADD: i8 = 1;
pub const YCHANGE_RETAIN: i8 = 0;
pub const YCHANGE_REMOVE: i8 = -1;

/// A chunk of text contents formatted with the same set of attributes.
#[repr(C)]
pub struct YChunk {
    /// Piece of YText formatted using the same `fmt` rules. It can be a string, embedded object
    /// or another y-type.
    pub data: YOutput,
    /// Number of formatting attributes attached to current chunk of text.
    pub fmt_len: u32,
    ///
    pub fmt: *mut YMapEntry,
}

impl From<Diff<YChange>> for YChunk {
    fn from(diff: Diff<YChange>) -> Self {
        let data = YOutput::from(diff.insert);
        let mut fmt_len = 0;
        let mut fmt = if let Some(attrs) = diff.attributes {
            fmt_len = attrs.len() as u32;
            let mut fmt = Vec::with_capacity(attrs.len());
            for (k, v) in attrs.into_iter() {
                let e = YMapEntry::new(k.as_ref(), Value::Any(v));
                fmt.push(e);
            }
            Box::into_raw(fmt.into_boxed_slice()) as *mut _
        } else {
            null_mut()
        };
        YChunk { data, fmt_len, fmt }
    }
}

impl Drop for YChunk {
    fn drop(&mut self) {
        if !self.fmt.is_null() {
            drop(unsafe {
                Vec::from_raw_parts(self.fmt, self.fmt_len as usize, self.fmt_len as usize)
            });
        }
    }
}

/// A data structure that is used to pass input values of various types supported by Yrs into a
/// shared document store.
///
/// `YInput` constructor function don't allocate any resources on their own, neither they take
/// ownership by pointers to memory blocks allocated by user - for this reason once an input cell
/// has been used, its content should be freed by the caller.
#[repr(C)]
pub struct YInput {
    /// Tag describing, which `value` type is being stored by this input cell. Can be one of:
    ///
    /// - [Y_JSON_BOOL] for boolean flags.
    /// - [Y_JSON_NUM] for 64-bit floating point numbers.
    /// - [Y_JSON_INT] for 64-bit signed integers.
    /// - [Y_JSON_STR] for null-terminated UTF-8 encoded strings.
    /// - [Y_JSON_BUF] for embedded binary data.
    /// - [Y_JSON_ARR] for arrays of JSON-like values.
    /// - [Y_JSON_MAP] for JSON-like objects build from key-value pairs.
    /// - [Y_JSON_NULL] for JSON-like null values.
    /// - [Y_JSON_UNDEF] for JSON-like undefined values.
    /// - [Y_ARRAY] for cells which contents should be used to initialize a `YArray` shared type.
    /// - [Y_MAP] for cells which contents should be used to initialize a `YMap` shared type.
    /// - [Y_DOC] for cells which contents should be used to nest a `YDoc` sub-document.
    pub tag: i8,

    /// Length of the contents stored by current `YInput` cell.
    ///
    /// For [Y_JSON_NULL] and [Y_JSON_UNDEF] its equal to `0`.
    ///
    /// For [Y_JSON_ARR], [Y_JSON_MAP], [Y_ARRAY] and [Y_MAP] it describes a number of passed
    /// elements.
    ///
    /// For other types it's always equal to `1`.
    pub len: u32,

    /// Union struct which contains a content corresponding to a provided `tag` field.
    value: YInputContent,
}

impl YInput {
    fn into(self) -> Any {
        let tag = self.tag;
        unsafe {
            if tag == Y_JSON_STR {
                let str: Box<str> = CStr::from_ptr(self.value.str).to_str().unwrap().into();
                Any::String(str)
            } else if tag == Y_JSON_ARR {
                let ptr = self.value.values;
                let mut dst: Vec<Any> = Vec::with_capacity(self.len as usize);
                let mut i = 0;
                while i < self.len as isize {
                    let value = ptr.offset(i).read();
                    let any = value.into();
                    dst.push(any);
                    i += 1;
                }
                Any::Array(dst.into_boxed_slice())
            } else if tag == Y_JSON_MAP {
                let mut dst = HashMap::with_capacity(self.len as usize);
                let keys = self.value.map.keys;
                let values = self.value.map.values;
                let mut i = 0;
                while i < self.len as isize {
                    let key = CStr::from_ptr(keys.offset(i).read())
                        .to_str()
                        .unwrap()
                        .to_owned();
                    let value = values.offset(i).read().into();
                    dst.insert(key, value);
                    i += 1;
                }
                Any::Map(Box::new(dst))
            } else if tag == Y_JSON_NULL {
                Any::Null
            } else if tag == Y_JSON_UNDEF {
                Any::Undefined
            } else if tag == Y_JSON_INT {
                Any::BigInt(self.value.integer as i64)
            } else if tag == Y_JSON_NUM {
                Any::Number(self.value.num as f64)
            } else if tag == Y_JSON_BOOL {
                Any::Bool(if self.value.flag == 0 { false } else { true })
            } else if tag == Y_JSON_BUF {
                let slice =
                    std::slice::from_raw_parts(self.value.buf as *mut u8, self.len as usize);
                let buf = Box::from(slice);
                Any::Buffer(buf)
            } else if tag == Y_DOC {
                Any::Undefined
            } else {
                panic!("Unrecognized YVal value tag.")
            }
        }
    }
}

#[repr(C)]
union YInputContent {
    flag: u8,
    num: f64,
    integer: i64,
    str: *mut c_char,
    buf: *mut c_char,
    values: *mut YInput,
    map: ManuallyDrop<YMapInputData>,
    doc: *mut Doc,
}

#[repr(C)]
struct YMapInputData {
    keys: *mut *mut c_char,
    values: *mut YInput,
}

impl Drop for YInput {
    fn drop(&mut self) {}
}

impl Prelim for YInput {
    type Return = Unused;

    fn into_content<'doc>(self, _: &mut yrs::TransactionMut<'doc>) -> (ItemContent, Option<Self>) {
        unsafe {
            if self.tag <= 0 {
                let value = self.into();
                (ItemContent::Any(vec![value]), None)
            } else if self.tag == Y_DOC {
                let doc = self.value.doc.as_ref().unwrap();
                (ItemContent::Doc(None, doc.clone()), None)
            } else {
                let type_ref = if self.tag == Y_MAP {
                    TYPE_REFS_MAP
                } else if self.tag == Y_ARRAY {
                    TYPE_REFS_ARRAY
                } else if self.tag == Y_XML_ELEM {
                    TYPE_REFS_XML_ELEMENT
                } else if self.tag == Y_XML_TEXT {
                    TYPE_REFS_XML_TEXT
                } else if self.tag == Y_XML_FRAG {
                    TYPE_REFS_XML_FRAGMENT
                } else {
                    panic!("Unrecognized YVal value tag.")
                };
                let name = if type_ref == TYPE_REFS_XML_ELEMENT {
                    let name: Rc<str> = CStr::from_ptr(self.value.str).to_str().unwrap().into();
                    Some(name)
                } else {
                    None
                };
                let inner = Branch::new(type_ref, name);
                (ItemContent::Type(inner), Some(self))
            }
        }
    }

    fn integrate(self, txn: &mut yrs::TransactionMut, inner_ref: BranchPtr) {
        unsafe {
            if self.tag == Y_MAP {
                let map = MapRef::from(inner_ref);
                let keys = self.value.map.keys;
                let values = self.value.map.values;
                let i = 0;
                while i < self.len as isize {
                    let key = CStr::from_ptr(keys.offset(i).read())
                        .to_str()
                        .unwrap()
                        .to_owned();
                    let value = values.offset(i).read().into();
                    map.insert(txn, key, value);
                }
            } else if self.tag == Y_ARRAY {
                let array = ArrayRef::from(inner_ref);
                let ptr = self.value.values;
                let len = self.len as isize;
                let mut i = 0;
                while i < len {
                    let value = ptr.offset(i).read();
                    array.push_back(txn, value);
                    i += 1;
                }
            } else if self.tag == Y_TEXT {
                let text = TextRef::from(inner_ref);
                let init = CStr::from_ptr(self.value.str).to_str().unwrap();
                text.push(txn, init);
            } else if self.tag == Y_XML_TEXT {
                let text = XmlTextRef::from(inner_ref);
                let init = CStr::from_ptr(self.value.str).to_str().unwrap();
                text.push(txn, init);
            };
        }
    }
}

/// An output value cell returned from yrs API methods. It describes a various types of data
/// supported by yrs shared data types.
///
/// Since `YOutput` instances are always created by calling the corresponding yrs API functions,
/// they eventually should be deallocated using [youtput_destroy] function.
#[repr(C)]

pub struct YOutput {
    /// Tag describing, which `value` type is being stored by this input cell. Can be one of:
    ///
    /// - [Y_JSON_BOOL] for boolean flags.
    /// - [Y_JSON_NUM] for 64-bit floating point numbers.
    /// - [Y_JSON_INT] for 64-bit signed integers.
    /// - [Y_JSON_STR] for null-terminated UTF-8 encoded strings.
    /// - [Y_JSON_BUF] for embedded binary data.
    /// - [Y_JSON_ARR] for arrays of JSON-like values.
    /// - [Y_JSON_MAP] for JSON-like objects build from key-value pairs.
    /// - [Y_JSON_NULL] for JSON-like null values.
    /// - [Y_JSON_UNDEF] for JSON-like undefined values.
    /// - [Y_TEXT] for pointers to `YText` data types.
    /// - [Y_ARRAY] for pointers to `YArray` data types.
    /// - [Y_MAP] for pointers to `YMap` data types.
    /// - [Y_XML_ELEM] for pointers to `YXmlElement` data types.
    /// - [Y_XML_TEXT] for pointers to `YXmlText` data types.
    /// - [Y_DOC] for pointers to nested `YDocRef` data types.
    pub tag: i8,

    /// Length of the contents stored by a current `YOutput` cell.
    ///
    /// For [Y_JSON_NULL] and [Y_JSON_UNDEF] its equal to `0`.
    ///
    /// For [Y_JSON_ARR], [Y_JSON_MAP] it describes a number of passed elements.
    ///
    /// For other types it's always equal to `1`.
    pub len: u32,

    /// Union struct which contains a content corresponding to a provided `tag` field.
    value: YOutputContent,
}

impl std::fmt::Display for YOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let tag = self.tag;
        unsafe {
            if tag == Y_JSON_INT {
                write!(f, "{}", self.value.integer)
            } else if tag == Y_JSON_NUM {
                write!(f, "{}", self.value.num)
            } else if tag == Y_JSON_BOOL {
                write!(
                    f,
                    "{}",
                    if self.value.flag == 0 {
                        "false"
                    } else {
                        "true"
                    }
                )
            } else if tag == Y_JSON_UNDEF {
                write!(f, "undefined")
            } else if tag == Y_JSON_NULL {
                write!(f, "null")
            } else if tag == Y_JSON_STR {
                write!(f, "{}", CString::from_raw(self.value.str).to_str().unwrap())
            } else if tag == Y_MAP {
                write!(f, "YMap")
            } else if tag == Y_ARRAY {
                write!(f, "YArray")
            } else if tag == Y_JSON_ARR {
                write!(f, "[")?;
                let slice = std::slice::from_raw_parts(self.value.array, self.len as usize);
                for o in slice {
                    write!(f, ", {}", o)?;
                }
                write!(f, "]")
            } else if tag == Y_JSON_MAP {
                write!(f, "{{")?;
                let slice = std::slice::from_raw_parts(self.value.map, self.len as usize);
                for e in slice {
                    write!(
                        f,
                        ", '{}' => {}",
                        CStr::from_ptr(e.key).to_str().unwrap(),
                        e.value
                    )?;
                }
                write!(f, "}}")
            } else if tag == Y_TEXT {
                write!(f, "YText")
            } else if tag == Y_XML_TEXT {
                write!(f, "YXmlText")
            } else if tag == Y_XML_ELEM {
                write!(f, "YXmlElement",)
            } else if tag == Y_JSON_BUF {
                write!(f, "YBinary(len: {})", self.len)
            } else {
                Ok(())
            }
        }
    }
}

impl Drop for YOutput {
    fn drop(&mut self) {
        let tag = self.tag;
        unsafe {
            if tag == Y_JSON_STR {
                drop(CString::from_raw(self.value.str));
            } else if tag == Y_JSON_ARR {
                drop(Vec::from_raw_parts(
                    self.value.array,
                    self.len as usize,
                    self.len as usize,
                ));
            } else if tag == Y_JSON_MAP {
                drop(Vec::from_raw_parts(
                    self.value.map,
                    self.len as usize,
                    self.len as usize,
                ));
            } else if tag == Y_JSON_BUF {
                drop(Vec::from_raw_parts(
                    self.value.buf,
                    self.len as usize,
                    self.len as usize,
                ));
            } else if tag == Y_DOC {
                drop(Box::from_raw(self.value.y_doc))
            }
        }
    }
}

impl From<Value> for YOutput {
    fn from(v: Value) -> Self {
        match v {
            Value::Any(v) => Self::from(v),
            Value::YText(v) => Self::from(v),
            Value::YArray(v) => Self::from(v),
            Value::YMap(v) => Self::from(v),
            Value::YXmlElement(v) => Self::from(v),
            Value::YXmlFragment(v) => Self::from(v),
            Value::YXmlText(v) => Self::from(v),
            Value::YDoc(v) => Self::from(v),
        }
    }
}

impl From<Any> for YOutput {
    fn from(v: Any) -> Self {
        unsafe {
            match v {
                Any::Null => YOutput {
                    tag: Y_JSON_NULL,
                    len: 0,
                    value: MaybeUninit::uninit().assume_init(),
                },
                Any::Undefined => YOutput {
                    tag: Y_JSON_UNDEF,
                    len: 0,
                    value: MaybeUninit::uninit().assume_init(),
                },
                Any::Bool(v) => YOutput {
                    tag: Y_JSON_BOOL,
                    len: 1,
                    value: YOutputContent {
                        flag: if v { Y_TRUE } else { Y_FALSE },
                    },
                },
                Any::Number(v) => YOutput {
                    tag: Y_JSON_NUM,
                    len: 1,
                    value: YOutputContent { num: v as _ },
                },
                Any::BigInt(v) => YOutput {
                    tag: Y_JSON_INT,
                    len: 1,
                    value: YOutputContent { integer: v },
                },
                Any::String(v) => YOutput {
                    tag: Y_JSON_STR,
                    len: v.len() as u32,
                    value: YOutputContent {
                        str: CString::new(v.as_ref()).unwrap().into_raw(),
                    },
                },
                Any::Buffer(v) => YOutput {
                    tag: Y_JSON_BUF,
                    len: v.len() as u32,
                    value: YOutputContent {
                        buf: Box::into_raw(v.clone()) as *mut _,
                    },
                },
                Any::Array(v) => {
                    let len = v.len() as u32;
                    let v = Vec::from(v);
                    let mut array: Vec<_> = v.into_iter().map(|v| YOutput::from(v)).collect();
                    array.shrink_to_fit();
                    let ptr = array.as_mut_ptr();
                    forget(array);
                    YOutput {
                        tag: Y_JSON_ARR,
                        len,
                        value: YOutputContent { array: ptr },
                    }
                }
                Any::Map(v) => {
                    let len = v.len() as u32;
                    let v = *v;
                    let mut array: Vec<_> = v
                        .into_iter()
                        .map(|(k, v)| YMapEntry::new(k.as_str(), Value::Any(v)))
                        .collect();
                    array.shrink_to_fit();
                    let ptr = array.as_mut_ptr();
                    forget(array);
                    YOutput {
                        tag: Y_JSON_MAP,
                        len,
                        value: YOutputContent { map: ptr },
                    }
                }
            }
        }
    }
}

impl From<TextRef> for YOutput {
    fn from(v: TextRef) -> Self {
        YOutput {
            tag: Y_TEXT,
            len: 1,
            value: YOutputContent {
                y_type: v.into_raw_branch(),
            },
        }
    }
}

impl From<ArrayRef> for YOutput {
    fn from(v: ArrayRef) -> Self {
        YOutput {
            tag: Y_ARRAY,
            len: 1,
            value: YOutputContent {
                y_type: v.into_raw_branch(),
            },
        }
    }
}

impl From<MapRef> for YOutput {
    fn from(v: MapRef) -> Self {
        YOutput {
            tag: Y_MAP,
            len: 1,
            value: YOutputContent {
                y_type: v.into_raw_branch(),
            },
        }
    }
}

impl From<XmlElementRef> for YOutput {
    fn from(v: XmlElementRef) -> Self {
        YOutput {
            tag: Y_XML_ELEM,
            len: 1,
            value: YOutputContent {
                y_type: v.into_raw_branch(),
            },
        }
    }
}

impl From<XmlTextRef> for YOutput {
    fn from(v: XmlTextRef) -> Self {
        YOutput {
            tag: Y_XML_TEXT,
            len: 1,
            value: YOutputContent {
                y_type: v.into_raw_branch(),
            },
        }
    }
}

impl From<XmlFragmentRef> for YOutput {
    fn from(v: XmlFragmentRef) -> Self {
        YOutput {
            tag: Y_XML_FRAG,
            len: 1,
            value: YOutputContent {
                y_type: v.into_raw_branch(),
            },
        }
    }
}

impl From<Doc> for YOutput {
    fn from(v: Doc) -> Self {
        YOutput {
            tag: Y_DOC,
            len: 1,
            value: YOutputContent {
                y_doc: Box::into_raw(Box::new(v.clone())),
            },
        }
    }
}

#[repr(C)]
union YOutputContent {
    flag: u8,
    num: f64,
    integer: i64,
    str: *mut c_char,
    buf: *mut c_char,
    array: *mut YOutput,
    map: *mut YMapEntry,
    y_type: *mut Branch,
    y_doc: *mut Doc,
}

/// Releases all resources related to a corresponding `YOutput` cell.
#[no_mangle]
pub unsafe extern "C" fn youtput_destroy(val: *mut YOutput) {
    if !val.is_null() {
        drop(Box::from_raw(val))
    }
}

/// Function constructor used to create JSON-like NULL `YInput` cell.
/// This function doesn't allocate any heap resources.
#[no_mangle]
pub unsafe extern "C" fn yinput_null() -> YInput {
    YInput {
        tag: Y_JSON_NULL,
        len: 0,
        value: MaybeUninit::uninit().assume_init(),
    }
}

/// Function constructor used to create JSON-like undefined `YInput` cell.
/// This function doesn't allocate any heap resources.
#[no_mangle]
pub unsafe extern "C" fn yinput_undefined() -> YInput {
    YInput {
        tag: Y_JSON_UNDEF,
        len: 0,
        value: MaybeUninit::uninit().assume_init(),
    }
}

/// Function constructor used to create JSON-like boolean `YInput` cell.
/// This function doesn't allocate any heap resources.
#[no_mangle]
pub unsafe extern "C" fn yinput_bool(flag: u8) -> YInput {
    YInput {
        tag: Y_JSON_BOOL,
        len: 1,
        value: YInputContent { flag },
    }
}

/// Function constructor used to create JSON-like 64-bit floating point number `YInput` cell.
/// This function doesn't allocate any heap resources.
#[no_mangle]
pub unsafe extern "C" fn yinput_float(num: f64) -> YInput {
    YInput {
        tag: Y_JSON_NUM,
        len: 1,
        value: YInputContent { num },
    }
}

/// Function constructor used to create JSON-like 64-bit signed integer `YInput` cell.
/// This function doesn't allocate any heap resources.
#[no_mangle]
pub unsafe extern "C" fn yinput_long(integer: i64) -> YInput {
    YInput {
        tag: Y_JSON_INT,
        len: 1,
        value: YInputContent { integer },
    }
}

/// Function constructor used to create a string `YInput` cell. Provided parameter must be
/// a null-terminated UTF-8 encoded string. This function doesn't allocate any heap resources,
/// and doesn't release any on its own, therefore its up to a caller to free resources once
/// a structure is no longer needed.
#[no_mangle]
pub unsafe extern "C" fn yinput_string(str: *const c_char) -> YInput {
    YInput {
        tag: Y_JSON_STR,
        len: 1,
        value: YInputContent {
            str: str as *mut c_char,
        },
    }
}

/// Function constructor used to create a binary `YInput` cell of a specified length.
/// This function doesn't allocate any heap resources and doesn't release any on its own, therefore
/// its up to a caller to free resources once a structure is no longer needed.
#[no_mangle]
pub unsafe extern "C" fn yinput_binary(buf: *const c_char, len: u32) -> YInput {
    YInput {
        tag: Y_JSON_BUF,
        len,
        value: YInputContent {
            buf: buf as *mut c_char,
        },
    }
}

/// Function constructor used to create a JSON-like array `YInput` cell of other JSON-like values of
/// a given length. This function doesn't allocate any heap resources and doesn't release any on its
/// own, therefore its up to a caller to free resources once a structure is no longer needed.
#[no_mangle]
pub unsafe extern "C" fn yinput_json_array(values: *mut YInput, len: u32) -> YInput {
    YInput {
        tag: Y_JSON_ARR,
        len,
        value: YInputContent { values },
    }
}

/// Function constructor used to create a JSON-like map `YInput` cell of other JSON-like key-value
/// pairs. These pairs are build from corresponding indexes of `keys` and `values`, which must have
/// the same specified length.
///
/// This function doesn't allocate any heap resources and doesn't release any on its own, therefore
/// its up to a caller to free resources once a structure is no longer needed.
#[no_mangle]
pub unsafe extern "C" fn yinput_json_map(
    keys: *mut *mut c_char,
    values: *mut YInput,
    len: u32,
) -> YInput {
    YInput {
        tag: Y_JSON_MAP,
        len,
        value: YInputContent {
            map: ManuallyDrop::new(YMapInputData { keys, values }),
        },
    }
}

/// Function constructor used to create a nested `YArray` `YInput` cell prefilled with other
/// values of a given length. This function doesn't allocate any heap resources and doesn't release
/// any on its own, therefore its up to a caller to free resources once a structure is no longer
/// needed.
#[no_mangle]
pub unsafe extern "C" fn yinput_yarray(values: *mut YInput, len: u32) -> YInput {
    YInput {
        tag: Y_ARRAY,
        len,
        value: YInputContent { values },
    }
}

/// Function constructor used to create a nested `YMap` `YInput` cell prefilled with other key-value
/// pairs. These pairs are build from corresponding indexes of `keys` and `values`, which must have
/// the same specified length.
///
/// This function doesn't allocate any heap resources and doesn't release any on its own, therefore
/// its up to a caller to free resources once a structure is no longer needed.
#[no_mangle]
pub unsafe extern "C" fn yinput_ymap(
    keys: *mut *mut c_char,
    values: *mut YInput,
    len: u32,
) -> YInput {
    YInput {
        tag: Y_MAP,
        len,
        value: YInputContent {
            map: ManuallyDrop::new(YMapInputData { keys, values }),
        },
    }
}

/// Function constructor used to create a nested `YText` `YInput` cell prefilled with a specified
/// string, which must be a null-terminated UTF-8 character pointer.
///
/// This function doesn't allocate any heap resources and doesn't release any on its own, therefore
/// its up to a caller to free resources once a structure is no longer needed.
#[no_mangle]
pub unsafe extern "C" fn yinput_ytext(str: *mut c_char) -> YInput {
    YInput {
        tag: Y_TEXT,
        len: 1,
        value: YInputContent { str },
    }
}

/// Function constructor used to create a nested `YXmlElement` `YInput` cell with a specified
/// tag name, which must be a null-terminated UTF-8 character pointer.
///
/// This function doesn't allocate any heap resources and doesn't release any on its own, therefore
/// its up to a caller to free resources once a structure is no longer needed.
#[no_mangle]
pub unsafe extern "C" fn yinput_yxmlelem(name: *mut c_char) -> YInput {
    YInput {
        tag: Y_XML_ELEM,
        len: 1,
        value: YInputContent { str: name },
    }
}

/// Function constructor used to create a nested `YXmlText` `YInput` cell prefilled with a specified
/// string, which must be a null-terminated UTF-8 character pointer.
///
/// This function doesn't allocate any heap resources and doesn't release any on its own, therefore
/// its up to a caller to free resources once a structure is no longer needed.
#[no_mangle]
pub unsafe extern "C" fn yinput_yxmltext(str: *mut c_char) -> YInput {
    YInput {
        tag: Y_XML_TEXT,
        len: 1,
        value: YInputContent { str },
    }
}

/// Function constructor used to create a nested `YDoc` `YInput` cell.
///
/// This function doesn't allocate any heap resources and doesn't release any on its own, therefore
/// its up to a caller to free resources once a structure is no longer needed.
#[no_mangle]
pub unsafe extern "C" fn yinput_ydoc(doc: *mut Doc) -> YInput {
    YInput {
        tag: Y_DOC,
        len: 1,
        value: YInputContent { doc },
    }
}

/// Attempts to read the value for a given `YOutput` pointer as a `YDocRef` reference to a nested
/// document.
#[no_mangle]
pub unsafe extern "C" fn youtput_read_ydoc(val: *const YOutput) -> *mut Doc {
    let v = val.as_ref().unwrap();
    if v.tag == Y_DOC {
        v.value.y_doc
    } else {
        std::ptr::null_mut()
    }
}

/// Attempts to read the value for a given `YOutput` pointer as a boolean flag, which can be either
/// `1` for truthy case and `0` otherwise. Returns a null pointer in case when a value stored under
/// current `YOutput` cell is not of a boolean type.
#[no_mangle]
pub unsafe extern "C" fn youtput_read_bool(val: *const YOutput) -> *const u8 {
    let v = val.as_ref().unwrap();
    if v.tag == Y_JSON_BOOL {
        &v.value.flag
    } else {
        std::ptr::null()
    }
}

/// Attempts to read the value for a given `YOutput` pointer as a 64-bit floating point number.
///
/// Returns a null pointer in case when a value stored under current `YOutput` cell
/// is not a floating point number.
#[no_mangle]
pub unsafe extern "C" fn youtput_read_float(val: *const YOutput) -> *const f64 {
    let v = val.as_ref().unwrap();
    if v.tag == Y_JSON_NUM {
        &v.value.num
    } else {
        std::ptr::null()
    }
}

/// Attempts to read the value for a given `YOutput` pointer as a 64-bit signed integer.
///
/// Returns a null pointer in case when a value stored under current `YOutput` cell
/// is not a signed integer.
#[no_mangle]
pub unsafe extern "C" fn youtput_read_long(val: *const YOutput) -> *const i64 {
    let v = val.as_ref().unwrap();
    if v.tag == Y_JSON_INT {
        &v.value.integer
    } else {
        std::ptr::null()
    }
}

/// Attempts to read the value for a given `YOutput` pointer as a null-terminated UTF-8 encoded
/// string.
///
/// Returns a null pointer in case when a value stored under current `YOutput` cell
/// is not a string. Underlying string is released automatically as part of [youtput_destroy]
/// destructor.
#[no_mangle]
pub unsafe extern "C" fn youtput_read_string(val: *const YOutput) -> *mut c_char {
    let v = val.as_ref().unwrap();
    if v.tag == Y_JSON_STR {
        v.value.str
    } else {
        std::ptr::null_mut()
    }
}

/// Attempts to read the value for a given `YOutput` pointer as a binary payload (which length is
/// stored within `len` filed of a cell itself).
///
/// Returns a null pointer in case when a value stored under current `YOutput` cell
/// is not a binary type. Underlying binary is released automatically as part of [youtput_destroy]
/// destructor.
#[no_mangle]
pub unsafe extern "C" fn youtput_read_binary(val: *const YOutput) -> *const c_char {
    let v = val.as_ref().unwrap();
    if v.tag == Y_JSON_BUF {
        v.value.buf
    } else {
        std::ptr::null()
    }
}

/// Attempts to read the value for a given `YOutput` pointer as a JSON-like array of `YOutput`
/// values (which length is stored within `len` filed of a cell itself).
///
/// Returns a null pointer in case when a value stored under current `YOutput` cell
/// is not a JSON-like array. Underlying heap resources are released automatically as part of
/// [youtput_destroy] destructor.
#[no_mangle]
pub unsafe extern "C" fn youtput_read_json_array(val: *const YOutput) -> *mut YOutput {
    let v = val.as_ref().unwrap();
    if v.tag == Y_JSON_ARR {
        v.value.array
    } else {
        std::ptr::null_mut()
    }
}

/// Attempts to read the value for a given `YOutput` pointer as a JSON-like map of key-value entries
/// (which length is stored within `len` filed of a cell itself).
///
/// Returns a null pointer in case when a value stored under current `YOutput` cell
/// is not a JSON-like map. Underlying heap resources are released automatically as part of
/// [youtput_destroy] destructor.
#[no_mangle]
pub unsafe extern "C" fn youtput_read_json_map(val: *const YOutput) -> *mut YMapEntry {
    let v = val.as_ref().unwrap();
    if v.tag == Y_JSON_MAP {
        v.value.map
    } else {
        std::ptr::null_mut()
    }
}

/// Attempts to read the value for a given `YOutput` pointer as an `YArray`.
///
/// Returns a null pointer in case when a value stored under current `YOutput` cell
/// is not an `YArray`. Underlying heap resources are released automatically as part of
/// [youtput_destroy] destructor.
#[no_mangle]
pub unsafe extern "C" fn youtput_read_yarray(val: *const YOutput) -> *mut Branch {
    let v = val.as_ref().unwrap();
    if v.tag == Y_ARRAY {
        v.value.y_type
    } else {
        std::ptr::null_mut()
    }
}

/// Attempts to read the value for a given `YOutput` pointer as an `YXmlElement`.
///
/// Returns a null pointer in case when a value stored under current `YOutput` cell
/// is not an `YXmlElement`. Underlying heap resources are released automatically as part of
/// [youtput_destroy] destructor.
#[no_mangle]
pub unsafe extern "C" fn youtput_read_yxmlelem(val: *const YOutput) -> *mut Branch {
    let v = val.as_ref().unwrap();
    if v.tag == Y_XML_ELEM {
        v.value.y_type
    } else {
        std::ptr::null_mut()
    }
}

/// Attempts to read the value for a given `YOutput` pointer as an `YMap`.
///
/// Returns a null pointer in case when a value stored under current `YOutput` cell
/// is not an `YMap`. Underlying heap resources are released automatically as part of
/// [youtput_destroy] destructor.
#[no_mangle]
pub unsafe extern "C" fn youtput_read_ymap(val: *const YOutput) -> *mut Branch {
    let v = val.as_ref().unwrap();
    if v.tag == Y_MAP {
        v.value.y_type
    } else {
        std::ptr::null_mut()
    }
}

/// Attempts to read the value for a given `YOutput` pointer as an `YText`.
///
/// Returns a null pointer in case when a value stored under current `YOutput` cell
/// is not an `YText`. Underlying heap resources are released automatically as part of
/// [youtput_destroy] destructor.
#[no_mangle]
pub unsafe extern "C" fn youtput_read_ytext(val: *const YOutput) -> *mut Branch {
    let v = val.as_ref().unwrap();
    if v.tag == Y_TEXT {
        v.value.y_type
    } else {
        std::ptr::null_mut()
    }
}

/// Attempts to read the value for a given `YOutput` pointer as an `YXmlText`.
///
/// Returns a null pointer in case when a value stored under current `YOutput` cell
/// is not an `YXmlText`. Underlying heap resources are released automatically as part of
/// [youtput_destroy] destructor.
#[no_mangle]
pub unsafe extern "C" fn youtput_read_yxmltext(val: *const YOutput) -> *mut Branch {
    let v = val.as_ref().unwrap();
    if v.tag == Y_XML_TEXT {
        v.value.y_type
    } else {
        std::ptr::null_mut()
    }
}

/// Subscribes a given callback function `cb` to changes made by this `YText` instance. Callbacks
/// are triggered whenever a `ytransaction_commit` is called.
/// Returns a subscription ID which can be then used to unsubscribe this callback by using
/// `ytext_unobserve` function.
#[no_mangle]
pub unsafe extern "C" fn ytext_observe(
    txt: *const Branch,
    state: *mut c_void,
    cb: extern "C" fn(*mut c_void, *const YTextEvent),
) -> u32 {
    assert!(!txt.is_null());

    let mut txt = TextRef::from_raw_branch(txt);
    let observer = txt.observe(move |txn, e| {
        let e = YTextEvent::new(e, txn);
        cb(state, &e as *const YTextEvent);
    });
    let subscription_id: u32 = observer.into();
    subscription_id
}

/// Subscribes a given callback function `cb` to changes made by this `YMap` instance. Callbacks
/// are triggered whenever a `ytransaction_commit` is called.
/// Returns a subscription ID which can be then used to unsubscribe this callback by using
/// `ymap_unobserve` function.
#[no_mangle]
pub unsafe extern "C" fn ymap_observe(
    map: *const Branch,
    state: *mut c_void,
    cb: extern "C" fn(*mut c_void, *const YMapEvent),
) -> u32 {
    assert!(!map.is_null());

    let mut map = MapRef::from_raw_branch(map);
    let observer = map.observe(move |txn, e| {
        let e = YMapEvent::new(e, txn);
        cb(state, &e as *const YMapEvent);
    });
    let subscription_id: u32 = observer.into();
    subscription_id
}

/// Subscribes a given callback function `cb` to changes made by this `YArray` instance. Callbacks
/// are triggered whenever a `ytransaction_commit` is called.
/// Returns a subscription ID which can be then used to unsubscribe this callback by using
/// `yarray_unobserve` function.
#[no_mangle]
pub unsafe extern "C" fn yarray_observe(
    array: *const Branch,
    state: *mut c_void,
    cb: extern "C" fn(*mut c_void, *const YArrayEvent),
) -> u32 {
    assert!(!array.is_null());

    let mut array = ArrayRef::from_raw_branch(array);
    let observer = array.observe(move |txn, e| {
        let e = YArrayEvent::new(e, txn);
        cb(state, &e as *const YArrayEvent);
    });
    let subscription_id: u32 = observer.into();
    subscription_id
}

/// Subscribes a given callback function `cb` to changes made by this `YXmlElement` instance.
/// Callbacks are triggered whenever a `ytransaction_commit` is called.
/// Returns a subscription ID which can be then used to unsubscribe this callback by using
/// `yxmlelem_unobserve` function.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_observe(
    xml: *const Branch,
    state: *mut c_void,
    cb: extern "C" fn(*mut c_void, *const YXmlEvent),
) -> u32 {
    assert!(!xml.is_null());

    let mut xml = XmlElementRef::from_raw_branch(xml);
    let observer = xml.observe(move |txn, e| {
        let e = YXmlEvent::new(e, txn);
        cb(state, &e as *const YXmlEvent);
    });
    let subscription_id: u32 = observer.into();
    subscription_id
}

/// Subscribes a given callback function `cb` to changes made by this `YXmlText` instance. Callbacks
/// are triggered whenever a `ytransaction_commit` is called.
/// Returns a subscription ID which can be then used to unsubscribe this callback by using
/// `yxmltext_unobserve` function.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_observe(
    xml: *const Branch,
    state: *mut c_void,
    cb: extern "C" fn(*mut c_void, *const YXmlTextEvent),
) -> u32 {
    assert!(!xml.is_null());

    let mut xml = XmlTextRef::from_raw_branch(xml);
    let observer = xml.observe(move |txn, e| {
        let e = YXmlTextEvent::new(e, txn);
        cb(state, &e as *const YXmlTextEvent);
    });
    let subscription_id: u32 = observer.into();
    subscription_id
}

/// Subscribes a given callback function `cb` to changes made by this shared type instance as well
/// as all nested shared types living within it. Callbacks are triggered whenever a
/// `ytransaction_commit` is called.
///
/// Returns a subscription ID which can be then used to unsubscribe this callback by using
/// `yunobserve_deep` function.
#[no_mangle]
pub unsafe extern "C" fn yobserve_deep(
    ytype: *mut Branch,
    state: *mut c_void,
    cb: extern "C" fn(*mut c_void, u32, *const YEvent),
) -> u32 {
    assert!(!ytype.is_null());

    let branch = ytype.as_mut().unwrap();
    let observer = branch.observe_deep(move |txn, events| {
        let events: Vec<_> = events.iter().map(|e| YEvent::new(txn, e)).collect();
        let len = events.len() as u32;
        cb(state, len, events.as_ptr());
    });
    let subscription_id: u32 = observer.into();
    subscription_id
}

/// Event generated for callbacks subscribed using `ydoc_observe_after_transaction`. It contains
/// snapshot of changes made within any committed transaction.
#[repr(C)]
pub struct YAfterTransactionEvent {
    /// Descriptor of a document state at the moment of creating the transaction.
    pub before_state: YStateVector,
    /// Descriptor of a document state at the moment of committing the transaction.
    pub after_state: YStateVector,
    /// Information about all items deleted within the scope of a transaction.
    pub delete_set: YDeleteSet,
}

impl YAfterTransactionEvent {
    unsafe fn new(e: &TransactionCleanupEvent) -> Self {
        YAfterTransactionEvent {
            before_state: YStateVector::new(&e.before_state),
            after_state: YStateVector::new(&e.after_state),
            delete_set: YDeleteSet::new(&e.delete_set),
        }
    }
}

#[repr(C)]
pub struct YSubdocsEvent {
    added_len: u32,
    removed_len: u32,
    loaded_len: u32,
    added: *mut *mut Doc,
    removed: *mut *mut Doc,
    loaded: *mut *mut Doc,
}

impl YSubdocsEvent {
    unsafe fn new(e: &SubdocsEvent) -> Self {
        fn into_ptr(v: SubdocsEventIter) -> *mut *mut Doc {
            let array: Vec<_> = v.map(|doc| Box::into_raw(Box::new(doc.clone()))).collect();
            let mut boxed = array.into_boxed_slice();
            let ptr = boxed.as_mut_ptr();
            forget(boxed);
            ptr
        }

        let added = e.added();
        let removed = e.removed();
        let loaded = e.loaded();

        YSubdocsEvent {
            added_len: added.len() as u32,
            removed_len: removed.len() as u32,
            loaded_len: loaded.len() as u32,
            added: into_ptr(added),
            removed: into_ptr(removed),
            loaded: into_ptr(loaded),
        }
    }
}

impl Drop for YSubdocsEvent {
    fn drop(&mut self) {
        fn release(len: u32, buf: *mut *mut Doc) {
            unsafe {
                let docs = Vec::from_raw_parts(buf, len as usize, len as usize);
                for d in docs {
                    drop(Box::from_raw(d));
                }
            }
        }

        release(self.added_len, self.added);
        release(self.removed_len, self.removed);
        release(self.loaded_len, self.loaded);
    }
}

/// Struct representing a state of a document. It contains the last seen clocks for blocks submitted
/// per any of the clients collaborating on document updates.
#[repr(C)]
pub struct YStateVector {
    /// Number of clients. It describes a length of both `client_ids` and `clocks` arrays.
    pub entries_count: u32,
    /// Array of unique client identifiers (length is given in `entries_count` field). Each client
    /// ID has corresponding clock attached, which can be found in `clocks` field under the same
    /// index.
    pub client_ids: *mut u64,
    /// Array of clocks (length is given in `entries_count` field) known for each client. Each clock
    /// has a corresponding client identifier attached, which can be found in `client_ids` field
    /// under the same index.
    pub clocks: *mut u32,
}

impl YStateVector {
    unsafe fn new(sv: &StateVector) -> Self {
        let entries_count = sv.len() as u32;
        let mut client_ids = Vec::with_capacity(sv.len());
        let mut clocks = Vec::with_capacity(sv.len());
        for (&client, &clock) in sv.iter() {
            client_ids.push(client as u64);
            clocks.push(clock as u32);
        }

        YStateVector {
            entries_count,
            client_ids: Box::into_raw(client_ids.into_boxed_slice()) as *mut _,
            clocks: Box::into_raw(clocks.into_boxed_slice()) as *mut _,
        }
    }
}

impl Drop for YStateVector {
    fn drop(&mut self) {
        let len = self.entries_count as usize;
        drop(unsafe { Vec::from_raw_parts(self.client_ids, len, len) });
        drop(unsafe { Vec::from_raw_parts(self.clocks, len, len) });
    }
}

/// Delete set is a map of `(ClientID, Range[])` entries. Length of a map is stored in
/// `entries_count` field. ClientIDs reside under `client_ids` and their corresponding range
/// sequences can be found under the same index of `ranges` field.
#[repr(C)]
pub struct YDeleteSet {
    /// Number of client identifier entries.
    pub entries_count: u32,
    /// Array of unique client identifiers (length is given in `entries_count` field). Each client
    /// ID has corresponding sequence of ranges attached, which can be found in `ranges` field under
    /// the same index.
    pub client_ids: *mut u64,
    /// Array of range sequences (length is given in `entries_count` field). Each sequence has
    /// a corresponding client ID attached, which can be found in `client_ids` field under
    /// the same index.
    pub ranges: *mut YIdRangeSeq,
}

impl YDeleteSet {
    unsafe fn new(ds: &DeleteSet) -> Self {
        let len = ds.len();
        let mut client_ids = Vec::with_capacity(len);
        let mut ranges = Vec::with_capacity(len);

        for (&client, range) in ds.iter() {
            client_ids.push(client);
            let seq: Vec<_> = range
                .iter()
                .map(|r| YIdRange {
                    start: r.start as u32,
                    end: r.end as u32,
                })
                .collect();
            ranges.push(YIdRangeSeq {
                len: seq.len() as u32,
                seq: Box::into_raw(seq.into_boxed_slice()) as *mut _,
            })
        }

        YDeleteSet {
            entries_count: len as u32,
            client_ids: Box::into_raw(client_ids.into_boxed_slice()) as *mut _,
            ranges: Box::into_raw(ranges.into_boxed_slice()) as *mut _,
        }
    }
}

impl Drop for YDeleteSet {
    fn drop(&mut self) {
        let len = self.entries_count as usize;
        drop(unsafe { Vec::from_raw_parts(self.client_ids, len, len) });
        drop(unsafe { Vec::from_raw_parts(self.ranges, len, len) });
    }
}

/// Fixed-length sequence of ID ranges. Each range is a pair of [start, end) values, describing the
/// range of items identified by clock values, that this range refers to.
#[repr(C)]
pub struct YIdRangeSeq {
    /// Number of ranges stored in this sequence.
    pub len: u32,
    /// Array (length is stored in `len` field) or ranges. Each range is a pair of [start, end)
    /// values, describing continuous collection of items produced by the same client, identified
    /// by clock values, that this range refers to.
    pub seq: *mut YIdRange,
}

impl Drop for YIdRangeSeq {
    fn drop(&mut self) {
        let len = self.len as usize;
        drop(unsafe { Vec::from_raw_parts(self.seq, len, len) })
    }
}

#[repr(C)]
pub struct YIdRange {
    pub start: u32,
    pub end: u32,
}

#[repr(C)]
pub struct YEvent {
    /// Tag describing, which shared type emitted this event.
    ///
    /// - [Y_TEXT] for pointers to `YText` data types.
    /// - [Y_ARRAY] for pointers to `YArray` data types.
    /// - [Y_MAP] for pointers to `YMap` data types.
    /// - [Y_XML_ELEM] for pointers to `YXmlElement` data types.
    /// - [Y_XML_TEXT] for pointers to `YXmlText` data types.
    pub tag: i8,

    /// A nested event type, specific for a shared data type that triggered it. Type of an
    /// event can be verified using `tag` field.
    pub content: YEventContent,
}

impl YEvent {
    fn new<'doc>(txn: &yrs::TransactionMut<'doc>, e: &Event) -> YEvent {
        match e {
            Event::Text(e) => YEvent {
                tag: Y_TEXT,
                content: YEventContent {
                    text: YTextEvent::new(e, txn),
                },
            },
            Event::Array(e) => YEvent {
                tag: Y_ARRAY,
                content: YEventContent {
                    array: YArrayEvent::new(e, txn),
                },
            },
            Event::Map(e) => YEvent {
                tag: Y_MAP,
                content: YEventContent {
                    map: YMapEvent::new(e, txn),
                },
            },
            Event::XmlFragment(e) => YEvent {
                tag: if let XmlNode::Fragment(_) = e.target() {
                    Y_XML_FRAG
                } else {
                    Y_XML_ELEM
                },
                content: YEventContent {
                    xml_elem: YXmlEvent::new(e, txn),
                },
            },
            Event::XmlText(e) => YEvent {
                tag: Y_XML_TEXT,
                content: YEventContent {
                    xml_text: YXmlTextEvent::new(e, txn),
                },
            },
        }
    }
}

#[repr(C)]
pub union YEventContent {
    pub text: YTextEvent,
    pub map: YMapEvent,
    pub array: YArrayEvent,
    pub xml_elem: YXmlEvent,
    pub xml_text: YXmlTextEvent,
}

/// Event pushed into callbacks registered with `ytext_observe` function. It contains delta of all
/// text changes made within a scope of corresponding transaction (see: `ytext_event_delta`) as
/// well as navigation data used to identify a `YText` instance which triggered this event.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct YTextEvent {
    inner: *const c_void,
    txn: *const yrs::TransactionMut<'static>,
}

impl YTextEvent {
    fn new<'dev>(inner: &TextEvent, txn: &yrs::TransactionMut<'dev>) -> Self {
        let inner = inner as *const TextEvent as *const _;
        let txn: &yrs::TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        let txn = txn as *const _;
        YTextEvent { inner, txn }
    }

    fn txn(&self) -> &yrs::TransactionMut {
        unsafe { self.txn.as_ref().unwrap() }
    }
}

impl Deref for YTextEvent {
    type Target = TextEvent;

    fn deref(&self) -> &Self::Target {
        unsafe { (self.inner as *const TextEvent).as_ref().unwrap() }
    }
}

/// Event pushed into callbacks registered with `yarray_observe` function. It contains delta of all
/// content changes made within a scope of corresponding transaction (see: `yarray_event_delta`) as
/// well as navigation data used to identify a `YArray` instance which triggered this event.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct YArrayEvent {
    inner: *const c_void,
    txn: *const yrs::TransactionMut<'static>,
}

impl YArrayEvent {
    fn new<'doc>(inner: &ArrayEvent, txn: &yrs::TransactionMut<'doc>) -> Self {
        let inner = inner as *const ArrayEvent as *const _;
        let txn: &yrs::TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        let txn = txn as *const _;
        YArrayEvent { inner, txn }
    }

    fn txn(&self) -> &yrs::TransactionMut {
        unsafe { self.txn.as_ref().unwrap() }
    }
}

impl Deref for YArrayEvent {
    type Target = ArrayEvent;

    fn deref(&self) -> &Self::Target {
        unsafe { (self.inner as *const ArrayEvent).as_ref().unwrap() }
    }
}

/// Event pushed into callbacks registered with `ymap_observe` function. It contains all
/// key-value changes made within a scope of corresponding transaction (see: `ymap_event_keys`) as
/// well as navigation data used to identify a `YMap` instance which triggered this event.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct YMapEvent {
    inner: *const c_void,
    txn: *const yrs::TransactionMut<'static>,
}

impl YMapEvent {
    fn new<'doc>(inner: &MapEvent, txn: &yrs::TransactionMut<'doc>) -> Self {
        let inner = inner as *const MapEvent as *const _;
        let txn: &yrs::TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        let txn = txn as *const _;
        YMapEvent { inner, txn }
    }

    fn txn(&self) -> &yrs::TransactionMut<'static> {
        unsafe { self.txn.as_ref().unwrap() }
    }
}

impl Deref for YMapEvent {
    type Target = MapEvent;

    fn deref(&self) -> &Self::Target {
        unsafe { (self.inner as *const MapEvent).as_ref().unwrap() }
    }
}

/// Event pushed into callbacks registered with `yxmlelem_observe` function. It contains
/// all attribute changes made within a scope of corresponding transaction
/// (see: `yxmlelem_event_keys`) as well as child XML nodes changes (see: `yxmlelem_event_delta`)
/// and navigation data used to identify a `YXmlElement` instance which triggered this event.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct YXmlEvent {
    inner: *const c_void,
    txn: *const yrs::TransactionMut<'static>,
}

impl YXmlEvent {
    fn new<'doc>(inner: &XmlEvent, txn: &yrs::TransactionMut<'doc>) -> Self {
        let inner = inner as *const XmlEvent as *const _;
        let txn: &yrs::TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        let txn = txn as *const _;
        YXmlEvent { inner, txn }
    }

    fn txn(&self) -> &yrs::TransactionMut<'static> {
        unsafe { self.txn.as_ref().unwrap() }
    }
}

impl Deref for YXmlEvent {
    type Target = XmlEvent;

    fn deref(&self) -> &Self::Target {
        unsafe { (self.inner as *const XmlEvent).as_ref().unwrap() }
    }
}

/// Event pushed into callbacks registered with `yxmltext_observe` function. It contains
/// all attribute changes made within a scope of corresponding transaction
/// (see: `yxmltext_event_keys`) as well as text edits (see: `yxmltext_event_delta`)
/// and navigation data used to identify a `YXmlText` instance which triggered this event.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct YXmlTextEvent {
    inner: *const c_void,
    txn: *const yrs::TransactionMut<'static>,
}

impl YXmlTextEvent {
    fn new<'doc>(inner: &XmlTextEvent, txn: &yrs::TransactionMut<'doc>) -> Self {
        let inner = inner as *const XmlTextEvent as *const _;
        let txn: &yrs::TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        let txn = txn as *const _;
        YXmlTextEvent { inner, txn }
    }

    fn txn(&self) -> &yrs::TransactionMut<'static> {
        unsafe { self.txn.as_ref().unwrap() }
    }
}

impl Deref for YXmlTextEvent {
    type Target = XmlTextEvent;

    fn deref(&self) -> &Self::Target {
        unsafe { (self.inner as *const XmlTextEvent).as_ref().unwrap() }
    }
}

/// Releases a callback subscribed via `ytext_observe` function represented by passed
/// observer parameter.
#[no_mangle]
pub unsafe extern "C" fn ytext_unobserve(txt: *const Branch, subscription_id: u32) {
    let txt = TextRef::from_raw_branch(txt);
    txt.unobserve(subscription_id as SubscriptionId);
}

/// Releases a callback subscribed via `yarray_observe` function represented by passed
/// observer parameter.
#[no_mangle]
pub unsafe extern "C" fn yarray_unobserve(array: *const Branch, subscription_id: u32) {
    let txt = ArrayRef::from_raw_branch(array);
    txt.unobserve(subscription_id as SubscriptionId);
}

/// Releases a callback subscribed via `ymap_observe` function represented by passed
/// observer parameter.
#[no_mangle]
pub unsafe extern "C" fn ymap_unobserve(map: *const Branch, subscription_id: u32) {
    let map = MapRef::from_raw_branch(map);
    map.unobserve(subscription_id as SubscriptionId);
}

/// Releases a callback subscribed via `yxmlelem_observe` function represented by passed
/// observer parameter.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_unobserve(xml: *const Branch, subscription_id: u32) {
    let xml = XmlElementRef::from_raw_branch(xml);
    xml.unobserve(subscription_id as SubscriptionId);
}

/// Releases a callback subscribed via `yxmltext_observe` function represented by passed
/// observer parameter.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_unobserve(xml: *const Branch, subscription_id: u32) {
    let xml = XmlTextRef::from_raw_branch(xml);
    xml.unobserve(subscription_id as SubscriptionId);
}

/// Releases a callback subscribed via `yobserve_deep` function represented by passed
/// observer parameter.
#[no_mangle]
pub unsafe extern "C" fn yunobserve_deep(ytype: *mut Branch, subscription_id: u32) {
    assert!(!ytype.is_null());
    let branch = ytype.as_mut().unwrap();
    branch.unobserve_deep(subscription_id as SubscriptionId);
}

/// Returns a pointer to a shared collection, which triggered passed event `e`.
#[no_mangle]
pub unsafe extern "C" fn ytext_event_target(e: *const YTextEvent) -> *mut Branch {
    assert!(!e.is_null());
    let out = (&*e).target().clone();
    out.into_raw_branch()
}

/// Returns a pointer to a shared collection, which triggered passed event `e`.
#[no_mangle]
pub unsafe extern "C" fn yarray_event_target(e: *const YArrayEvent) -> *mut Branch {
    assert!(!e.is_null());
    let out = (&*e).target().clone();
    Box::into_raw(Box::new(out)) as *mut _
}

/// Returns a pointer to a shared collection, which triggered passed event `e`.
#[no_mangle]
pub unsafe extern "C" fn ymap_event_target(e: *const YMapEvent) -> *mut Branch {
    assert!(!e.is_null());
    let out = (&*e).target().clone();
    Box::into_raw(Box::new(out)) as *mut _
}

/// Returns a pointer to a shared collection, which triggered passed event `e`.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_event_target(e: *const YXmlEvent) -> *mut Branch {
    assert!(!e.is_null());
    let out = (&*e).target().clone();
    Box::into_raw(Box::new(out)) as *mut _
}

/// Returns a pointer to a shared collection, which triggered passed event `e`.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_event_target(e: *const YXmlTextEvent) -> *mut Branch {
    assert!(!e.is_null());
    let out = (&*e).target().clone();
    Box::into_raw(Box::new(out)) as *mut _
}

/// Returns a path from a root type down to a current shared collection (which can be obtained using
/// `ytext_event_target` function). It can consist of either integer indexes (used by sequence
/// components) of *char keys (used by map components). `len` output parameter is used to provide
/// information about length of the path.
///
/// Path returned this way should be eventually released using `ypath_destroy`.
#[no_mangle]
pub unsafe extern "C" fn ytext_event_path(
    e: *const YTextEvent,
    len: *mut u32,
) -> *mut YPathSegment {
    assert!(!e.is_null());
    let e = &*e;
    let path: Vec<_> = e.path().into_iter().map(YPathSegment::from).collect();
    let out = path.into_boxed_slice();
    *len = out.len() as u32;
    Box::into_raw(out) as *mut _
}

/// Returns a path from a root type down to a current shared collection (which can be obtained using
/// `ymap_event_target` function). It can consist of either integer indexes (used by sequence
/// components) of *char keys (used by map components). `len` output parameter is used to provide
/// information about length of the path.
///
/// Path returned this way should be eventually released using `ypath_destroy`.
#[no_mangle]
pub unsafe extern "C" fn ymap_event_path(e: *const YMapEvent, len: *mut u32) -> *mut YPathSegment {
    assert!(!e.is_null());
    let e = &*e;
    let path: Vec<_> = e.path().into_iter().map(YPathSegment::from).collect();
    let out = path.into_boxed_slice();
    *len = out.len() as u32;
    Box::into_raw(out) as *mut _
}

/// Returns a path from a root type down to a current shared collection (which can be obtained using
/// `yxmlelem_event_path` function). It can consist of either integer indexes (used by sequence
/// components) of *char keys (used by map components). `len` output parameter is used to provide
/// information about length of the path.
///
/// Path returned this way should be eventually released using `ypath_destroy`.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_event_path(
    e: *const YXmlEvent,
    len: *mut u32,
) -> *mut YPathSegment {
    assert!(!e.is_null());
    let e = &*e;
    let path: Vec<_> = e.path().into_iter().map(YPathSegment::from).collect();
    let out = path.into_boxed_slice();
    *len = out.len() as u32;
    Box::into_raw(out) as *mut _
}

/// Returns a path from a root type down to a current shared collection (which can be obtained using
/// `yxmltext_event_path` function). It can consist of either integer indexes (used by sequence
/// components) of *char keys (used by map components). `len` output parameter is used to provide
/// information about length of the path.
///
/// Path returned this way should be eventually released using `ypath_destroy`.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_event_path(
    e: *const YXmlTextEvent,
    len: *mut u32,
) -> *mut YPathSegment {
    assert!(!e.is_null());
    let e = &*e;
    let path: Vec<_> = e.path().into_iter().map(YPathSegment::from).collect();
    let out = path.into_boxed_slice();
    *len = out.len() as u32;
    Box::into_raw(out) as *mut _
}

/// Returns a path from a root type down to a current shared collection (which can be obtained using
/// `yarray_event_target` function). It can consist of either integer indexes (used by sequence
/// components) of *char keys (used by map components). `len` output parameter is used to provide
/// information about length of the path.
///
/// Path returned this way should be eventually released using `ypath_destroy`.
#[no_mangle]
pub unsafe extern "C" fn yarray_event_path(
    e: *const YArrayEvent,
    len: *mut u32,
) -> *mut YPathSegment {
    assert!(!e.is_null());
    let e = &*e;
    let path: Vec<_> = e.path().into_iter().map(YPathSegment::from).collect();
    let out = path.into_boxed_slice();
    *len = out.len() as u32;
    Box::into_raw(out) as *mut _
}

/// Releases allocated memory used by objects returned from path accessor functions of shared type
/// events.
#[no_mangle]
pub unsafe extern "C" fn ypath_destroy(path: *mut YPathSegment, len: u32) {
    if !path.is_null() {
        drop(Vec::from_raw_parts(path, len as usize, len as usize));
    }
}

/// Returns a sequence of changes produced by sequence component of shared collections (such as
/// `YText`, `YXmlText` and XML nodes added to `YXmlElement`). `len` output parameter is used to
/// provide information about number of changes produced.
///
/// Delta returned from this function should eventually be released using `yevent_delta_destroy`
/// function.
#[no_mangle]
pub unsafe extern "C" fn ytext_event_delta(e: *const YTextEvent, len: *mut u32) -> *mut YDelta {
    assert!(!e.is_null());
    let e = &*e;
    let delta: Vec<_> = e.delta(e.txn()).into_iter().map(YDelta::from).collect();

    let out = delta.into_boxed_slice();
    *len = out.len() as u32;
    Box::into_raw(out) as *mut _
}

/// Returns a sequence of changes produced by sequence component of shared collections (such as
/// `YText`, `YXmlText` and XML nodes added to `YXmlElement`). `len` output parameter is used to
/// provide information about number of changes produced.
///
/// Delta returned from this function should eventually be released using `yevent_delta_destroy`
/// function.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_event_delta(
    e: *const YXmlTextEvent,
    len: *mut u32,
) -> *mut YDelta {
    assert!(!e.is_null());
    let e = &*e;
    let delta: Vec<_> = e.delta(e.txn()).into_iter().map(YDelta::from).collect();

    let out = delta.into_boxed_slice();
    *len = out.len() as u32;
    Box::into_raw(out) as *mut _
}

/// Returns a sequence of changes produced by sequence component of shared collections (such as
/// `YText`, `YXmlText` and XML nodes added to `YXmlElement`). `len` output parameter is used to
/// provide information about number of changes produced.
///
/// Delta returned from this function should eventually be released using `yevent_delta_destroy`
/// function.
#[no_mangle]
pub unsafe extern "C" fn yarray_event_delta(
    e: *const YArrayEvent,
    len: *mut u32,
) -> *mut YEventChange {
    assert!(!e.is_null());
    let e = &*e;
    let delta: Vec<_> = e
        .delta(e.txn())
        .into_iter()
        .map(YEventChange::from)
        .collect();

    let out = delta.into_boxed_slice();
    *len = out.len() as u32;
    Box::into_raw(out) as *mut _
}

/// Returns a sequence of changes produced by sequence component of shared collections (such as
/// `YText`, `YXmlText` and XML nodes added to `YXmlElement`). `len` output parameter is used to
/// provide information about number of changes produced.
///
/// Delta returned from this function should eventually be released using `yevent_delta_destroy`
/// function.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_event_delta(
    e: *const YXmlEvent,
    len: *mut u32,
) -> *mut YEventChange {
    assert!(!e.is_null());
    let e = &*e;
    let delta: Vec<_> = e
        .delta(e.txn())
        .into_iter()
        .map(YEventChange::from)
        .collect();

    let out = delta.into_boxed_slice();
    *len = out.len() as u32;
    Box::into_raw(out) as *mut _
}

/// Releases memory allocated by the object returned from `yevent_delta` function.
#[no_mangle]
pub unsafe extern "C" fn ytext_delta_destroy(delta: *mut YDelta, len: u32) {
    if !delta.is_null() {
        let delta = Vec::from_raw_parts(delta, len as usize, len as usize);
        drop(delta);
    }
}

/// Releases memory allocated by the object returned from `yevent_delta` function.
#[no_mangle]
pub unsafe extern "C" fn yevent_delta_destroy(delta: *mut YEventChange, len: u32) {
    if !delta.is_null() {
        let delta = Vec::from_raw_parts(delta, len as usize, len as usize);
        drop(delta);
    }
}

/// Returns a sequence of changes produced by map component of shared collections (such as
/// `YMap` and `YXmlText`/`YXmlElement` attribute changes). `len` output parameter is used to
/// provide information about number of changes produced.
///
/// Delta returned from this function should eventually be released using `yevent_keys_destroy`
/// function.
#[no_mangle]
pub unsafe extern "C" fn ymap_event_keys(
    e: *const YMapEvent,
    len: *mut u32,
) -> *mut YEventKeyChange {
    assert!(!e.is_null());
    let e = &*e;
    let delta: Vec<_> = e
        .keys(e.txn())
        .into_iter()
        .map(|(k, v)| YEventKeyChange::new(k.as_ref(), v))
        .collect();

    let out = delta.into_boxed_slice();
    *len = out.len() as u32;
    Box::into_raw(out) as *mut _
}

/// Returns a sequence of changes produced by map component of shared collections.
/// `len` output parameter is used to provide information about number of changes produced.
///
/// Delta returned from this function should eventually be released using `yevent_keys_destroy`
/// function.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_event_keys(
    e: *const YXmlEvent,
    len: *mut u32,
) -> *mut YEventKeyChange {
    assert!(!e.is_null());
    let e = &*e;
    let delta: Vec<_> = e
        .keys(e.txn())
        .into_iter()
        .map(|(k, v)| YEventKeyChange::new(k.as_ref(), v))
        .collect();

    let out = delta.into_boxed_slice();
    *len = out.len() as u32;
    Box::into_raw(out) as *mut _
}

/// Returns a sequence of changes produced by map component of shared collections.
/// `len` output parameter is used to provide information about number of changes produced.
///
/// Delta returned from this function should eventually be released using `yevent_keys_destroy`
/// function.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_event_keys(
    e: *const YXmlTextEvent,
    len: *mut u32,
) -> *mut YEventKeyChange {
    assert!(!e.is_null());
    let e = &*e;
    let delta: Vec<_> = e
        .keys(e.txn())
        .into_iter()
        .map(|(k, v)| YEventKeyChange::new(k.as_ref(), v))
        .collect();

    let out = delta.into_boxed_slice();
    *len = out.len() as u32;
    Box::into_raw(out) as *mut _
}

/// Releases memory allocated by the object returned from `yxml_event_keys` and `ymap_event_keys`
/// functions.
#[no_mangle]
pub unsafe extern "C" fn yevent_keys_destroy(keys: *mut YEventKeyChange, len: u32) {
    if !keys.is_null() {
        drop(Vec::from_raw_parts(keys, len as usize, len as usize));
    }
}

#[repr(C)]
pub struct YUndoManagerOptions {
    pub capture_timeout_millis: u32,
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager(
    doc: *const Doc,
    ytype: *const Branch,
    options: *const YUndoManagerOptions,
) -> *mut UndoManager {
    let doc = doc.as_ref().unwrap();
    let branch = ytype.as_ref().unwrap();

    let mut o = yrs::undo::Options::default();
    if let Some(options) = options.as_ref() {
        if options.capture_timeout_millis >= 0 {
            o.capture_timeout_millis = options.capture_timeout_millis as u64;
        }
    };
    let boxed = Box::new(UndoManager::with_options(doc, &BranchPtr::from(branch), o));
    Box::into_raw(boxed)
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager_destroy(mgr: *mut UndoManager) {
    drop(Box::from_raw(mgr));
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager_add_origin(
    mgr: *mut UndoManager,
    origin_len: u32,
    origin: *const c_char,
) {
    let mgr = mgr.as_mut().unwrap();
    let bytes = std::slice::from_raw_parts(origin as *const u8, origin_len as usize);
    mgr.include_origin(Origin::from(bytes));
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager_remove_origin(
    mgr: *mut UndoManager,
    origin_len: u32,
    origin: *const c_char,
) {
    let mgr = mgr.as_mut().unwrap();
    let bytes = std::slice::from_raw_parts(origin as *const u8, origin_len as usize);
    mgr.exclude_origin(Origin::from(bytes));
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager_add_scope(mgr: *mut UndoManager, ytype: *const Branch) {
    let mgr = mgr.as_mut().unwrap();
    let branch = ytype.as_ref().unwrap();
    mgr.expand_scope(&BranchPtr::from(branch));
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager_clear(mgr: *mut UndoManager) -> u8 {
    let mgr = mgr.as_mut().unwrap();
    match mgr.clear() {
        Ok(_) => Y_TRUE,
        Err(_) => Y_FALSE,
    }
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager_stop(mgr: *mut UndoManager) {
    let mgr = mgr.as_mut().unwrap();
    mgr.reset();
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager_undo(mgr: *mut UndoManager) -> u8 {
    let mgr = mgr.as_mut().unwrap();
    match mgr.undo() {
        Ok(_) => Y_TRUE,
        Err(_) => Y_FALSE,
    }
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager_redo(mgr: *mut UndoManager) -> u8 {
    let mgr = mgr.as_mut().unwrap();
    match mgr.redo() {
        Ok(_) => Y_TRUE,
        Err(_) => Y_FALSE,
    }
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager_can_undo(mgr: *mut UndoManager) -> u8 {
    let mgr = mgr.as_mut().unwrap();
    if mgr.can_undo() {
        Y_TRUE
    } else {
        Y_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager_can_redo(mgr: *mut UndoManager) -> u8 {
    let mgr = mgr.as_mut().unwrap();
    if mgr.can_redo() {
        Y_TRUE
    } else {
        Y_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager_observe_added(
    mgr: *mut UndoManager,
    state: *mut c_void,
    cb: extern "C" fn(*mut c_void, *const YUndoEvent),
) -> u32 {
    let mgr = mgr.as_mut().unwrap();
    let subscription_id: SubscriptionId = mgr
        .observe_item_added(move |_, e| {
            let event = YUndoEvent::new(e);
            cb(state, &event as *const YUndoEvent);
        })
        .into();
    subscription_id
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager_unobserve_added(
    mgr: *mut UndoManager,
    subscription_id: u32,
) {
    let mgr = mgr.as_mut().unwrap();
    mgr.unobserve_item_added(subscription_id as SubscriptionId);
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager_observe_popped(
    mgr: *mut UndoManager,
    state: *mut c_void,
    cb: extern "C" fn(*mut c_void, *const YUndoEvent),
) -> u32 {
    let mgr = mgr.as_mut().unwrap();
    let subscription_id: SubscriptionId = mgr
        .observe_item_popped(move |_, e| {
            let event = YUndoEvent::new(e);
            cb(state, &event as *const YUndoEvent);
        })
        .into();
    subscription_id
}

#[no_mangle]
pub unsafe extern "C" fn yundo_manager_unobserve_popped(
    mgr: *mut UndoManager,
    subscription_id: u32,
) {
    let mgr = mgr.as_mut().unwrap();
    mgr.unobserve_item_popped(subscription_id as SubscriptionId);
}

pub const Y_KIND_UNDO: c_char = 0;
pub const Y_KIND_REDO: c_char = 1;

#[repr(C)]
pub struct YUndoEvent {
    pub kind: c_char,
    pub origin: *const c_char,
    pub origin_len: u32,
    pub insertions: YDeleteSet,
    pub deletions: YDeleteSet,
}

impl YUndoEvent {
    unsafe fn new(e: &yrs::undo::Event) -> Self {
        let (origin, origin_len) = if let Some(origin) = e.origin.as_ref() {
            let bytes = origin.as_ref();
            let origin_len = bytes.len() as u32;
            let origin = bytes.as_ptr() as *const c_char;
            (origin, origin_len)
        } else {
            (null(), 0)
        };
        YUndoEvent {
            kind: match e.kind {
                EventKind::Undo => Y_KIND_UNDO,
                EventKind::Redo => Y_KIND_REDO,
            },
            origin,
            origin_len,
            insertions: YDeleteSet::new(e.item.insertions()),
            deletions: YDeleteSet::new(e.item.deletions()),
        }
    }
}

/// Returns a value informing what kind of Yrs shared collection given `branch` represents.
/// Returns either 0 when `branch` is null or one of values: `Y_ARRAY`, `Y_TEXT`, `Y_MAP`,
/// `Y_XML_ELEM`, `Y_XML_TEXT`.
#[no_mangle]
pub unsafe extern "C" fn ytype_kind(branch: *const Branch) -> i8 {
    if let Some(branch) = branch.as_ref() {
        match branch.type_ref() {
            TYPE_REFS_ARRAY => Y_ARRAY,
            TYPE_REFS_MAP => Y_MAP,
            TYPE_REFS_TEXT => Y_TEXT,
            TYPE_REFS_XML_ELEMENT => Y_XML_ELEM,
            TYPE_REFS_XML_TEXT => Y_XML_TEXT,
            TYPE_REFS_XML_FRAGMENT => Y_XML_FRAG,
            TYPE_REFS_DOC => Y_DOC,
            other => panic!("Unknown kind: {}", other),
        }
    } else {
        0
    }
}

/// Tag used to identify `YPathSegment` storing a *char parameter.
pub const Y_EVENT_PATH_KEY: c_char = 1;

/// Tag used to identify `YPathSegment` storing an int parameter.
pub const Y_EVENT_PATH_INDEX: c_char = 2;

/// A single segment of a path returned from `yevent_path` function. It can be one of two cases,
/// recognized by it's `tag` field:
///
/// 1. `Y_EVENT_PATH_KEY` means that segment value can be accessed by `segment.value.key` and is
/// referring to a string key used by map component (eg. `YMap` entry).
/// 2. `Y_EVENT_PATH_INDEX` means that segment value can be accessed by `segment.value.index` and is
/// referring to an int index used by sequence component (eg. `YArray` item or `YXmlElement` child).
#[repr(C)]
pub struct YPathSegment {
    /// Tag used to identify which case current segment is referring to:
    ///
    /// 1. `Y_EVENT_PATH_KEY` means that segment value can be accessed by `segment.value.key` and is
    /// referring to a string key used by map component (eg. `YMap` entry).
    /// 2. `Y_EVENT_PATH_INDEX` means that segment value can be accessed by `segment.value.index`
    /// and is referring to an int index used by sequence component (eg. `YArray` item or
    /// `YXmlElement` child).
    pub tag: c_char,

    /// Union field containing either `key` or `index`. A particular case can be recognized by using
    /// segment's `tag` field.
    pub value: YPathSegmentCase,
}

impl From<PathSegment> for YPathSegment {
    fn from(ps: PathSegment) -> Self {
        match ps {
            PathSegment::Key(key) => {
                let key = CString::new(key.as_ref()).unwrap().into_raw() as *const _;
                YPathSegment {
                    tag: Y_EVENT_PATH_KEY,
                    value: YPathSegmentCase { key },
                }
            }
            PathSegment::Index(index) => YPathSegment {
                tag: Y_EVENT_PATH_INDEX,
                value: YPathSegmentCase {
                    index: index as u32,
                },
            },
        }
    }
}

impl Drop for YPathSegment {
    fn drop(&mut self) {
        if self.tag == Y_EVENT_PATH_KEY {
            unsafe {
                ystring_destroy(self.value.key as *mut _);
            }
        }
    }
}

#[repr(C)]
pub union YPathSegmentCase {
    pub key: *const c_char,
    pub index: u32,
}

/// Tag used to identify `YEventChange` (see: `yevent_delta` function) case, when a new element
/// has been added to an observed collection.
pub const Y_EVENT_CHANGE_ADD: c_char = 1;

/// Tag used to identify `YEventChange` (see: `yevent_delta` function) case, when an existing
/// element has been removed from an observed collection.
pub const Y_EVENT_CHANGE_DELETE: c_char = 2;

/// Tag used to identify `YEventChange` (see: `yevent_delta` function) case, when no changes have
/// been detected for a particular range of observed collection.
pub const Y_EVENT_CHANGE_RETAIN: c_char = 3;

/// A data type representing a single change detected over an observed shared collection. A type
/// of change can be detected using a `tag` field:
///
/// 1. `Y_EVENT_CHANGE_ADD` marks a new elements added to a collection. In this case `values` field
/// contains a pointer to a list of newly inserted values, while `len` field informs about their
/// count.
/// 2. `Y_EVENT_CHANGE_DELETE` marks an existing elements removed from the collection. In this case
/// `len` field informs about number of removed elements.
/// 3. `Y_EVENT_CHANGE_RETAIN` marks a number of elements that have not been changed, counted from
/// the previous element. `len` field informs about number of retained elements.
///
/// A list of changes returned by `yarray_event_delta`/`yxml_event_delta` enables to locate a
/// position of all changes within an observed collection by using a combination of added/deleted
/// change structs separated by retained changes (marking eg. number of elements that can be safely
/// skipped, since they remained unchanged).
#[repr(C)]
pub struct YEventChange {
    /// Tag field used to identify particular type of change made:
    ///
    /// 1. `Y_EVENT_CHANGE_ADD` marks a new elements added to a collection. In this case `values`
    /// field contains a pointer to a list of newly inserted values, while `len` field informs about
    /// their count.
    /// 2. `Y_EVENT_CHANGE_DELETE` marks an existing elements removed from the collection. In this
    /// case `len` field informs about number of removed elements.
    /// 3. `Y_EVENT_CHANGE_RETAIN` marks a number of elements that have not been changed, counted
    /// from the previous element. `len` field informs about number of retained elements.
    pub tag: c_char,

    /// Number of element affected by current type of a change. It can refer to a number of
    /// inserted `values`, number of deleted element or a number of retained (unchanged) values.  
    pub len: u32,

    /// Used in case when current change is of `Y_EVENT_CHANGE_ADD` type. Contains a list (of
    /// length stored in `len` field) of newly inserted values.
    pub values: *const YOutput,
}

impl<'a> From<&'a Change> for YEventChange {
    fn from(change: &'a Change) -> Self {
        match change {
            Change::Added(values) => {
                let out: Vec<_> = values
                    .into_iter()
                    .map(|v| YOutput::from(v.clone()))
                    .collect();
                let len = out.len() as u32;
                let out = out.into_boxed_slice();
                let values = Box::into_raw(out) as *mut _;

                YEventChange {
                    tag: Y_EVENT_CHANGE_ADD,
                    len,
                    values,
                }
            }
            Change::Removed(len) => YEventChange {
                tag: Y_EVENT_CHANGE_DELETE,
                len: *len as u32,
                values: null(),
            },
            Change::Retain(len) => YEventChange {
                tag: Y_EVENT_CHANGE_RETAIN,
                len: *len as u32,
                values: null(),
            },
        }
    }
}

impl Drop for YEventChange {
    fn drop(&mut self) {
        if self.tag == Y_EVENT_CHANGE_ADD {
            unsafe {
                let len = self.len as usize;
                let values = Vec::from_raw_parts(self.values as *mut YOutput, len, len);
                drop(values);
            }
        }
    }
}

/// A data type representing a single change detected over an observed `YText`/`YXmlText`. A type
/// of change can be detected using a `tag` field:
///
/// 1. `Y_EVENT_CHANGE_ADD` marks a new characters added to a collection. In this case `insert`
/// field contains a pointer to a list of newly inserted values, while `len` field informs about
/// their count. Additionally `attributes_len` nad `attributes` carry information about optional
/// formatting attributes applied to edited blocks.
/// 2. `Y_EVENT_CHANGE_DELETE` marks an existing elements removed from the collection. In this case
/// `len` field informs about number of removed elements.
/// 3. `Y_EVENT_CHANGE_RETAIN` marks a number of characters that have not been changed, counted from
/// the previous element. `len` field informs about number of retained elements. Additionally
/// `attributes_len` nad `attributes` carry information about optional formatting attributes applied
/// to edited blocks.
///
/// A list of changes returned by `ytext_event_delta`/`yxmltext_event_delta` enables to locate
/// a position of all changes within an observed collection by using a combination of added/deleted
/// change structs separated by retained changes (marking eg. number of elements that can be safely
/// skipped, since they remained unchanged).
#[repr(C)]
pub struct YDelta {
    /// Tag field used to identify particular type of change made:
    ///
    /// 1. `Y_EVENT_CHANGE_ADD` marks a new elements added to a collection. In this case `values`
    /// field contains a pointer to a list of newly inserted values, while `len` field informs about
    /// their count.
    /// 2. `Y_EVENT_CHANGE_DELETE` marks an existing elements removed from the collection. In this
    /// case `len` field informs about number of removed elements.
    /// 3. `Y_EVENT_CHANGE_RETAIN` marks a number of elements that have not been changed, counted
    /// from the previous element. `len` field informs about number of retained elements.
    pub tag: c_char,

    /// Number of element affected by current type of a change. It can refer to a number of
    /// inserted `values`, number of deleted element or a number of retained (unchanged) values.  
    pub len: u32,

    /// Used in case when current change is of `Y_EVENT_CHANGE_ADD` type. Contains a list (of
    /// length stored in `len` field) of newly inserted values.
    pub insert: *mut YOutput,

    /// A number of formatting attributes assigned to an edited area represented by this delta.
    pub attributes_len: u32,

    /// A nullable pointer to a list of formatting attributes assigned to an edited area represented
    /// by this delta.
    pub attributes: *mut YDeltaAttr,
}

impl YDelta {
    fn insert(value: &Value, attrs: &Option<Box<Attrs>>) -> Self {
        let insert = Box::into_raw(Box::new(YOutput::from(value.clone())));
        let (attributes_len, attributes) = if let Some(attrs) = attrs {
            let len = attrs.len() as u32;
            let attrs: Vec<_> = attrs.iter().map(|(k, v)| YDeltaAttr::new(k, v)).collect();
            let attrs = Box::into_raw(attrs.into_boxed_slice()) as *mut _;
            (len, attrs)
        } else {
            (0, null_mut())
        };

        YDelta {
            tag: Y_EVENT_CHANGE_ADD,
            len: 1,
            insert,
            attributes_len,
            attributes,
        }
    }

    fn retain(len: u32, attrs: &Option<Box<Attrs>>) -> Self {
        let (attributes_len, attributes) = if let Some(attrs) = attrs {
            let len = attrs.len() as u32;
            let attrs: Vec<_> = attrs.iter().map(|(k, v)| YDeltaAttr::new(k, v)).collect();
            let attrs = Box::into_raw(attrs.into_boxed_slice()) as *mut _;
            (len, attrs)
        } else {
            (0, null_mut())
        };
        YDelta {
            tag: Y_EVENT_CHANGE_RETAIN,
            len: len as u32,
            insert: null_mut(),
            attributes_len,
            attributes,
        }
    }

    fn delete(len: u32) -> Self {
        YDelta {
            tag: Y_EVENT_CHANGE_DELETE,
            len: len as u32,
            insert: null_mut(),
            attributes_len: 0,
            attributes: null_mut(),
        }
    }
}

impl<'a> From<&'a Delta> for YDelta {
    fn from(d: &Delta) -> Self {
        match d {
            Delta::Inserted(value, attrs) => YDelta::insert(value, attrs),
            Delta::Retain(len, attrs) => YDelta::retain(*len, attrs),
            Delta::Deleted(len) => YDelta::delete(*len),
        }
    }
}

impl Drop for YDelta {
    fn drop(&mut self) {
        unsafe {
            if !self.attributes.is_null() {
                let len = self.attributes_len as usize;
                drop(Vec::from_raw_parts(self.attributes, len, len));
            }
            if !self.attributes.is_null() {
                drop(Box::from_raw(self.insert));
            }
        }
    }
}

/// A single instance of formatting attribute stored as part of `YDelta` instance.
#[repr(C)]
pub struct YDeltaAttr {
    /// A null-terminated UTF-8 encoded string containing a unique formatting attribute name.
    pub key: *const c_char,
    /// A value assigned to a formatting attribute.
    pub value: YOutput,
}

impl YDeltaAttr {
    fn new(k: &Rc<str>, v: &Any) -> Self {
        let key = CString::new(k.as_ref()).unwrap().into_raw() as *const _;
        let value = YOutput::from(v.clone());
        YDeltaAttr { key, value }
    }
}

impl Drop for YDeltaAttr {
    fn drop(&mut self) {
        unsafe { ystring_destroy(self.key as *mut _) }
    }
}

/// Tag used to identify `YEventKeyChange` (see: `yevent_keys` function) case, when a new entry has
/// been inserted into a map component of shared collection.
pub const Y_EVENT_KEY_CHANGE_ADD: c_char = 4;

/// Tag used to identify `YEventKeyChange` (see: `yevent_keys` function) case, when an existing
/// entry has been removed from a map component of shared collection.
pub const Y_EVENT_KEY_CHANGE_DELETE: c_char = 5;

/// Tag used to identify `YEventKeyChange` (see: `yevent_keys` function) case, when an existing
/// entry has been overridden with a new value within a map component of shared collection.
pub const Y_EVENT_KEY_CHANGE_UPDATE: c_char = 6;

/// A data type representing a single change made over a map component of shared collection types,
/// such as `YMap` entries or `YXmlText`/`YXmlElement` attributes. A `key` field provides a
/// corresponding unique key string of a changed entry, while `tag` field informs about specific
/// type of change being done:
///
/// 1. `Y_EVENT_KEY_CHANGE_ADD` used to identify a newly added entry. In this case an `old_value`
/// field is NULL, while `new_value` field contains an inserted value.
/// 1. `Y_EVENT_KEY_CHANGE_DELETE` used to identify an existing entry being removed. In this case
/// an `old_value` field contains the removed value.
/// 1. `Y_EVENT_KEY_CHANGE_UPDATE` used to identify an existing entry, which value has been changed.
/// In this case `old_value` field contains replaced value, while `new_value` contains a newly
/// inserted one.
#[repr(C)]
pub struct YEventKeyChange {
    /// A UTF8-encoded null-terminated string containing a key of a changed entry.
    pub key: *const c_char,
    /// Tag field informing about type of change current struct refers to:
    ///
    /// 1. `Y_EVENT_KEY_CHANGE_ADD` used to identify a newly added entry. In this case an
    /// `old_value` field is NULL, while `new_value` field contains an inserted value.
    /// 1. `Y_EVENT_KEY_CHANGE_DELETE` used to identify an existing entry being removed. In this
    /// case an `old_value` field contains the removed value.
    /// 1. `Y_EVENT_KEY_CHANGE_UPDATE` used to identify an existing entry, which value has been
    /// changed. In this case `old_value` field contains replaced value, while `new_value` contains
    /// a newly inserted one.
    pub tag: c_char,

    /// Contains a removed entry's value or replaced value of an updated entry.
    pub old_value: *const YOutput,

    /// Contains a value of newly inserted entry or an updated entry's new value.
    pub new_value: *const YOutput,
}

impl YEventKeyChange {
    fn new(key: &str, change: &EntryChange) -> Self {
        let key = CString::new(key).unwrap().into_raw() as *const _;
        match change {
            EntryChange::Inserted(new) => YEventKeyChange {
                key,
                tag: Y_EVENT_KEY_CHANGE_ADD,
                old_value: null(),
                new_value: Box::into_raw(Box::new(YOutput::from(new.clone()))),
            },
            EntryChange::Updated(old, new) => YEventKeyChange {
                key,
                tag: Y_EVENT_KEY_CHANGE_UPDATE,
                old_value: Box::into_raw(Box::new(YOutput::from(old.clone()))),
                new_value: Box::into_raw(Box::new(YOutput::from(new.clone()))),
            },
            EntryChange::Removed(old) => YEventKeyChange {
                key,
                tag: Y_EVENT_KEY_CHANGE_DELETE,
                old_value: Box::into_raw(Box::new(YOutput::from(old.clone()))),
                new_value: null(),
            },
        }
    }
}

impl Drop for YEventKeyChange {
    fn drop(&mut self) {
        unsafe {
            ystring_destroy(self.key as *mut _);
            youtput_destroy(self.old_value as *mut _);
            youtput_destroy(self.new_value as *mut _);
        }
    }
}

trait BranchPointable {
    fn into_raw_branch(self) -> *mut Branch;
    fn from_raw_branch(branch: *const Branch) -> Self;
}

impl<T> BranchPointable for T
where
    T: AsRef<Branch> + From<BranchPtr>,
{
    fn into_raw_branch(self) -> *mut Branch {
        let branch_ref = self.as_ref();
        branch_ref as *const Branch as *mut Branch
    }

    fn from_raw_branch(branch: *const Branch) -> Self {
        let b = unsafe { branch.as_ref().unwrap() };
        let branch_ref = BranchPtr::from(b);
        T::from(branch_ref)
    }
}

/// A sticky index is based on the Yjs model and is not affected by document changes.
/// E.g. If you place a sticky index before a certain character, it will always point to this character.
/// If you place a sticky index at the end of a type, it will always point to the end of the type.
///
/// A numeric position is often unsuited for user selections, because it does not change when content is inserted
/// before or after.
///
/// ```Insert(0, 'x')('a.bc') = 'xa.bc'``` Where `.` is the sticky index position.
///
/// Instances of `YStickyIndex` can be freed using `ysticky_index_destroy`.
#[repr(transparent)]
pub struct YStickyIndex(StickyIndex);

impl From<StickyIndex> for YStickyIndex {
    #[inline(always)]
    fn from(value: StickyIndex) -> Self {
        YStickyIndex(value)
    }
}

/// Releases resources allocated by `YStickyIndex` pointers.
#[no_mangle]
pub unsafe extern "C" fn ysticky_index_destroy(pos: *mut YStickyIndex) {
    drop(Box::from_raw(pos))
}

/// Returns association of current `YStickyIndex`.
/// If association is **after** the referenced inserted character, returned number will be >= 0.
/// If association is **before** the referenced inserted character, returned number will be < 0.
#[no_mangle]
pub unsafe extern "C" fn ysticky_index_assoc(pos: *const YStickyIndex) -> i8 {
    let pos = pos.as_ref().unwrap();
    match pos.0.assoc {
        Assoc::After => 0,
        Assoc::Before => -1,
    }
}

/// Retrieves a `YStickyIndex` corresponding to a given human-readable `index` pointing into
/// the shared y-type `branch`. Unlike standard indexes sticky one enables to track
/// the location inside of a shared y-types, even in the face of concurrent updates.
///
/// If association is >= 0, the resulting position will point to location **after** the referenced index.
/// If association is < 0, the resulting position will point to location **before** the referenced index.
#[no_mangle]
pub unsafe extern "C" fn ysticky_index_from_index(
    branch: *const Branch,
    txn: *mut Transaction,
    index: u32,
    assoc: i8,
) -> *mut YStickyIndex {
    assert!(!branch.is_null());
    assert!(!txn.is_null());

    let branch = BranchPtr::from_raw_branch(branch);
    let txn = txn.as_mut().unwrap();
    let index = index as u32;
    let assoc = if assoc >= 0 {
        Assoc::After
    } else {
        Assoc::Before
    };

    if let Some(txn) = txn.as_mut() {
        if let Some(pos) = StickyIndex::at(txn, branch, index, assoc) {
            Box::into_raw(Box::new(YStickyIndex(pos)))
        } else {
            null_mut()
        }
    } else {
        panic!("ysticky_index_from_index requires a read-write transaction");
    }
}

/// Serializes `YStickyIndex` into binary representation. `len` parameter is updated with byte
/// length of the generated binary. Returned binary can be free'd using `ybinary_destroy`.  
#[no_mangle]
pub unsafe extern "C" fn ysticky_index_encode(
    pos: *const YStickyIndex,
    len: *mut u32,
) -> *mut c_char {
    let pos = pos.as_ref().unwrap();
    let binary = pos.0.encode_v1().into_boxed_slice();
    *len = binary.len() as u32;
    Box::into_raw(binary) as *mut c_char
}

/// Deserializes `YStickyIndex` from the payload previously serialized using `ysticky_index_encode`.
#[no_mangle]
pub unsafe extern "C" fn ysticky_index_decode(
    binary: *const c_char,
    len: u32,
) -> *mut YStickyIndex {
    let slice = std::slice::from_raw_parts(binary as *const u8, len as usize);
    if let Ok(pos) = StickyIndex::decode_v1(slice) {
        Box::into_raw(Box::new(YStickyIndex(pos)))
    } else {
        null_mut()
    }
}

/// Given `YStickyIndex` and transaction reference, if computes a human-readable index in a
/// context of the referenced shared y-type.
///
/// `out_branch` is getting assigned with a corresponding shared y-type reference.
/// `out_index` will be used to store computed human-readable index.
#[no_mangle]
pub unsafe extern "C" fn ysticky_index_read(
    pos: *const YStickyIndex,
    txn: *const Transaction,
    out_branch: *mut *mut Branch,
    out_index: *mut u32,
) {
    let pos = pos.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    if let Some(abs) = pos.0.get_offset(txn) {
        *out_branch = abs.branch.as_ref() as *const Branch as *mut Branch;
        *out_index = abs.index as u32;
    }
}

#[cfg(test)]
mod test {
    use crate::*;

    #[test]
    fn yval_preliminary_types() {
        unsafe {
            let doc = ydoc_new();
            let array_name = CString::new("test").unwrap();
            let array = yarray(doc, array_name.as_ptr());
            let txn = ydoc_write_transaction(doc, 0, null());

            let y_true = yinput_bool(Y_TRUE);
            let y_false = yinput_bool(Y_FALSE);
            let y_float = yinput_float(0.5);
            let y_int = yinput_long(11);
            let input = CString::new("hello").unwrap();
            let y_str = yinput_string(input.as_ptr());

            let values = &[y_true, y_false, y_float, y_int, y_str];

            yarray_insert_range(array, txn, 0, values.as_ptr(), 5);

            ytransaction_commit(txn);
            ydoc_destroy(doc);
        }
    }
}
