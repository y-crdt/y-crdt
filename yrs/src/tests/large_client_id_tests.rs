use crate::block::ClientID;
use crate::update::Update;
use crate::updates::decoder::Decode;
use crate::{ArrayPrelim, Doc, GetString, MapPrelim, ReadTxn, StateVector, Text, Transact, Array, Map};

/// Test that DecoderV1 correctly handles large client IDs (> u32::MAX)
///
/// This is a regression test for the issue where yjs documents with large client IDs
/// (up to JavaScript's MAX_SAFE_INTEGER, ~53 bits) would be incorrectly decoded by yrs.
///
/// The issue was that DecoderV1::read_id(), DecoderV1::read_client() and IdSet::decode()
/// were reading client IDs as u32, causing truncation for large values.
#[test]
fn large_client_id_decoding_v1() {
    // Large client ID that exceeds u32::MAX
    // This value is from a real yjs document
    let large_client_id: ClientID = 4624497851153597;
    assert!(large_client_id > u32::MAX as u64, "Test requires a client ID > u32::MAX");

    // Create a document with the large client ID
    let doc1 = Doc::with_options(crate::Options {
        client_id: large_client_id,
        ..Default::default()
    });

    let txt = doc1.get_or_insert_text("test");
    {
        let mut txn = doc1.transact_mut();
        txt.insert(&mut txn, 0, "hello");
    }

    // Encode as v1 update
    let update_v1 = doc1.transact().encode_state_as_update_v1(&StateVector::default());

    // Decode the v1 update - this is where the bug manifested
    let decoded_update = Update::decode_v1(&update_v1).expect("Failed to decode v1 update");

    // Verify the state vector contains the correct large client ID
    let sv = decoded_update.state_vector();
    assert!(
        sv.get(&large_client_id) > 0,
        "State vector should contain the large client ID {}, got: {:?}",
        large_client_id,
        sv
    );

    // Apply to a new document and verify it works correctly
    let doc2 = Doc::new();
    {
        let mut txn = doc2.transact_mut();
        txn.apply_update(decoded_update).expect("Failed to apply update");
    }

    // Verify the text was correctly applied
    let txt2 = doc2.get_or_insert_text("test");
    assert_eq!(txt2.get_string(&doc2.transact()), "hello");

    // Verify the document's state vector contains the large client ID
    let sv2 = doc2.transact().state_vector();
    assert!(
        sv2.get(&large_client_id) > 0,
        "Applied document's state vector should contain the large client ID"
    );
}

/// Test that IdSet correctly handles large client IDs when decoding delete sets
#[test]
fn large_client_id_in_delete_set() {
    let large_client_id: ClientID = 2877773755261346;
    assert!(large_client_id > u32::MAX as u64, "Test requires a client ID > u32::MAX");

    let doc1 = Doc::with_options(crate::Options {
        client_id: large_client_id,
        ..Default::default()
    });

    let txt = doc1.get_or_insert_text("test");
    {
        let mut txn = doc1.transact_mut();
        txt.insert(&mut txn, 0, "hello world");
        // Delete some content to create entries in the delete set
        txt.remove_range(&mut txn, 0, 5); // Delete "hello"
    }

    // Encode as v1 update (includes delete set with large client ID)
    let update_v1 = doc1.transact().encode_state_as_update_v1(&StateVector::default());

    // Decode and verify
    let decoded_update = Update::decode_v1(&update_v1).expect("Failed to decode");

    // Apply to new document
    let doc2 = Doc::new();
    {
        let mut txn = doc2.transact_mut();
        txn.apply_update(decoded_update).expect("Failed to apply");
    }

    // Verify the result - should have " world" after deletion
    let txt2 = doc2.get_or_insert_text("test");
    assert_eq!(txt2.get_string(&doc2.transact()), " world");
}

