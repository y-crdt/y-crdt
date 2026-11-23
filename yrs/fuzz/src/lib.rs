//! Shared helpers used by yrs fuzz targets.
//!
//! Fuzz targets feed untrusted payloads into decoding and integration routines of yrs and then
//! exercise the resulting document (reading, encoding and editing it). Any panic, abort, timeout
//! or sanitizer report is a bug: malformed input should only ever result in errors.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use yrs::types::text::{Diff, YChange};
use yrs::types::{Attrs, ToJson};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Any, Array, ArrayPrelim, ClientID, Doc, GetString, Map, MapPrelim, Options, Out, Quotable,
    ReadTxn, StateVector, Text, TextPrelim, Transact, Update, Xml, XmlElementPrelim, XmlFragment,
    XmlTextPrelim,
};

/// Names of root-level collections used by seed documents and local edits.
pub const TEXT: &str = "text";
pub const ARRAY: &str = "array";
pub const MAP: &str = "map";
pub const XML: &str = "xml";

pub fn doc_with_client(client_id: u64, skip_gc: bool) -> Doc {
    Doc::with_options(Options {
        client_id: ClientID::new(client_id),
        skip_gc,
        ..Options::default()
    })
}

/// Builds a set of documents covering most of the content types and features of yrs.
/// Returns the final document together with incremental updates (v1 and v2) produced
/// by every transaction.
pub fn seed_doc(skip_gc: bool) -> (Doc, Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let doc = doc_with_client(1, skip_gc);
    let text = doc.get_or_insert_text(TEXT);
    let array = doc.get_or_insert_array(ARRAY);
    let map = doc.get_or_insert_map(MAP);
    let xml = doc.get_or_insert_xml_fragment(XML);
    let mut v1 = Vec::new();
    let mut v2 = Vec::new();
    let mut commit = |txn: yrs::TransactionMut| {
        v1.push(txn.encode_update_v1());
        v2.push(txn.encode_update_v2());
    };

    let mut txn = doc.transact_mut();
    text.insert(&mut txn, 0, "hello 😀 world ąę");
    let bold: Attrs = HashMap::from([(Arc::from("bold"), Any::Bool(true))]);
    text.format(&mut txn, 0, 5, bold);
    commit(txn);

    let mut txn = doc.transact_mut();
    text.insert_embed(
        &mut txn,
        3,
        Any::from(HashMap::from([("img".to_string(), Any::from("x"))])),
    );
    text.remove_range(&mut txn, 1, 2);
    commit(txn);

    let mut txn = doc.transact_mut();
    map.insert(&mut txn, "k1", 1);
    map.insert(&mut txn, "k2", "v");
    map.insert(&mut txn, "k3", vec![1u8, 2, 3]);
    let nested = map.insert(&mut txn, "nested", MapPrelim::from([("a", 1)]));
    nested.insert(&mut txn, "b", Any::Null);
    map.insert(&mut txn, "sub", Doc::new());
    commit(txn);

    let mut txn = doc.transact_mut();
    array.insert_range(&mut txn, 0, [1, 2, 3, 4, 5]);
    array.insert(&mut txn, 2, ArrayPrelim::from([true, false]));
    array.insert(&mut txn, 1, TextPrelim::new("abc"));
    commit(txn);

    let mut txn = doc.transact_mut();
    array.remove(&mut txn, 3);
    map.remove(&mut txn, "k1");
    map.insert(&mut txn, "k2", "overwritten");
    commit(txn);

    let mut txn = doc.transact_mut();
    let div = xml.insert(&mut txn, 0, XmlElementPrelim::empty("div"));
    div.insert_attribute(&mut txn, "class", "x");
    let inner = div.insert(&mut txn, 0, XmlTextPrelim::new("inner"));
    inner.insert(&mut txn, 2, "zz");
    commit(txn);

    let mut txn = doc.transact_mut();
    if let Ok(quote) = text.quote(&txn, 1..4) {
        array.insert(&mut txn, 0, quote);
    }
    if let Ok(quote) = array.quote(&txn, 1..3) {
        map.insert(&mut txn, "link", quote);
    }
    commit(txn);

    // concurrent edits of another client
    let other = doc_with_client(2, skip_gc);
    {
        let state = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let mut txn = other.transact_mut();
        let _ = txn.apply_update(Update::decode_v1(&state).unwrap());
    }
    let text2 = other.get_or_insert_text(TEXT);
    let array2 = other.get_or_insert_array(ARRAY);
    let mut txn = other.transact_mut();
    text2.insert(&mut txn, 4, "XYZ");
    text2.remove_range(&mut txn, 0, 1);
    array2.insert(&mut txn, 0, "first");
    v1.push(txn.encode_update_v1());
    v2.push(txn.encode_update_v2());
    drop(txn);
    {
        let state = other
            .transact()
            .encode_state_as_update_v1(&doc.transact().state_vector());
        let mut txn = doc.transact_mut();
        let _ = txn.apply_update(Update::decode_v1(&state).unwrap());
    }

    (doc, v1, v2)
}

