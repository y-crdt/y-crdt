//! ContentJSON (content ref 2) must read back exactly as it is written: a count, then that many JSON strings,
//! as in Yjs (`src/structs/ContentJSON.js`). JSON content is legacy (Yjs reads it, nothing writes it any more),
//! but old documents hold it.
use yrs::block::{ItemContent, Prelim, Unused};
use yrs::branch::BranchPtr;
use yrs::updates::decoder::Decode;
use yrs::{Array, Doc, ReadTxn, StateVector, Transact, TransactionMut, Update};

/// Inserts raw content, as an old Yjs client would have written it.
struct Raw(ItemContent);
impl Prelim for Raw {
    type Return = Unused;
    fn into_content(self, _: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        (self.0, None)
    }
    fn integrate(self, _: &mut TransactionMut, _: BranchPtr) {}
}

fn json_doc() -> Vec<u8> {
    let doc = Doc::with_client_id(1);
    let arr = doc.get_or_insert_array("a");
    let mut txn = doc.transact_mut();
    arr.insert(&mut txn, 0, Raw(ItemContent::JSON(vec!["1".into(), "\"x\"".into()])));
    arr.insert(&mut txn, 1, "after"); // an item after it, which a miscounted read would eat
    txn.encode_state_as_update_v1(&StateVector::default())
}

#[test]
fn content_json_round_trips() {
    let bytes = json_doc();
    let update = Update::decode_v1(&bytes).expect("yrs reads the JSON content it wrote");
    let doc = Doc::new();
    doc.transact_mut().apply_update(update).unwrap();
    let arr = doc.get_or_insert_array("a");
    let txn = doc.transact();
    assert_eq!(arr.len(&txn), 3);
    assert_eq!(txn.encode_state_as_update_v1(&StateVector::default()), bytes);
}

/// What Yjs 13.6 writes for `new ContentJSON([1, 'x'])` in array "a" (client 1).
const FROM_YJS: [u8; 16] = [1, 1, 1, 0, 2, 1, 1, b'a', 2, 1, b'1', 3, b'"', b'x', b'"', 0];

#[test]
fn content_json_from_yjs() {
    let update = Update::decode_v1(&FROM_YJS).expect("yrs reads Yjs's JSON content");
    let doc = Doc::new();
    let arr = doc.get_or_insert_array("a");
    doc.transact_mut().apply_update(update).unwrap();
    let txn = doc.transact();
    assert_eq!(arr.len(&txn), 2);
    assert_eq!(txn.encode_state_as_update_v1(&StateVector::default()), FROM_YJS);
}

#[test]
fn content_json_count_is_not_trusted() {
    // a count far past the data: an error, not a huge reservation or a panic
    let bytes = [1, 1, 1, 0, 2, 1, 1, b'a', 0xff, 0xff, 0xff, 0xff, 0x0f, 1, b'1', 0];
    assert!(Update::decode_v1(&bytes).is_err());
}
