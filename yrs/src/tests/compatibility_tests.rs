use std::collections::HashMap;
use std::convert::TryInto;
use std::fs::File;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::block::{ClientID, Item, ItemContent};
use crate::branch::Branch;
use crate::encoding::read::Read;
use crate::id_set::{DeleteSet, IdSet};
use crate::store::Store;
use crate::types::xml::XmlFragment;
use crate::types::{ToJson, TypePtr, TypeRef};
use crate::update::{BlockCarrier, Update};
use crate::updates::decoder::{Decode, Decoder, DecoderV1};
use crate::updates::encoder::Encode;
use crate::{
    Any, ArrayPrelim, Doc, GetString, Map, MapPrelim, MapRef, ReadTxn, StateVector, Transact, Xml,
    XmlElementRef, XmlTextRef, ID,
};

#[test]
fn text_insert_delete() {
    /* Generated via:
        ```js
           const doc = new Y.Doc()
           const ytext = doc.getText('type')
           doc..transact_mut()(function () {
               ytext.insert(0, 'def')
               ytext.insert(0, 'abc')
               ytext.insert(6, 'ghi')
               ytext.delete(2, 5)
           })
           const update = Y.encodeStateAsUpdate(doc)
           ytext.toString() // => 'abhi'
        ```
        This way we confirm that we can decode and apply:
        1. blocks without left/right origin consisting of multiple characters
        2. blocks with left/right origin consisting of multiple characters
        3. delete sets
    */
    let update = &[
        1, 5, 152, 234, 173, 126, 0, 1, 1, 4, 116, 121, 112, 101, 3, 68, 152, 234, 173, 126, 0, 2,
        97, 98, 193, 152, 234, 173, 126, 4, 152, 234, 173, 126, 0, 1, 129, 152, 234, 173, 126, 2,
        1, 132, 152, 234, 173, 126, 6, 2, 104, 105, 1, 152, 234, 173, 126, 2, 0, 3, 5, 2,
    ];
    const CLIENT_ID: ClientID = 264992024;
    let expected_blocks = vec![
        Item::new(
            ID::new(CLIENT_ID, 0),
            None,
            None,
            None,
            None,
            TypePtr::Named("type".into()),
            None,
            ItemContent::Deleted(3),
        )
        .unwrap(),
        Item::new(
            ID::new(CLIENT_ID, 3),
            None,
            None,
            None,
            Some(ID::new(CLIENT_ID, 0)),
            TypePtr::Unknown,
            None,
            ItemContent::String("ab".into()),
        )
        .unwrap(),
        Item::new(
            ID::new(CLIENT_ID, 5),
            None,
            Some(ID::new(CLIENT_ID, 4)),
            None,
            Some(ID::new(CLIENT_ID, 0)),
            TypePtr::Unknown,
            None,
            ItemContent::Deleted(1),
        )
        .unwrap(),
        Item::new(
            ID::new(CLIENT_ID, 6),
            None,
            Some(ID::new(CLIENT_ID, 2)),
            None,
            None,
            TypePtr::Unknown,
            None,
            ItemContent::Deleted(1),
        )
        .unwrap(),
        Item::new(
            ID::new(CLIENT_ID, 7),
            None,
            Some(ID::new(CLIENT_ID, 6)),
            None,
            None,
            TypePtr::Unknown,
            None,
            ItemContent::String("hi".into()),
        )
        .unwrap(),
    ];
    let expected_ds = {
        let mut ds = IdSet::new();
        ds.insert(ID::new(CLIENT_ID, 0), 3);
        ds.insert(ID::new(CLIENT_ID, 5), 2);
        DeleteSet::from(ds)
    };
    let visited = Arc::new(AtomicBool::new(false));
    let setter = visited.clone();

    let doc = Doc::new();
    let txt = doc.get_or_insert_text("type");
    let _sub = doc.observe_update_v1(move |_, e| {
        let u = Update::decode_v1(&e.update).unwrap();
        for (actual, expected) in u.blocks.blocks().zip(expected_blocks.as_slice()) {
            if let BlockCarrier::Item(block) = actual {
                assert_eq!(block, expected);
            }
        }
        assert_eq!(u.delete_set, expected_ds);
        setter.store(true, Ordering::Relaxed);
    });
    {
        let mut txn = doc.transact_mut();
        let u = Update::decode_v1(update).unwrap();
        txn.apply_update(u).unwrap();
    }
    assert_eq!(txt.get_string(&doc.transact()), "abhi".to_string());
    assert!(visited.load(Ordering::Relaxed));
}

