use lib0::any::Any;
use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::mem::{forget, ManuallyDrop, MaybeUninit};
use std::ops::Deref;
use std::os::raw::{c_char, c_float, c_int, c_long, c_longlong, c_uchar, c_uint, c_ulong};
use std::ptr::{null, null_mut};
use yrs::block::{ItemContent, Prelim};
use yrs::types::array::ArrayEvent;
use yrs::types::map::MapEvent;
use yrs::types::text::TextEvent;
use yrs::types::xml::{XmlEvent, XmlTextEvent};
use yrs::types::{
    Attrs, BranchPtr, Change, Delta, EntryChange, PathSegment, Value, TYPE_REFS_ARRAY,
    TYPE_REFS_MAP, TYPE_REFS_TEXT, TYPE_REFS_XML_ELEMENT, TYPE_REFS_XML_TEXT,
};
use yrs::updates::decoder::{Decode, DecoderV1, DecoderV2};
use yrs::updates::encoder::{Encode, Encoder, EncoderV1, EncoderV2};
use yrs::{Array, Map, OffsetKind, Text, Update, XmlElement, XmlText};
use yrs::{Options, StateVector};
use yrs::{SubscriptionId, Xml};

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

/// Flag used to mark a truthy boolean numbers.
pub const Y_TRUE: c_char = 1;

/// Flag used to mark a falsy boolean numbers.
pub const Y_FALSE: c_char = 0;

/// Flag used by `YOptions` to determine, that text operations offsets and length will be counted by
/// the byte number of UTF8-encoded string.
pub const Y_OFFSET_BYTES: c_int = 0;

/// Flag used by `YOptions` to determine, that text operations offsets and length will be counted by
/// UTF-16 chars of encoded string.
pub const Y_OFFSET_UTF16: c_int = 1;

/// Flag used by `YOptions` to determine, that text operations offsets and length will be counted by
/// by UTF-32 chars of encoded string.
pub const Y_OFFSET_UTF32: c_int = 2;

/* pub types below are used by cbindgen for c header generation */

/// A Yrs document type. Documents are most important units of collaborative resources management.
/// All shared collections live within a scope of their corresponding documents. All updates are
/// generated on per document basis (rather than individual shared type). All operations on shared
/// collections happen via `YTransaction`, which lifetime is also bound to a document.
///
/// Document manages so called root types, which are top-level shared types definitions (as opposed
/// to recursively nested types).
pub type Doc = yrs::Doc;

/// Transaction is one of the core types in Yrs. All operations that need to touch a document's
/// contents (a.k.a. block store), need to be executed in scope of a transaction.
pub type Transaction = yrs::Transaction;

/// A common shared data type. All Yrs instances can be refered to using this data type (use
/// `ytype_kind` function if a specific type needs to be determined). Branch pointers are passed
/// over type-specific functions like `ytext_insert`, `yarray_insert` or `ymap_insert` to perform
/// a specific shared type operations.
///
/// Using write methods of different shared types (eg. `ytext_insert` and `yarray_insert`) over
/// the same branch may result in undefined behavior.
pub type Branch = yrs::types::Branch;

/// Iterator structure used by shared array data type.
pub type ArrayIter = yrs::types::array::ArrayIter<'static>;

/// Iterator structure used by shared map data type. Map iterators are unordered - there's no
/// specific order in which map entries will be returned during consecutive iterator calls.
pub type MapIter = yrs::types::map::MapIter<'static>;

/// Iterator structure used by XML nodes (elements and text) to iterate over node's attributes.
/// Attribute iterators are unordered - there's no specific order in which map entries will be
/// returned during consecutive iterator calls.
pub type Attributes = yrs::types::xml::Attributes<'static>;

/// Iterator used to traverse over the complex nested tree structure of a XML node. XML node
/// iterator walks only over `YXmlElement` and `YXmlText` nodes. It does so in ordered manner (using
/// the order in which children are ordered within their parent nodes) and using **depth-first**
/// traverse.
pub type TreeWalker = yrs::types::xml::TreeWalker<'static>;

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
    pub id: c_ulong,

    /// Encoding used by text editing operations on this document. It's used to compute
    /// `YText`/`YXmlText` insertion offsets and text lengths. Either:
    ///
    /// - `Y_ENCODING_BYTES`
    /// - `Y_ENCODING_UTF16`
    /// - `Y_ENCODING_UTF32`
    pub encoding: c_int,

    /// Boolean flag used to determine if deleted blocks should be garbage collected or not
    /// during the transaction commits. Setting this value to 0 means GC will be performed.
    pub skip_gc: c_int,
}