/// Test incremental updates between documents with large client IDs
#[test]
fn large_client_id_incremental_update() {
    let large_client_id: ClientID = 368355040673452296;
    assert!(large_client_id > u32::MAX as u64, "Test requires a client ID > u32::MAX");

    // Create base document with large client ID
    let doc1 = Doc::with_options(crate::Options {
        client_id: large_client_id,
        ..Default::default()
    });

    let map = doc1.get_or_insert_map("blocks");
    {
        let mut txn = doc1.transact_mut();
        let note = map.insert(&mut txn, "note1", MapPrelim::default());
        let children = note.insert(&mut txn, "children", ArrayPrelim::default());
        children.insert(&mut txn, 0, "child1");
        children.insert(&mut txn, 1, "child2");
    }

    // Get base state
    let base_update = doc1.transact().encode_state_as_update_v1(&StateVector::default());
    let base_sv = doc1.transact().state_vector();

    // Make incremental changes
    {
        let mut txn = doc1.transact_mut();
        if let Some(crate::Out::YMap(note)) = map.get(&txn, "note1") {
            if let Some(crate::Out::YArray(children)) = note.get(&txn, "children") {
                children.insert(&mut txn, 2, "child3");
            }
        }
    }

    // Get incremental update
    let inc_update = doc1.transact().encode_state_as_update_v1(&base_sv);

    // Apply base to a new document
    let doc2 = Doc::new();
    {
        let base_decoded = Update::decode_v1(&base_update).expect("decode base");
        let mut txn = doc2.transact_mut();
        txn.apply_update(base_decoded).expect("apply base");
    }

    // Verify base state
    let map2 = doc2.get_or_insert_map("blocks");
    {
        let txn = doc2.transact();
        if let Some(crate::Out::YMap(note)) = map2.get(&txn, "note1") {
            if let Some(crate::Out::YArray(children)) = note.get(&txn, "children") {
                assert_eq!(children.len(&txn), 2, "Base should have 2 children");
            } else {
                panic!("children not found");
            }
        } else {
            panic!("note1 not found after applying base");
        }
    }

    // Apply incremental update
    {
        let inc_decoded = Update::decode_v1(&inc_update).expect("decode inc");
        let mut txn = doc2.transact_mut();
        txn.apply_update(inc_decoded).expect("apply inc");
    }

    // Verify incremental update was applied correctly
    {
        let txn = doc2.transact();
        if let Some(crate::Out::YMap(note)) = map2.get(&txn, "note1") {
            if let Some(crate::Out::YArray(children)) = note.get(&txn, "children") {
                assert_eq!(
                    children.len(&txn),
                    3,
                    "After inc update should have 3 children"
                );
            }
        }
    }
}

/// Test that v2 encoding also handles large client IDs correctly
#[test]
fn large_client_id_v2_encoding() {
    let large_client_id: ClientID = 9007199254740991; // Number.MAX_SAFE_INTEGER
    assert!(large_client_id > u32::MAX as u64, "Test requires a client ID > u32::MAX");

    let doc1 = Doc::with_options(crate::Options {
        client_id: large_client_id,
        ..Default::default()
    });

    let txt = doc1.get_or_insert_text("test");
    {
        let mut txn = doc1.transact_mut();
        txt.insert(&mut txn, 0, "hello v2");
    }

    // Encode as v2 update
    let update_v2 = doc1.transact().encode_state_as_update_v2(&StateVector::default());

    // Decode the v2 update
    let decoded_update = Update::decode_v2(&update_v2).expect("Failed to decode v2 update");

    // Verify the state vector contains the correct large client ID
    let sv = decoded_update.state_vector();
    assert!(
        sv.get(&large_client_id) > 0,
        "V2 state vector should contain the large client ID"
    );

    // Apply to a new document
    let doc2 = Doc::new();
    {
        let mut txn = doc2.transact_mut();
        txn.apply_update(decoded_update).expect("Failed to apply v2 update");
    }

    let txt2 = doc2.get_or_insert_text("test");
    assert_eq!(txt2.get_string(&doc2.transact()), "hello v2");
}