#[test]
fn map_set() {
    /* Generated via:
        ```js
           const doc = new Y.Doc()
           const x = doc.getMap('test')
           x.set('k1', 'v1')
           x.set('k2', 'v2')
           const payload_v1 = Y.encodeStateAsUpdate(doc)
           console.log(payload_v1);
           const payload_v2 = Y.encodeStateAsUpdateV2(doc)
           console.log(payload_v2);
        ```
    */
    const CLIENT_ID: ClientID = 440166001;
    let expected = vec![
        Item::new(
            ID::new(CLIENT_ID, 0),
            None,
            None,
            None,
            None,
            TypePtr::Named("test".into()),
            Some("k1".into()),
            ItemContent::Any(vec![Any::from("v1")]),
        )
        .unwrap()
        .into(),
        Item::new(
            ID::new(CLIENT_ID, 1),
            None,
            None,
            None,
            None,
            TypePtr::Named("test".into()),
            Some("k2".into()),
            ItemContent::Any(vec![Any::from("v2")]),
        )
        .unwrap()
        .into(),
    ];

    let payload = &[
        1, 2, 241, 204, 241, 209, 1, 0, 40, 1, 4, 116, 101, 115, 116, 2, 107, 49, 1, 119, 2, 118,
        49, 40, 1, 4, 116, 101, 115, 116, 2, 107, 50, 1, 119, 2, 118, 50, 0,
    ];
    roundtrip_v1(payload, &expected);

    let payload = &[
        0, 0, 5, 177, 153, 227, 163, 3, 0, 0, 1, 40, 17, 12, 116, 101, 115, 116, 107, 49, 116, 101,
        115, 116, 107, 50, 4, 2, 4, 2, 1, 1, 0, 2, 65, 0, 1, 2, 0, 119, 2, 118, 49, 119, 2, 118,
        50, 0,
    ];
    roundtrip_v2(payload, &expected);
}

#[test]
fn array_insert() {
    /* Generated via:
        ```js
           const doc = new Y.Doc()
           const x = doc.getArray('test')
           x.push(['a']);
           x.push(['b']);
           const payload_v1 = Y.encodeStateAsUpdate(doc)
           console.log(payload_v1);
           const payload_v2 = Y.encodeStateAsUpdateV2(doc)
           console.log(payload_v2);
        ```
    */
    const CLIENT_ID: ClientID = 2525665872;
    let expected = vec![Item::new(
        ID::new(CLIENT_ID, 0),
        None,
        None,
        None,
        None,
        TypePtr::Named("test".into()),
        None,
        ItemContent::Any(vec![Any::String("a".into()), Any::String("b".into())]),
    )
    .unwrap()
    .into()];

    let payload = &[
        1, 1, 208, 180, 170, 180, 9, 0, 8, 1, 4, 116, 101, 115, 116, 2, 119, 1, 97, 119, 1, 98, 0,
    ];
    roundtrip_v1(payload, &expected);

    let payload = &[
        0, 0, 5, 144, 233, 212, 232, 18, 0, 0, 1, 8, 6, 4, 116, 101, 115, 116, 4, 1, 1, 0, 1, 2, 1,
        1, 0, 119, 1, 97, 119, 1, 98, 0,
    ];
    roundtrip_v2(payload, &expected);
}