impl Into<Options> for YOptions {
    fn into(self) -> Options {
        let encoding = match self.encoding {
            Y_OFFSET_BYTES => OffsetKind::Bytes,
            Y_OFFSET_UTF16 => OffsetKind::Utf16,
            Y_OFFSET_UTF32 => OffsetKind::Utf32,
            _ => panic!("Unrecognized YOptions.encoding type"),
        };
        Options {
            client_id: self.id as u64 & 0x3fffffffffffff,
            skip_gc: if self.skip_gc == 0 { false } else { true },
            offset_kind: encoding,
        }
    }
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
pub unsafe extern "C" fn ybinary_destroy(ptr: *mut c_uchar, len: c_int) {
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

/// Creates a new [Doc] instance with a specified `options`.
///
/// Use [ydoc_destroy] in order to release created [Doc] resources.
#[no_mangle]
pub extern "C" fn ydoc_new_with_options(options: YOptions) -> *mut Doc {
    Box::into_raw(Box::new(Doc::with_options(options.into())))
}

/// Returns a unique client identifier of this [Doc] instance.
#[no_mangle]
pub unsafe extern "C" fn ydoc_id(doc: *mut Doc) -> c_ulong {
    let doc = doc.as_ref().unwrap();
    doc.client_id as c_ulong
}

/// Starts a new read-write transaction on a given document. All other operations happen in context
/// of a transaction. Yrs transactions do not follow ACID rules. Once a set of operations is
/// complete, a transaction can be finished using [ytransaction_commit] function.
#[no_mangle]
pub unsafe extern "C" fn ytransaction_new(doc: *mut Doc) -> *mut Transaction {
    assert!(!doc.is_null());

    let doc = doc.as_mut().unwrap();
    Box::into_raw(Box::new(doc.transact()))
}

/// Commit and dispose provided transaction. This operation releases allocated resources, triggers
/// update events and performs a storage compression over all operations executed in scope of
/// current transaction.
#[no_mangle]
pub unsafe extern "C" fn ytransaction_commit(txn: *mut Transaction) {
    assert!(!txn.is_null());
    drop(Box::from_raw(txn)); // transaction is auto-committed when dropped
}

/// Gets or creates a new shared `YText` data type instance as a root-level type of a given document.
/// This structure can later be accessed using its `name`, which must be a null-terminated UTF-8
/// compatible string.
///
/// Use [ytext_destroy] in order to release pointer returned that way - keep in mind that this will
/// not remove `YText` instance from the document itself (once created it'll last for the entire
/// lifecycle of a document).
#[no_mangle]
pub unsafe extern "C" fn ytext(txn: *mut Transaction, name: *const c_char) -> *mut Branch {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let txt = txn.as_mut().unwrap().get_text(name);
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
pub unsafe extern "C" fn yarray(txn: *mut Transaction, name: *const c_char) -> *mut Branch {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    txn.as_mut().unwrap().get_array(name).into_raw_branch()
}

/// Gets or creates a new shared `YMap` data type instance as a root-level type of a given document.
/// This structure can later be accessed using its `name`, which must be a null-terminated UTF-8
/// compatible string.
///
/// Use [ymap_destroy] in order to release pointer returned that way - keep in mind that this will
/// not remove `YMap` instance from the document itself (once created it'll last for the entire
/// lifecycle of a document).
#[no_mangle]
pub unsafe extern "C" fn ymap(txn: *mut Transaction, name: *const c_char) -> *mut Branch {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    txn.as_mut().unwrap().get_map(name).into_raw_branch()
}

/// Gets or creates a new shared `YXmlElement` data type instance as a root-level type of a given
/// document. This structure can later be accessed using its `name`, which must be a null-terminated
/// UTF-8 compatible string.
///
/// Use [yxmlelem_destroy] in order to release pointer returned that way - keep in mind that this
/// will not remove `YXmlElement` instance from the document itself (once created it'll last for
/// the entire lifecycle of a document).
#[no_mangle]
pub unsafe extern "C" fn yxmlelem(txn: *mut Transaction, name: *const c_char) -> *mut Branch {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    txn.as_mut()
        .unwrap()
        .get_xml_element(name)
        .into_raw_branch()
}

/// Gets or creates a new shared `YXmlText` data type instance as a root-level type of a given
/// document. This structure can later be accessed using its `name`, which must be a null-terminated
/// UTF-8 compatible string.
///
/// Use [yxmltext_destroy] in order to release pointer returned that way - keep in mind that this
/// will not remove `YXmlText` instance from the document itself (once created it'll last for
/// the entire lifecycle of a document).
#[no_mangle]
pub unsafe extern "C" fn yxmltext(txn: *mut Transaction, name: *const c_char) -> *mut Branch {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    txn.as_mut().unwrap().get_xml_text(name).into_raw_branch()
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
    len: *mut c_int,
) -> *mut c_uchar {
    assert!(!txn.is_null());

    let state_vector = txn.as_ref().unwrap().state_vector();
    let binary = state_vector.encode_v1().into_boxed_slice();

    *len = binary.len() as c_int;
    Box::into_raw(binary) as *mut c_uchar
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
    sv: *const c_uchar,
    sv_len: c_int,
    len: *mut c_int,
) -> *mut c_uchar {
    assert!(!txn.is_null());

    let sv = {
        if sv.is_null() {
            StateVector::default()
        } else {
            let sv_slice = std::slice::from_raw_parts(sv as *const u8, sv_len as usize);
            StateVector::decode_v1(sv_slice)
        }
    };

    let mut encoder = EncoderV1::new();
    txn.as_ref().unwrap().encode_diff(&sv, &mut encoder);
    let binary = encoder.to_vec().into_boxed_slice();
    *len = binary.len() as c_int;
    Box::into_raw(binary) as *mut c_uchar
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
    sv: *const c_uchar,
    sv_len: c_int,
    len: *mut c_int,
) -> *mut c_uchar {
    assert!(!txn.is_null());

    let sv = {
        if sv.is_null() {
            StateVector::default()
        } else {
            let sv_slice = std::slice::from_raw_parts(sv as *const u8, sv_len as usize);
            StateVector::decode_v1(sv_slice)
        }
    };

    let mut encoder = EncoderV2::new();
    txn.as_ref().unwrap().encode_diff(&sv, &mut encoder);
    let binary = encoder.to_vec().into_boxed_slice();
    *len = binary.len() as c_int;
    Box::into_raw(binary) as *mut c_uchar
}

/// Applies an diff update (generated by [ytransaction_state_diff_v1]) to a local transaction's
/// document.
///
/// A length of generated `diff` binary must be passed within a `diff_len` out parameter.
#[no_mangle]
pub unsafe extern "C" fn ytransaction_apply(
    txn: *mut Transaction,
    diff: *const c_uchar,
    diff_len: c_int,
) {
    assert!(!txn.is_null());
    assert!(!diff.is_null());

    let update = std::slice::from_raw_parts(diff as *const u8, diff_len as usize);
    let mut decoder = DecoderV1::from(update);
    let update = Update::decode(&mut decoder);
    txn.as_mut().unwrap().apply_update(update)
}

/// Applies an diff update (generated by [ytransaction_state_diff_v2]) to a local transaction's
/// document.
///
/// A length of generated `diff` binary must be passed within a `diff_len` out parameter.
#[no_mangle]
pub unsafe extern "C" fn ytransaction_apply_v2(
    txn: *mut Transaction,
    diff: *const c_uchar,
    diff_len: c_int,
) {
    assert!(!txn.is_null());
    assert!(!diff.is_null());

    let update = std::slice::from_raw_parts(diff as *const u8, diff_len as usize);
    let mut decoder = DecoderV2::from(update);
    let update = Update::decode(&mut decoder);
    txn.as_mut().unwrap().apply_update(update)
}

/// Returns the length of the `YText` string content in bytes (without the null terminator character)
#[no_mangle]
pub unsafe extern "C" fn ytext_len(txt: *const Branch) -> c_int {
    assert!(!txt.is_null());
    let txt = Text::from_raw_branch(txt);
    txt.len() as c_int
}

/// Returns a null-terminated UTF-8 encoded string content of a current `YText` shared data type.
///
/// Generated string resources should be released using [ystring_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn ytext_string(txt: *const Branch) -> *mut c_char {
    assert!(!txt.is_null());

    let txt = Text::from_raw_branch(txt);
    let str = txt.to_string();
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
    index: c_int,
    value: *const c_char,
    attrs: *const YInput,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!value.is_null());

    let chunk = CStr::from_ptr(value).to_str().unwrap();
    let txn = txn.as_mut().unwrap();
    let txt = Text::from_raw_branch(txt);
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
    index: c_int,
    len: c_int,
    attrs: *const YInput,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!attrs.is_null());

