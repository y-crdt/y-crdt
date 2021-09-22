use lib0::any::Any;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::mem::{forget, MaybeUninit, ManuallyDrop};
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

#[no_mangle]
#[export_name = "Y_TRUE"]
pub static Y_TRUE: c_char = 1;

#[no_mangle]
#[export_name = "Y_FALSE"]
pub static Y_FALSE: c_char = 0;

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
pub unsafe extern "C" fn yxmlelem_destroy(value: *mut XmlElement) {
    if !value.is_null() {
        drop(Box::from_raw(value));
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_destroy(value: *mut XmlText) {
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
pub unsafe extern "C" fn yxmlattr_destroy(attr: *mut YXmlAttr) {
    if !attr.is_null() {
        drop(Box::from_raw(attr));
    }
}

#[no_mangle]
pub unsafe extern "C" fn ystring_destroy(str: *mut c_char) {
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
pub unsafe extern "C" fn ytransaction_new(doc: *mut Doc) -> *mut Transaction {
    assert!(!doc.is_null());

    let doc = doc.as_mut().unwrap();
    Box::into_raw(Box::new(doc.transact()))
}

#[no_mangle]
pub unsafe extern "C" fn ytransaction_commit(txn: *mut Transaction) {
    assert!(!txn.is_null());
    drop(Box::from_raw(txn)); // transaction is auto-committed when dropped
}

#[no_mangle]
pub unsafe extern "C" fn ytext(txn: *mut Transaction, name: *const c_char) -> *mut Text {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let value = txn.as_mut().unwrap().get_text(name);
    Box::into_raw(Box::new(value))
}

#[no_mangle]
pub unsafe extern "C" fn yarray(txn: *mut Transaction, name: *const c_char) -> *mut Array {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let value = txn.as_mut().unwrap().get_array(name);
    Box::into_raw(Box::new(value))
}

#[no_mangle]
pub unsafe extern "C" fn ymap(txn: *mut Transaction, name: *const c_char) -> *mut Map {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let value = txn.as_mut().unwrap().get_map(name);
    Box::into_raw(Box::new(value))
}

#[no_mangle]
pub unsafe extern "C" fn yxmlelem(
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
pub unsafe extern "C" fn yxmltext(txn: *mut Transaction, name: *const c_char) -> *mut XmlText {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let value = txn.as_mut().unwrap().get_xml_text(name);
    Box::into_raw(Box::new(value))
}

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

#[no_mangle]
pub unsafe extern "C" fn ytransaction_apply(txn: *mut Transaction, diff: *const c_uchar, diff_len: c_int) {
    assert!(!txn.is_null());
    assert!(!diff.is_null());

    let update = std::slice::from_raw_parts(diff as *const u8, diff_len as usize);
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
pub unsafe extern "C" fn ytext_string(txt: *const Text, txn: *const Transaction) -> *mut c_char {
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
) -> *mut YOutput {
    assert!(!array.is_null());
    assert!(!txn.is_null());

    let array = array.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    if let Some(val) = array.get(txn, idx as u32) {
        Box::into_raw(Box::new(YOutput::from(val)))
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yarray_insert_range(
    array: *const Array,
    txn: *mut Transaction,
    idx: c_int,
    values: *const YInput,
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
    let mut vec: Vec<Any> = Vec::with_capacity(len as usize);

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
pub unsafe extern "C" fn yarray_iter_next(iter: *mut ArrayIter) -> *mut YOutput {
    assert!(!iter.is_null());

    let iter = iter.as_mut().unwrap();
    if let Some(v) = iter.next() {
        Box::into_raw(Box::new(YOutput::from(v)))
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
        Box::into_raw(Box::new(YMapEntry::new(key.as_str(), value)))
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
    value: *const YInput,
) {
    assert!(!map.is_null());
    assert!(!txn.is_null());
    assert!(!key.is_null());
    assert!(!value.is_null());

    let cstr = CStr::from_ptr(key);
    let key = cstr.to_str().unwrap().to_string();

    let map = map.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    map.insert(txn, key, value.read());
}

#[no_mangle]
pub unsafe extern "C" fn ymap_remove(
    map: *const Map,
    txn: *mut Transaction,
    key: *const c_char,
) -> c_char {
    assert!(!map.is_null());
    assert!(!txn.is_null());
    assert!(!key.is_null());

    let key = CStr::from_ptr(key).to_str().unwrap();

    let map = map.as_ref().unwrap();
    let txn = txn.as_mut().unwrap();

    if let Some(_) = map.remove(txn, key) {
        Y_TRUE
    } else {
        Y_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn ymap_get(
    map: *const Map,
    txn: *const Transaction,
    key: *const c_char,
) -> *mut YOutput {
    assert!(!map.is_null());
    assert!(!txn.is_null());
    assert!(!key.is_null());

    let key = CStr::from_ptr(key).to_str().unwrap();

    let map = map.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    if let Some(value) = map.get(txn, key) {
        Box::into_raw(Box::new(YOutput::from(value)))
    } else {
        std::ptr::null_mut()
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
pub unsafe extern "C" fn yxmlelem_tag(xml: *const XmlElement) -> *mut c_char {
    assert!(!xml.is_null());
    let xml = xml.as_ref().unwrap();
    let tag = xml.tag();
    CString::new(tag).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn yxmlelem_string(
    xml: *const XmlElement,
    txn: *const Transaction,
) -> *mut c_char {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    let str = xml.to_string(txn);
    CString::new(str).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn yxmlelem_insert_attr(
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
pub unsafe extern "C" fn yxmlelem_remove_attr(
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
pub unsafe extern "C" fn yxmlelem_get_attr(
    xml: *const XmlElement,
    txn: *const Transaction,
    attr_name: *const c_char,
) -> *mut c_char {
    assert!(!xml.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    let key = CStr::from_ptr(attr_name).to_str().unwrap();
    if let Some(value) = xml.get_attribute(txn, key) {
        CString::new(value).unwrap().into_raw()
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmlattr_iter(
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
pub unsafe extern "C" fn yxmlattr_iter_destroy(iter: *mut Attributes) {
    if !iter.is_null() {
        drop(Box::from_raw(iter))
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmlattr_iter_next(iter: *mut Attributes) -> *mut YXmlAttr {
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
pub unsafe extern "C" fn yxmlelem_next_sibling(
    xml: *const XmlElement,
    txn: *const Transaction,
) -> *mut YOutput {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    if let Some(next) = xml.next_sibling(txn) {
        match next {
            Xml::Element(v) =>
                Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            Xml::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
        }
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmlelem_prev_sibling(
    xml: *const XmlElement,
    txn: *const Transaction,
) -> *mut YOutput {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    if let Some(next) = xml.prev_sibling(txn) {
        match next {
            Xml::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            Xml::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
        }
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_next_sibling(
    xml: *const XmlText,
    txn: *const Transaction,
) -> *mut YOutput {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    if let Some(next) = xml.next_sibling(txn) {
        match next {
            Xml::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            Xml::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
        }
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_prev_sibling(
    xml: *const XmlText,
    txn: *const Transaction,
) -> *mut YOutput {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    if let Some(next) = xml.prev_sibling(txn) {
        match next {
            Xml::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            Xml::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
        }
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmlelem_parent(
    xml: *const XmlElement,
    txn: *const Transaction,
) -> *mut XmlElement {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    if let Some(parent) = xml.parent(txn) {
        Box::into_raw(Box::new(parent))
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmlelem_child_len(xml: *const XmlElement, txn: *const Transaction) -> c_int {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    xml.len(txn) as c_int
}


#[no_mangle]
pub unsafe extern "C" fn yxmlelem_first_child(xml: *const XmlElement, txn: *const Transaction) -> *mut YOutput {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    if let Some(value) = xml.first_child(txn) {
        match value {
            Xml::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            Xml::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
        }
    } else {
        std::ptr::null_mut()
    }
}


#[no_mangle]
pub unsafe extern "C" fn yxmlelem_tree_walker(
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
pub unsafe extern "C" fn yxmlelem_tree_walker_destroy(iter: *mut TreeWalker) {
    if !iter.is_null() {
        drop(Box::from_raw(iter))
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmlelem_tree_walker_next(iter: *mut TreeWalker) -> *mut YOutput {
    assert!(!iter.is_null());

    let iter = iter.as_mut().unwrap();

    if let Some(next) = iter.next() {
        match next {
            Xml::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            Xml::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
        }
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmlelem_insert_elem(
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
pub unsafe extern "C" fn yxmlelem_insert_text(
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
pub unsafe extern "C" fn yxmlelem_remove_range(
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
pub unsafe extern "C" fn yxmlelem_get(
    xml: *const XmlElement,
    txn: *const Transaction,
    idx: c_int,
) -> *const YOutput {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let xml = xml.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();

    if let Some(child) = xml.get(txn, idx as u32) {
        match child {
            Xml::Element(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlElement(v)))),
            Xml::Text(v) => Box::into_raw(Box::new(YOutput::from(Value::YXmlText(v)))),
        }
    } else {
        std::ptr::null()
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
) -> *mut c_char {
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
) -> *mut c_char {
    assert!(!txt.is_null());
    assert!(!txn.is_null());
    assert!(!attr_name.is_null());

    let txt = txt.as_ref().unwrap();
    let txn = txn.as_ref().unwrap();
    let name = CStr::from_ptr(attr_name).to_str().unwrap();

    if let Some(value) = txt.get_attribute(txn, name) {
        CString::new(value).unwrap().into_raw()
    } else {
        std::ptr::null_mut()
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
pub struct YInput {
    pub tag: c_char,
    pub len: c_int,
    value: YInputContent,
}

impl YInput {
    fn into(self) -> Any {
        let tag = self.tag;
        unsafe {
            if tag == Y_JSON_STR {
                let str = CStr::from_ptr(self.value.str).to_str().unwrap().to_owned();
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
                Any::Array(dst)
            } else if tag == Y_JSON_MAP {
                let mut dst = HashMap::with_capacity(self.len as usize);
                let mut keys = self.value.map.keys;
                let mut values = self.value.map.values;
                let mut i = 0;
                while i < self.len as isize {
                    let key = CStr::from_ptr(keys.offset(i).read()).to_str().unwrap().to_owned();
                    let value = values.offset(i).read().into();
                    dst.insert(key, value);
                    i += 1;
                }
                Any::Map(dst)
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
                let slice = std::slice::from_raw_parts(self.value.buf as *mut u8, self.len as usize);
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
    fn drop(&mut self) {
    }
}

impl Prelim for YInput {
    fn into_content(
        self,
        _txn: &mut yrs::Transaction,
        ptr: TypePtr,
    ) -> (ItemContent, Option<Self>) {
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
                let inner = BranchRef::new(Branch::new(ptr, type_ref, None));
                (ItemContent::Type(inner), Some(self))
            }
        }
    }

    fn integrate(self, txn: &mut yrs::Transaction, inner_ref: BranchRef) {
        unsafe {
            if self.tag == Y_MAP {
                let map = Map::from(inner_ref);
                let mut keys = self.value.map.keys;
                let mut values = self.value.map.values;
                let mut i = 0;
                while i < self.len as isize {
                    let key = CStr::from_ptr(keys.offset(i).read()).to_str().unwrap().to_owned();
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
            } else {
                panic!("Cannot use given type tag as preinitialized value");
            };
        }
    }
}

#[repr(C)]
pub struct YOutput {
    pub tag: c_char,
    pub len: c_int,
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
                write!(f, "{}", if self.value.flag == 0 { "false" } else { "true" })
            } else if tag == Y_JSON_UNDEF {
                write!(f, "undefined")
            } else if tag == Y_JSON_NULL {
                write!(f, "null")
            } else if tag == Y_JSON_STR {
                write!(f,"{}", CString::from_raw(self.value.str).to_str().unwrap())
            } else if tag == Y_MAP {
                write!(f,"YMap")
            } else if tag == Y_ARRAY {
                write!(f,"YArray")
            } else if tag == Y_JSON_ARR {
                write!(f,"[")?;
                let slice = std::slice::from_raw_parts(self.value.array, self.len as usize);
                for o in slice {
                    write!(f, ", {}", o)?;
                }
                write!(f,"]")
            } else if tag == Y_JSON_MAP {
                write!(f,"{{")?;
                let slice = std::slice::from_raw_parts(self.value.map, self.len as usize);
                for e in slice {
                    write!(f, ", '{}' => {}", CStr::from_ptr(e.key).to_str().unwrap(), e.value)?;
                }
                write!(f,"}}")
            } else if tag == Y_TEXT {
                write!(f,"YText")
            } else if tag == Y_XML_TEXT {
                write!(f,"YXmlText")
            } else if tag == Y_XML_ELEM {
                write!(f,"YXmlElement", )
            } else if tag == Y_JSON_BUF {
                write!(f,"YBinary(len: {})", self.len)
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
            } else if tag == Y_MAP {
                drop(Box::from_raw(self.value.y_map));
            } else if tag == Y_ARRAY {
                drop(Box::from_raw(self.value.y_array));
            } else if tag == Y_JSON_ARR {
                drop(Vec::from_raw_parts(self.value.array, self.len as usize, self.len as usize));
            } else if tag == Y_JSON_MAP {
                drop(Vec::from_raw_parts(self.value.map, self.len as usize, self.len as usize));
            } else if tag == Y_TEXT {
                drop(Box::from_raw(self.value.y_text));
            } else if tag == Y_XML_TEXT {
                drop(Box::from_raw(self.value.y_xmltext));
            } else if tag == Y_XML_ELEM {
                drop(Box::from_raw(self.value.y_xmlelem));
            } else if tag == Y_JSON_BUF {
                drop(Vec::from_raw_parts(self.value.buf, self.len as usize, self.len as usize));
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
                    value: MaybeUninit::uninit().assume_init()
                },
                Any::Undefined => YOutput {
                    tag: Y_JSON_UNDEF,
                    len: 0,
                    value: MaybeUninit::uninit().assume_init()
                },
                Any::Bool(v) => YOutput {
                    tag: Y_JSON_BOOL,
                    len: 1,
                    value: YOutputContent { flag: if v { Y_TRUE } else { Y_FALSE } }
                },
                Any::Number(v) => YOutput {
                    tag: Y_JSON_NUM,
                    len: 1,
                    value: YOutputContent { num: v as _ }
                },
                Any::BigInt(v) => YOutput {
                    tag: Y_JSON_INT,
                    len: 1,
                    value: YOutputContent { integer: v as _ }
                },
                Any::String(v) => YOutput {
                    tag: Y_JSON_STR,
                    len: v.len() as c_int,
                    value: YOutputContent { str: CString::new(v).unwrap().into_raw() }
                },
                Any::Buffer(v) => YOutput {
                    tag: Y_JSON_BUF,
                    len: v.len() as c_int,
                    value: YOutputContent { buf: Box::into_raw(v) as *mut _ }
                },
                Any::Array(v) => {
                    let len = v.len() as c_int;
                    let mut array: Vec<_> = v.into_iter().map(|v| YOutput::from(v)).collect();
                    array.shrink_to_fit();
                    let ptr = array.as_mut_ptr();
                    forget(array);
                    YOutput {
                        tag: Y_JSON_ARR,
                        len,
                        value: YOutputContent { array: ptr }
                    }
                }
                Any::Map(v) => {
                    let len = v.len() as c_int;
                    let mut array: Vec<_> = v.into_iter()
                        .map(|(k,v)| YMapEntry::new(k.as_str(), Value::Any(v)))
                        .collect();
                    array.shrink_to_fit();
                    let ptr = array.as_mut_ptr();
                    forget(array);
                    YOutput {
                        tag: Y_JSON_MAP,
                        len,
                        value: YOutputContent { map: ptr }
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
            value: YOutputContent { y_text: Box::into_raw(Box::new(v)) }
        }
    }
}

impl From<Array> for YOutput {
    fn from(v: Array) -> Self {
        YOutput {
            tag: Y_ARRAY,
            len: 1,
            value: YOutputContent { y_array: Box::into_raw(Box::new(v)) }
        }
    }
}

impl From<Map> for YOutput {
    fn from(v: Map) -> Self {
        YOutput {
            tag: Y_MAP,
            len: 1,
            value: YOutputContent { y_map: Box::into_raw(Box::new(v)) }
        }
    }
}

impl From<XmlElement> for YOutput {
    fn from(v: XmlElement) -> Self {
        YOutput {
            tag: Y_XML_ELEM,
            len: 1,
            value: YOutputContent { y_xmlelem: Box::into_raw(Box::new(v)) }
        }
    }
}

impl From<XmlText> for YOutput {
    fn from(v: XmlText) -> Self {
        YOutput {
            tag: Y_XML_TEXT,
            len: 1,
            value: YOutputContent { y_xmltext: Box::into_raw(Box::new(v)) }
        }
    }
}

#[repr(C)]
union YOutputContent {
    flag: c_char,
    num: c_float,
    integer: c_long,
    str: *mut c_char,
    buf: *mut c_uchar,
    array: *mut YOutput,
    map: *mut YMapEntry,
    y_array: *mut Array,
    y_map: *mut Map,
    y_text: *mut Text,
    y_xmlelem: *mut XmlElement,
    y_xmltext: *mut XmlText,
}

#[no_mangle]
pub unsafe extern "C" fn youtput_destroy(val: *mut YOutput) {
    if !val.is_null() {
        drop(Box::from_raw(val))
    }
}

#[no_mangle]
pub unsafe extern "C" fn yinput_null() -> YInput {
    YInput {
        tag: Y_JSON_NULL,
        len: 0,
        value: MaybeUninit::uninit().assume_init(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn yinput_undefined() -> YInput {
    YInput {
        tag: Y_JSON_UNDEF,
        len: 0,
        value: MaybeUninit::uninit().assume_init(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn yinput_bool(flag: c_char) -> YInput {
    YInput {
        tag: Y_JSON_BOOL,
        len: 1,
        value: YInputContent { flag },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yinput_float(num: c_float) -> YInput {
    YInput {
        tag: Y_JSON_NUM,
        len: 1,
        value: YInputContent { num },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yinput_long(integer: c_long) -> YInput {
    YInput {
        tag: Y_JSON_INT,
        len: 1,
        value: YInputContent { integer },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yinput_string(str: *const c_char) -> YInput {
    YInput {
        tag: Y_JSON_STR,
        len: 1,
        value: YInputContent { str: str as *mut c_char },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yinput_binary(buf: *const u8, len: c_int) -> YInput {
    YInput {
        tag: Y_JSON_BUF,
        len,
        value: YInputContent { buf: buf as *mut u8 },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yinput_json_array(values: *mut YInput, len: c_int) -> YInput {
    YInput {
        tag: Y_JSON_ARR,
        len,
        value: YInputContent { values },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yinput_json_map(keys: *mut *mut c_char, values: *mut YInput, len: c_int) -> YInput {
    YInput {
        tag: Y_JSON_ARR,
        len,
        value: YInputContent { map: ManuallyDrop::new(YMapInputData { keys, values }) },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yinput_yarray(values: *mut YInput, len: c_int) -> YInput {
    YInput {
        tag: Y_ARRAY,
        len,
        value: YInputContent { values },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yinput_ymap(keys: *mut *mut c_char, values: *mut YInput, len: c_int) -> YInput {
    YInput {
        tag: Y_MAP,
        len,
        value: YInputContent { map: ManuallyDrop::new(YMapInputData { keys, values }) },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yinput_ytext(str: *mut c_char) -> YInput {
    YInput {
        tag: Y_TEXT,
        len: 1,
        value: YInputContent { str },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yinput_yxmlelem(xml_children: *mut YInput, len: c_int) -> YInput {
    YInput {
        tag: Y_XML_ELEM,
        len,
        value: YInputContent { values: xml_children },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yinput_yxmltext(str: *mut c_char) -> YInput {
    YInput {
        tag: Y_XML_TEXT,
        len: 1,
        value: YInputContent { str },
    }
}

#[no_mangle]
pub unsafe extern "C" fn youtput_read_bool(val: *const YOutput) -> *const c_char {
    let v = val.as_ref().unwrap();
    if v.tag == Y_JSON_BOOL {
        &v.value.flag
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn youtput_read_float(val: *const YOutput) -> *const c_float {
    let v = val.as_ref().unwrap();
    if v.tag == Y_JSON_NUM {
        &v.value.num
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn youtput_read_long(val: *const YOutput) -> *const c_long {
    let v = val.as_ref().unwrap();
    if v.tag == Y_JSON_INT {
        &v.value.integer
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn youtput_read_string(val: *const YOutput) -> *mut c_char {
    let v = val.as_ref().unwrap();
    if v.tag == Y_JSON_STR {
        v.value.str
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn youtput_read_binary(val: *const YOutput) -> *const c_uchar {
    let v = val.as_ref().unwrap();
    if v.tag == Y_JSON_BUF {
        v.value.buf
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn youtput_read_json_array(val: *const YOutput) -> *mut YOutput {
    let v = val.as_ref().unwrap();
    if v.tag == Y_JSON_ARR {
        v.value.array
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn youtput_read_json_map(val: *const YOutput) -> *mut YMapEntry {
    let v = val.as_ref().unwrap();
    if v.tag == Y_JSON_MAP {
        v.value.map
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn youtput_read_yarray(val: *const YOutput) -> *mut Array {
    let v = val.as_ref().unwrap();
    if v.tag == Y_ARRAY {
        v.value.y_array
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn youtput_read_yxmlelem(val: *const YOutput) -> *mut XmlElement {
    let v = val.as_ref().unwrap();
    if v.tag == Y_XML_ELEM {
        v.value.y_xmlelem
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn youtput_read_ymap(val: *const YOutput) -> *mut Map {
    let v = val.as_ref().unwrap();
    if v.tag == Y_MAP {
        v.value.y_map
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn youtput_read_ytext(val: *const YOutput) -> *mut Text {
    let v = val.as_ref().unwrap();
    if v.tag == Y_TEXT {
        v.value.y_text
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn youtput_read_yxmltext(val: *const YOutput) -> *mut XmlText {
    let v = val.as_ref().unwrap();
    if v.tag == Y_XML_TEXT {
        v.value.y_xmltext
    } else {
        std::ptr::null_mut()
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
            let array = yarray(txn, CString::new("test").unwrap().as_ptr());

            let y_true = yinput_bool(Y_TRUE);
            let y_false = yinput_bool(Y_FALSE);
            let y_float = yinput_float(0.5);
            let y_int = yinput_long(11);
            let y_str = yinput_string(CString::new("hello").unwrap().as_ptr());

            let values = &[
                y_true,
                y_false,
                y_float,
                y_int,
                y_str
            ];

            yarray_insert_range(array, txn, 0, values.as_ptr(), 5);

            ytransaction_commit(txn);
            ydoc_destroy(doc);
        }
    }

}
