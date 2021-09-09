use crate::block::{ItemContent, Prelim};
use crate::id_set::DeleteSet;
use crate::types::{
    Branch, BranchRef, TypePtr, Value, TYPE_REFS_ARRAY, TYPE_REFS_MAP, TYPE_REFS_XML_ELEMENT,
    TYPE_REFS_XML_TEXT,
};
use crate::update::Update;
use crate::updates::decoder::{Decode, DecoderV1};
use crate::updates::encoder::{Encode, Encoder, EncoderV1};
use crate::{Array, Doc, Map, StateVector, Text, Transaction, Xml, XmlElement, XmlText};
use lib0::any::Any;
use std::ffi::{CStr, CString};
use std::mem::{forget, MaybeUninit};
use std::os::raw::{c_char, c_float, c_int, c_long};

#[export_name = "Y_JSON_BOOL"]
pub static Y_JSON_BOOL: c_char = -8;

#[export_name = "Y_JSON_NUM"]
pub static Y_JSON_NUM: c_char = -7;

#[export_name = "Y_JSON_INT"]
pub static Y_JSON_INT: c_char = -6;

#[export_name = "Y_JSON_STR"]
pub static Y_JSON_STR: c_char = -5;

#[export_name = "Y_JSON_BUF"]
pub static Y_JSON_BUF: c_char = -4;

#[export_name = "Y_JSON_ARRAY"]
pub static Y_JSON_ARR: c_char = -3;

#[export_name = "Y_JSON_OBJECT"]
pub static Y_JSON_MAP: c_char = -2;

#[export_name = "Y_JSON_NULL"]
pub static Y_JSON_NULL: c_char = -1;

#[export_name = "Y_JSON_UNDEF"]
pub static Y_JSON_UNDEF: c_char = 0;

#[export_name = "Y_ARRAY"]
pub static Y_ARRAY: c_char = 1;

#[export_name = "Y_MAP"]
pub static Y_MAP: c_char = 2;

#[export_name = "Y_TEXT"]
pub static Y_TEXT: c_char = 3;

#[export_name = "Y_XML_ELEM"]
pub static Y_XML_ELEM: c_char = 4;

#[export_name = "Y_XML_TEXT"]
pub static Y_XML_TEXT: c_char = 5;

#[repr(C)]
pub struct YDoc {
    _inner: Doc,
}

#[repr(C)]
pub struct YTxn {
    _inner: *mut Transaction<'static>,
}

#[repr(C)]
pub struct YText {
    _inner: Text,
}

#[repr(C)]
pub struct YArray {
    _inner: Array,
}

#[repr(C)]
pub struct YMap {
    _inner: Map,
}

#[repr(C)]
pub struct YMapEntry {
    pub key: *const c_char,
    pub value: *const YVal,
}

#[repr(C)]
pub struct YXmlElem {
    _inner: XmlElement,
}

#[repr(C)]
pub struct YXmlText {
    _inner: XmlText,
}

#[repr(C)]
pub struct YVal {
    pub tag: c_char,
    pub is_prelim: u8,
    pub len: c_int,
    value: YValContent,
}

impl Prelim for YVal {
    fn into_content(self, txn: &mut Transaction, ptr: TypePtr) -> (ItemContent, Option<Self>) {
        unsafe {
            if self.tag <= 0 {
                let value = val_into_any(&self);
                (ItemContent::Any(vec![value]), None)
            } else {
                assert_ne!(self.is_prelim, 0);

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
                let inner = BranchRef::new(Branch::new(ptr, type_ref, None));
                (ItemContent::Type(inner), Some(self))
            }
        }
    }

    fn integrate(self, txn: &mut Transaction, inner_ref: BranchRef) {
        unsafe {
            if self.tag == Y_MAP {
                let map = Map::from(inner_ref);
                let src = std::slice::from_raw_parts(self.value.map, self.len as usize);
                for e in src {
                    let key = CStr::from_ptr(e.key).to_str().unwrap().to_owned();
                    let value = e.value.read();
                    map.insert(txn, key, value);
                }
            } else if self.tag == Y_ARRAY {
                let array = Array::from(inner_ref);
                let ptr = self.value.array;
                let len = self.len as isize;
                let mut i = 0;
                while i < len {
                    let value = ptr.offset(i).read();
                    array.push_back(txn, value);
                    i += 1;
                }
            } else {
                panic!("Cannot use given type tag as preinitialized value");
            };
        }
    }
}