    if let Some(attrs) = map_attrs(attrs.read().into()) {
        let txt = Text::from_raw_branch(txt);
        let txn = txn.as_mut().unwrap();
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
    index: c_int,
    content: *const YInput,
    attrs: *const YInput,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!content.is_null());

    let txn = txn.as_mut().unwrap();
    let txt = Text::from_raw_branch(txt);
    let index = index as u32;
    let content: Any = content.read().into();
    if attrs.is_null() {
        txt.insert_embed(txn, index, content)
    } else {
        if let Some(attrs) = map_attrs(attrs.read().into()) {
            txt.insert_embed_with_attributes(txn, index, content, attrs)
        } else {
            panic!("ytext_insert_embed: passed attributes are not of map type")
        }
    }
}

fn map_attrs(attrs: Any) -> Option<Attrs> {
    if let Any::Map(attrs) = attrs {
        let attrs = attrs
            .into_iter()
            .map(|(k, v)| (k.into_boxed_str(), v))
            .collect();
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
    index: c_int,
    length: c_int,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let txn = txn.as_mut().unwrap();
    let txt = Text::from_raw_branch(txt);
    txt.remove_range(txn, index as u32, length as u32)
}

/// Returns a number of elements stored within current instance of `YArray`.
#[no_mangle]
pub unsafe extern "C" fn yarray_len(array: *const Branch) -> c_int {
    assert!(!array.is_null());

    let array = array.as_ref().unwrap();
    array.len() as c_int
}

/// Returns a pointer to a `YOutput` value stored at a given `index` of a current `YArray`.
/// If `index` is outside of the bounds of an array, a null pointer will be returned.
///
/// A value returned should be eventually released using [youtput_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yarray_get(array: *const Branch, index: c_int) -> *mut YOutput {
    assert!(!array.is_null());

    let array = Array::from_raw_branch(array);

    if let Some(val) = array.get(index as u32) {
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
    index: c_int,
    items: *const YInput,
    items_len: c_int,
) {
    assert!(!array.is_null());
    assert!(!txn.is_null());
    assert!(!items.is_null());

    let array = Array::from_raw_branch(array);
    let txn = txn.as_mut().unwrap();

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
    index: c_int,
    len: c_int,
) {
    assert!(!array.is_null());
    assert!(!txn.is_null());

    let array = Array::from_raw_branch(array);
    let txn = txn.as_mut().unwrap();

    array.remove_range(txn, index as u32, len as u32)
}

/// Returns an iterator, which can be used to traverse over all elements of an `array` (`array`'s
/// length can be determined using [yarray_len] function).
///
/// Use [yarray_iter_next] function in order to retrieve a consecutive array elements.
/// Use [yarray_iter_destroy] function in order to close the iterator and release its resources.
#[no_mangle]
pub unsafe extern "C" fn yarray_iter(array: *const Branch) -> *mut ArrayIter {
    assert!(!array.is_null());

    let array = &Array::from_raw_branch(array) as *const Array;
    Box::into_raw(Box::new(array.as_ref().unwrap().iter()))
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
    if let Some(v) = iter.next() {
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
pub unsafe extern "C" fn ymap_iter(map: *const Branch) -> *mut MapIter {
    assert!(!map.is_null());

    let map = &Map::from_raw_branch(map) as *const Map;
    Box::into_raw(Box::new(map.as_ref().unwrap().iter()))
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
    if let Some((key, value)) = iter.next() {
        Box::into_raw(Box::new(YMapEntry::new(key, value)))
    } else {
        std::ptr::null_mut()
    }
}

/// Returns a number of entries stored within a `map`.
#[no_mangle]
pub unsafe extern "C" fn ymap_len(map: *const Branch) -> c_int {
    assert!(!map.is_null());

    let map = Map::from_raw_branch(map);

    map.len() as c_int
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

    let map = Map::from_raw_branch(map);
    let txn = txn.as_mut().unwrap();

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
) -> c_char {
    assert!(!map.is_null());
    assert!(!txn.is_null());
    assert!(!key.is_null());

    let key = CStr::from_ptr(key).to_str().unwrap();

    let map = Map::from_raw_branch(map);
    let txn = txn.as_mut().unwrap();

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
pub unsafe extern "C" fn ymap_get(map: *const Branch, key: *const c_char) -> *mut YOutput {
    assert!(!map.is_null());
    assert!(!key.is_null());

    let key = CStr::from_ptr(key).to_str().unwrap();

    let map = Map::from_raw_branch(map);

    if let Some(value) = map.get(key) {
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

    let map = Map::from_raw_branch(map);
    let txn = txn.as_mut().unwrap();

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
    let xml = XmlElement::from_raw_branch(xml);
    let tag = xml.tag();
    CString::new(tag).unwrap().into_raw()
}

/// Converts current `YXmlElement` together with its children and attributes into a flat string
/// representation (no padding) eg. `<UNDEFINED><title key="value">sample text</title></UNDEFINED>`.
///
/// Returned value is a null-terminated UTF-8 string, which must be released using [ystring_destroy]
/// function.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_string(xml: *const Branch) -> *mut c_char {
    assert!(!xml.is_null());

    let xml = XmlElement::from_raw_branch(xml);

    let str = xml.to_string();
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

    let xml = XmlElement::from_raw_branch(xml);
    let txn = txn.as_mut().unwrap();

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

    let xml = XmlElement::from_raw_branch(xml);
    let txn = txn.as_mut().unwrap();

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
    attr_name: *const c_char,
) -> *mut c_char {
    assert!(!xml.is_null());
    assert!(!attr_name.is_null());

    let xml = XmlElement::from_raw_branch(xml);

    let key = CStr::from_ptr(attr_name).to_str().unwrap();
    if let Some(value) = xml.get_attribute(key) {
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
pub unsafe extern "C" fn yxmlelem_attr_iter(xml: *const Branch) -> *mut Attributes {
    assert!(!xml.is_null());

    let xml = &XmlElement::from_raw_branch(xml) as *const XmlElement;
    Box::into_raw(Box::new(xml.as_ref().unwrap().attributes()))
}

/// Returns an iterator over the `YXmlText` attributes.
///
/// Use [yxmlattr_iter_next] function in order to retrieve a consecutive (**unordered**) attributes.
/// Use [yxmlattr_iter_destroy] function in order to close the iterator and release its resources.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_attr_iter(xml: *const Branch) -> *mut Attributes {
    assert!(!xml.is_null());

    let xml = &XmlText::from_raw_branch(xml) as *const XmlText;
    Box::into_raw(Box::new(xml.as_ref().unwrap().attributes()))
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

    if let Some((name, value)) = iter.next() {
        Box::into_raw(Box::new(YXmlAttr {
            name: CString::new(name).unwrap().into_raw(),
            value: CString::new(value).unwrap().into_raw(),
        }))
    } else {
        std::ptr::null_mut()
    }
}

/// Returns a next sibling of a current `YXmlElement`, which can be either another `YXmlElement`
/// or a `YXmlText`. Together with [yxmlelem_first_child] it may be used to iterate over the direct
/// children of an XML node (in order to iterate over the nested XML structure use
/// [yxmlelem_tree_walker]).
///
/// If current `YXmlElement` is the last child, this function returns a null pointer.
/// A returned value should be eventually released using [youtput_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_next_sibling(xml: *const Branch) -> *mut YOutput {
    assert!(!xml.is_null());

    let xml = XmlElement::from_raw_branch(xml);

    if let Some(next) = xml.next_sibling() {
        match next {
            Xml::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            Xml::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
        }
    } else {
        std::ptr::null_mut()
    }
}

/// Returns a previous sibling of a current `YXmlElement`, which can be either another `YXmlElement`
/// or a `YXmlText`.
///
/// If current `YXmlElement` is the first child, this function returns a null pointer.
/// A returned value should be eventually released using [youtput_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_prev_sibling(xml: *const Branch) -> *mut YOutput {
    assert!(!xml.is_null());

    let xml = XmlElement::from_raw_branch(xml);

    if let Some(next) = xml.prev_sibling() {
        match next {
            Xml::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            Xml::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
        }
    } else {
        std::ptr::null_mut()
    }
}

/// Returns a next sibling of a current `YXmlText`, which can be either another `YXmlText` or
/// an `YXmlElement`. Together with [yxmlelem_first_child] it may be used to iterate over the direct
/// children of an XML node (in order to iterate over the nested XML structure use
/// [yxmlelem_tree_walker]).
///
/// If current `YXmlText` is the last child, this function returns a null pointer.
/// A returned value should be eventually released using [youtput_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_next_sibling(xml: *const Branch) -> *mut YOutput {
    assert!(!xml.is_null());

    let xml = XmlElement::from_raw_branch(xml);

    if let Some(next) = xml.next_sibling() {
        match next {
            Xml::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            Xml::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
        }
    } else {
        std::ptr::null_mut()
    }
}

/// Returns a previous sibling of a current `YXmlText`, which can be either another `YXmlText` or
/// an `YXmlElement`.
///
/// If current `YXmlText` is the first child, this function returns a null pointer.
/// A returned value should be eventually released using [youtput_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_prev_sibling(xml: *const Branch) -> *mut YOutput {
    assert!(!xml.is_null());

    let xml = XmlElement::from_raw_branch(xml);

    if let Some(next) = xml.prev_sibling() {
        match next {
            Xml::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            Xml::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
        }
    } else {
        std::ptr::null_mut()
    }
}

/// Returns a parent `YXmlElement` of a current node, or null pointer when current `YXmlElement` is
/// a root-level shared data type.
///
/// A returned value should be eventually released using [youtput_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_parent(xml: *const Branch) -> *mut Branch {
    assert!(!xml.is_null());

    let xml = XmlElement::from_raw_branch(xml);

    if let Some(parent) = xml.parent() {
        parent.into_raw_branch()
    } else {
        std::ptr::null_mut()
    }
}

