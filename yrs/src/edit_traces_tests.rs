use crate::edit_traces::load_testing_data;
use crate::{Doc, GetString, OffsetKind, Options, Text, Transact};

#[test]
fn edit_trace_automerge() {
    test_editing_trace("editing-traces/sequential_traces/automerge-paper.json.gz");
}

#[test]
fn edit_trace_friendsforever() {
    test_editing_trace("editing-traces/sequential_traces/friendsforever_flat.json.gz");
}

#[test]
fn edit_trace_sephblog1() {
    test_editing_trace("editing-traces/sequential_traces/seph-blog1.json.gz");
}

#[test]
fn edit_trace_sveltecomponent() {
    test_editing_trace("editing-traces/sequential_traces/sveltecomponent.json.gz");
}

#[test]
fn edit_trace_rustcode() {
    test_editing_trace("editing-traces/sequential_traces/rustcode.json.gz");
}

fn test_editing_trace(fpath: &str) {
    let data = load_testing_data(fpath);
    let doc = Doc::with_options(Options {
        offset_kind: if data.using_byte_positions {
            OffsetKind::Bytes
        } else {
            OffsetKind::Utf32
        },
        ..Options::default()
    });
    let txt = doc.get_or_insert_text("text");
    println!("using byte indexes: {}", data.using_byte_positions);
    println!("start content: '{}'", data.start_content);
    for t in data.txns {
        let mut txn = doc.transact_mut();
        for patch in t.patches {
            let at = patch.0 as u32;
            let delete_count = patch.1 as u32;
            let content = patch.2;

            if delete_count != 0 {
                println!("{at} delete {delete_count} elements");
                txt.remove_range(&mut txn, at, delete_count);
            } else {
                println!("{at} insert '{content}'");
                txt.insert(&mut txn, at, &content);
            }
        }
    }

    assert_eq!(txt.get_string(&doc.transact()), data.end_content);
}