#[test]
fn xml_fragment_insert() {
    /* Generated via:
        ```js
           const ydoc = new Y.Doc()
           const yxmlFragment = ydoc.getXmlFragment('fragment-name')
           const yxmlNested = new Y.XmlFragment('fragment-name')
           const yxmlText = new Y.XmlText()
           yxmlFragment.insert(0, [yxmlText])
           yxmlFragment.firstChild === yxmlText
           yxmlFragment.insertAfter(yxmlText, [new Y.XmlElement('node-name')])
           const payload_v1 = Y.encodeStateAsUpdate(ydoc)
           console.log(payload_v1);
           const payload_v2 = Y.encodeStateAsUpdateV2(ydoc)
           console.log(payload_v2);
        ```
    */
    const CLIENT_ID: ClientID = 2459881872;
    let expected = vec![
        Item::new(
            ID::new(CLIENT_ID, 0),
            None,
            None,
            None,
            None,
            TypePtr::Named("fragment-name".into()),
            None,
            ItemContent::Type(Branch::new(TypeRef::XmlText)),
        )
        .unwrap()
        .into(),
        Item::new(
            ID::new(CLIENT_ID, 1),
            None,
            Some(ID::new(CLIENT_ID, 0)),
            None,
            None,
            TypePtr::Unknown,
            None,
            ItemContent::Type(Branch::new(TypeRef::XmlElement("node-name".into()))),
        )
        .unwrap()
        .into(),
    ];

    let payload = &[
        1, 2, 144, 163, 251, 148, 9, 0, 7, 1, 13, 102, 114, 97, 103, 109, 101, 110, 116, 45, 110,
        97, 109, 101, 6, 135, 144, 163, 251, 148, 9, 0, 3, 9, 110, 111, 100, 101, 45, 110, 97, 109,
        101, 0,
    ];
    roundtrip_v1(payload, &expected);

    let payload = &[
        0, 1, 0, 6, 208, 198, 246, 169, 18, 0, 1, 0, 0, 3, 7, 0, 135, 25, 22, 102, 114, 97, 103,
        109, 101, 110, 116, 45, 110, 97, 109, 101, 110, 111, 100, 101, 45, 110, 97, 109, 101, 13,
        9, 1, 1, 2, 6, 3, 0, 1, 2, 0, 0,
    ];
    roundtrip_v2(payload, &expected);
}

#[test]
fn state_vector() {
    /* Generated via:
      ```js
         const a = new Y.Doc()
         const ta = a.getText('test')
         ta.insert(0, 'abc')

         const b = new Y.Doc()
         const tb = b.getText('test')
         tb.insert(0, 'de')

         Y.applyUpdate(a, Y.encodeStateAsUpdate(b))
         console.log(Y.encodeStateVector(a))
      ```
    */
    let payload = &[2, 178, 219, 218, 44, 3, 190, 212, 225, 6, 2];
    let mut expected = StateVector::default();
    expected.inc_by(14182974, 2);
    expected.inc_by(93760946, 3);

    let sv = StateVector::decode_v1(payload).unwrap();
    assert_eq!(sv, expected);

    let serialized = sv.encode_v1();
    assert_eq!(serialized.as_slice(), payload);
}

