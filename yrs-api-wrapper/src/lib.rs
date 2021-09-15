use lib0::any::Any;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::mem::{forget, MaybeUninit};
use std::os::raw::{c_char, c_float, c_int, c_long, c_uchar, c_ulong};
use yrs::block::{ItemContent, Prelim};
use yrs::types::{
    Branch, BranchRef, TypePtr, Value, TYPE_REFS_ARRAY, TYPE_REFS_MAP, TYPE_REFS_XML_ELEMENT,
    TYPE_REFS_XML_TEXT,
};
use yrs::updates::decoder::{Decode, DecoderV1};
use yrs::updates::encoder::{Encode, Encoder, EncoderV1};
use yrs::StateVector;
use yrs::Update;
use yrs::{DeleteSet, Xml};

#[no_mangle]
#[export_name = "Y_JSON_BOOL"]
pub static Y_JSON_BOOL: c_char = -8;

#[no_mangle]
#[export_name = "Y_JSON_NUM"]
pub static Y_JSON_NUM: c_char = -7;

#[no_mangle]
#[export_name = "Y_JSON_INT"]
pub static Y_JSON_INT: c_char = -6;

#[no_mangle]
#[export_name = "Y_JSON_STR"]
pub static Y_JSON_STR: c_char = -5;

#[no_mangle]
#[export_name = "Y_JSON_BUF"]
pub static Y_JSON_BUF: c_char = -4;

#[no_mangle]
#[export_name = "Y_JSON_ARRAY"]
pub static Y_JSON_ARR: c_char = -3;

#[no_mangle]
#[export_name = "Y_JSON_OBJECT"]
pub static Y_JSON_MAP: c_char = -2;

#[no_mangle]
#[export_name = "Y_JSON_NULL"]
pub static Y_JSON_NULL: c_char = -1;

#[no_mangle]
#[export_name = "Y_JSON_UNDEF"]
pub static Y_JSON_UNDEF: c_char = 0;

#[no_mangle]
#[export_name = "Y_ARRAY"]
pub static Y_ARRAY: c_char = 1;

#[no_mangle]
#[export_name = "Y_MAP"]
pub static Y_MAP: c_char = 2;

#[no_mangle]
#[export_name = "Y_TEXT"]
pub static Y_TEXT: c_char = 3;

#[no_mangle]
#[export_name = "Y_XML_ELEM"]
pub static Y_XML_ELEM: c_char = 4;

#[no_mangle]
#[export_name = "Y_XML_TEXT"]
pub static Y_XML_TEXT: c_char = 5;

/* pub types below are used by cbindgen for c header generation */

pub type Doc = yrs::Doc;
pub type Transaction = yrs::Transaction<'static>;
pub type Map = yrs::Map;
pub type Text = yrs::Text;
pub type Array = yrs::Array;
pub type XmlElement = yrs::XmlElement;
pub type XmlText = yrs::XmlText;
pub type ArrayIter = yrs::types::array::ArrayIter<'static, 'static>;
pub type MapIter = yrs::types::map::MapIter<'static, 'static>;
pub type Attributes = yrs::types::xml::Attributes<'static, 'static>;
pub type TreeWalker = yrs::types::xml::TreeWalker<'static, 'static>;

#[repr(C)]
pub struct YMapEntry {
    pub key: *const c_char,
    pub value: *const YVal,
}

impl Drop for YMapEntry {
    fn drop(&mut self) {
        unsafe {
            drop(CString::from_raw(self.key as *mut c_char));
            drop(Box::from_raw(self.value as *mut YVal))
        }
    }
}

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

#[no_mangle]
pub unsafe extern "C" fn ydoc_destroy(value: *mut Doc) {
    if !value.is_null() {
        drop(Box::from_raw(value));
    }
}

#[no_mangle]
pub unsafe extern "C" fn ytext_destroy(value: *mut Text) {
    if !value.is_null() {
        drop(Box::from_raw(value));
    }
}

#[no_mangle]
pub unsafe extern "C" fn yarray_destroy(value: *mut Array) {
    if !value.is_null() {
        drop(Box::from_raw(value));
    }
}