/// Returns a number of child nodes (both `YXmlElement` and `YXmlText`) living under a current XML
/// element. This function doesn't count a recursive nodes, only direct children of a current node.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_child_len(xml: *const Branch) -> c_int {
    assert!(!xml.is_null());

    let xml = XmlElement::from_raw_branch(xml);

    xml.len() as c_int
}

/// Returns a first child node of a current `YXmlElement`, or null pointer if current XML node is
/// empty. Returned value could be either another `YXmlElement` or `YXmlText`.
///
/// A returned value should be eventually released using [youtput_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_first_child(xml: *const Branch) -> *mut YOutput {
    assert!(!xml.is_null());

    let xml = XmlElement::from_raw_branch(xml);

    if let Some(value) = xml.first_child() {
        match value {
            Xml::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            Xml::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
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
pub unsafe extern "C" fn yxmlelem_tree_walker(xml: *const Branch) -> *mut TreeWalker {
    assert!(!xml.is_null());

    let xml = &XmlElement::from_raw_branch(xml) as *const XmlElement;
    Box::into_raw(Box::new(xml.as_ref().unwrap().successors()))
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

    if let Some(next) = iter.next() {
        match next {
            Xml::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            Xml::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
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
    index: c_int,
    name: *const c_char,
) -> *mut Branch {
    assert!(!xml.is_null());
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let xml = XmlElement::from_raw_branch(xml);
    let txn = txn.as_mut().unwrap();

    let name = CStr::from_ptr(name).to_str().unwrap();
    xml.insert_elem(txn, index as u32, name).into_raw_branch()
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
    index: c_int,
) -> *mut Branch {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = XmlElement::from_raw_branch(xml);
    let txn = txn.as_mut().unwrap();
    xml.insert_text(txn, index as u32).into_raw_branch()
}

/// Removes a consecutive range of child elements (of specified length) from the current
/// `YXmlElement`, starting at the given `index`. Specified range must fit into boundaries of current
/// XML node children, otherwise this function will panic at runtime.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_remove_range(
    xml: *const Branch,
    txn: *mut Transaction,
    index: c_int,
    len: c_int,
) {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = XmlElement::from_raw_branch(xml);
    let txn = txn.as_mut().unwrap();

    xml.remove_range(txn, index as u32, len as u32)
}

/// Returns an XML child node (either a `YXmlElement` or `YXmlText`) stored at a given `index` of
/// a current `YXmlElement`. Returns null pointer if `index` was outside of the bound of current XML
/// node children.
///
/// Returned value should be eventually released using [youtput_destroy].
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_get(xml: *const Branch, index: c_int) -> *const YOutput {
    assert!(!xml.is_null());

    let xml = XmlElement::from_raw_branch(xml);

    if let Some(child) = xml.get(index as u32) {
        match child {
            Xml::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            Xml::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
        }
    } else {
        std::ptr::null()
    }
}

/// Returns the length of the `YXmlText` string content in bytes (without the null terminator
/// character)
#[no_mangle]
pub unsafe extern "C" fn yxmltext_len(txt: *const Branch, txn: *const Transaction) -> c_int {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let txt = XmlText::from_raw_branch(txt);

    txt.len() as c_int
}

/// Returns a null-terminated UTF-8 encoded string content of a current `YXmlText` shared data type.
///
/// Generated string resources should be released using [ystring_destroy] function.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_string(txt: *const Branch) -> *mut c_char {
    assert!(!txt.is_null());

    let txt = XmlText::from_raw_branch(txt);

    let str = txt.to_string();
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
    index: c_int,
    str: *const c_char,
    attrs: *const YInput,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!str.is_null());

    let txt = XmlText::from_raw_branch(txt);
    let txn = txn.as_mut().unwrap();
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
    index: c_int,
    content: *const YInput,
    attrs: *const YInput,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!content.is_null());

    let txn = txn.as_mut().unwrap();
    let txt = XmlText::from_raw_branch(txt);
    let index = index as u32;
    let content: Any = content.read().into();
    if attrs.is_null() {
        txt.insert_embed(txn, index, content)
    } else {
        if let Some(attrs) = map_attrs(attrs.read().into()) {
            txt.insert_embed_with_attributes(txn, index, content, attrs)
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
    index: c_int,
    len: c_int,
    attrs: *const YInput,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!attrs.is_null());

    if let Some(attrs) = map_attrs(attrs.read().into()) {
        let txt = XmlText::from_raw_branch(txt);
        let txn = txn.as_mut().unwrap();
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
    idx: c_int,
    len: c_int,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let txt = XmlText::from_raw_branch(txt);
    let txn = txn.as_mut().unwrap();
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

    let txt = XmlText::from_raw_branch(txt);
    let txn = txn.as_mut().unwrap();

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

    let txt = XmlText::from_raw_branch(txt);
    let txn = txn.as_mut().unwrap();
    let name = CStr::from_ptr(attr_name).to_str().unwrap();

    txt.remove_attribute(txn, name)
}

/// Returns the value of a current `YXmlText`, given its name, or a null pointer if not attribute
/// with such name has been found. Returned pointer is a null-terminated UTF-8 encoded string, which
/// should be released using [ystring_destroy] function.
///
/// An `attr_name` must be a null-terminated UTF-8 encoded string.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_get_attr(
    txt: *const Branch,
    attr_name: *const c_char,
) -> *mut c_char {
    assert!(!txt.is_null());
    assert!(!attr_name.is_null());

    let txt = XmlText::from_raw_branch(txt);
    let name = CStr::from_ptr(attr_name).to_str().unwrap();

    if let Some(value) = txt.get_attribute(name) {
        CString::new(value).unwrap().into_raw()
    } else {
        std::ptr::null_mut()
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
    pub tag: i8,

    /// Length of the contents stored by current `YInput` cell.
    ///
    /// For [Y_JSON_NULL] and [Y_JSON_UNDEF] its equal to `0`.
    ///
    /// For [Y_JSON_ARR], [Y_JSON_MAP], [Y_ARRAY] and [Y_MAP] it describes a number of passed
    /// elements.
    ///
    /// For other types it's always equal to `1`.
    pub len: c_int,

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
            } else {
                panic!("Unrecognized YVal value tag.")
            }
        }
    }
}

