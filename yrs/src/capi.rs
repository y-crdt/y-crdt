use crate::{Doc, Transaction, Text, Array, Map, XmlElement, XmlText};
use std::ffi::c_void;
use std::os::raw::{c_float, c_long, c_char, c_ulong};
use std::mem::ManuallyDrop;


#[export_name = "Y_JSON_BOOL"]
pub const Y_JSON_BOOL: c_char = -8;

#[export_name = "Y_JSON_NUM"]
pub const Y_JSON_NUM: c_char = -7;

#[export_name = "Y_JSON_INT"]
pub const Y_JSON_INT: c_char = -6;

#[export_name = "Y_JSON_STR"]
pub const Y_JSON_STR: c_char = -5;

#[export_name = "Y_JSON_BUF"]
pub const Y_JSON_BUF: c_char = -4;

#[export_name = "Y_JSON_ARRAY"]
pub const Y_JSON_ARR: c_char = -3;

#[export_name = "Y_JSON_OBJECT"]
pub const Y_JSON_MAP: c_char = -2;

#[export_name = "Y_JSON_NULL"]
pub const Y_JSON_NULL: c_char = -1;

#[export_name = "Y_JSON_UNDEF"]
pub const Y_JSON_UNDEF: c_char = 0;

#[export_name = "Y_ARRAY"]
pub const Y_ARRAY: c_char = 1;

#[export_name = "Y_MAP"]
pub const Y_MAP: c_char = 2;

#[export_name = "Y_TEXT"]
pub const Y_TEXT: c_char = 3;

#[export_name = "Y_XML_ELEM"]
pub const Y_XML_ELEMENT: c_char = 4;

#[export_name = "Y_XML_TEXT"]
pub const Y_XML_TEXT: c_char = 5;

#[repr(C)]
struct YDoc {
    _inner: *mut Doc
}

#[repr(C)]
struct YTxn {
    _inner: *mut Transaction<'static>
}

#[repr(C)]
struct YText {
    _inner: *const Text
}

#[repr(C)]
struct YArray {
    _inner: *const Array
}

#[repr(C)]
struct YMap {
    _inner: *const Map
}

#[repr(C)]
struct YMapEntry {
    pub key: *const c_char,
    pub value: *const YVal,
}

#[repr(C)]
struct YXmlElem {
    _inner: *const XmlElement
}

#[repr(C)]
struct YXmlText {
    _inner: *const XmlText
}

#[repr(C)]
struct YVal {
    pub tag: c_char,
    pub value: *const c_void
}

