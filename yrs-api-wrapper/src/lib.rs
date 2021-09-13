use lib0::any::Any;
use std::ffi::{CStr, CString};
use std::mem::{forget, ManuallyDrop, MaybeUninit};
use std::os::raw::{c_char, c_float, c_int, c_long, c_void};
use yrs::block::{ItemContent, Prelim};
use yrs::types::{
    Branch, BranchRef, TypePtr, Value, TYPE_REFS_ARRAY, TYPE_REFS_MAP, TYPE_REFS_XML_ELEMENT,
    TYPE_REFS_XML_TEXT,
};
use yrs::updates::decoder::{Decode, DecoderV1};
use yrs::updates::encoder::{Encode, Encoder, EncoderV1};
use yrs::DeleteSet;
use yrs::Update;
use yrs::{Array, Doc, Map, StateVector, Text, Transaction, Xml, XmlElement, XmlText};

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

#[repr(C)]
pub struct YDoc {
    _inner: *mut c_void,
}

#[repr(C)]
pub struct YTxn {
    _inner: *mut c_void,
}

#[repr(C)]
pub struct YText {
    _inner: *mut c_void,
}

#[repr(C)]
pub struct YArray {
    _inner: *mut c_void,
}

#[repr(C)]
pub struct YMap {
    _inner: *mut c_void,
}

#[repr(C)]
pub struct YXmlElem {
    _inner: *mut c_void,
}

#[repr(C)]
pub struct YXmlText {
    _inner: *mut c_void,
}

#[repr(C)]
pub struct YMapEntry {
    pub key: *const c_char,
    pub value: *const YVal,
}

#[repr(C)]
pub struct YXmlAttr {
    pub name: *const c_char,
    pub value: *const c_char,
}

#[repr(C)]
pub struct YVal {
    pub tag: c_char,
    pub prelim: u8,
    pub len: c_int,
    value: YValContent,
}