#[repr(C)]
union YInputContent {
    flag: c_char,
    num: c_float,
    integer: c_long,
    str: *mut c_char,
    buf: *mut c_uchar,
    values: *mut YInput,
    map: ManuallyDrop<YMapInputData>,
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
    fn into_content(self, _txn: &mut Transaction) -> (ItemContent, Option<Self>) {
        unsafe {
            if self.tag <= 0 {
                let value = self.into();
                (ItemContent::Any(vec![value]), None)
            } else {
                let type_ref = if self.tag == Y_MAP {
                    TYPE_REFS_MAP
                } else if self.tag == Y_ARRAY {
                    TYPE_REFS_ARRAY
                } else if self.tag == Y_XML_ELEM {
                    TYPE_REFS_XML_ELEMENT
                } else if self.tag == Y_XML_TEXT {
                    TYPE_REFS_XML_TEXT
                } else {
                    panic!("Unrecognized YVal value tag.")
                };
                let name = if type_ref == TYPE_REFS_XML_ELEMENT {
                    let name = CStr::from_ptr(self.value.str).to_str().unwrap().to_owned();
                    Some(name)
                } else {
                    None
                };
                let inner = Branch::new(type_ref, name);
                (ItemContent::Type(inner), Some(self))
            }
        }
    }