/// Reads entire document state and encodes it, touching most of the read paths.
pub fn exercise(doc: &Doc) {
    let txn = doc.transact();
    let roots: Vec<_> = txn.root_refs().map(|(_, value)| value).collect();
    for value in roots {
        let _ = value.clone().to_string(&txn);
        match value {
            Out::YText(text) => {
                let _ = text.get_string(&txn);
                let _: Vec<Diff<YChange>> = text.diff(&txn, YChange::identity);
                let _ = text.len(&txn);
            }
            Out::YArray(array) => {
                let _ = array.to_json(&txn);
                let _ = array.len(&txn);
                let _: Vec<_> = array.iter(&txn).collect();
            }
            Out::YMap(map) => {
                let _ = map.to_json(&txn);
                let _: Vec<_> = map.iter(&txn).collect();
            }
            Out::YXmlFragment(xml) => {
                let _ = xml.get_string(&txn);
            }
            Out::YXmlElement(xml) => {
                let _ = xml.get_string(&txn);
            }
            Out::YXmlText(xml) => {
                let _ = xml.get_string(&txn);
            }
            _ => {}
        }
    }
    let _ = txn.encode_state_as_update_v1(&StateVector::default());
    let _ = txn.encode_state_as_update_v2(&StateVector::default());
    let _ = txn.snapshot().encode_v1();
    let sv = txn.state_vector();
    let _ = txn.encode_diff_v1(&sv);
}

/// Performs local edits at valid positions of root-level collections.
pub fn local_edits(doc: &Doc) {
    let text = doc.get_or_insert_text(TEXT);
    let array = doc.get_or_insert_array(ARRAY);
    let map = doc.get_or_insert_map(MAP);
    let xml = doc.get_or_insert_xml_fragment(XML);
    let mut txn = doc.transact_mut();
    let len = text.len(&txn);
    text.insert(&mut txn, len / 2, "Z");
    if len > 2 {
        text.remove_range(&mut txn, 0, 1);
    }
    let len = array.len(&txn);
    array.insert(&mut txn, len / 2, 1);
    if len > 1 {
        array.remove(&mut txn, 0);
    }
    map.insert(&mut txn, "k2", 5);
    let len = xml.len(&txn);
    xml.insert(&mut txn, len / 2, XmlTextPrelim::new("t"));
}

/// Encoded seed document state, computed once per process.
pub struct Seeds {
    /// Full state of a seed document (GC enabled), v1 encoding.
    pub state_v1: Vec<u8>,
    /// Full state of a seed document (GC disabled), v1 encoding.
    pub state_no_gc_v1: Vec<u8>,
    /// State vector of a seed document, v1 encoding.
    pub state_vector_v1: Vec<u8>,
    /// Incremental updates produced while building the seed document.
    pub updates_v1: Vec<Vec<u8>>,
    pub updates_v2: Vec<Vec<u8>>,
}

pub fn seeds() -> &'static Seeds {
    static SEEDS: OnceLock<Seeds> = OnceLock::new();
    SEEDS.get_or_init(|| {
        let (doc, updates_v1, updates_v2) = seed_doc(false);
        let (no_gc, _, _) = seed_doc(true);
        let state_no_gc_v1 = no_gc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let txn = doc.transact();
        Seeds {
            state_v1: txn.encode_state_as_update_v1(&StateVector::default()),
            state_no_gc_v1,
            state_vector_v1: txn.state_vector().encode_v1(),
            updates_v1,
            updates_v2,
        }
    })
}

/// Creates a document from a v1-encoded state.
pub fn load_doc(state_v1: &[u8], skip_gc: bool) -> Doc {
    let doc = doc_with_client(100, skip_gc);
    {
        let mut txn = doc.transact_mut();
        let _ = txn.apply_update(Update::decode_v1(state_v1).unwrap());
    }
    doc
}

/// Applies an update to a fresh document and to a document with existing state, then reads,
/// encodes and edits both of them.
pub fn apply_update(decode: impl Fn() -> Option<Update>) {
    if let Some(update) = decode() {
        let _ = update.state_vector();
        let doc = doc_with_client(100, false);
        let _ = doc.transact_mut().apply_update(update);
        exercise(&doc);
        local_edits(&doc);
        exercise(&doc);
    }
    if let Some(update) = decode() {
        let doc = load_doc(&seeds().state_v1, false);
        let _ = doc.transact_mut().apply_update(update);
        exercise(&doc);
        local_edits(&doc);
        exercise(&doc);
    }
}
