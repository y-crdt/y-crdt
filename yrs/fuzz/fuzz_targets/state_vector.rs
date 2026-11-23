#![no_main]
//! Decodes state vectors and delete sets (eg. received as y-sync step 1 message) and computes
//! document diffs based on them.

use libfuzzer_sys::fuzz_target;
use yrs::updates::decoder::Decode;
use yrs::{IdSet, ReadTxn, StateVector, Transact};

fuzz_target!(|data: &[u8]| {
    let _ = IdSet::decode_v1(data);
    let _ = IdSet::decode_v2(data);
    for sv in [StateVector::decode_v1(data), StateVector::decode_v2(data)] {
        if let Ok(sv) = sv {
            let doc = yrs_fuzz::load_doc(&yrs_fuzz::seeds().state_v1, false);
            let txn = doc.transact();
            let _ = txn.encode_state_as_update_v1(&sv);
            let _ = txn.encode_state_as_update_v2(&sv);
            let _ = txn.encode_diff_v1(&sv);
        }
    }
});
