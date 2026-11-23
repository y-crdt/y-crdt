#![no_main]
//! Decodes lib0-encoded and JSON-encoded `Any` values.

use libfuzzer_sys::fuzz_target;
use yrs::encoding::read::Cursor;
use yrs::updates::decoder::DecoderV1;
use yrs::Any;

fuzz_target!(|data: &[u8]| {
    if let Ok(any) = Any::decode(&mut DecoderV1::new(Cursor::new(data))) {
        let mut buf = Vec::new();
        any.encode(&mut buf);
        let mut json = String::new();
        any.to_json(&mut json);
    }
    if let Ok(json) = std::str::from_utf8(data) {
        let _ = Any::from_json(json);
    }
});
