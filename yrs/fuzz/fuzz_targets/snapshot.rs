#![no_main]
//! Decodes snapshots and uses them to restore past document state and compute text diffs.

use libfuzzer_sys::fuzz_target;
use yrs::types::text::{Diff, YChange};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::EncoderV1;
use yrs::{ReadTxn, Snapshot, Text, Transact, WriteTxn};

fuzz_target!(|data: &[u8]| {
    for snapshot in [Snapshot::decode_v1(data), Snapshot::decode_v2(data)] {
        if let Ok(snapshot) = snapshot {
            let doc = yrs_fuzz::load_doc(&yrs_fuzz::seeds().state_no_gc_v1, true);
            {
                let txn = doc.transact();
                let mut encoder = EncoderV1::new();
                let _ = txn.encode_state_from_snapshot(&snapshot, &mut encoder);
            }
            let mut txn = doc.transact_mut();
            let text = txn.get_or_insert_text(yrs_fuzz::TEXT);
            let _: Vec<Diff<YChange>> =
                text.diff_range(&mut txn, Some(&snapshot), None, YChange::identity);
        }
    }
});
