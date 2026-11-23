#![no_main]
//! Decodes sticky indexes (ie. remote cursor positions) and resolves them against a document.

use libfuzzer_sys::fuzz_target;
use yrs::updates::decoder::Decode;
use yrs::{StickyIndex, Transact};

fuzz_target!(|data: &[u8]| {
    for index in [StickyIndex::decode_v1(data), StickyIndex::decode_v2(data)] {
        if let Ok(index) = index {
            let doc = yrs_fuzz::load_doc(&yrs_fuzz::seeds().state_v1, false);
            let txn = doc.transact();
            let _ = index.get_offset(&txn);
        }
    }
});