    fn integrate(self, txn: &mut yrs::Transaction, inner_ref: BranchPtr) {
        unsafe {
            if self.tag == Y_MAP {
                let map = Map::from(inner_ref);
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
                let array = Array::from(inner_ref);
                let ptr = self.value.values;
                let len = self.len as isize;
                let mut i = 0;
                while i < len {
                    let value = ptr.offset(i).read();
                    array.push_back(txn, value);
                    i += 1;
                }
            } else if self.tag == Y_TEXT {
                let text = Text::from(inner_ref);
                let init = CStr::from_ptr(self.value.str).to_str().unwrap();
                text.push(txn, init);
            } else if self.tag == Y_XML_TEXT {
                let text = XmlText::from(inner_ref);
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
    pub tag: i8,

    /// Length of the contents stored by a current `YOutput` cell.
    ///
    /// For [Y_JSON_NULL] and [Y_JSON_UNDEF] its equal to `0`.
    ///
    /// For [Y_JSON_ARR], [Y_JSON_MAP] it describes a number of passed elements.
    ///
    /// For other types it's always equal to `1`.
    pub len: c_int,

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
            Value::YXmlText(v) => Self::from(v),
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
                    value: YOutputContent {
                        integer: v as c_longlong,
                    },
                },
                Any::String(v) => YOutput {
                    tag: Y_JSON_STR,
                    len: v.len() as c_int,
                    value: YOutputContent {
                        str: CString::new(v.as_ref()).unwrap().into_raw(),
                    },
                },
                Any::Buffer(v) => YOutput {
                    tag: Y_JSON_BUF,
                    len: v.len() as c_int,
                    value: YOutputContent {
                        buf: Box::into_raw(v.clone()) as *mut _,
                    },
                },
                Any::Array(v) => {
                    let len = v.len() as c_int;
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
                    let len = v.len() as c_int;
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

impl From<Text> for YOutput {
    fn from(v: Text) -> Self {
        YOutput {
            tag: Y_TEXT,
            len: 1,
            value: YOutputContent {
                y_type: v.into_raw_branch(),
            },
        }
    }
}

impl From<Array> for YOutput {
    fn from(v: Array) -> Self {
        YOutput {
            tag: Y_ARRAY,
            len: 1,
            value: YOutputContent {
                y_type: v.into_raw_branch(),
            },
        }
    }
}

impl From<Map> for YOutput {
    fn from(v: Map) -> Self {
        YOutput {
            tag: Y_MAP,
            len: 1,
            value: YOutputContent {
                y_type: v.into_raw_branch(),
            },
        }
    }
}

impl From<XmlElement> for YOutput {
    fn from(v: XmlElement) -> Self {
        YOutput {
            tag: Y_XML_ELEM,
            len: 1,
            value: YOutputContent {
                y_type: v.into_raw_branch(),
            },
        }
    }
}

impl From<XmlText> for YOutput {
    fn from(v: XmlText) -> Self {
        YOutput {
            tag: Y_XML_TEXT,
            len: 1,
            value: YOutputContent {
                y_type: v.into_raw_branch(),
            },
        }
    }
}

#[repr(C)]
union YOutputContent {
    flag: c_char,
    num: c_float,
    integer: c_longlong,
    str: *mut c_char,
    buf: *mut c_uchar,
    array: *mut YOutput,
    map: *mut YMapEntry,
    y_type: *mut Branch,
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
pub unsafe extern "C" fn yinput_bool(flag: c_char) -> YInput {
    YInput {
        tag: Y_JSON_BOOL,
        len: 1,
        value: YInputContent { flag },
    }
}

/// Function constructor used to create JSON-like 64-bit floating point number `YInput` cell.
/// This function doesn't allocate any heap resources.
#[no_mangle]
pub unsafe extern "C" fn yinput_float(num: c_float) -> YInput {
    YInput {
        tag: Y_JSON_NUM,
        len: 1,
        value: YInputContent { num },
    }
}

/// Function constructor used to create JSON-like 64-bit signed integer `YInput` cell.
/// This function doesn't allocate any heap resources.
#[no_mangle]
pub unsafe extern "C" fn yinput_long(integer: c_long) -> YInput {
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
pub unsafe extern "C" fn yinput_binary(buf: *const u8, len: c_int) -> YInput {
    YInput {
        tag: Y_JSON_BUF,
        len,
        value: YInputContent {
            buf: buf as *mut u8,
        },
    }
}

/// Function constructor used to create a JSON-like array `YInput` cell of other JSON-like values of
/// a given length. This function doesn't allocate any heap resources and doesn't release any on its
/// own, therefore its up to a caller to free resources once a structure is no longer needed.
#[no_mangle]
pub unsafe extern "C" fn yinput_json_array(values: *mut YInput, len: c_int) -> YInput {
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
    len: c_int,
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
pub unsafe extern "C" fn yinput_yarray(values: *mut YInput, len: c_int) -> YInput {
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
    len: c_int,
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

/// Attempts to read the value for a given `YOutput` pointer as a boolean flag, which can be either
/// `1` for truthy case and `0` otherwise. Returns a null pointer in case when a value stored under
/// current `YOutput` cell is not of a boolean type.
#[no_mangle]
pub unsafe extern "C" fn youtput_read_bool(val: *const YOutput) -> *const c_char {
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
pub unsafe extern "C" fn youtput_read_float(val: *const YOutput) -> *const c_float {
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
pub unsafe extern "C" fn youtput_read_long(val: *const YOutput) -> *const c_longlong {
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
pub unsafe extern "C" fn youtput_read_binary(val: *const YOutput) -> *const c_uchar {
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
) -> c_uint {
    assert!(!txt.is_null());

    let mut txt = Text::from_raw_branch(txt);
    let observer = txt.observe(move |txn, e| {
        let e = YTextEvent::new(e, txn);
        cb(state, &e as *const YTextEvent);
    });
    let subscription_id: u32 = observer.into();
    subscription_id as c_uint
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
) -> c_uint {
    assert!(!map.is_null());

    let mut map = Map::from_raw_branch(map);
    let observer = map.observe(move |txn, e| {
        let e = YMapEvent::new(e, txn);
        cb(state, &e as *const YMapEvent);
    });
    let subscription_id: u32 = observer.into();
    subscription_id as c_uint
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
) -> c_uint {
    assert!(!array.is_null());

    let mut array = Array::from_raw_branch(array);
    let observer = array.observe(move |txn, e| {
        let e = YArrayEvent::new(e, txn);
        cb(state, &e as *const YArrayEvent);
    });
    let subscription_id: u32 = observer.into();
    subscription_id as c_uint
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
) -> c_uint {
    assert!(!xml.is_null());

    let mut xml = XmlElement::from_raw_branch(xml);
    let observer = xml.observe(move |txn, e| {
        let e = YXmlEvent::new(e, txn);
        cb(state, &e as *const YXmlEvent);
    });
    let subscription_id: u32 = observer.into();
    subscription_id as c_uint
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
) -> c_uint {
    assert!(!xml.is_null());

    let mut xml = XmlText::from_raw_branch(xml);
    let observer = xml.observe(move |txn, e| {
        let e = YXmlTextEvent::new(e, txn);
        cb(state, &e as *const YXmlTextEvent);
    });
    let subscription_id: u32 = observer.into();
    subscription_id as c_uint
}

/// Event pushed into callbacks registered with `ytext_observe` function. It contains delta of all
/// text changes made within a scope of corresponding transaction (see: `ytext_event_delta`) as
/// well as navigation data used to identify a `YText` instance which triggered this event.
#[repr(C)]
pub struct YTextEvent {
    inner: *const c_void,
    pub txn: *const Transaction,
}

impl YTextEvent {
    fn new(inner: &TextEvent, txn: &Transaction) -> Self {
        let inner = inner as *const TextEvent as *const _;
        let txn = txn as *const Transaction;
        YTextEvent { inner, txn }
    }

    fn txn(&self) -> &Transaction {
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
pub struct YArrayEvent {
    inner: *const c_void,
    pub txn: *const Transaction,
}

impl YArrayEvent {
    fn new(inner: &ArrayEvent, txn: &Transaction) -> Self {
        let inner = inner as *const ArrayEvent as *const _;
        let txn = txn as *const Transaction;
        YArrayEvent { inner, txn }
    }

    fn txn(&self) -> &Transaction {
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
pub struct YMapEvent {
    inner: *const c_void,
    pub txn: *const Transaction,
}

impl YMapEvent {
    fn new(inner: &MapEvent, txn: &Transaction) -> Self {
        let inner = inner as *const MapEvent as *const _;
        let txn = txn as *const Transaction;
        YMapEvent { inner, txn }
    }

    fn txn(&self) -> &Transaction {
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
pub struct YXmlEvent {
    inner: *const c_void,
    pub txn: *const Transaction,
}

impl YXmlEvent {
    fn new(inner: &XmlEvent, txn: &Transaction) -> Self {
        let inner = inner as *const XmlEvent as *const _;
        let txn = txn as *const Transaction;
        YXmlEvent { inner, txn }
    }

    fn txn(&self) -> &Transaction {
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
pub struct YXmlTextEvent {
    inner: *const c_void,
    pub txn: *const Transaction,
}

impl YXmlTextEvent {
    fn new(inner: &XmlTextEvent, txn: &Transaction) -> Self {
        let inner = inner as *const XmlTextEvent as *const _;
        let txn = txn as *const Transaction;
        YXmlTextEvent { inner, txn }
    }

    fn txn(&self) -> &Transaction {
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
pub unsafe extern "C" fn ytext_unobserve(txt: *const Branch, subscription_id: c_uint) {
    let mut txt = Text::from_raw_branch(txt);
    txt.unobserve(subscription_id as SubscriptionId);
}

/// Releases a callback subscribed via `yarray_observe` function represented by passed
/// observer parameter.
#[no_mangle]
pub unsafe extern "C" fn yarray_unobserve(array: *const Branch, subscription_id: c_uint) {
    let mut txt = Array::from_raw_branch(array);
    txt.unobserve(subscription_id as SubscriptionId);
}

/// Releases a callback subscribed via `ymap_observe` function represented by passed
/// observer parameter.
#[no_mangle]
pub unsafe extern "C" fn ymap_unobserve(map: *const Branch, subscription_id: c_uint) {
    let mut txt = Map::from_raw_branch(map);
    txt.unobserve(subscription_id as SubscriptionId);
}

/// Releases a callback subscribed via `yxmlelem_observe` function represented by passed
/// observer parameter.
#[no_mangle]
pub unsafe extern "C" fn yxmlelem_unobserve(xml: *const Branch, subscription_id: c_uint) {
    let mut xml = XmlElement::from_raw_branch(xml);
    xml.unobserve(subscription_id as SubscriptionId);
}

/// Releases a callback subscribed via `yxmltext_observe` function represented by passed
/// observer parameter.
#[no_mangle]
pub unsafe extern "C" fn yxmltext_unobserve(xml: *const Branch, subscription_id: c_uint) {
    let mut xml = XmlText::from_raw_branch(xml);
    xml.unobserve(subscription_id as SubscriptionId);
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
    len: *mut c_int,
) -> *mut YPathSegment {
    assert!(!e.is_null());
    let e = &*e;
    let path: Vec<_> = e.path().into_iter().map(YPathSegment::from).collect();
    let out = path.into_boxed_slice();
    *len = out.len() as c_int;
    Box::into_raw(out) as *mut _
}

/// Returns a path from a root type down to a current shared collection (which can be obtained using
/// `ymap_event_target` function). It can consist of either integer indexes (used by sequence
/// components) of *char keys (used by map components). `len` output parameter is used to provide
/// information about length of the path.
///
/// Path returned this way should be eventually released using `ypath_destroy`.
#[no_mangle]
pub unsafe extern "C" fn ymap_event_path(
    e: *const YMapEvent,
    len: *mut c_int,
) -> *mut YPathSegment {
    assert!(!e.is_null());
    let e = &*e;
    let path: Vec<_> = e.path().into_iter().map(YPathSegment::from).collect();
    let out = path.into_boxed_slice();
    *len = out.len() as c_int;
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
    len: *mut c_int,
) -> *mut YPathSegment {
    assert!(!e.is_null());
    let e = &*e;
    let path: Vec<_> = e.path().into_iter().map(YPathSegment::from).collect();
    let out = path.into_boxed_slice();
    *len = out.len() as c_int;
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
    len: *mut c_int,
) -> *mut YPathSegment {
    assert!(!e.is_null());
    let e = &*e;
    let path: Vec<_> = e.path().into_iter().map(YPathSegment::from).collect();
    let out = path.into_boxed_slice();
    *len = out.len() as c_int;
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
    len: *mut c_int,
) -> *mut YPathSegment {
    assert!(!e.is_null());
    let e = &*e;
    let path: Vec<_> = e.path().into_iter().map(YPathSegment::from).collect();
    let out = path.into_boxed_slice();
    *len = out.len() as c_int;
    Box::into_raw(out) as *mut _
}

/// Releases allocated memory used by objects returned from path accessor functions of shared type
/// events.
#[no_mangle]
pub unsafe extern "C" fn ypath_destroy(path: *mut YPathSegment, len: c_int) {
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
pub unsafe extern "C" fn ytext_event_delta(e: *const YTextEvent, len: *mut c_int) -> *mut YDelta {
    assert!(!e.is_null());
    let e = &*e;
    let delta: Vec<_> = e.delta(e.txn()).into_iter().map(YDelta::from).collect();

    let out = delta.into_boxed_slice();
    *len = out.len() as c_int;
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
    len: *mut c_int,
) -> *mut YDelta {
    assert!(!e.is_null());
    let e = &*e;
    let delta: Vec<_> = e.delta(e.txn()).into_iter().map(YDelta::from).collect();

    let out = delta.into_boxed_slice();
    *len = out.len() as c_int;
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
    len: *mut c_int,
) -> *mut YEventChange {
    assert!(!e.is_null());
    let e = &*e;
    let delta: Vec<_> = e
        .delta(e.txn())
        .into_iter()
        .map(YEventChange::from)
        .collect();

    let out = delta.into_boxed_slice();
    *len = out.len() as c_int;
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
    len: *mut c_int,
) -> *mut YEventChange {
    assert!(!e.is_null());
    let e = &*e;
    let delta: Vec<_> = e
        .delta(e.txn())
        .into_iter()
        .map(YEventChange::from)
        .collect();

    let out = delta.into_boxed_slice();
    *len = out.len() as c_int;
    Box::into_raw(out) as *mut _
}

/// Releases memory allocated by the object returned from `yevent_delta` function.
#[no_mangle]
pub unsafe extern "C" fn ytext_delta_destroy(delta: *mut YDelta, len: c_int) {
    if !delta.is_null() {
        let delta = Vec::from_raw_parts(delta, len as usize, len as usize);
        drop(delta);
    }
}

/// Releases memory allocated by the object returned from `yevent_delta` function.
#[no_mangle]
pub unsafe extern "C" fn yevent_delta_destroy(delta: *mut YEventChange, len: c_int) {
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
    len: *mut c_int,
) -> *mut YEventKeyChange {
    assert!(!e.is_null());
    let e = &*e;
    let delta: Vec<_> = e
        .keys(e.txn())
        .into_iter()
        .map(|(k, v)| YEventKeyChange::new(k.as_ref(), v))
        .collect();

    let out = delta.into_boxed_slice();
    *len = out.len() as c_int;
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
    len: *mut c_int,
) -> *mut YEventKeyChange {
    assert!(!e.is_null());
    let e = &*e;
    let delta: Vec<_> = e
        .keys(e.txn())
        .into_iter()
        .map(|(k, v)| YEventKeyChange::new(k.as_ref(), v))
        .collect();

    let out = delta.into_boxed_slice();
    *len = out.len() as c_int;
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
    len: *mut c_int,
) -> *mut YEventKeyChange {
    assert!(!e.is_null());
    let e = &*e;
    let delta: Vec<_> = e
        .keys(e.txn())
        .into_iter()
        .map(|(k, v)| YEventKeyChange::new(k.as_ref(), v))
        .collect();

    let out = delta.into_boxed_slice();
    *len = out.len() as c_int;
    Box::into_raw(out) as *mut _
}

/// Releases memory allocated by the object returned from `yxml_event_keys` and `ymap_event_keys`
/// functions.
#[no_mangle]
pub unsafe extern "C" fn yevent_keys_destroy(keys: *mut YEventKeyChange, len: c_int) {
    if !keys.is_null() {
        drop(Vec::from_raw_parts(keys, len as usize, len as usize));
    }
}

/// Returns a value informing what kind of Yrs shared collection given `branch` represents.
/// Returns either 0 when `branch` is null or one of values: `Y_ARRAY`, `Y_TEXT`, `Y_MAP`,
/// `Y_XML_ELEM`, `Y_XML_TEXT`.
#[no_mangle]
pub unsafe extern "C" fn ytype_kind(branch: *const Branch) -> c_char {
    if let Some(branch) = branch.as_ref() {
        match branch.type_ref() {
            TYPE_REFS_ARRAY => Y_ARRAY,
            TYPE_REFS_MAP => Y_MAP,
            TYPE_REFS_TEXT => Y_TEXT,
            TYPE_REFS_XML_ELEMENT => Y_XML_ELEM,
            TYPE_REFS_XML_TEXT => Y_XML_TEXT,
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
                    index: index as c_int,
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
    pub index: c_int,
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
    pub len: c_int,

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
                let len = out.len() as c_int;
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
                len: *len as c_int,
                values: null(),
            },
            Change::Retain(len) => YEventChange {
                tag: Y_EVENT_CHANGE_RETAIN,
                len: *len as c_int,
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
    pub len: c_int,

    /// Used in case when current change is of `Y_EVENT_CHANGE_ADD` type. Contains a list (of
    /// length stored in `len` field) of newly inserted values.
    pub insert: *mut YOutput,

    /// A number of formatting attributes assigned to an edited area represented by this delta.
    pub attributes_len: c_int,

    /// A nullable pointer to a list of formatting attributes assigned to an edited area represented
    /// by this delta.
    pub attributes: *mut YDeltaAttr,
}

impl YDelta {
    fn insert(value: &Value, attrs: &Option<Box<Attrs>>) -> Self {
        let insert = Box::into_raw(Box::new(YOutput::from(value.clone())));
        let (attributes_len, attributes) = if let Some(attrs) = attrs {
            let len = attrs.len() as c_int;
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
            let len = attrs.len() as c_int;
            let attrs: Vec<_> = attrs.iter().map(|(k, v)| YDeltaAttr::new(k, v)).collect();
            let attrs = Box::into_raw(attrs.into_boxed_slice()) as *mut _;
            (len, attrs)
        } else {
            (0, null_mut())
        };
        YDelta {
            tag: Y_EVENT_CHANGE_RETAIN,
            len: len as c_int,
            insert: null_mut(),
            attributes_len,
            attributes,
        }
    }

    fn delete(len: u32) -> Self {
        YDelta {
            tag: Y_EVENT_CHANGE_DELETE,
            len: len as c_int,
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
    fn new(k: &Box<str>, v: &Any) -> Self {
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

#[cfg(test)]
mod test {
    use crate::*;

    #[test]
    fn yval_preliminary_types() {
        unsafe {
            let doc = ydoc_new();
            let txn = ytransaction_new(doc);
            let array_name = CString::new("test").unwrap();
            let array = yarray(txn, array_name.as_ptr());

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
