#![no_main]
//! Merges and diffs updates without integrating them into a document (`merge_updates_v*`,
//! `diff_updates_v*`, `encode_state_vector_from_update_v*`).
//!
//! The first byte selects the encoding version (lowest bit) and the position at which the rest
//! of the input is split into two payloads.

use libfuzzer_sys::fuzz_target;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    diff_updates_v1, diff_updates_v2, encode_state_vector_from_update_v1,
    encode_state_vector_from_update_v2, merge_updates_v1, merge_updates_v2, StateVector, Update,
};

fuzz_target!(|data: &[u8]| {
    let Some((&header, rest)) = data.split_first() else {
        return;
    };
    let split = if rest.is_empty() {
        0
    } else {
        (header as usize >> 1) % (rest.len() + 1)
    };
    let (a, b) = rest.split_at(split);
    let seeds = yrs_fuzz::seeds();
    if header & 1 == 0 {
        let seed = &seeds.state_v1;
        let _ = merge_updates_v1([a, b]);
        let _ = merge_updates_v1([rest, seed.as_slice()]);
        let _ = encode_state_vector_from_update_v1(rest);
        let _ = diff_updates_v1(rest, &seeds.state_vector_v1);
        let _ = diff_updates_v1(seed, b);
        let _ = diff_updates_v1(a, b);
    } else {
        let seed = Update::decode_v1(&seeds.state_v1).unwrap().encode_v2();
        let sv = StateVector::decode_v1(&seeds.state_vector_v1)
            .unwrap()
            .encode_v2();
        let _ = merge_updates_v2([a, b]);
        let _ = merge_updates_v2([rest, seed.as_slice()]);
        let _ = encode_state_vector_from_update_v2(rest);
        let _ = diff_updates_v2(rest, &sv);
        let _ = diff_updates_v2(&seed, b);
        let _ = diff_updates_v2(a, b);
    }
});
