//! Generates seed corpus for all fuzz targets into `corpus/<target>/`.
//!
//! Usage (from `yrs/fuzz` directory): `cargo run --example generate_corpus`

use std::collections::HashMap;
use std::path::Path;
use yrs::sync::awareness::Awareness;
use yrs::sync::{Message, SyncMessage};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Any, Assoc, IndexedSequence, ReadTxn, StateVector, Transact, Update, WriteTxn};

fn write(target: &str, seeds: &[Vec<u8>]) {
    let dir = Path::new("corpus").join(target);
    std::fs::create_dir_all(&dir).unwrap();
    for (i, seed) in seeds.iter().enumerate() {
        std::fs::write(dir.join(format!("seed-{:03}", i)), seed).unwrap();
    }
    println!("{}: {} seeds", target, seeds.len());
}

fn main() {
    let seeds = yrs_fuzz::seeds();
    let (doc, _, _) = yrs_fuzz::seed_doc(false);
    let (no_gc, _, _) = yrs_fuzz::seed_doc(true);
    let full = Update::decode_v1(&seeds.state_v1).unwrap();

    // updates: incremental updates, full state and diffs against partial states
    let mut updates_v1 = seeds.updates_v1.clone();
    updates_v1.push(seeds.state_v1.clone());
    updates_v1.push(seeds.state_no_gc_v1.clone());
    let mut updates_v2 = seeds.updates_v2.clone();
    updates_v2.push(full.encode_v2());
    updates_v2.push(
        Update::decode_v1(&seeds.state_no_gc_v1)
            .unwrap()
            .encode_v2(),
    );
    for i in 1..seeds.updates_v1.len() {
        // state of a document that has seen only the first `i` updates
        let partial = yrs::merge_updates_v1(&seeds.updates_v1[..i]).unwrap();
        let sv = yrs::encode_state_vector_from_update_v1(&partial).unwrap();
        let sv = StateVector::decode_v1(&sv).unwrap();
        updates_v1.push(doc.transact().encode_state_as_update_v1(&sv));
        updates_v2.push(doc.transact().encode_state_as_update_v2(&sv));
    }
    write("update_v1", &updates_v1);
    write("update_v2", &updates_v2);

    // merge_diff: header byte (encoding version + split position) followed by two updates
    let mut merge = Vec::new();
    for (version, updates) in [(0u8, &updates_v1), (1u8, &updates_v2)] {
        for pair in updates.windows(2) {
            let split = pair[0].len().min(127) as u8;
            let mut input = vec![(split << 1) | version];
            input.extend_from_slice(&pair[0][..split as usize]);
            input.extend_from_slice(&pair[1]);
            merge.push(input);
        }
    }
    write("merge_diff", &merge);

    // state vectors and delete sets
    let txn = no_gc.transact();
    let snapshot = txn.snapshot();
    write(
        "state_vector",
        &[
            txn.state_vector().encode_v1(),
            txn.state_vector().encode_v2(),
            snapshot.delete_set.encode_v1(),
            snapshot.delete_set.encode_v2(),
        ],
    );
    write("snapshot", &[snapshot.encode_v1(), snapshot.encode_v2()]);
    drop(txn);

    // y-sync protocol messages
    let mut awareness = Awareness::new(yrs_fuzz::doc_with_client(7, false));
    awareness.set_local_state_raw(r##"{"user":{"name":"a","color":"#fff"},"cursor":[1,2]}"##);
    let txn = doc.transact();
    let mut stream = Vec::new();
    stream.extend(Message::Sync(SyncMessage::SyncStep1(txn.state_vector())).encode_v1());
    stream.extend(Message::Sync(SyncMessage::SyncStep2(seeds.state_v1.clone())).encode_v1());
    for update in &seeds.updates_v1 {
        stream.extend(Message::Sync(SyncMessage::Update(update.clone())).encode_v1());
    }
    let awareness_update = awareness.update().unwrap();
    let mut with_awareness = Message::Awareness(awareness_update.clone()).encode_v1();
    with_awareness.extend(Message::Auth(Some("denied".into())).encode_v1());
    with_awareness.extend(Message::AwarenessQuery.encode_v1());
    write(
        "sync_protocol",
        &[stream, with_awareness, awareness_update.encode_v1()],
    );
    drop(txn);

    // sticky indexes
    let mut txn = doc.transact_mut();
    let text = txn.get_or_insert_text(yrs_fuzz::TEXT);
    let array = txn.get_or_insert_array(yrs_fuzz::ARRAY);
    let mut sticky = Vec::new();
    for (index, assoc) in [(0, Assoc::Before), (3, Assoc::After), (5, Assoc::Before)] {
        if let Some(index) = text.sticky_index(&txn, index, assoc) {
            sticky.push(index.encode_v1());
            sticky.push(index.encode_v2());
        }
        if let Some(index) = array.sticky_index(&txn, index, assoc) {
            sticky.push(index.encode_v1());
        }
    }
    write("sticky_index", &sticky);
    drop(txn);

    // Any values
    let value = Any::from(HashMap::from([
        (
            "a".to_string(),
            Any::from(vec![
                Any::from(1),
                Any::from("x"),
                Any::Null,
                Any::Undefined,
            ]),
        ),
        ("b".to_string(), Any::from(i64::MAX)),
        ("c".to_string(), Any::from(1.5)),
        ("d".to_string(), Any::from(vec![0u8, 1, 2])),
    ]));
    let mut encoded = Vec::new();
    value.encode(&mut encoded);
    let mut json = String::new();
    value.to_json(&mut json);
    write("any", &[encoded, json.into_bytes()]);
}
