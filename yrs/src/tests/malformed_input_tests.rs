//! Regression tests for decoding and applying malformed (potentially malicious) payloads.
//! None of these inputs should cause a panic, abort or undefined behaviour - they should either
//! be rejected with an error or be handled gracefully.

use crate::encoding::read::{Cursor, Error};
use crate::types::text::{Diff, YChange};
use crate::types::ToJson;
use crate::updates::decoder::{Decode, DecoderV1};
use crate::updates::encoder::{Encoder, EncoderV1};
use crate::{
    Any, Array, Doc, GetString, IdSet, Options, ReadTxn, Snapshot, StateVector, Text, Transact,
    Update,
};

#[test]
fn deeply_nested_any_is_rejected() {
    let mut input = Vec::new();
    for _ in 0..100_000 {
        input.push(117); // array
        input.push(1); // of length 1
    }
    input.push(126); // null
    assert!(Any::decode(&mut DecoderV1::new(Cursor::new(&input))).is_err());
}

#[test]
fn moderately_nested_any_is_accepted() {
    let mut value = Any::Null;
    for _ in 0..100 {
        value = Any::from(vec![value]);
    }
    let mut encoder = EncoderV1::new();
    encoder.write_any(&value);
    let bin = encoder.to_vec();
    let decoded = Any::decode(&mut DecoderV1::new(Cursor::new(&bin))).unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn delete_set_range_overflow() {
    // 1 client, client 0, 1 range: clock u32::MAX, len 2
    let input = [1, 0, 1, 0xff, 0xff, 0xff, 0xff, 0x0f, 2];
    assert!(IdSet::decode_v1(&input).is_err());
}

#[test]
fn json_content_reads_declared_number_of_elements() {
    // 1 client, 1 block, client 1, clock 0, info: JSON content (no origins),
    // parent: root type "a", content: 2 JSON strings: "1" and "2", empty delete set
    let input = [1u8, 1, 1, 0, 0x02, 1, 1, b'a', 2, 1, b'1', 1, b'2', 0];
    let update = Update::decode_v1(&input).unwrap();
    let doc = Doc::new();
    let array = doc.get_or_insert_array("a");
    doc.transact_mut().apply_update(update).ok();
    assert_eq!(array.len(&doc.transact()), 2);
}

fn snapshot_doc() -> Doc {
    let doc = Doc::with_options(Options {
        client_id: crate::ClientID::new(1),
        skip_gc: true,
        ..Options::default()
    });
    let txt = doc.get_or_insert_text("text");
    txt.insert(&mut doc.transact_mut(), 0, "hello world");
    txt.remove_range(&mut doc.transact_mut(), 2, 3);
    doc
}

#[test]
fn snapshot_with_zero_clock() {
    let doc = snapshot_doc();
    // empty delete set, state vector: { 1: 0 }
    let snapshot = Snapshot::decode_v1(&[0, 1, 1, 0]).unwrap();
    let mut encoder = EncoderV1::new();
    doc.transact()
        .encode_state_from_snapshot(&snapshot, &mut encoder)
        .unwrap();
}

#[test]
fn snapshot_with_unknown_client_in_delete_set() {
    let doc = snapshot_doc();
    // delete set: { 33: [1..3) }, state vector: { 1: 20 }
    let snapshot = Snapshot::decode_v1(&[1, 33, 1, 1, 2, 1, 1, 20]).unwrap();
    let mut encoder = EncoderV1::new();
    doc.transact()
        .encode_state_from_snapshot(&snapshot, &mut encoder)
        .unwrap();
    let txt = doc.get_or_insert_text("text");
    let mut txn = doc.transact_mut();
    let _: Vec<Diff<YChange>> = txt.diff_range(&mut txn, Some(&snapshot), None, YChange::identity);
}

#[test]
fn block_next_to_gc_is_garbage_collected() {
    let doc = Doc::with_client_id(1);
    let array = doc.get_or_insert_array("arr");
    array.insert(&mut doc.transact_mut(), 0, 1);
    // client 2: GC block (2#0), followed by an item (2#1) with left origin 1#0 (array element)
    // and right origin 2#0 (GC block). Same as in Yjs, such item must be garbage collected
    // instead of being integrated using parent of its left neighbour.
    let input = [1u8, 2, 2, 0, 0, 1, 0xC8, 1, 0, 2, 0, 1, 125, 5, 0];
    let update = Update::decode_v1(&input).unwrap();
    doc.transact_mut().apply_update(update).ok();
    assert_eq!(
        array.to_json(&doc.transact()),
        Any::from(vec![Any::from(1)])
    );
}

/// V2 update describing ~2^32 GC blocks in 27 bytes: info column repeats GC forever and length
/// column repeats `1` for ~2^32 times.
const V2_GC_BOMB: [u8; 27] = [
    0, // feature flag
    0, // key clock column: empty
    1, 1, // client column: client 1
    0, // left clock column: empty
    0, // right clock column: empty
    1, 0, // info column: 0 (GC) repeated forever
    1, 0, // string column: no strings
    0, // parent info column: empty
    0, // type ref column: empty
    6, 0x41, 0xfd, 0xff, 0xff, 0xff, 0x0f, // length column: 1 repeated ~2^32 times
    1, 0xff, 0xff, 0xff, 0xff, 0x0f, 0, // 1 client, ~2^32 blocks, starting at clock 0
    0, // empty delete set
];

#[test]
fn v2_run_length_encoded_blocks_are_limited() {
    assert!(matches!(
        Update::decode_v2(&V2_GC_BOMB),
        Err(Error::Custom(_))
    ));
    assert!(crate::merge_updates_v2([V2_GC_BOMB.as_slice()]).is_err());
}

#[test]
fn v2_run_length_encoded_json_elements_are_limited() {
    // single block with JSON content of ~2^32 empty strings
    let input = [
        0, // feature flag
        0, // key clock column: empty
        1, 1, // client column: client 1
        0, // left clock column: empty
        0, // right clock column: empty
        1, 2, // info column: JSON content without origins
        7, 0, 0x40, 0xfd, 0xff, 0xff, 0xff, 0x0f, // string column: empty strings ~2^32 times
        1, 1, // parent info column: named root type
        0, // type ref column: empty
        5, 0xbe, 0xff, 0xff, 0xff, 0x1f, // length column: 2^32 - 2
        1, 1, 0, // 1 client, 1 block, starting at clock 0
        0, // empty delete set
    ];
    assert!(matches!(Update::decode_v2(&input), Err(Error::Custom(_))));
}

#[test]
fn v2_many_empty_shared_types_within_limits() {
    // run-length encoding legitimately describes many consecutive empty shared types
    // using a very small payload
    let doc = Doc::with_client_id(1);
    let array = doc.get_or_insert_array("array");
    {
        let mut txn = doc.transact_mut();
        for i in 0..20_000 {
            array.insert(&mut txn, i, crate::MapPrelim::default());
        }
    }
    let update = doc
        .transact()
        .encode_state_as_update_v2(&StateVector::default());
    assert!(update.len() < 100);

    let doc2 = Doc::new();
    let array2 = doc2.get_or_insert_array("array");
    doc2.transact_mut()
        .apply_update(Update::decode_v2(&update).unwrap())
        .ok();
    assert_eq!(array2.len(&doc2.transact()), 20_000);

    // the limit can be adjusted
    use crate::updates::decoder::DecoderV2;
    let mut decoder = DecoderV2::new(Cursor::new(&update))
        .unwrap()
        .with_element_limit(1000);
    assert!(Update::decode(&mut decoder).is_err());
    let mut decoder = DecoderV2::new(Cursor::new(&V2_GC_BOMB))
        .unwrap()
        .with_element_limit(1000);
    assert!(Update::decode(&mut decoder).is_err());
}

#[test]
fn block_clock_overflow_is_rejected() {
    let doc = Doc::with_client_id(1);
    let txt = doc.get_or_insert_text("t");
    txt.insert(&mut doc.transact_mut(), 0, "abc");
    // client 1: GC block at clock 2^32-2 of length 2, followed by a string block, which clock
    // overflows to 0 - overlapping with already integrated "abc". Such block used to replace
    // (and free) integrated item still referenced by the document (use-after-free).
    let input = [
        1u8, 2, 1, 0xfe, 0xff, 0xff, 0xff, 0x0f, // 1 client, 2 blocks, client 1, clock 2^32-2
        0, 2, // GC block of length 2
        4, 1, 1, b't', 1, b'x', // string "x" in root type "t"
        0,    // empty delete set
    ];
    assert!(Update::decode_v1(&input).is_err());
    assert_eq!(txt.get_string(&doc.transact()), "abc");
}

#[test]
fn pending_update_does_not_poison_document() {
    // fuzzer-generated update with JSON content: when it couldn't be integrated it was stored
    // as pending, and every subsequent attempt to encode document state panicked
    let input = [
        1u8, 2, 10, 0, 196, 89, 74, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 29, 1,
    ];
    let doc = Doc::with_client_id(100);
    if let Ok(update) = Update::decode_v1(&input) {
        doc.transact_mut().apply_update(update).ok();
    }
    let txn = doc.transact();
    let _ = txn.encode_state_as_update_v1(&StateVector::default());
    let _ = txn.encode_state_as_update_v2(&StateVector::default());
}