#[no_mangle]
pub unsafe extern "C" fn ymap_destroy(value: *mut Map) {
    if !value.is_null() {
        drop(Box::from_raw(value));
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_destroy(value: *mut XmlElement) {
    if !value.is_null() {
        drop(Box::from_raw(value));
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_text_destroy(value: *mut XmlText) {
    if !value.is_null() {
        drop(Box::from_raw(value));
    }
}

#[no_mangle]
pub unsafe extern "C" fn ymap_entry_destroy(value: *mut YMapEntry) {
    if !value.is_null() {
        drop(Box::from_raw(value));
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_attr_destroy(attr: *mut YXmlAttr) {
    if !attr.is_null() {
        drop(Box::from_raw(attr));
    }
}

#[no_mangle]
pub unsafe extern "C" fn ystr_destroy(str: *mut c_char) {
    if !str.is_null() {
        drop(CString::from_raw(str));
    }
}

#[no_mangle]
pub unsafe extern "C" fn ybinary_destroy(ptr: *mut c_uchar, len: c_int) {
    if !ptr.is_null() {
        drop(Vec::from_raw_parts(ptr, len as usize, len as usize));
    }
}

#[no_mangle]
pub extern "C" fn ydoc_new() -> *mut Doc {
    Box::into_raw(Box::new(Doc::new()))
}

#[no_mangle]
pub extern "C" fn ydoc_new_with_id(id: c_ulong) -> *mut Doc {
    Box::into_raw(Box::new(Doc::with_client_id(id as u64)))
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_new(doc: *mut Doc) -> *mut Transaction {
    assert!(!doc.is_null());

    let doc = doc.as_mut().unwrap();
    Box::into_raw(Box::new(doc.transact()))
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_commit(txn: *mut Transaction) {
    assert!(!txn.is_null());
    drop(Box::from_raw(txn)); // transaction is auto-committed when dropped
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_text(txn: *mut Transaction, name: *const c_char) -> *mut Text {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let value = txn.as_mut().unwrap().get_text(name);
    Box::into_raw(Box::new(value))
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_array(txn: *mut Transaction, name: *const c_char) -> *mut Array {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let value = txn.as_mut().unwrap().get_array(name);
    Box::into_raw(Box::new(value))
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_map(txn: *mut Transaction, name: *const c_char) -> *mut Map {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let value = txn.as_mut().unwrap().get_map(name);
    Box::into_raw(Box::new(value))
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_xml_elem(
    txn: *mut Transaction,
    name: *const c_char,
) -> *mut XmlElement {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let value = txn.as_mut().unwrap().get_xml_element(name);
    Box::into_raw(Box::new(value))
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_xml_text(txn: *mut Transaction, name: *const c_char) -> *mut XmlText {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let value = txn.as_mut().unwrap().get_xml_text(name);
    Box::into_raw(Box::new(value))
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_state_vector_v1(
    txn: *const Transaction,
    len: *mut c_int,
) -> *const c_uchar {
    assert!(!txn.is_null());

    let state_vector = txn.as_ref().unwrap().state_vector();
    let binary = state_vector.encode_v1().into_boxed_slice();

    *len = binary.len() as c_int;
    Box::into_raw(binary) as *const c_uchar
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_state_diff_v1(
    txn: *const Transaction,
    sv: *const c_uchar,
    sv_len: c_int,
    len: *mut c_int,
) -> *const c_uchar {
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
    Box::into_raw(binary) as *const c_uchar
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_apply(txn: *mut Transaction, diff: *const u8, diff_len: c_int) {
    assert!(!txn.is_null());
    assert!(!diff.is_null());

    let update = std::slice::from_raw_parts(diff, diff_len as usize);
    let mut decoder = DecoderV1::from(update);
    let update = Update::decode(&mut decoder);
    let ds = DeleteSet::decode(&mut decoder);
    txn.as_mut().unwrap().apply_update(update, ds)
}

#[no_mangle]
pub unsafe extern "C" fn ytext_len(txt: *const Text) -> c_int {
    assert!(!txt.is_null());
    txt.as_ref().unwrap().len() as c_int
}

#[no_mangle]
pub unsafe extern "C" fn ytext_string(txt: *const Text, txn: *const Transaction) -> *const c_char {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let txn = txn.as_ref().unwrap();
    let str = txt.as_ref().unwrap().to_string(txn);
    CString::new(str).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn ytext_insert(
    txt: *const Text,
    txn: *mut Transaction,
    idx: c_int,
    value: *const c_char,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!value.is_null());

    let chunk = CStr::from_ptr(value).to_str().unwrap();
    let txn = txn.as_mut().unwrap();
    let txt = txt.as_ref().unwrap();
    txt.insert(txn, idx as u32, chunk)
}

#[no_mangle]
pub unsafe extern "C" fn ytext_remove_range(
    txt: *const Text,
    txn: *mut Transaction,
    idx: c_int,
    len: c_int,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let txn = txn.as_mut().unwrap();
    let txt = txt.as_ref().unwrap();
    txt.remove_range(txn, idx as u32, len as u32)
}

#[no_mangle]
pub unsafe extern "C" fn yarray_len(array: *const Array) -> c_int {
    assert!(!array.is_null());

    let array = array.as_ref().unwrap();
    array.len() as c_int
}

#[no_mangle]
pub unsafe extern "C" fn yarray_get(
    array: *const Array,
    txn: *mut Transaction,
    idx: c_int,
) -> *const YVal {
    assert!(!array.is_null());
    assert!(!txn.is_null());

    let array = array.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    if let Some(val) = array.get(txn, idx as u32) {
        yval_from_value(val)
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yarray_insert_range(
    array: *const Array,
    txn: *mut Transaction,
    idx: c_int,
    values: *const YVal,
    len: c_int,
) {
    assert!(!array.is_null());
    assert!(!txn.is_null());
    assert!(!values.is_null());

    let arr = array.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    let ptr = values;
    let mut i = 0;
    let len = len as isize;
    let mut vec = Vec::with_capacity(len as usize);

    // try read as many values a JSON-like primitives and insert them at once
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
        arr.insert_range(txn, idx as u32, vec);
    }

    // insert remaining values one by one
    while i < len {
        let val = ptr.offset(i).read();
        arr.push_back(txn, val);
        i += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn yarray_remove_range(
    array: *const Array,
    txn: *mut Transaction,
    idx: c_int,
    len: c_int,
) {
    assert!(!array.is_null());
    assert!(!txn.is_null());

    let array = array.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    array.remove_range(txn, idx as u32, len as u32)
}

#[no_mangle]
pub unsafe extern "C" fn yarray_iter(
    array: *const Array,
    txn: *const Transaction,
) -> *mut ArrayIter {
    assert!(!array.is_null());
    assert!(!txn.is_null());

    let array = array.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();
    Box::into_raw(Box::new(array.iter(txn)))
}

#[no_mangle]
pub unsafe extern "C" fn yarray_iter_destroy(iter: *mut ArrayIter) {
    if !iter.is_null() {
        drop(Box::from_raw(iter))
    }
}

#[no_mangle]
pub unsafe extern "C" fn yarray_iter_next(iter: *mut ArrayIter) -> *mut YVal {
    assert!(!iter.is_null());

    let iter = iter.as_mut().unwrap();
    if let Some(v) = iter.next() {
        yval_from_value(v)
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn ymap_iter(map: *const Map, txn: *const Transaction) -> *mut MapIter {
    assert!(!map.is_null());
    assert!(!txn.is_null());

    let map = map.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();
    Box::into_raw(Box::new(map.iter(txn)))
}

#[no_mangle]
pub unsafe extern "C" fn ymap_iter_destroy(iter: *mut MapIter) {
    if !iter.is_null() {
        drop(Box::from_raw(iter))
    }
}

#[no_mangle]
pub unsafe extern "C" fn ymap_iter_next(iter: *mut MapIter) -> *mut YMapEntry {
    assert!(!iter.is_null());

    let iter = iter.as_mut().unwrap();
    if let Some((key, value)) = iter.next() {
        Box::into_raw(Box::new(YMapEntry {
            key: CString::new(key.as_str()).unwrap().into_raw(),
            value: yval_from_value(value),
        }))
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn ymap_len(map: *const Map, txn: *const Transaction) -> c_int {
    assert!(!map.is_null());
    assert!(!txn.is_null());

    let map = map.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    map.len(txn) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn ymap_insert(
    map: *const Map,
    txn: *mut Transaction,
    key: *const c_char,
    value: *const YVal,
) -> *const YMapEntry {
    assert!(!map.is_null());
    assert!(!txn.is_null());
    assert!(!key.is_null());
    assert!(!value.is_null());

    let cstr = CStr::from_ptr(key);
    let key = cstr.to_str().unwrap().to_string();

    let map = map.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    if let Some(prev) = map.insert(txn, key, value.read()) {
        Box::into_raw(Box::new(YMapEntry {
            key: cstr.as_ptr(),
            value: yval_from_value(prev),
        }))
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn ymap_remove(
    map: *const Map,
    txn: *mut Transaction,
    key: *const c_char,
) -> *const YVal {
    assert!(!map.is_null());
    assert!(!txn.is_null());
    assert!(!key.is_null());

    let key = CStr::from_ptr(key).to_str().unwrap();

    let map = map.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    if let Some(value) = map.remove(txn, key) {
        yval_from_value(value)
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn ymap_get(
    map: *const Map,
    txn: *const Transaction,
    key: *const c_char,
) -> *const YVal {
    assert!(!map.is_null());
    assert!(!txn.is_null());
    assert!(!key.is_null());

    let key = CStr::from_ptr(key).to_str().unwrap();

    let map = map.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    if let Some(value) = map.get(txn, key) {
        yval_from_value(value)
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn ymap_remove_all(map: *const Map, txn: *mut Transaction) {
    assert!(!map.is_null());
    assert!(!txn.is_null());

    let map = map.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    map.clear(txn);
}

#[no_mangle]
pub unsafe extern "C" fn yxml_tag(xml: *const XmlElement) -> *const c_char {
    assert!(!xml.is_null());
    let xml = xml.as_ref().unwrap();
    let tag = xml.tag();
    CString::new(tag).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn yxml_string(
    xml: *const XmlElement,
    txn: *const Transaction,
) -> *const c_char {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    let str = xml.to_string(txn);
    CString::new(str).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn yxml_insert_attr(
    xml: *const XmlElement,
    txn: *mut Transaction,
    attr_name: *const c_char,
    attr_value: *const c_char,
) {
    assert!(!xml.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());
    assert!(!attr_value.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    let key = CStr::from_ptr(attr_name).to_str().unwrap();
    let value = CStr::from_ptr(attr_value).to_str().unwrap();

    xml.insert_attribute(txn, key, value);
}

#[no_mangle]
pub unsafe extern "C" fn yxml_remove_attr(
    xml: *const XmlElement,
    txn: *mut Transaction,
    attr_name: *const c_char,
) {
    assert!(!xml.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    let key = CStr::from_ptr(attr_name).to_str().unwrap();
    xml.remove_attribute(txn, key);
}

#[no_mangle]
pub unsafe extern "C" fn yxml_get_attr(
    xml: *const XmlElement,
    txn: *const Transaction,
    attr_name: *const c_char,
) -> *const c_char {
    assert!(!xml.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    let key = CStr::from_ptr(attr_name).to_str().unwrap();
    if let Some(value) = xml.get_attribute(txn, key) {
        CString::new(value).unwrap().into_raw()
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_attr_len(xml: *const XmlElement, txn: *const Transaction) -> c_int {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    xml.len(txn) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn yxml_attr_iter(
    xml: *const XmlElement,
    txn: *const Transaction,
) -> *mut Attributes {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    Box::into_raw(Box::new(xml.attributes(txn)))
}

#[no_mangle]
pub unsafe extern "C" fn yxml_attr_iter_destroy(iter: *mut Attributes) {
    if !iter.is_null() {
        drop(Box::from_raw(iter))
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_attr_iter_next(iter: *mut Attributes) -> *mut YXmlAttr {
    assert!(!iter.is_null());

    let iter = iter.as_mut().unwrap();

    if let Some((name, value)) = iter.next() {
        Box::into_raw(Box::new(YXmlAttr {
            name: CString::new(name).unwrap().into_raw(),
            value: CString::new(value).unwrap().into_raw(),
        }))
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_next_sibling(
    xml: *const XmlElement,
    txn: *const Transaction,
) -> *const YVal {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    if let Some(next) = xml.next_sibling(txn) {
        match next {
            Xml::Element(v) => yval_from_value(Value::YXmlElement(v)),
            Xml::Text(v) => yval_from_value(Value::YXmlText(v)),
        }
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_prev_sibling(
    xml: *const XmlElement,
    txn: *const Transaction,
) -> *const YVal {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    if let Some(next) = xml.prev_sibling(txn) {
        match next {
            Xml::Element(v) => yval_from_value(Value::YXmlElement(v)),
            Xml::Text(v) => yval_from_value(Value::YXmlText(v)),
        }
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_parent(
    xml: *const XmlElement,
    txn: *const Transaction,
) -> *const YVal {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    if let Some(parent) = xml.parent(txn) {
        yval_from_value(Value::YXmlElement(parent))
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_child_len(xml: *const XmlElement, txn: *const Transaction) -> c_int {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    xml.len(txn) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn yxml_tree_walker(
    xml: *const XmlElement,
    txn: *const Transaction,
) -> *mut TreeWalker {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    Box::into_raw(Box::new(xml.successors(txn)))
}

#[no_mangle]
pub unsafe extern "C" fn yxml_tree_walker_destroy(iter: *mut TreeWalker) {
    if !iter.is_null() {
        drop(Box::from_raw(iter))
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_tree_walker_next(iter: *mut TreeWalker) -> *const YVal {
    assert!(!iter.is_null());

    let iter = iter.as_mut().unwrap();

    if let Some(next) = iter.next() {
        match next {
            Xml::Element(v) => yval_from_value(Value::YXmlElement(v)),
            Xml::Text(v) => yval_from_value(Value::YXmlText(v)),
        }
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_insert_elem(
    xml: *const XmlElement,
    txn: *mut Transaction,
    idx: c_int,
    name: *const c_char,
) -> *mut XmlElement {
    assert!(!xml.is_null());
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    let name = CStr::from_ptr(name).to_str().unwrap();
    let child = xml.insert_elem(txn, idx as u32, name);

    Box::into_raw(Box::new(child))
}

#[no_mangle]
pub unsafe extern "C" fn yxml_insert_text(
    xml: *const XmlElement,
    txn: *mut Transaction,
    idx: c_int,
) -> *mut XmlText {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();
    let child = xml.insert_text(txn, idx as u32);

    Box::into_raw(Box::new(child))
}

#[no_mangle]
pub unsafe extern "C" fn yxml_remove_range(
    xml: *const XmlElement,
    txn: *mut Transaction,
    idx: c_int,
    len: c_int,
) {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    xml.remove_range(txn, idx as u32, len as u32)
}

#[no_mangle]
pub unsafe extern "C" fn yxml_get(
    xml: *const XmlElement,
    txn: *const Transaction,
    idx: c_int,
) -> *const YVal {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    if let Some(child) = xml.get(txn, idx as u32) {
        match child {
            Xml::Element(v) => yval_from_value(Value::YXmlElement(v)),
            Xml::Text(v) => yval_from_value(Value::YXmlText(v)),
        }
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_destroy(txt: *mut XmlText) {
    if !txt.is_null() {
        drop(Box::from_raw(txt));
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_len(txt: *const XmlText, txn: *const Transaction) -> c_int {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let txt = txt.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    txt.len(txn) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_string(
    txt: *const XmlText,
    txn: *const Transaction,
) -> *const c_char {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let txt = txt.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    let str = txt.to_string(txn);
    CString::new(str).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_insert(
    txt: *const XmlText,
    txn: *mut Transaction,
    idx: c_int,
    str: *const c_char,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!str.is_null());

    let txt = txt.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    let chunk = CStr::from_ptr(str).to_str().unwrap();
    txt.insert(txn, idx as u32, chunk)
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_remove_range(
    txt: *const XmlText,
    txn: *mut Transaction,
    idx: c_int,
    len: c_int,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let txt = txt.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();
    txt.remove_range(txn, idx as u32, len as u32)
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_insert_attr(
    txt: *const XmlText,
    txn: *mut Transaction,
    attr_name: *const c_char,
    attr_value: *const c_char,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());
    assert!(!attr_value.is_null());

    let txt = txt.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    let name = CStr::from_ptr(attr_name).to_str().unwrap();
    let value = CStr::from_ptr(attr_value).to_str().unwrap();

    txt.insert_attribute(txn, name, value)
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_remove_attr(
    txt: *const XmlText,
    txn: *mut Transaction,
    attr_name: *const c_char,
) {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());

    let txt = txt.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();
    let name = CStr::from_ptr(attr_name).to_str().unwrap();

    txt.remove_attribute(txn, name)
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_get_attr(
    txt: *const XmlText,
    txn: *const Transaction,
    attr_name: *const c_char,
) -> *const c_char {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());

    let txt = txt.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();
    let name = CStr::from_ptr(attr_name).to_str().unwrap();

    if let Some(value) = txt.get_attribute(txn, name) {
        CString::new(value).unwrap().into_raw()
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_attrs_iter(
    txt: *const XmlText,
    txn: *const Transaction,
) -> *mut Attributes {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let txt = txt.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    Box::into_raw(Box::new(txt.attributes(txn)))
}

#[repr(C)]
pub struct YVal {
    pub tag: c_char,
    pub prelim: u8,
    pub len: c_int,
    value: YValContent,
}

impl Drop for YVal {
    fn drop(&mut self) {
        let tag = self.tag;
        unsafe {
            if tag == Y_JSON_STR {
                ystr_destroy(self.value.str as *mut _);
            } else if tag == Y_JSON_BUF {
                ybinary_destroy(self.value.buf as *mut _, self.len);
            } else if tag == Y_JSON_ARR {
                yval_list_destroy(self.value.array, self.len);
            } else if tag == Y_JSON_MAP {
                ymap_entry_list_destroy(self.value.map, self.len);
            } else if self.prelim != 0 {
                if tag == Y_MAP {
                    ymap_entry_list_destroy(self.value.map, self.len);
                } else if tag == Y_ARRAY {
                    yval_list_destroy(self.value.array, self.len);
                }
            }
        }
    }
}

impl Prelim for YVal {
    fn into_content(
        self,
        _txn: &mut yrs::Transaction,
        ptr: TypePtr,
    ) -> (ItemContent, Option<Self>) {
        unsafe {
            if self.tag <= 0 {
                let value = val_into_any(&self);
                (ItemContent::Any(vec![value]), None)
            } else {
                assert_ne!(self.prelim, 0);

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

    fn integrate(self, txn: &mut yrs::Transaction, inner_ref: BranchRef) {
        unsafe {
            if self.tag == Y_MAP {
                let map = Map::from(inner_ref);
                let src = std::slice::from_raw_parts(self.value.map, self.len as usize);
                for e in src {
                    let e = e.as_ref().unwrap();
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
                    let value = ptr.offset(i).read().read();
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
    buf: *const c_uchar,
    array: *const *const YVal,
    map: *const *const YMapEntry,
    y_array: *const Array,
    y_map: *const Map,
    y_text: *const Text,
    y_xml_elem: *const XmlElement,
    y_xml_text: *const XmlText,
}

#[no_mangle]
pub unsafe extern "C" fn yval_destroy(val: *mut YVal) {}

unsafe fn yval_list_destroy(array: *const *const YVal, len: c_int) {
    let values = Vec::from_raw_parts(array as *mut *const YVal, len as usize, len as usize);
    for ptr in values {
        yval_destroy(ptr as *mut _);
    }
}

unsafe fn ymap_entry_list_destroy(map: *const *const YMapEntry, len: c_int) {
    let values = Vec::from_raw_parts(map as *mut *const YMapEntry, len as usize, len as usize);
    for ptr in values {
        ymap_entry_destroy(ptr as *mut _);
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_null() -> YVal {
    YVal {
        tag: Y_JSON_NULL,
        prelim: 1,
        len: 0,
        value: MaybeUninit::uninit().assume_init(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_undef() -> YVal {
    YVal {
        tag: Y_JSON_UNDEF,
        prelim: 1,
        len: 0,
        value: MaybeUninit::uninit().assume_init(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_bool(flag: u8) -> YVal {
    YVal {
        tag: Y_JSON_BOOL,
        prelim: 1,
        len: 1,
        value: YValContent { flag },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_float(num: c_float) -> YVal {
    YVal {
        tag: Y_JSON_NUM,
        prelim: 1,
        len: 1,
        value: YValContent { num },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_long(integer: c_long) -> YVal {
    YVal {
        tag: Y_JSON_INT,
        prelim: 1,
        len: 1,
        value: YValContent { integer },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_str(str: *const c_char, len: c_int) -> YVal {
    YVal {
        tag: Y_JSON_STR,
        prelim: 1,
        len,
        value: YValContent { str },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_buf(buf: *const u8, len: c_int) -> YVal {
    YVal {
        tag: Y_JSON_BUF,
        prelim: 1,
        len,
        value: YValContent { buf },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_json_array(json_array: *const *const YVal, len: c_int) -> YVal {
    YVal {
        tag: Y_JSON_ARR,
        prelim: 1,
        len,
        value: YValContent { array: json_array },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_json_map(json_map: *const *const YMapEntry, len: c_int) -> YVal {
    YVal {
        tag: Y_JSON_ARR,
        prelim: 1,
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
pub unsafe extern "C" fn yval_read_json_array(val: *const YVal) -> *const *const YVal {
    if (*val).tag == Y_JSON_ARR {
        (*val).value.array
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_json_map(val: *const YVal) -> *const *const YMapEntry {
    if (*val).tag == Y_JSON_MAP {
        (*val).value.map
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_yarray(val: *const YVal) -> *const Array {
    if (*val).tag == Y_ARRAY {
        &*(*val).value.y_array as *const _
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_ymap(val: *const YVal) -> *const Map {
    if (*val).tag == Y_MAP {
        &*(*val).value.y_map as *const _
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_ytext(val: *const YVal) -> *const Text {
    if (*val).tag == Y_TEXT {
        &*(*val).value.y_text as *const _
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_yxml_text(val: *const YVal) -> *const XmlText {
    if (*val).tag == Y_XML_TEXT {
        &*(*val).value.y_xml_text as *const _
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_yxml_elem(val: *const YVal) -> *const XmlElement {
    if (*val).tag == Y_XML_ELEM {
        &*(*val).value.y_xml_elem as *const _
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
        let ptr = val.value.array;
        let mut dst = Vec::with_capacity(val.len as usize);
        let mut i = 0;
        while i < val.len as isize {
            let value = ptr.offset(i).read().as_ref().unwrap();
            let any = val_into_any(value);
            dst.push(any);
            i += 1;
        }
        Any::Array(dst)
    } else if tag == Y_JSON_MAP {
        let ptr = val.value.map;
        let mut dst = HashMap::with_capacity(val.len as usize);
        let mut i = 0;
        while i < val.len as isize {
            let e = ptr.offset(i).read().as_ref().unwrap();
            let key = CStr::from_ptr(e.key).to_str().unwrap().to_owned();
            let value = val_into_any(e.value.as_ref().unwrap());
            dst.insert(key, value);
            i += 1;
        }
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
        let slice = std::slice::from_raw_parts(val.value.buf as *mut u8, val.len as usize);
        let buf = Box::from(slice);
        Any::Buffer(buf)
    } else {
        panic!("Unrecognized YVal value tag.")
    }
}

fn yval_from_any(v: Any) -> *mut YVal {
    let result = match v {
        Any::Null => YVal {
            tag: Y_JSON_NULL,
            prelim: 0,
            len: 0,
            value: unsafe { MaybeUninit::uninit().assume_init() },
        },
        Any::Undefined => YVal {
            tag: Y_JSON_UNDEF,
            prelim: 0,
            len: 0,
            value: unsafe { MaybeUninit::uninit().assume_init() },
        },
        Any::Bool(v) => YVal {
            tag: Y_JSON_BOOL,
            prelim: 0,
            len: 1,
            value: YValContent {
                flag: if v { 1 } else { 0 },
            },
        },
        Any::Number(v) => YVal {
            tag: Y_JSON_NUM,
            prelim: 0,
            len: 1,
            value: YValContent { num: v as c_float },
        },
        Any::BigInt(v) => YVal {
            tag: Y_JSON_INT,
            prelim: 0,
            len: 1,
            value: YValContent {
                integer: v as c_long,
            },
        },
        Any::String(v) => {
            let len = v.len() as c_int;
            let str = CString::new(v).unwrap().into_raw();
            YVal {
                tag: Y_JSON_STR,
                prelim: 0,
                len,
                value: YValContent { str },
            }
        }
        Any::Buffer(v) => YVal {
            tag: Y_JSON_BUF,
            prelim: 0,
            len: v.len() as c_int,
            value: YValContent {
                buf: Box::into_raw(v) as *const _,
            },
        },
        Any::Array(v) => {
            let values: Vec<_> = v
                .into_iter()
                .map(|v| yval_from_any(v) as *const YVal)
                .collect();
            let len = values.len() as c_int;
            let boxed = values.into_boxed_slice();
            let array = boxed.as_ptr();
            forget(boxed);
            YVal {
                tag: Y_JSON_ARR,
                prelim: 0,
                len,
                value: YValContent { array },
            }
        }
        Any::Map(v) => {
            let entries: Vec<_> = v
                .into_iter()
                .map(|(k, v)| {
                    let key = CString::new(k).unwrap().into_raw();
                    let value = yval_from_any(v);
                    Box::into_raw(Box::new(YMapEntry { key, value })) as *const YMapEntry
                })
                .collect();
            let len = entries.len() as c_int;
            let boxed = entries.into_boxed_slice();
            let map = boxed.as_ptr();
            YVal {
                tag: Y_JSON_MAP,
                prelim: 0,
                len,
                value: YValContent { map },
            }
        }
    };
    Box::into_raw(Box::new(result))
}

fn yval_from_value(v: Value) -> *mut YVal {
    match v {
        Value::Any(v) => yval_from_any(v),
        Value::YText(v) => Box::into_raw(Box::new(YVal {
            tag: Y_TEXT,
            prelim: 0,
            len: 1,
            value: YValContent {
                y_text: Box::into_raw(Box::new(v)),
            },
        })),
        Value::YArray(v) => Box::into_raw(Box::new(YVal {
            tag: Y_ARRAY,
            prelim: 0,
            len: 1,
            value: YValContent {
                y_array: Box::into_raw(Box::new(v)),
            },
        })),
        Value::YMap(v) => Box::into_raw(Box::new(YVal {
            tag: Y_MAP,
            prelim: 0,
            len: 1,
            value: YValContent {
                y_map: Box::into_raw(Box::new(v)),
            },
        })),
        Value::YXmlElement(v) => Box::into_raw(Box::new(YVal {
            tag: Y_XML_ELEM,
            prelim: 0,
            len: 1,
            value: YValContent {
                y_xml_elem: Box::into_raw(Box::new(v)),
            },
        })),
        Value::YXmlText(v) => Box::into_raw(Box::new(YVal {
            tag: Y_XML_TEXT,
            prelim: 0,
            len: 1,
            value: YValContent {
                y_xml_text: Box::into_raw(Box::new(v)),
            },
        })),
    }
}