#[link(name = "yrs")]
extern "C" {

    fn ydoc_new() -> *mut YDoc;
    fn ydoc_new_with_id(id: c_ulong) -> *mut YDoc;

    fn ytxn_new(doc: *mut YDoc) -> *mut YTxn;
    fn ytxn_commit(txn: *mut YTxn);
    fn ytxn_text(txn: *mut YTxn, name: *const c_char) -> *const YText;
    fn ytxn_array(txn: *mut YTxn, name: *const c_char) -> *const YArray;
    fn ytxn_map(txn: *mut YTxn, name: *const c_char) -> *const YMap;
    fn ytxn_xml_elem(txn: *mut YTxn, name: *const c_char) -> *const YXmlElem;
    fn ytxn_xml_text(txn: *mut YTxn, name: *const c_char) -> *const YXmlText;
    fn ytxn_state_vector_v1(txn: *const YTxn, len: *mut size_t) -> *const c_char;
    fn ytxn_state_diff_v1(txn: *const YTxn, sv: *const c_char, sv_len: size_t, len: *mut size_t) -> *const c_char;
    fn ytxn_apply(txn: *const YTxn, diff: *const c_char, diff_len: size_t);

    fn ytext_len(txt: *const YText) -> size_t;
    fn ytext_string(txt: *const YText, txn: *mut YTxn) -> *c_char;
    fn ytext_insert(txt: *const YText, txn: *mut YTxn, idx: size_t, value: *const c_char, len: size_t);
    fn ytext_remove_range(txt: *const YText, txn: *mut YTxn, idx: size_t, len: size_t);

    fn yarray_len(txt: *const YArray) -> size_t;
    fn yarray_get(txt: *const YArray, txn: *mut YTxn, idx: size_t) -> *const YVal;
    fn yarray_insert_range(txt: *const YArray, txn: *mut YTxn, idx: size_t, values: *const YVal, len: size_t);
    fn yarray_remove_range(txt: *const YArray, txn: *mut YTxn, idx: size_t, len: size_t);
    fn yarray_values(txt: *const YArray, txn: *mut YTxn) -> *const YVal;

    fn ymap_len(map: *const YMap, txn: *const YTxn) -> size_t;
    fn ymap_entries(map: *const YMap, txn: *const YTxn) -> *const YMapEntry;
    fn ymap_insert(map: *const YMap, txn: *const YTxn, key: *const c_char, value: *const YVal) -> *const YMapEntry;
    fn ymap_remove(map: *const YMap, txn: *const YTxn, key: *const c_char) -> *const YVal;
    fn ymap_get(map: *const YMap, txn: *const YTxn, key: *const c_char) -> *const YVal;
    fn ymap_clear(map: *const YMap, txn: *const YTxn);

    fn yxml_tag(xml: *const YXmlElem) -> *const c_char;
    fn yxml_string(xml: *const YXmlElem, txn: *const YTxn) -> *const c_char;
    fn yxml_insert_attr(xml: *const YXmlElem, txn: *mut YTxn, attr_name: *const c_char, attr_value: *const c_char);
    fn yxml_remove_attr(xml: *const YXmlElem, txn: *mut YTxn, attr_name: *const c_char);
    fn yxml_get_attr(xml: *const YXmlElem, txn: *const YTxn, attr_name: *const c_char) -> *const c_char;
    fn yxml_attr_len(xml: *const YXmlElem, txn: *const YTxn) -> size_t;
    fn yxml_attrs(xml: *const YXmlElem, txn: *const YTxn) -> **const c_char;
    fn yxml_next_sibling(xml: *const YXmlElem, txn: *const YTxn) -> *const YVal;
    fn yxml_prev_sibling(xml: *const YXmlElem, txn: *const YTxn) -> *const YVal;
    fn yxml_parent(xml: *const YXmlElem, txn: *const YTxn) -> *const YVal;
    fn yxml_child_len(xml: *const YXmlElem, txn: *const YTxn) -> size_t;
    fn yxml_children(xml: *const YXmlElem, txn: *const YTxn) -> *const YVal;
    fn yxml_insert_elem(xml: *const YXmlElem, txn: *const YTxn, idx: size_t, name: *const c_char) -> *const YXmlElem;
    fn yxml_insert_text(xml: *const YXmlElem, txn: *const YTxn, idx: size_t) -> *const YXmlText;
    fn yxml_remove_range(xml: *const YXmlElem, txn: *const YTxn, idx: size_t, len: size_t);
    fn yxml_get(xml: *const YXmlElem, txn: *const YTxn, idx: size_t) -> *const YVal;

    fn yxmltext_len(txt: *const YXmlText) -> size_t;
    fn yxmltext_string(txt: *const YXmlText, txn: *mut YTxn) -> *c_char;
    fn yxmltext_insert(txt: *const YXmlText, txn: *mut YTxn, idx: size_t, value: *const c_char, len: size_t);
    fn yxmltext_remove_range(txt: *const YXmlText, txn: *mut YTxn, idx: size_t, len: size_t);
    fn yxmltext_insert_attr(txt: *const YXmlText, txn: *mut YTxn, attr_name: *const c_char, attr_value: *const c_char);
    fn yxmltext_remove_attr(txt: *const YXmlText, txn: *mut YTxn, attr_name: *const c_char);
    fn yxmltext_get_attr(txt: *const YXmlText, txn: *const YTxn, attr_name: *const c_char) -> *const c_char;
    fn yxmltext_attr_len(txt: *const YXmlText, txn: *const YTxn) -> size_t;
    fn yxmltext_attrs(txt: *const YXmlText, txn: *const YTxn) -> **const c_char;

    fn yval_free(val: *const YVal);
    fn yval_bool(val: *const YVal) -> *const c_char;
    fn yval_num(val: *const YVal) -> *const c_float;
    fn yval_int(val: *const YVal) -> *const c_long;
    fn yval_str(val: *const YVal) -> *const c_char;
    fn yval_buf(val: *const YVal, len: *mut size_t) -> *const c_char;
    fn yval_arr(val: *const YVal, len: *mut size_t) -> *const YVal;
    fn yval_map(val: *const YVal, len: *mut size_t) -> *const YMapEntry;
    fn yval_ytext(val: *const YVal) -> *const YText;
    fn yval_yarray(val: *const YVal) -> *const YArray;
    fn yval_ymap(val: *const YVal) -> *const YMap;
    fn yval_yxml_elem(val: *const YVal) -> *const YXmlElem;
    fn yval_yxml_text(val: *const YVal) -> *const YXmlText;
}