#[test]
fn utf32_lib0_v2_decoding() {
    let data = &[
        0, 1, 0, 11, 144, 161, 211, 222, 18, 226, 133, 156, 142, 8, 25, 23, 1, 0, 4, 6, 0, 14, 0,
        16, 14, 1, 2, 14, 4, 2, 4, 2, 20, 4, 10, 8, 10, 8, 10, 1, 56, 55, 40, 4, 39, 0, 4, 0, 161,
        0, 0, 0, 167, 0, 4, 0, 167, 0, 4, 0, 167, 0, 4, 0, 7, 0, 1, 0, 0, 0, 40, 3, 71, 0, 1, 0,
        132, 0, 129, 0, 132, 0, 129, 0, 132, 0, 129, 0, 132, 0, 129, 0, 132, 0, 129, 0, 132, 237,
        1, 208, 1, 110, 111, 116, 101, 46, 103, 117, 105, 100, 110, 111, 116, 101, 71, 117, 105,
        100, 110, 111, 116, 101, 46, 111, 119, 110, 101, 114, 111, 119, 110, 101, 114, 110, 111,
        116, 101, 46, 116, 121, 112, 101, 110, 111, 116, 101, 84, 121, 112, 101, 110, 111, 116,
        101, 46, 112, 114, 105, 118, 97, 116, 101, 105, 115, 80, 114, 105, 118, 97, 116, 101, 110,
        111, 116, 101, 46, 99, 114, 101, 97, 116, 101, 84, 105, 109, 101, 99, 114, 101, 97, 116,
        101, 84, 105, 109, 101, 110, 111, 116, 101, 46, 116, 105, 116, 108, 101, 116, 105, 116,
        108, 101, 102, 102, 195, 188, 108, 108, 101, 110, 102, 195, 188, 108, 104, 108, 101, 110,
        102, 195, 188, 104, 108, 101, 110, 112, 114, 111, 115, 101, 109, 105, 114, 114, 111, 114,
        112, 105, 110, 100, 101, 110, 116, 116, 97, 103, 78, 97, 109, 101, 108, 105, 110, 101, 72,
        101, 105, 103, 104, 116, 98, 95, 105, 100, 229, 156, 168, 227, 129, 174, 233, 159, 169,
        229, 155, 189, 240, 159, 135, 176, 240, 159, 135, 183, 240, 159, 135, 168, 240, 159, 135,
        179, 240, 159, 135, 175, 240, 159, 135, 181, 9, 8, 10, 5, 9, 8, 12, 9, 15, 74, 0, 5, 1, 6,
        7, 6, 11, 1, 6, 7, 10, 4, 65, 0, 2, 68, 1, 7, 1, 5, 0, 3, 1, 0, 0, 4, 66, 2, 3, 6, 10, 65,
        4, 2, 65, 4, 66, 0, 10, 69, 1, 2, 5, 0, 119, 22, 66, 71, 108, 122, 109, 85, 106, 50, 84,
        82, 45, 108, 100, 106, 102, 113, 49, 90, 112, 82, 49, 81, 125, 34, 125, 0, 121, 119, 13,
        49, 54, 53, 50, 57, 51, 51, 50, 50, 50, 56, 56, 50, 30, 0, 125, 0, 119, 3, 100, 105, 118,
        119, 0, 119, 11, 74, 88, 98, 65, 83, 97, 45, 97, 57, 50, 106, 1, 226, 130, 142, 135, 4, 8,
        0, 19, 8, 1, 5, 1, 1, 1, 1, 9, 2, 4, 4, 4, 4, 4,
    ];
    let doc = Doc::new();
    let xml = doc.get_or_insert_xml_fragment("prosemirror");
    let mut txn = doc.transact_mut();
    let update = Update::decode_v2(data).unwrap();
    txn.apply_update(update).unwrap();
    let actual: XmlElementRef = xml.get(&txn, 0).unwrap().try_into().unwrap();

    let expected_attrs = HashMap::from([
        ("b_id", "JXbASa-a92j".to_string()),
        ("indent", "0".to_string()),
        ("tagName", "div".to_string()),
        ("lineHeight", "".to_string()),
    ]);
    let actual_attrs: HashMap<&str, String> = actual
        .attributes(&txn)
        .map(|(k, v)| (k, v.to_string(&txn)))
        .collect();
    assert_eq!(actual_attrs, expected_attrs);

    let txt: XmlTextRef = actual.get(&txn, 0).unwrap().try_into().unwrap();

    assert_eq!(txt.get_string(&txn), "在の韩国🇰🇷🇨🇳🇯🇵");
}

