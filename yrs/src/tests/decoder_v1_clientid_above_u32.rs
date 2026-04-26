//! Regression gates for issue #609 — `DecoderV1::read_client` and
//! `read_id` silently truncated clientIDs > 2^32 to u32 via
//! `read_var_u32`'s `wrapping_shl`. The encoder side and the wire
//! format both supported the full `ClientID = u64` range; only the V1
//! decoder dropped the upper bits. `DecoderV2` was always correct.
//!
//! These tests fail on the pre-fix DecoderV1 (clientID `0x19266f5beab768`
//! decodes as `0x5bfbb768` — lower 32 bits XOR-rotated) and pass on
//! the fixed DecoderV1.

use crate::updates::decoder::Decode;
use crate::{Doc, GetString, Options, ReadTxn, StateVector, Text, Transact, Update};

/// `DecoderV1::read_client` site — the per-client header in the v1
/// update binary.
#[test]
fn decoderv1_read_client_round_trips_clientids_above_u32() {
    // 0x19266f5beab768 ≈ 7×10^15 — fits in 53 bits (the JS-yjs safe range)
    // but well above 2^32 so the bug truncates it.
    let cid: u64 = 7_079_134_143_100_776;
    assert!(cid > u64::from(u32::MAX));

    let doc = Doc::with_options(Options {
        client_id: cid,
        skip_gc: false,
        ..Default::default()
    });
    let text = doc.get_or_insert_text("content");
    {
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "x");
    }
    let bytes = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());

    let update = Update::decode_v1(&bytes).expect("decode_v1 must succeed on self-encoded update");
    let decoded: Vec<u64> = update.state_vector().iter().map(|(c, _)| *c).collect();
    assert_eq!(
        decoded,
        vec![cid],
        "DecoderV1::read_client must round-trip clientIDs > 2^32 losslessly. \
         Encoded {cid:#x}, decoded {decoded:?}.",
    );
}

/// `DecoderV1::read_id` site — block-origin (`left_id` / `right_id`)
/// references. More dangerous than `read_client`: corrupts CRDT
/// ordering, not just attribution metadata. Construct a multi-writer
/// merge update where the second writer's op has a left-origin
/// pointing at the first writer's > 2^32 clientID; assert the post-
/// apply doc's state_vector contains BOTH original clientIDs un-
/// truncated.
#[test]
fn decoderv1_read_id_round_trips_block_origin_clientids_above_u32() {
    let cid_a: u64 = 7_079_134_143_100_776; // 0x19266f5beab768
    let cid_b: u64 = 6_332_093_782_882_047; // 0x167f01789cf6ff (distinct, > u32::MAX)
    assert!(cid_a > u64::from(u32::MAX) && cid_b > u64::from(u32::MAX));
    assert_ne!(cid_a, cid_b);

    // Step 1: doc_a inserts "a".
    let doc_a = Doc::with_options(Options {
        client_id: cid_a,
        skip_gc: false,
        ..Default::default()
    });
    let text_a = doc_a.get_or_insert_text("content");
    {
        let mut txn = doc_a.transact_mut();
        text_a.insert(&mut txn, 0, "a");
    }
    let doc_a_state = doc_a
        .transact()
        .encode_state_as_update_v1(&StateVector::default());

    // Step 2: doc_b applies doc_a's state, then inserts "b" at offset 1
    // (left-origin = "a"'s ID, which carries cid_a).
    let doc_b = Doc::with_options(Options {
        client_id: cid_b,
        skip_gc: false,
        ..Default::default()
    });
    let text_b = doc_b.get_or_insert_text("content");
    {
        let mut txn = doc_b.transact_mut();
        let update_a = Update::decode_v1(&doc_a_state).expect("decode doc_a state");
        txn.apply_update(update_a).expect("apply doc_a state in doc_b");
    }
    {
        let mut txn = doc_b.transact_mut();
        text_b.insert(&mut txn, 1, "b");
    }
    let doc_b_state = doc_b
        .transact()
        .encode_state_as_update_v1(&StateVector::default());

    // Step 3: doc_c (fresh, distinct < u32 clientID) applies doc_b's state.
    let doc_c = Doc::with_options(Options {
        client_id: 0x0011_0011, // < u32::MAX, sentinel
        skip_gc: false,
        ..Default::default()
    });
    let _text_c = doc_c.get_or_insert_text("content");
    {
        let mut txn = doc_c.transact_mut();
        let update_b = Update::decode_v1(&doc_b_state).expect("decode doc_b state");
        txn.apply_update(update_b).expect("apply doc_b state in doc_c");
    }

    // Both writer clientIDs (cid_a, cid_b) MUST appear un-truncated
    // in doc_c's post-apply state_vector. Drop the local mirror
    // clientID (artifact of get_or_insert_text local write).
    let txn_c = doc_c.transact();
    let sv = txn_c.state_vector();
    let mut clients: std::collections::BTreeSet<u64> = sv
        .iter()
        .map(|(c, _)| *c)
        .collect::<std::collections::BTreeSet<u64>>();
    clients.remove(&0x0011_0011);

    let expected: std::collections::BTreeSet<u64> =
        std::array::IntoIter::new([cid_a, cid_b]).collect();
    assert_eq!(
        clients, expected,
        "DecoderV1::read_id must round-trip block-origin clientIDs > 2^32 losslessly. \
         Expected {{cid_a={cid_a:#x}, cid_b={cid_b:#x}}}, got {clients:?}.",
    );

    // Sanity: the merge actually happened (both characters present).
    let text_c = txn_c
        .get_text("content")
        .expect("doc_c content text root must exist post-apply")
        .get_string(&txn_c);
    assert!(
        text_c.contains('a') && text_c.contains('b') && text_c.len() == 2,
        "merged text malformed: {text_c:?}",
    );
}
