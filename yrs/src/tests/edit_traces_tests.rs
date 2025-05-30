use crate::tests::edit_traces::load_testing_data;
use crate::{Doc, GetString, OffsetKind, Options, Text};
use std::time::Instant;

#[test]
fn edit_trace_automerge() {
    test_editing_trace("../assets/editing-traces/sequential_traces/automerge-paper.json.gz");
}

#[test]
fn edit_trace_friendsforever() {
    test_editing_trace("../assets/editing-traces/sequential_traces/friendsforever_flat.json.gz");
}

#[test]
fn edit_trace_sephblog1() {
    test_editing_trace("../assets/editing-traces/sequential_traces/seph-blog1.json.gz");
}

#[test]
fn edit_trace_sveltecomponent() {
    test_editing_trace("../assets/editing-traces/sequential_traces/sveltecomponent.json.gz");
}

#[test]
fn edit_trace_rustcode() {
    test_editing_trace("../assets/editing-traces/sequential_traces/rustcode.json.gz");
}

fn test_editing_trace(fpath: &str) {
    let data = load_testing_data(fpath);
    let mut doc = Doc::with_options(Options {
        offset_kind: if data.using_byte_positions {
            OffsetKind::Bytes
        } else {
            OffsetKind::Utf16
        },
        ..Options::default()
    });
    let txt = doc.get_or_insert_text("text");
    let start = Instant::now();
    for t in data.txns {
        let mut txn = doc.transact_mut();
        for patch in t.patches {
            let at = patch.0;
            let delete = patch.1;
            let content = patch.2;

            if delete != 0 {
                txt.remove_range(&mut txn, at as u32, delete as u32);
            }
            if !content.is_empty() {
                txt.insert(&mut txn, at as u32, &content);
            }
        }
    }
    let finish = Instant::now();
    println!("elapsed: {}ms", (finish - start).as_millis());
    assert_eq!(txt.get_string(&doc.transact()), data.end_content);
}