/// Verify if given `payload` can be deserialized into series
/// of `expected` blocks, then serialize them back and check
/// if produced binary is equivalent to `payload`.
fn roundtrip_v1(payload: &[u8], expected: &Vec<BlockCarrier>) {
    let u = Update::decode_v1(payload).unwrap();
    let expected: Vec<&BlockCarrier> = expected.iter().collect();
    let blocks: Vec<&BlockCarrier> = u.blocks.blocks().collect();
    assert_eq!(blocks, expected, "failed to decode V1");

    let store: Store = u.into();
    let serialized = store.encode_v1();
    assert_eq!(serialized, payload, "failed to encode V1");
}

/// Same as [roundtrip_v2] but using lib0 v2 encoding.
fn roundtrip_v2(payload: &[u8], expected: &Vec<BlockCarrier>) {
    let u = Update::decode_v2(payload).unwrap();
    let expected: Vec<&BlockCarrier> = expected.iter().collect();
    let blocks: Vec<&BlockCarrier> = u.blocks.blocks().collect();
    assert_eq!(blocks, expected, "failed to decode V2");

    let store: Store = u.into();
    let serialized = store.encode_v2();
    assert_eq!(serialized, payload, "failed to encode V2");
}

#[test]
fn negative_zero_decoding_v2() {
    let doc = Doc::new();
    let root = doc.get_or_insert_map("root");
    let mut txn = doc.transact_mut();

    root.insert(&mut txn, "sequence", MapPrelim::default()); //NOTE: This is how I put nested map.
    let sequence = root
        .get(&txn, "sequence")
        .unwrap()
        .cast::<MapRef>()
        .unwrap();
    sequence.insert(&mut txn, "id", "V9Uk9pxUKZIrW6cOkC0Rg".to_string());
    sequence.insert(&mut txn, "cuts", ArrayPrelim::default());
    sequence.insert(&mut txn, "name", "new sequence".to_string());

    root.insert(&mut txn, "__version__", 1);
    root.insert(&mut txn, "face_expressions", ArrayPrelim::default());
    root.insert(&mut txn, "characters", ArrayPrelim::default());
    let expected = root.to_json(&txn);

    let buffer = txn.encode_state_as_update_v2(&StateVector::default());

    let u = Update::decode_v2(&buffer).unwrap();

    let doc2 = Doc::new();
    let root = doc2.get_or_insert_map("root");
    let mut txn = doc2.transact_mut();
    txn.apply_update(u).unwrap();
    let actual = root.to_json(&txn);

    assert_eq!(actual, expected);
}

#[test]
fn test_small_data_set() {
    test_data_set("../assets/bench-input/small-test-dataset.bin")
}

#[test]
fn test_medium_data_set() {
    test_data_set("../assets/bench-input/medium-test-dataset.bin")
}

fn test_data_set<P: AsRef<std::path::Path>>(path: P) {
    let data = {
        let mut reader = BufReader::new(File::open(path).unwrap());
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut buf).unwrap();
        buf
    };
    let mut decoder = DecoderV1::from(data.as_slice());
    let test_count: u32 = decoder.read_var().unwrap();
    for test_num in 0..test_count {
        let updates_len: u32 = decoder.read_var().unwrap();
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("text");
        let map = doc.get_or_insert_map("map");
        let arr = doc.get_or_insert_array("array");
        for _ in 0..updates_len {
            let update = Update::decode_v1(decoder.read_buf().unwrap()).unwrap();
            doc.transact_mut().apply_update(update).unwrap();
        }
        let expected = decoder.read_string().unwrap();
        assert_eq!(
            txt.get_string(&doc.transact()),
            expected,
            "failed at {} run",
            test_num
        );

        let expected = decoder.read_any().unwrap();
        let actual = map.to_json(&doc.transact());
        assert_eq!(actual, expected, "failed at {} run", test_num);

        let expected = decoder.read_any().unwrap();
        assert_eq!(
            arr.to_json(&doc.transact()),
            expected,
            "failed at {} run",
            test_num
        );
    }
}