impl Prelim for YVal {
    fn into_content(self, _txn: &mut Transaction, ptr: TypePtr) -> (ItemContent, Option<Self>) {
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
    y_array: ManuallyDrop<YArray>,
    y_map: ManuallyDrop<YMap>,
    y_text: ManuallyDrop<YText>,
    y_xml_elem: ManuallyDrop<YXmlElem>,
    y_xml_text: ManuallyDrop<YXmlText>,
}

#[no_mangle]
pub unsafe extern "C" fn ystr_free(str: *mut c_char) {
    let str = CString::from_raw(str);
    drop(str);
}

#[no_mangle]
pub unsafe extern "C" fn yxml_attr_free(attr: *mut YXmlAttr) {
    ystr_free((*attr).name as *mut _);
    ystr_free((*attr).value as *mut _);
}

#[no_mangle]
pub unsafe extern "C" fn ymap_entry_free(e: *mut YMapEntry) {
    ystr_free((*e).key as *mut _);
    yval_free((*e).value as *mut _);
}

#[no_mangle]
pub extern "C" fn ydoc_new() -> YDoc {
    YDoc {
        _inner: Box::into_raw(Box::new(Doc::new())) as *mut c_void,
    }
}

#[no_mangle]
pub extern "C" fn ydoc_new_with_id(id: u64) -> YDoc {
    YDoc {
        _inner: Box::into_raw(Box::new(Doc::with_client_id(id))) as *mut c_void,
    }
}

#[no_mangle]
pub unsafe extern "C" fn ydoc_free(doc: *mut YDoc) {
    assert!(!doc.is_null());

    let boxed = Box::from_raw((*doc)._inner as *mut Doc);
    drop(boxed);
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_new(doc: *mut YDoc) -> YTxn {
    assert!(!doc.is_null());

    YTxn {
        _inner: Box::into_raw(Box::new(
            ((*doc)._inner as *mut Doc).as_mut().unwrap().transact(),
        )) as *mut c_void,
    }
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_commit(txn: *mut YTxn) {
    assert!(!txn.is_null());
    ((*txn)._inner as *mut Transaction<'static>)
        .as_mut()
        .unwrap()
        .commit();
    let boxed = Box::from_raw((*txn)._inner as *mut Transaction<'static>);
    drop(boxed);
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_text(txn: *mut YTxn, name: *const c_char) -> YText {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let value = ((*txn)._inner as *mut Transaction<'static>)
        .as_mut()
        .unwrap()
        .get_text(name);
    YText {
        _inner: Box::into_raw(Box::new(value)) as *mut c_void,
    }
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_array(txn: *mut YTxn, name: *const c_char) -> YArray {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    YArray {
        _inner: Box::into_raw(Box::new(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_mut()
                .unwrap()
                .get_array(name),
        )) as *mut c_void,
    }
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_map(txn: *mut YTxn, name: *const c_char) -> YMap {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    YMap {
        _inner: Box::into_raw(Box::new(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_mut()
                .unwrap()
                .get_map(name),
        )) as *mut c_void,
    }
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_xml_elem(txn: *mut YTxn, name: *const c_char) -> YXmlElem {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    YXmlElem {
        _inner: Box::into_raw(Box::new(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_mut()
                .unwrap()
                .get_xml_element(name),
        )) as *mut c_void,
    }
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_xml_text(txn: *mut YTxn, name: *const c_char) -> YXmlText {
    assert!(!txn.is_null());
    assert!(!name.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    YXmlText {
        _inner: Box::into_raw(Box::new(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_mut()
                .unwrap()
                .get_xml_text(name),
        )) as *mut c_void,
    }
}

#[no_mangle]
pub unsafe extern "C" fn ytxn_state_vector_v1(txn: *const YTxn, len: *mut c_int) -> *const u8 {
    assert!(!txn.is_null());

    let state_vector = ((*txn)._inner as *const Transaction<'static>)
        .as_ref()
        .unwrap()
        .state_vector();
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
    ((*txn)._inner as *const Transaction<'static>)
        .as_ref()
        .unwrap()
        .encode_diff(&sv, &mut encoder);
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
    ((*txn)._inner as *mut Transaction<'static>)
        .as_mut()
        .unwrap()
        .apply_update(update, ds)
}

#[no_mangle]
pub unsafe extern "C" fn ytext_free(txt: *mut YText) {
    assert!(!txt.is_null());
    let boxed = Box::from_raw((*txt)._inner as *mut Text);
    drop(boxed);
}

#[no_mangle]
pub unsafe extern "C" fn ytext_len(txt: *const YText) -> c_int {
    assert!(!txt.is_null());
    ((*txt)._inner as *const Text).as_ref().unwrap().len() as c_int
}

#[no_mangle]
pub unsafe extern "C" fn ytext_string(txt: *const YText, txn: *mut YTxn) -> *const c_char {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let str = ((*txt)._inner as *const Text).as_ref().unwrap().to_string(
        ((*txn)._inner as *mut Transaction<'static>)
            .as_ref()
            .unwrap(),
    );
    let cstr = CString::new(str).unwrap();
    cstr.into_raw()
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
    ((*txt)._inner as *const Text).as_ref().unwrap().insert(
        ((*txn)._inner as *mut Transaction<'static>)
            .as_mut()
            .unwrap(),
        idx as u32,
        chunk,
    )
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

    ((*txt)._inner as *const Text)
        .as_ref()
        .unwrap()
        .remove_range(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_mut()
                .unwrap(),
            idx as u32,
            len as u32,
        )
}

#[no_mangle]
pub unsafe extern "C" fn yarray_free(array: *mut YArray) {
    assert!(!array.is_null());
    let boxed = Box::from_raw((*array)._inner as *mut Array);
    drop(boxed);
}

#[no_mangle]
pub unsafe extern "C" fn yarray_len(array: *const YArray) -> c_int {
    assert!(!array.is_null());

    ((*array)._inner as *const Array).as_ref().unwrap().len() as c_int
}

#[no_mangle]
pub unsafe extern "C" fn yarray_get(
    array: *const YArray,
    txn: *mut YTxn,
    idx: c_int,
) -> *const YVal {
    assert!(!array.is_null());
    assert!(!txn.is_null());

    if let Some(val) = ((*array)._inner as *const Array).as_ref().unwrap().get(
        ((*txn)._inner as *const Transaction<'static>)
            .as_ref()
            .unwrap(),
        idx as u32,
    ) {
        Box::into_raw(Box::new(yval_from_value(val)))
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

    let txn = ((*txn)._inner as *mut Transaction<'static>)
        .as_mut()
        .unwrap();
    let arr = ((*array)._inner as *const Array).as_ref().unwrap();

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
    array: *const YArray,
    txn: *mut YTxn,
    idx: c_int,
    len: c_int,
) {
    assert!(!array.is_null());
    assert!(!txn.is_null());

    ((*array)._inner as *const Array)
        .as_ref()
        .unwrap()
        .remove_range(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_mut()
                .unwrap(),
            idx as u32,
            len as u32,
        )
}

#[no_mangle]
pub unsafe extern "C" fn yarray_values(
    array: *const YArray,
    txn: *const YTxn,
    len: *mut c_int,
) -> *const YVal {
    assert!(!array.is_null());
    assert!(!txn.is_null());

    let values: Vec<_> = ((*array)._inner as *const Array)
        .as_ref()
        .unwrap()
        .iter(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_ref()
                .unwrap(),
        )
        .map(yval_from_value)
        .collect();
    *len = values.len() as c_int;
    let ptr = values.as_ptr();
    forget(values);
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn ymap_free(map: *mut YMap) {
    assert!(!map.is_null());
    let boxed = Box::from_raw((*map)._inner as *mut Map);
    drop(boxed);
}

#[no_mangle]
pub unsafe extern "C" fn ymap_len(map: *const YMap, txn: *const YTxn) -> c_int {
    assert!(!map.is_null());
    assert!(!txn.is_null());

    ((*map)._inner as *const Map).as_ref().unwrap().len(
        ((*txn)._inner as *mut Transaction<'static>)
            .as_ref()
            .unwrap(),
    ) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn ymap_entries(
    map: *const YMap,
    txn: *const YTxn,
    len: *mut c_int,
) -> *const YMapEntry {
    assert!(!map.is_null());
    assert!(!txn.is_null());

    let entries: Vec<_> = ((*map)._inner as *const Map)
        .as_ref()
        .unwrap()
        .iter(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_ref()
                .unwrap(),
        )
        .map(|(k, v)| {
            let key = CString::new(k.as_str()).unwrap().into_raw();
            let value: Vec<_> = v.into_iter().map(yval_from_value).collect();
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

    if let Some(prev) = ((*map)._inner as *const Map).as_ref().unwrap().insert(
        ((*txn)._inner as *mut Transaction<'static>)
            .as_mut()
            .unwrap(),
        key,
        value.read(),
    ) {
        Box::into_raw(Box::new(YMapEntry {
            key: cstr.as_ptr(),
            value: Box::into_raw(Box::new(yval_from_value(prev))),
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

    if let Some(value) = ((*map)._inner as *const Map).as_ref().unwrap().remove(
        ((*txn)._inner as *mut Transaction<'static>)
            .as_mut()
            .unwrap(),
        key,
    ) {
        Box::into_raw(Box::new(yval_from_value(value)))
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

    if let Some(value) = ((*map)._inner as *const Map).as_ref().unwrap().get(
        ((*txn)._inner as *const Transaction<'static>)
            .as_ref()
            .unwrap(),
        key,
    ) {
        Box::into_raw(Box::new(yval_from_value(value)))
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn ymap_remove_all(map: *const YMap, txn: *mut YTxn) {
    assert!(!map.is_null());
    assert!(!txn.is_null());

    ((*map)._inner as *const Map).as_ref().unwrap().clear(
        ((*txn)._inner as *mut Transaction<'static>)
            .as_mut()
            .unwrap(),
    );
}

#[no_mangle]
pub unsafe extern "C" fn yxml_free(xml: *mut YXmlElem) {
    assert!(!xml.is_null());
    let boxed = Box::from_raw((*xml)._inner as *mut XmlElement);
    drop(boxed);
}

#[no_mangle]
pub unsafe extern "C" fn yxml_tag(xml: *const YXmlElem) -> *const c_char {
    assert!(!xml.is_null());
    let tag = ((*xml)._inner as *const XmlElement).as_ref().unwrap().tag();
    CString::new(tag).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn yxml_string(xml: *const YXmlElem, txn: *const YTxn) -> *const c_char {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let str = ((*xml)._inner as *const XmlElement)
        .as_ref()
        .unwrap()
        .to_string(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_ref()
                .unwrap(),
        );
    CString::new(str).unwrap().into_raw()
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

    ((*xml)._inner as *const XmlElement)
        .as_ref()
        .unwrap()
        .insert_attribute(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_mut()
                .unwrap(),
            key,
            value,
        );
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
    ((*xml)._inner as *const XmlElement)
        .as_ref()
        .unwrap()
        .remove_attribute(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_mut()
                .unwrap(),
            key,
        );
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
    if let Some(value) = ((*xml)._inner as *const XmlElement)
        .as_ref()
        .unwrap()
        .get_attribute(
            ((*txn)._inner as *const Transaction<'static>)
                .as_ref()
                .unwrap(),
            key,
        )
    {
        CString::new(value).unwrap().into_raw()
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_attr_len(xml: *const YXmlElem, txn: *const YTxn) -> c_int {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    ((*xml)._inner as *const XmlElement).as_ref().unwrap().len(
        ((*txn)._inner as *mut Transaction<'static>)
            .as_ref()
            .unwrap(),
    ) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn yxml_attrs(
    xml: *const YXmlElem,
    txn: *const YTxn,
    len: *mut c_int,
) -> *const YXmlAttr {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let attrs: Vec<_> = ((*xml)._inner as *const XmlElement)
        .as_ref()
        .unwrap()
        .attributes(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_ref()
                .unwrap(),
        )
        .map(|(k, v)| YXmlAttr {
            name: CString::new(k).unwrap().into_raw(),
            value: CString::new(v).unwrap().into_raw(),
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

    if let Some(next) = ((*xml)._inner as *const XmlElement)
        .as_ref()
        .unwrap()
        .next_sibling(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_ref()
                .unwrap(),
        )
    {
        let val = match next {
            Xml::Element(v) => yval_from_value(Value::YXmlElement(v)),
            Xml::Text(v) => yval_from_value(Value::YXmlText(v)),
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

    if let Some(prev) = ((*xml)._inner as *const XmlElement)
        .as_ref()
        .unwrap()
        .prev_sibling(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_ref()
                .unwrap(),
        )
    {
        let val = match prev {
            Xml::Element(v) => yval_from_value(Value::YXmlElement(v)),
            Xml::Text(v) => yval_from_value(Value::YXmlText(v)),
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

    if let Some(parent) = ((*xml)._inner as *const XmlElement)
        .as_ref()
        .unwrap()
        .parent(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_ref()
                .unwrap(),
        )
    {
        let val = yval_from_value(Value::YXmlElement(parent));
        Box::into_raw(Box::new(val))
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_child_len(xml: *const YXmlElem, txn: *const YTxn) -> c_int {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    ((*xml)._inner as *const XmlElement).as_ref().unwrap().len(
        ((*txn)._inner as *mut Transaction<'static>)
            .as_ref()
            .unwrap(),
    ) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn yxml_successors(
    xml: *const YXmlElem,
    txn: *const YTxn,
    len: *mut c_int,
) -> *const YVal {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let successors: Vec<_> = ((*xml)._inner as *const XmlElement)
        .as_ref()
        .unwrap()
        .successors(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_ref()
                .unwrap(),
        )
        .map(|child| match child {
            Xml::Element(v) => yval_from_value(Value::YXmlElement(v)),
            Xml::Text(v) => yval_from_value(Value::YXmlText(v)),
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
) -> YXmlElem {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let name = CStr::from_ptr(name).to_str().unwrap();
    let elem = ((*xml)._inner as *const XmlElement)
        .as_ref()
        .unwrap()
        .insert_elem(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_mut()
                .unwrap(),
            idx as u32,
            name,
        );

    YXmlElem {
        _inner: Box::into_raw(Box::new(elem)) as *mut c_void,
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxml_insert_text(
    xml: *const YXmlElem,
    txn: *mut YTxn,
    idx: c_int,
) -> YXmlText {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    let text = ((*xml)._inner as *const XmlElement)
        .as_ref()
        .unwrap()
        .insert_text(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_mut()
                .unwrap(),
            idx as u32,
        );
    YXmlText {
        _inner: Box::into_raw(Box::new(text)) as *mut c_void,
    }
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

    ((*xml)._inner as *const XmlElement)
        .as_ref()
        .unwrap()
        .remove_range(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_mut()
                .unwrap(),
            idx as u32,
            len as u32,
        )
}

#[no_mangle]
pub unsafe extern "C" fn yxml_get(
    xml: *const YXmlElem,
    txn: *const YTxn,
    idx: c_int,
) -> *const YVal {
    assert!(!xml.is_null());
    assert!(!txn.is_null());

    if let Some(child) = ((*xml)._inner as *const XmlElement).as_ref().unwrap().get(
        ((*txn)._inner as *const Transaction<'static>)
            .as_ref()
            .unwrap(),
        idx as u32,
    ) {
        let value = match child {
            Xml::Element(v) => yval_from_value(Value::YXmlElement(v)),
            Xml::Text(v) => yval_from_value(Value::YXmlText(v)),
        };
        Box::into_raw(Box::new(value))
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_free(txt: *mut YXmlText) {
    assert!(!txt.is_null());
    let boxed = Box::from_raw((*txt)._inner as *mut XmlText);
    drop(boxed);
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_len(txt: *const YXmlText, txn: *const YTxn) -> c_int {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    ((*txt)._inner as *const XmlText).as_ref().unwrap().len(
        ((*txn)._inner as *mut Transaction<'static>)
            .as_ref()
            .unwrap(),
    ) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn yxmltext_string(txt: *const YXmlText, txn: *const YTxn) -> *const c_char {
    assert!(!txt.is_null());
    assert!(!txn.is_null());

    let str = ((*txt)._inner as *const XmlText)
        .as_ref()
        .unwrap()
        .to_string(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_ref()
                .unwrap(),
        );
    CString::new(str).unwrap().into_raw()
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
    ((*txt)._inner as *const XmlText).as_ref().unwrap().insert(
        ((*txn)._inner as *mut Transaction<'static>)
            .as_mut()
            .unwrap(),
        idx as u32,
        chunk,
    )
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

    ((*txt)._inner as *const XmlText)
        .as_ref()
        .unwrap()
        .remove_range(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_mut()
                .unwrap(),
            idx as u32,
            len as u32,
        )
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

    ((*txt)._inner as *const XmlText)
        .as_ref()
        .unwrap()
        .insert_attribute(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_mut()
                .unwrap(),
            name,
            value,
        )
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
    ((*txt)._inner as *const XmlText)
        .as_ref()
        .unwrap()
        .remove_attribute(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_mut()
                .unwrap(),
            name,
        );
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
    if let Some(value) = ((*txt)._inner as *const XmlText)
        .as_ref()
        .unwrap()
        .get_attribute(
            ((*txn)._inner as *const Transaction<'static>)
                .as_ref()
                .unwrap(),
            name,
        )
    {
        CString::new(value).unwrap().into_raw()
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

    let attrs: Vec<_> = ((*txt)._inner as *const XmlText)
        .as_ref()
        .unwrap()
        .attributes(
            ((*txn)._inner as *mut Transaction<'static>)
                .as_ref()
                .unwrap(),
        )
        .map(|(k, v)| YXmlAttr {
            name: CString::new(k).unwrap().into_raw(),
            value: CString::new(v).unwrap().into_raw(),
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
    } else if (*val).prelim != 0 {
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
pub unsafe extern "C" fn yval_json_array(json_array: *const YVal, len: c_int) -> YVal {
    YVal {
        tag: Y_JSON_ARR,
        prelim: 1,
        len,
        value: YValContent { array: json_array },
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_json_map(json_map: *const YMapEntry, len: c_int) -> YVal {
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
        &*(*val).value.y_array as *const _
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_ymap(val: *const YVal) -> *const YMap {
    if (*val).tag == Y_MAP {
        &*(*val).value.y_map as *const _
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_ytext(val: *const YVal) -> *const YText {
    if (*val).tag == Y_TEXT {
        &*(*val).value.y_text as *const _
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_yxml_text(val: *const YVal) -> *const YXmlText {
    if (*val).tag == Y_XML_TEXT {
        &*(*val).value.y_xml_text as *const _
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn yval_read_yxml_elem(val: *const YVal) -> *const YXmlElem {
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

fn yval_from_any(v: Any) -> YVal {
    match v {
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
            let values: Vec<_> = v.into_iter().map(yval_from_any).collect();
            let array = values.as_ptr();
            let len = values.len() as c_int;
            forget(values);
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
                    let value = Box::into_raw(Box::new(yval_from_any(v)));
                    YMapEntry { key, value }
                })
                .collect();
            let len = entries.len() as c_int;
            let map = entries.as_ptr();
            forget(entries);
            YVal {
                tag: Y_JSON_MAP,
                prelim: 0,
                len,
                value: YValContent { map },
            }
        }
    }
}

fn yval_from_value(v: Value) -> YVal {
    match v {
        Value::Any(v) => yval_from_any(v),
        Value::YText(v) => YVal {
            tag: Y_TEXT,
            prelim: 0,
            len: 1,
            value: YValContent {
                y_text: ManuallyDrop::new(YText {
                    _inner: Box::into_raw(Box::new(v)) as *mut c_void,
                }),
            },
        },
        Value::YArray(v) => YVal {
            tag: Y_ARRAY,
            prelim: 0,
            len: 1,
            value: YValContent {
                y_array: ManuallyDrop::new(YArray {
                    _inner: Box::into_raw(Box::new(v)) as *mut c_void,
                }),
            },
        },
        Value::YMap(v) => YVal {
            tag: Y_MAP,
            prelim: 0,
            len: 1,
            value: YValContent {
                y_map: ManuallyDrop::new(YMap {
                    _inner: Box::into_raw(Box::new(v)) as *mut c_void,
                }),
            },
        },
        Value::YXmlElement(v) => YVal {
            tag: Y_XML_ELEM,
            prelim: 0,
            len: 1,
            value: YValContent {
                y_xml_elem: ManuallyDrop::new(YXmlElem {
                    _inner: Box::into_raw(Box::new(v)) as *mut c_void,
                }),
            },
        },
        Value::YXmlText(v) => YVal {
            tag: Y_XML_TEXT,
            prelim: 0,
            len: 1,
            value: YValContent {
                y_xml_text: ManuallyDrop::new(YXmlText {
                    _inner: Box::into_raw(Box::new(v)) as *mut c_void,
                }),
            },
        },
    }
}