#[repr(C)]
union YValContent {
    flag: u8,
    num: c_float,
    integer: c_long,
    str: *const c_char,
    buf: *const u8,
    array: *const YVal,
    map: *const YMapEntry,
    y_array: *const YArray,
    y_map: *const YMap,
    y_text: *const YText,
    y_xml_elem: *const YXmlElem,
    y_xml_text: *const YXmlText,
}

#[repr(C)]
pub struct YXmlAttr {
    pub name: *const c_char,
    pub value: *const c_char,
}

#[no_mangle]
pub extern "C" fn ydoc_new() -> *mut YDoc {
    Box::into_raw(Box::new(YDoc { _inner: Doc::new() }))
}

#[no_mangle]
pub extern "C" fn ydoc_new_with_id(id: u64) -> *mut YDoc {
    Box::into_raw(Box::new(YDoc {
        _inner: Doc::with_client_id(id),
    }))
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_new(doc: *mut YDoc) -> YTxn {
    assert!(!doc.is_null());
    let txn = Box::new((*doc)._inner.transact());
    YTxn {
        _inner: Box::into_raw(txn),
    }
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_commit(txn: *mut YTxn) {
    assert!(!txn.is_null());
    (&mut *(*txn)._inner).commit();
    std::mem::drop((*txn)._inner);
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_text(txn: *mut YTxn, name: *const c_char) -> *const YText {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let t = (&mut *(*txn)._inner).get_text(name);
    Box::into_raw(Box::new(YText { _inner: t })) as *const _
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_array(txn: *mut YTxn, name: *const c_char) -> *const YArray {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let t = (&mut *(*txn)._inner).get_array(name);
    Box::into_raw(Box::new(YArray { _inner: t })) as *const _
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_map(txn: *mut YTxn, name: *const c_char) -> *const YMap {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let t = (&mut *(*txn)._inner).get_map(name);
    Box::into_raw(Box::new(YMap { _inner: t })) as *const _
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_xml_elem(txn: *mut YTxn, name: *const c_char) -> *const YXmlElem {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let t = (&mut *(*txn)._inner).get_xml_element(name);
    Box::into_raw(Box::new(YXmlElem { _inner: t })) as *const _
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_xml_text(txn: *mut YTxn, name: *const c_char) -> *const YXmlText {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let t = (&mut *(*txn)._inner).get_xml_text(name);
    Box::into_raw(Box::new(YXmlText { _inner: t })) as *const _
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_state_vector_v1(txn: *const YTxn, len: *mut c_int) -> *const u8 {
    assert!(!txn.is_null());

    let state_vector = (&mut *(*txn)._inner).store.blocks.get_state_vector();
    let binary = state_vector.encode_v1();

    let ptr = binary.as_ptr();
    *len = binary.len() as c_int;
    forget(binary);
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_state_diff_v1(
    txn: *const YTxn,
    sv: *const u8,
    sv_len: c_int,
    len: *mut c_int,
) -> *const u8 {
    assert!(!txn.is_null());

    let sv = {
        if sv.is_null() {
            StateVector::default()
        } else {
            let sv_slice = std::slice::from_raw_parts(sv, sv_len as usize);
            StateVector::decode_v1(sv_slice)
        }
    };

    let mut encoder = EncoderV1::new();
    (&*(*txn)._inner).store.encode_diff(&sv, &mut encoder);
    let binary = encoder.to_vec();

    let ptr = binary.as_ptr();
    *len = binary.len() as c_int;
    forget(binary);
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_apply(txn: *mut YTxn, diff: *const u8, diff_len: c_int) {
    assert!(!txn.is_null());
    assert!(!diff.is_null());

    let update = std::slice::from_raw_parts(diff, diff_len as usize);
    let mut decoder = DecoderV1::from(update);
    let update = Update::decode(&mut decoder);
    let ds = DeleteSet::decode(&mut decoder);
    (&mut *(*txn)._inner).apply_update(update, ds)
}

#[no_mangle]
pub unsafe extern "C" fn ytext_len(txt: *const YText) -> c_int {
    assert!(!txt.is_null());
    (*txt)._inner.len() as c_int
}

#[no_mangle]
pub unsafe extern "C" fn ytext_string(txt: *const YText, txn: *mut YTxn) -> *const c_char {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let str = (*txt)._inner.to_string(&*(*txn)._inner);
    let cstr = CString::new(str).unwrap();
    cstr.as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn ytext_insert(
    txt: *const YText,
    txn: *mut YTxn,
    idx: c_int,
    value: *const c_char,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!value.is_null());

    //TODO: maybe it would be better to replace null terminated string with slice-like capability
    // (value ptr to beginning of a string + len of data to copy)
    let chunk = CStr::from_ptr(value).to_str().unwrap();
    (*txt)._inner.insert(&mut *(*txn)._inner, idx as u32, chunk)
}

#[no_mangle]
pub unsafe extern "C" fn ytext_remove_range(
    txt: *const YText,
    txn: *mut YTxn,
    idx: c_int,
    len: c_int,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    (*txt)
        ._inner
        .remove_range(&mut *(*txn)._inner, idx as u32, len as u32)
}

#[no_mangle]
pub unsafe extern "C" fn yarray_len(array: *const YArray) -> c_int {
    assert!(!array.is_null());

    (*array)._inner.len() as c_int
}

#[no_mangle]
pub unsafe extern "C" fn yarray_get(
    array: *const YArray,
    txn: *mut YTxn,
    idx: c_int,
) -> *const YVal {
    assert!(!array.is_null());
    assert!(!txn.is_null());

    if let Some(val) = (*array)._inner.get(&*(*txn)._inner, idx as u32) {
        Box::into_raw(Box::new(into_val(val)))
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yarray_insert_range(
    array: *const YArray,
    txn: *mut YTxn,
    idx: c_int,
    values: *const YVal,
    len: c_int,
) {
    assert!(!array.is_null());
    assert!(!txn.is_null());
    assert!(!values.is_null());

    let mut ptr = values;
    let mut i = 0;
    let len = len as isize;
    let mut vec = Vec::with_capacity(len as usize);

    /// try read as many values a JSON-like primitives and insert them at once
    while i < len {
        let val = ptr.offset(i);
        if (*val).tag <= 0 {
            let any = val_into_any(val.as_ref().unwrap());
            vec.push(any);
        } else {
            break;
        }
        i += 1;
    }

    if !vec.is_empty() {
        let len = (*array)._inner.len();
        (*array)._inner.insert_range(&mut *(*txn)._inner, len, vec);
    }

    /// insert remaining values one by one
    while i < len {
        let val = ptr.offset(i).read();
        (*array)._inner.push_back(&mut *(*txn)._inner, val);
        i += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn yarray_remove_range(
    array: *const YArray,
    txn: *mut YTxn,
    idx: c_int,
    len: c_int,
) {
    assert!(!array.is_null());
    assert!(!txn.is_null());

    (*array)
        ._inner
        .remove_range(&mut *(*txn)._inner, idx as u32, len as u32)
}

#[no_mangle]
pub unsafe extern "C" fn yarray_values(
    array: *const YArray,
    txn: *mut YTxn,
    len: *mut c_int,
) -> *const YVal {
    assert!(!array.is_null());
    assert!(!txn.is_null());

    let values: Vec<_> = (*array)
        ._inner
        .iter(&*(*txn)._inner)
        .map(|v| into_val(v))
        .collect();
    *len = values.len() as c_int;
    let ptr = values.as_ptr();
    forget(values);
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn ymap_len(map: *const YMap, txn: *const YTxn) -> c_int {
    assert!(!map.is_null());
    assert!(!txn.is_null());

    (*map)._inner.len(&*(*txn)._inner) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn ymap_entries(
    map: *const YMap,
    txn: *const YTxn,
    len: *mut c_int,
) -> *const YMapEntry {
    assert!(!map.is_null());
    assert!(!txn.is_null());

    let entries: Vec<_> = (*map)
        ._inner
        .iter(&*(*txn)._inner)
        .map(|(k, v)| {
            let key = CString::new(k.as_str()).unwrap().as_ptr();
            let value: Vec<_> = v.into_iter().map(|v| into_val(v)).collect();
            let value = value.as_ptr();
            forget(value);
            YMapEntry { key, value }
        })
        .collect();
    *len = entries.len() as c_int;
    let ptr = entries.as_ptr();
    forget(entries);
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn ymap_insert(
    map: *const YMap,
    txn: *mut YTxn,
    key: *const c_char,
    value: *const YVal,
) -> *const YMapEntry {
    assert!(!map.is_null());
    assert!(!txn.is_null());
    assert!(!key.is_null());
    assert!(!value.is_null());

    let cstr = CStr::from_ptr(key);
    let key = cstr.to_str().unwrap().to_string();

    if let Some(prev) = (*map)._inner.insert(&mut *(*txn)._inner, key, value.read()) {
        Box::into_raw(Box::new(YMapEntry {
            key: cstr.as_ptr(),
            value: Box::into_raw(Box::new(into_val(prev))),
        }))
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn ymap_remove(
    map: *const YMap,
    txn: *mut YTxn,
    key: *const c_char,
) -> *const YVal {
    assert!(!map.is_null());
    assert!(!txn.is_null());
    assert!(!key.is_null());

    let key = CStr::from_ptr(key).to_str().unwrap();

    if let Some(value) = (*map)._inner.remove(&mut *(*txn)._inner, key) {
        Box::into_raw(Box::new(into_val(value)))
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn ymap_get(
    map: *const YMap,
    txn: *const YTxn,
    key: *const c_char,
) -> *const YVal {
    assert!(!map.is_null());
    assert!(!txn.is_null());
    assert!(!key.is_null());

    let key = CStr::from_ptr(key).to_str().unwrap();

    if let Some(value) = (*map)._inner.get(&*(*txn)._inner, key) {
        Box::into_raw(Box::new(into_val(value)))
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn ymap_clear(map: *const YMap, txn: *mut YTxn) {
    assert!(!map.is_null());
    assert!(!txn.is_null());

    (*map)._inner.clear(&mut *(*txn)._inner);
}

#[no_mangle]
pub unsafe extern "C" fn yxml_tag(xml: *const YXmlElem) -> *const c_char {
    assert!(!xml.is_null());
    let tag = (*xml)._inner.tag();
    CString::new(tag).unwrap().as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn yxml_string(xml: *const YXmlElem, txn: *const YTxn) -> *const c_char {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let str = (*xml)._inner.to_string(&*(*txn)._inner);
    CString::new(str).unwrap().as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn yxml_insert_attr(
    xml: *const YXmlElem,
    txn: *mut YTxn,
    attr_name: *const c_char,
    attr_value: *const c_char,
) {
    assert!(!xml.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());
    assert!(!attr_value.is_null());

    let key = CStr::from_ptr(attr_name).to_str().unwrap();
    let value = CStr::from_ptr(attr_value).to_str().unwrap();

    (*xml)
        ._inner
        .insert_attribute(&mut *(*txn)._inner, key, value);
}

#[no_mangle]
pub unsafe extern "C" fn yxml_remove_attr(
    xml: *const YXmlElem,
    txn: *mut YTxn,
    attr_name: *const c_char,
) {
    assert!(!xml.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());

    let key = CStr::from_ptr(attr_name).to_str().unwrap();
    (*xml)._inner.remove_attribute(&mut *(*txn)._inner, key);
}

#[no_mangle]
pub unsafe extern "C" fn yxml_get_attr(
    xml: *const YXmlElem,
    txn: *const YTxn,
    attr_name: *const c_char,
) -> *const c_char {
    assert!(!xml.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());

    let key = CStr::from_ptr(attr_name).to_str().unwrap();
    if let Some(value) = (*xml)._inner.get_attribute(&*(*txn)._inner, key) {
        CString::new(value).unwrap().as_ptr()
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_attr_len(xml: *const YXmlElem, txn: *const YTxn) -> c_int {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    (*xml)._inner.len(&*(*txn)._inner) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn yxml_attrs(
    xml: *const YXmlElem,
    txn: *const YTxn,
    len: *mut c_int,
) -> *const YXmlAttr {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let attrs: Vec<_> = (*xml)
        ._inner
        .attributes(&*(*txn)._inner)
        .map(|(k, v)| YXmlAttr {
            name: CString::new(k).unwrap().as_ptr(),
            value: CString::new(v).unwrap().as_ptr(),
        })
        .collect();
    *len = attrs.len() as c_int;
    let ptr = attrs.as_ptr();
    forget(attrs);
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn yxml_next_sibling(xml: *const YXmlElem, txn: *const YTxn) -> *const YVal {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    if let Some(next) = (*xml)._inner.next_sibling(&*(*txn)._inner) {
        let val = match next {
            Xml::Element(v) => into_val(Value::YXmlElement(v)),
            Xml::Text(v) => into_val(Value::YXmlText(v)),
        };
        Box::into_raw(Box::new(val))
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_prev_sibling(xml: *const YXmlElem, txn: *const YTxn) -> *const YVal {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    if let Some(prev) = (*xml)._inner.prev_sibling(&*(*txn)._inner) {
        let val = match prev {
            Xml::Element(v) => into_val(Value::YXmlElement(v)),
            Xml::Text(v) => into_val(Value::YXmlText(v)),
        };
        Box::into_raw(Box::new(val))
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_parent(xml: *const YXmlElem, txn: *const YTxn) -> *const YVal {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    if let Some(parent) = (*xml)._inner.parent(&*(*txn)._inner) {
        let val = into_val(Value::YXmlElement(parent));
        Box::into_raw(Box::new(val))
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_child_len(xml: *const YXmlElem, txn: *const YTxn) -> c_int {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    (*xml)._inner.len(&*(*txn)._inner) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn yxml_successors(
    xml: *const YXmlElem,
    txn: *const YTxn,
    len: *mut c_int,
) -> *const YVal {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let successors: Vec<_> = (*xml)
        ._inner
        .successors(&*(*txn)._inner)
        .map(|child| match child {
            Xml::Element(v) => into_val(Value::YXmlElement(v)),
            Xml::Text(v) => into_val(Value::YXmlText(v)),
        })
        .collect();
    *len = successors.len() as c_int;
    let ptr = successors.as_ptr();
    forget(successors);
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn yxml_insert_elem(
    xml: *const YXmlElem,
    txn: *mut YTxn,
    idx: c_int,
    name: *const c_char,
) -> *const YXmlElem {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let elem = (*xml)
        ._inner
        .insert_elem(&mut *(*txn)._inner, idx as u32, name);
    Box::into_raw(Box::new(YXmlElem { _inner: elem }))
}

#[no_mangle]
pub unsafe extern "C" fn yxml_insert_text(
    xml: *const YXmlElem,
    txn: *mut YTxn,
    idx: c_int,
) -> *const YXmlText {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let text = (*xml)._inner.insert_text(&mut *(*txn)._inner, idx as u32);
    Box::into_raw(Box::new(YXmlText { _inner: text }))
}

#[no_mangle]
pub unsafe extern "C" fn yxml_remove_range(
    xml: *const YXmlElem,
    txn: *mut YTxn,
    idx: c_int,
    len: c_int,
) {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    (*xml)
        ._inner
        .remove_range(&mut *(*txn)._inner, idx as u32, len as u32)
}

#[no_mangle]
pub unsafe extern "C" fn yxml_get(
    xml: *const YXmlElem,
    txn: *const YTxn,
    idx: c_int,
) -> *const YVal {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    if let Some(child) = (*xml)._inner.get(&*(*txn)._inner, idx as u32) {
        let value = match child {
            Xml::Element(v) => into_val(Value::YXmlElement(v)),
            Xml::Text(v) => into_val(Value::YXmlText(v)),
        };
        Box::into_raw(Box::new(value))
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_len(txt: *const YXmlText, txn: *const YTxn) -> c_int {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    (*txt)._inner.len(&*(*txn)._inner) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_string(txt: *const YXmlText, txn: *const YTxn) -> *const c_char {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let str = (*txt)._inner.to_string(&*(*txn)._inner);
    CString::new(str).unwrap().as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_insert(
    txt: *const YXmlText,
    txn: *mut YTxn,
    idx: c_int,
    str: *const c_char,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!str.is_null());

    let chunk = CStr::from_ptr(str).to_str().unwrap();
    (*txt)._inner.insert(&mut *(*txn)._inner, idx as u32, chunk)
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_remove_range(
    txt: *const YXmlText,
    txn: *mut YTxn,
    idx: c_int,
    len: c_int,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    (*txt)
        ._inner
        .remove_range(&mut *(*txn)._inner, idx as u32, len as u32)
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_insert_attr(
    txt: *const YXmlText,
    txn: *mut YTxn,
    attr_name: *const c_char,
    attr_value: *const c_char,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());
    assert!(!attr_value.is_null());

    let name = CStr::from_ptr(attr_name).to_str().unwrap();
    let value = CStr::from_ptr(attr_value).to_str().unwrap();

    (*txt)
        ._inner
        .insert_attribute(&mut *(*txn)._inner, name, value)
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_remove_attr(
    txt: *const YXmlText,
    txn: *mut YTxn,
    attr_name: *const c_char,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());

    let name = CStr::from_ptr(attr_name).to_str().unwrap();
    (*txt)._inner.remove_attribute(&mut *(*txn)._inner, name);
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_get_attr(
    txt: *const YXmlText,
    txn: *const YTxn,
    attr_name: *const c_char,
) -> *const c_char {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());

    let name = CStr::from_ptr(attr_name).to_str().unwrap();
    if let Some(value) = (*txt)._inner.get_attribute(&*(*txn)._inner, name) {
        CString::new(value).unwrap().as_ptr()
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_attrs(
    txt: *const YXmlText,
    txn: *const YTxn,
    len: *mut c_int,
) -> *const YXmlAttr {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let attrs: Vec<_> = (*txt)
        ._inner
        .attributes(&*(*txn)._inner)
        .map(|(k, v)| YXmlAttr {
            name: CString::new(k).unwrap().as_ptr(),
            value: CString::new(v).unwrap().as_ptr(),
        })
        .collect();
    *len = attrs.len() as c_int;
    let ptr = attrs.as_ptr();
    forget(attrs);
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn yval_free(val: *mut YVal) {
    let tag = (*val).tag;
    if tag == Y_JSON_STR {
        std::mem::drop((*val).value.str);
    } else if tag == Y_JSON_BUF {
        std::mem::drop((*val).value.buf);
    } else if tag == Y_JSON_ARR {
        yval_free_json_array((*val).value.array, (*val).len);
    } else if tag == Y_JSON_MAP {
        yval_free_json_map((*val).value.map, (*val).len);
    } else if (*val).is_prelim != 0 {
        if tag == Y_MAP {
            yval_free_json_map((*val).value.map, (*val).len);
        } else if tag == Y_ARRAY {
            yval_free_json_array((*val).value.array, (*val).len);
        }
    }
}

unsafe fn yval_free_json_array(array: *const YVal, len: c_int) {
    for i in 0..len as isize {
        yval_free(array.offset(i) as *mut _)
    }
    std::mem::drop(array);
}

unsafe fn yval_free_json_map(map: *const YMapEntry, len: c_int) {
    for i in 0..len as isize {
        let e = &*map.offset(i);
        drop(e.key);
        yval_free(e.value as *mut _);
    }
    std::mem::drop(map);
}

#[no_mangle]
pub unsafe extern "C" fn yval_null() -> YVal {
    YVal {
        tag: Y_JSON_NULL,
        is_prelim: 1,
        len: 0,
        value: MaybeUninit::uninit().assume_init(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_undef() -> YVal {
    YVal {
        tag: Y_JSON_UNDEF,
        is_prelim: 1,
        len: 0,
        value: MaybeUninit::uninit().assume_init(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_bool(flag: u8) -> YVal {
    YVal {
        tag: Y_JSON_BOOL,
        is_prelim: 1,
        len: 1,
        value: YValContent { flag },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_float(num: c_float) -> YVal {
    YVal {
        tag: Y_JSON_NUM,
        is_prelim: 1,
        len: 1,
        value: YValContent { num },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_long(integer: c_long) -> YVal {
    YVal {
        tag: Y_JSON_INT,
        is_prelim: 1,
        len: 1,
        value: YValContent { integer },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_str(str: *const c_char, len: c_int) -> YVal {
    YVal {
        tag: Y_JSON_STR,
        is_prelim: 1,
        len,
        value: YValContent { str },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_buf(buf: *const u8, len: c_int) -> YVal {
    YVal {
        tag: Y_JSON_BUF,
        is_prelim: 1,
        len,
        value: YValContent { buf },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_json_array(json_array: *const YVal, len: c_int) -> YVal {
    YVal {
        tag: Y_JSON_ARR,
        is_prelim: 1,
        len,
        value: YValContent { array: json_array },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_json_map(json_map: *const YMapEntry, len: c_int) -> YVal {
    YVal {
        tag: Y_JSON_ARR,
        is_prelim: 1,
        len,
        value: YValContent { map: json_map },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_is_json(val: *const YVal) -> c_char {
    if (*val).tag <= 0 {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_bool(val: *const YVal) -> *const c_char {
    if (*val).tag == Y_JSON_BOOL {
        &(*val).value.flag as *const u8 as *const c_char
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_float(val: *const YVal) -> *const c_float {
    if (*val).tag == Y_JSON_NUM {
        &(*val).value.num as *const _
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_long(val: *const YVal) -> *const c_long {
    if (*val).tag == Y_JSON_INT {
        &(*val).value.integer as *const _
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_str(val: *const YVal) -> *const c_char {
    if (*val).tag == Y_JSON_STR {
        (*val).value.str
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_buf(val: *const YVal) -> *const u8 {
    if (*val).tag == Y_JSON_BUF {
        (*val).value.buf
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_json_array(val: *const YVal) -> *const YVal {
    if (*val).tag == Y_JSON_ARR {
        (*val).value.array
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_json_map(val: *const YVal) -> *const YMapEntry {
    if (*val).tag == Y_JSON_MAP {
        (*val).value.map
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_yarray(val: *const YVal) -> *const YArray {
    if (*val).tag == Y_ARRAY {
        (*val).value.y_array
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_ymap(val: *const YVal) -> *const YMap {
    if (*val).tag == Y_MAP {
        (*val).value.y_map
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_ytext(val: *const YVal) -> *const YText {
    if (*val).tag == Y_TEXT {
        (*val).value.y_text
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_yxml_text(val: *const YVal) -> *const YXmlText {
    if (*val).tag == Y_XML_TEXT {
        (*val).value.y_xml_text
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_yxml_elem(val: *const YVal) -> *const YXmlElem {
    if (*val).tag == Y_XML_ELEM {
        (*val).value.y_xml_elem
    } else {
        std::ptr::null()
    }
}

unsafe fn val_into_any(val: &YVal) -> Any {
    let tag = val.tag;
    if tag == Y_JSON_STR {
        let str = CStr::from_ptr(val.value.str).to_str().unwrap().to_owned();
        Any::String(str)
    } else if tag == Y_JSON_ARR {
        let src = std::slice::from_raw_parts(val.value.array, val.len as usize);
        let dst = src.into_iter().map(|val| val_into_any(val)).collect();
        Any::Array(dst)
    } else if tag == Y_JSON_MAP {
        let src = std::slice::from_raw_parts(val.value.map, val.len as usize);
        let dst = src
            .into_iter()
            .map(|e| {
                let key = CStr::from_ptr(e.key).to_str().unwrap().to_owned();
                let value = val_into_any(e.value.as_ref().unwrap());
                (key, value)
            })
            .collect();
        Any::Map(dst)
    } else if tag == Y_JSON_NULL {
        Any::Null
    } else if tag == Y_JSON_UNDEF {
        Any::Undefined
    } else if tag == Y_JSON_INT {
        Any::BigInt(val.value.integer as i64)
    } else if tag == Y_JSON_NUM {
        Any::Number(val.value.num as f64)
    } else if tag == Y_JSON_BOOL {
        Any::Bool(if val.value.flag == 0 { false } else { true })
    } else if tag == Y_JSON_BUF {
        let slice = std::slice::from_raw_parts(val.value.buf, val.len as usize);
        let buf = Box::from(slice);
        Any::Buffer(buf)
    } else {
        panic!("Unrecognized YVal value tag.")
    }
}

unsafe fn any_into_val(v: Any) -> YVal {
    match v {
        Any::Null => YVal {
            tag: Y_JSON_NULL,
            is_prelim: 0,
            len: 0,
            value: MaybeUninit::uninit().assume_init(),
        },
        Any::Undefined => YVal {
            tag: Y_JSON_UNDEF,
            is_prelim: 0,
            len: 0,
            value: MaybeUninit::uninit().assume_init(),
        },
        Any::Bool(v) => YVal {
            tag: Y_JSON_BOOL,
            is_prelim: 0,
            len: 1,
            value: YValContent {
                flag: if v { 1 } else { 0 },
            },
        },
        Any::Number(v) => YVal {
            tag: Y_JSON_NUM,
            is_prelim: 0,
            len: 1,
            value: YValContent { num: v as c_float },
        },
        Any::BigInt(v) => YVal {
            tag: Y_JSON_INT,
            is_prelim: 0,
            len: 1,
            value: YValContent {
                integer: v as c_long,
            },
        },
        Any::String(v) => {
            let len = v.len() as c_int;
            let str = CString::new(v).unwrap().as_ptr();
            YVal {
                tag: Y_JSON_STR,
                is_prelim: 0,
                len,
                value: YValContent { str },
            }
        }
        Any::Buffer(v) => YVal {
            tag: Y_JSON_BUF,
            is_prelim: 0,
            len: v.len() as c_int,
            value: YValContent {
                buf: Box::into_raw(v) as *const _,
            },
        },
        Any::Array(v) => {
            let values: Vec<_> = v.into_iter().map(|v| any_into_val(v)).collect();
            let array = values.as_ptr();
            let len = values.len() as c_int;
            forget(values);
            YVal {
                tag: Y_JSON_ARR,
                is_prelim: 0,
                len,
                value: YValContent { array },
            }
        }
        Any::Map(v) => {
            let entries: Vec<_> = v
                .into_iter()
                .map(|(k, v)| {
                    let key = CString::new(k).unwrap().into_raw();
                    let value = Box::into_raw(Box::new(any_into_val(v)));
                    YMapEntry { key, value }
                })
                .collect();
            let len = entries.len() as c_int;
            let map = entries.as_ptr();
            forget(entries);
            YVal {
                tag: Y_JSON_MAP,
                is_prelim: 0,
                len,
                value: YValContent { map },
            }
        }
    }
}

unsafe fn into_val(v: Value) -> YVal {
    match v {
        Value::Any(v) => any_into_val(v),
        Value::YText(v) => YVal {
            tag: Y_TEXT,
            is_prelim: 0,
            len: 1,
            value: YValContent {
                y_text: Box::into_raw(Box::new(YText { _inner: v })),
            },
        },
        Value::YArray(v) => YVal {
            tag: Y_ARRAY,
            is_prelim: 0,
            len: 1,
            value: YValContent {
                y_array: Box::into_raw(Box::new(YArray { _inner: v })),
            },
        },
        Value::YMap(v) => YVal {
            tag: Y_MAP,
            is_prelim: 0,
            len: 1,
            value: YValContent {
                y_map: Box::into_raw(Box::new(YMap { _inner: v })),
            },
        },
        Value::YXmlElement(v) => YVal {
            tag: Y_XML_ELEM,
            is_prelim: 0,
            len: 1,
            value: YValContent {
                y_xml_elem: Box::into_raw(Box::new(YXmlElem { _inner: v })),
            },
        },
        Value::YXmlText(v) => YVal {
            tag: Y_XML_TEXT,
            is_prelim: 0,
            len: 1,
            value: YValContent {
                y_xml_text: Box::into_raw(Box::new(YXmlText { _inner: v })),
            },
        },
    }
}
