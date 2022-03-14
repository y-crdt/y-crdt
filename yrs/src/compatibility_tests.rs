use crate::block::{Item, ItemContent};
use crate::id_set::{DeleteSet, IdSet};
use crate::store::Store;
use crate::types::{Branch, TypePtr, TYPE_REFS_XML_ELEMENT, TYPE_REFS_XML_TEXT};
use crate::update::{BlockCarrier, Update};
use crate::updates::decoder::Decode;
use crate::updates::encoder::Encode;
use crate::{Doc, StateVector, ID};
use lib0::any::Any;
use std::cell::Cell;
use std::rc::Rc;

#[test]
fn text_insert_delete() {
    /* Generated via:
        ```js
           const doc = new Y.Doc()
           const ytext = doc.getText('type')
           doc.transact(function () {
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
    const CLIENT_ID: u64 = 264992024;
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
        ),
        Item::new(
            ID::new(CLIENT_ID, 3),
            None,
            None,
            None,
            Some(ID::new(CLIENT_ID, 0)),
            TypePtr::Unknown,
            None,
            ItemContent::String("ab".into()),
        ),
        Item::new(
            ID::new(CLIENT_ID, 5),
            None,
            Some(ID::new(CLIENT_ID, 4)),
            None,
            Some(ID::new(CLIENT_ID, 0)),
            TypePtr::Unknown,
            None,
            ItemContent::Deleted(1),
        ),
        Item::new(
            ID::new(CLIENT_ID, 6),
            None,
            Some(ID::new(CLIENT_ID, 2)),
            None,
            None,
            TypePtr::Unknown,
            None,
            ItemContent::Deleted(1),
        ),
        Item::new(
            ID::new(CLIENT_ID, 7),
            None,
            Some(ID::new(CLIENT_ID, 6)),
            None,
            None,
            TypePtr::Unknown,
            None,
            ItemContent::String("hi".into()),
        ),
    ];
    let expected_ds = {
        let mut ds = IdSet::new();
        ds.insert(ID::new(CLIENT_ID, 0), 3);
        ds.insert(ID::new(CLIENT_ID, 5), 2);
        DeleteSet::from(ds)
    };
    let visited = Rc::new(Cell::new(false));
    let setter = visited.clone();

    let mut doc = Doc::new();
    let _sub = doc.on_update(move |_, e| {
        for (actual, expected) in e.update.blocks.blocks().zip(expected_blocks.as_slice()) {
            if let BlockCarrier::Block(block) = actual {
                assert_eq!(block, expected);
            }
        }
        assert_eq!(&e.update.delete_set, &expected_ds);
        setter.set(true);
    });
    let mut txn = doc.transact();
    let txt = txn.get_text("type");
    txn.apply_update(Update::decode_v1(update));
    assert_eq!(txt.to_string(), "abhi".to_string());
    assert!(visited.get());
}

#[test]
fn map_set() {
    /* Generated via:
        ```js
           const doc = new Y.Doc()
           const x = doc.getMap('test')
           x.set('k1', 'v1')
           x.set('k2', 'v2')
           const update = Y.encodeStateAsUpdate(doc)
           console.log(update);
        ```
    */
    let payload = &[
        1, 2, 183, 229, 212, 163, 3, 0, 40, 1, 4, 116, 101, 115, 116, 2, 107, 49, 1, 119, 2, 118,
        49, 40, 1, 4, 116, 101, 115, 116, 2, 107, 50, 1, 119, 2, 118, 50, 0,
    ];
    const CLIENT_ID: u64 = 880095927;
    let expected = vec![
        Item::new(
            ID::new(CLIENT_ID, 0),
            None,
            None,
            None,
            None,
            TypePtr::Named("test".into()),
            Some("k1".into()),
            ItemContent::Any(vec![Any::String("v1".into())]),
        )
        .into(),
        Item::new(
            ID::new(CLIENT_ID, 1),
            None,
            None,
            None,
            None,
            TypePtr::Named("test".into()),
            Some("k2".into()),
            ItemContent::Any(vec![Any::String("v2".into())]),
        )
        .into(),
    ];

    roundtrip(payload, expected);
}

#[test]
fn array_insert() {
    /* Generated via:
        ```js
           const doc = new Y.Doc()
           const x = doc.getArray('test')
           x.push(['a']);
           x.push(['b']);
           const update = Y.encodeStateAsUpdate(doc)
           console.log(update);
        ```
    */
    let payload = &[
        1, 1, 199, 195, 202, 51, 0, 8, 1, 4, 116, 101, 115, 116, 2, 119, 1, 97, 119, 1, 98, 0,
    ];
    const CLIENT_ID: u64 = 108175815;
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
    .into()];

    roundtrip(payload, expected);
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
        ```
    */
    let payload = &[
        1, 2, 219, 173, 215, 246, 1, 0, 7, 1, 13, 102, 114, 97, 103, 109, 101, 110, 116, 45, 110,
        97, 109, 101, 6, 135, 219, 173, 215, 246, 1, 0, 3, 9, 110, 111, 100, 101, 45, 110, 97, 109,
        101, 0,
    ];
    const CLIENT_ID: u64 = 517330651;
    let expected = vec![
        Item::new(
            ID::new(CLIENT_ID, 0),
            None,
            None,
            None,
            None,
            TypePtr::Named("fragment-name".into()),
            None,
            ItemContent::Type(Branch::new(TYPE_REFS_XML_TEXT, None)),
        )
        .into(),
        Item::new(
            ID::new(CLIENT_ID, 1),
            None,
            Some(ID::new(CLIENT_ID, 0)),
            None,
            None,
            TypePtr::Unknown,
            None,
            ItemContent::Type(Branch::new(
                TYPE_REFS_XML_ELEMENT,
                Some("node-name".to_string()),
            )),
        )
        .into(),
    ];

    roundtrip(payload, expected);
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

    let sv = StateVector::decode_v1(payload);
    assert_eq!(sv, expected);

    let serialized = sv.encode_v1();
    assert_eq!(serialized.as_slice(), payload);
}

/// Verify if given `payload` can be deserialized into series
/// of `expected` blocks, then serialize them back and check
/// if produced binary is equivalent to `payload`.
fn roundtrip(payload: &[u8], expected: Vec<BlockCarrier>) {
    let u = Update::decode_v1(payload);
    let expected: Vec<&BlockCarrier> = expected.iter().collect();
    let blocks: Vec<&BlockCarrier> = u.blocks.blocks().collect();
    assert_eq!(blocks, expected);

    let store: Store = u.into();
    let serialized = store.encode_v1();
    assert_eq!(serialized, payload);
}
