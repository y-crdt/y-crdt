#![no_main]
//! Decodes a v2 update and applies it to a fresh document and to a document with existing state.

use libfuzzer_sys::fuzz_target;
use yrs::updates::decoder::Decode;
use yrs::Update;

fuzz_target!(|data: &[u8]| {
    yrs_fuzz::apply_update(|| Update::decode_v2(data).ok());
});
