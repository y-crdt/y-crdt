//! Decoding input from a peer must never cost more than that input can justify: nesting has to
//! stop before it exhausts the stack, and a count claimed by a few bytes must not reserve memory
//! for elements that are not there.
//!
//! This is an integration test because it installs its own global allocator. Decoding a bomb
//! returns `Err` with or without a bound on the reservation, so checking the result proves nothing;
//! what has to be measured is how much memory was asked for along the way.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use yrs::encoding::read::{Cursor, Error, Read};
use yrs::encoding::varint::Signed;
use yrs::encoding::write::Write;
use yrs::sync::awareness::AwarenessUpdate;
use yrs::types::ToJson;
use yrs::updates::decoder::{Decode, DecoderV1, DecoderV2};
use yrs::updates::encoder::Encode;
use yrs::{Any, Array, Doc, GetString, ReadTxn, StateVector, Text, Transact, Update};

/// Tracks the most memory the process has held or asked for. Requests above [REFUSE_ABOVE] are
/// recorded and then refused, so a build without the bounds fails its assertions instead of
/// committing tens of gigabytes.
struct PeakAllocator;

const REFUSE_ABOVE: usize = 1 << 30;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn record_request(size: usize) {
    let requested = LIVE.load(Ordering::Relaxed).saturating_add(size);
    PEAK.fetch_max(requested, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for PeakAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_request(layout.size());
        if layout.size() > REFUSE_ABOVE {
            return std::ptr::null_mut();
        }
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_request(new_size);
        if new_size > REFUSE_ABOVE {
            return std::ptr::null_mut();
        }
        let new_ptr = System.realloc(ptr, layout, new_size);
        if !new_ptr.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            LIVE.fetch_add(new_size, Ordering::Relaxed);
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: PeakAllocator = PeakAllocator;

/// The allocator's counters are process-wide, so every test in this file runs alone.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs `f` and returns the most memory it had live or asked for, beyond what was already live.
fn peak_of(f: impl FnOnce()) -> usize {
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    f();
    PEAK.load(Ordering::Relaxed).saturating_sub(baseline)
}

/// Far more than any of the short inputs below can justify, far less than an unbounded claim asks.
const BUDGET: usize = 1 << 20;

/// A count of about half a billion, as sent by a peer that does not have that many elements.
const CLAIM: u64 = 500_000_000;

fn varint(value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    out.write_var(value);
    out
}

/// The same value spread over eleven 7-bit groups, the longest form the varint readers accept:
/// their `len > 70` check runs after `len += 7`, so the eleventh group is still read.
fn eleven_group_varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(11);
    for _ in 0..10 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8 & 0x7f);
    out
}

/// A non-negative value in the sign-carrying varint form used by [DecoderV2]'s integer streams.
fn signed_varint(value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    out.write_var_signed(&Signed::new(value as i64, false));
    out
}

/// Decoding `input` has to fail without reserving more than `budget` bytes for what it claims.
fn assert_input_stays_within<T>(
    budget: usize,
    input: &[u8],
    decode: impl Fn(&[u8]) -> Result<T, Error>,
) {
    let mut outcome = None;
    let peak = peak_of(|| outcome = Some(decode(input).map(|_| ())));
    assert!(outcome.unwrap().is_err(), "accepted {} bytes", input.len());
    assert!(peak <= budget, "{} bytes reserved, budget {}", peak, budget);
}

fn assert_input_is_bounded<T>(input: &[u8], decode: impl Fn(&[u8]) -> Result<T, Error>) {
    assert_input_stays_within(BUDGET, input, decode);
}

/// Feeds `CLAIM` through `frame` and `decode`, in its shortest form and padded to eleven groups.
fn assert_claim_is_bounded<T>(
    frame: impl Fn(&[u8]) -> Vec<u8>,
    decode: impl Fn(&[u8]) -> Result<T, Error>,
) {
    for claim in [varint(CLAIM), eleven_group_varint(CLAIM)] {
        assert_input_is_bounded(&frame(&claim), &decode);
    }
}

fn decode_any(input: &[u8]) -> Result<Any, Error> {
    Any::decode(&mut Cursor::new(input))
}

/// A v1 update holding one client, one block and the given item content.
fn v1_item(content_ref: u8, content: &[u8]) -> Vec<u8> {
    let mut input = vec![
        1, // clients
        1, // blocks
        1, // client id
        0, // clock
        content_ref,
        1, // the parent is a root type, named below
        1,
        b'a',
    ];
    input.extend_from_slice(content);
    input
}

/// The separate streams of a v2 update. The defaults describe one block with a root type parent.
struct V2Frame {
    client: Vec<u8>,
    info: Vec<u8>,
    len: Vec<u8>,
    main: Vec<u8>,
}

impl V2Frame {
    fn new(main: Vec<u8>) -> Self {
        V2Frame {
            client: vec![1],
            info: vec![8],
            len: vec![1],
            main,
        }
    }

    fn encode(&self) -> Vec<u8> {
        let empty: &[u8] = &[];
        // string payload "a" followed by its lengths
        let strings: &[u8] = &[1, b'a', 1];
        let parent_info: &[u8] = &[1];
        let streams: [&[u8]; 9] = [
            empty,
            &self.client,
            empty,
            empty,
            &self.info,
            strings,
            parent_info,
            empty,
            &self.len,
        ];
        let mut out = vec![0]; // feature flag
        for stream in streams {
            out.extend(varint(stream.len() as u64));
            out.extend_from_slice(stream);
        }
        out.extend_from_slice(&self.main);
        out
    }
}

fn deep_array_bytes(levels: usize) -> Vec<u8> {
    let mut input = [117u8, 1].repeat(levels);
    input.push(126);
    input
}

/// Nesting that continues through the last element rather than the first.
fn deep_array_through_last_element_bytes(levels: usize) -> Vec<u8> {
    let mut input = [117u8, 2, 126].repeat(levels);
    input.push(126);
    input
}

fn deep_map_bytes(levels: usize) -> Vec<u8> {
    let mut input = [118u8, 1, 1, b'k'].repeat(levels);
    input.push(126);
    input
}

fn deep_mixed_bytes(levels: usize) -> Vec<u8> {
    let mut input = Vec::new();
    for level in 0..levels {
        if level % 2 == 0 {
            input.extend_from_slice(&[117, 1]);
        } else {
            input.extend_from_slice(&[118, 1, 1, b'k']);
        }
    }
    input.push(126);
    input
}

fn deep_value(levels: usize) -> Any {
    let mut value = Any::Null;
    for level in 0..levels {
        value = if level % 2 == 0 {
            Any::Array(Arc::from(vec![value]))
        } else {
            Any::Map(Arc::new(HashMap::from([("k".to_string(), value)])))
        };
    }
    value
}

fn assert_refused(result: Result<impl std::fmt::Debug, Error>) {
    match result {
        Err(Error::UnexpectedValue) => {}
        other => panic!("expected UnexpectedValue, got {:?}", other),
    }
}

/// Deep enough to overflow any thread stack this suite could be given.
const BOMB_LEVELS: usize = 200_000;

#[test]
fn deeply_nested_array_is_refused() {
    let _serial = serial();
    assert_refused(decode_any(&deep_array_bytes(BOMB_LEVELS)));
}

#[test]
fn array_nested_through_its_last_element_is_refused() {
    let _serial = serial();
    assert_refused(decode_any(&deep_array_through_last_element_bytes(
        BOMB_LEVELS,
    )));
}

#[test]
fn deeply_nested_map_is_refused() {
    let _serial = serial();
    assert_refused(decode_any(&deep_map_bytes(BOMB_LEVELS)));
}

#[test]
fn alternating_arrays_and_maps_are_refused() {
    let _serial = serial();
    assert_refused(decode_any(&deep_mixed_bytes(BOMB_LEVELS)));
}

#[test]
fn deep_value_inside_a_v1_update_is_refused() {
    let _serial = serial();
    let mut content = vec![1]; // one value
    content.extend(deep_array_bytes(BOMB_LEVELS));
    assert_refused(Update::decode_v1(&v1_item(8, &content)));
}

#[test]
fn deep_value_inside_a_v2_update_is_refused() {
    let _serial = serial();
    let mut main = vec![1, 1, 0]; // clients, blocks, clock
    main.extend(deep_map_bytes(BOMB_LEVELS));
    assert_refused(Update::decode_v2(&V2Frame::new(main).encode()));
}

#[test]
fn nesting_limit_is_exact() {
    let _serial = serial();
    let limit = Any::MAX_DECODE_DEPTH;
    let at_limit = decode_any(&deep_array_bytes(limit)).unwrap();
    let mut encoded = Vec::new();
    at_limit.encode(&mut encoded);
    assert_eq!(encoded, deep_array_bytes(limit));
    decode_any(&deep_map_bytes(limit)).unwrap();
    assert_refused(decode_any(&deep_array_bytes(limit + 1)));
    assert_refused(decode_any(&deep_map_bytes(limit + 1)));
}

/// The limit has to leave a value that everything else can recurse over, on a stack the size
/// wasm gets by default. Dropping the value is the operation that matters most, because it cannot
/// be refused.
#[test]
fn value_at_the_limit_is_safe_on_a_small_stack() {
    let _serial = serial();
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let value = deep_value(Any::MAX_DECODE_DEPTH);
            let mut encoded = Vec::new();
            value.encode(&mut encoded);
            let decoded = decode_any(&encoded).unwrap();
            assert_eq!(decoded, value);
            let mut json = String::new();
            decoded.to_json(&mut json);
            drop(decoded);
            drop(value);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn state_vector_claim_is_bounded() {
    let _serial = serial();
    assert_claim_is_bounded(|claim| claim.to_vec(), StateVector::decode_v1);
    assert_claim_is_bounded(
        |claim| V2Frame::new(claim.to_vec()).encode(),
        StateVector::decode_v2,
    );
}

#[test]
fn update_client_claim_is_bounded() {
    let _serial = serial();
    assert_claim_is_bounded(|claim| claim.to_vec(), Update::decode_v1);
    assert_claim_is_bounded(
        |claim| V2Frame::new(claim.to_vec()).encode(),
        Update::decode_v2,
    );
}

#[test]
fn update_block_claim_is_bounded() {
    let _serial = serial();
    assert_claim_is_bounded(|claim| [&[1], claim, &[1, 0]].concat(), Update::decode_v1);
    assert_claim_is_bounded(
        |claim| V2Frame::new([&[1], claim, &[0]].concat()).encode(),
        Update::decode_v2,
    );
}

#[test]
fn update_block_claim_of_a_later_client_is_bounded() {
    let _serial = serial();
    assert_claim_is_bounded(
        // the first client is well formed and has no blocks
        |claim| [&[2, 0, 1, 0], claim, &[2, 0]].concat(),
        Update::decode_v1,
    );
}

#[test]
fn awareness_claim_is_bounded() {
    let _serial = serial();
    assert_claim_is_bounded(|claim| claim.to_vec(), AwarenessUpdate::decode_v1);
}

#[test]
fn any_array_claim_is_bounded() {
    let _serial = serial();
    assert_claim_is_bounded(|claim| [&[117], claim].concat(), decode_any);
}

#[test]
fn any_map_claim_is_bounded() {
    let _serial = serial();
    assert_claim_is_bounded(|claim| [&[118], claim].concat(), decode_any);
}

#[test]
fn item_any_content_claim_is_bounded() {
    let _serial = serial();
    assert_claim_is_bounded(|claim| v1_item(8, claim), Update::decode_v1);
    let frame = V2Frame {
        len: signed_varint(CLAIM),
        ..V2Frame::new(vec![1, 1, 0])
    };
    assert_input_is_bounded(&frame.encode(), Update::decode_v2);
}

#[test]
fn item_json_content_claim_is_bounded() {
    let _serial = serial();
    assert_claim_is_bounded(|claim| v1_item(2, claim), Update::decode_v1);
}

/// The JSON block reads its count as a signed value. A negative one has always been an error and
/// must stay one, rather than decoding as an empty block.
#[test]
fn item_json_content_with_a_negative_count_is_refused() {
    let _serial = serial();
    // the trailing zero is an empty delete set, so nothing but the count can fail this update
    let content = [varint(u32::MAX as u64), vec![0]].concat();
    assert!(Update::decode_v1(&v1_item(2, &content)).is_err());
}

/// The sixth 7-bit group shifts by 35. A `usize` count is read as `u64` and sees 2^35, while a
/// `u32` count wraps the shift and sees 8, so the same bytes make very different claims.
#[cfg(target_pointer_width = "64")]
#[test]
fn claim_above_u32_range_is_bounded() {
    let _serial = serial();
    let claim: &[u8] = &[0x80, 0x80, 0x80, 0x80, 0x80, 0x01];
    assert_input_is_bounded(&[&[117], claim].concat(), decode_any);
    assert_input_is_bounded(&[&[118], claim].concat(), decode_any);
    assert_input_is_bounded(claim, AwarenessUpdate::decode_v1);
    assert_input_is_bounded(claim, StateVector::decode_v1);
}

/// Every open container claims the same unread bytes, so a bound applied at each level on its own
/// would still reserve the input's size once per level.
#[test]
fn nested_claims_do_not_add_up() {
    let _serial = serial();
    let padding = 256 * 1024;
    let claim = varint(CLAIM);
    let array_level = [&[117u8], claim.as_slice()].concat();
    let map_level = [&[118u8], claim.as_slice(), &[1, b'k']].concat();
    for level in [array_level, map_level] {
        let mut input = level.repeat(Any::MAX_DECODE_DEPTH);
        input.resize(input.len() + padding, 0);
        assert_input_stays_within(input.len() * 128 + BUDGET, &input, decode_any);
    }
}

#[test]
fn count_equal_to_the_remaining_input_is_accepted() {
    let _serial = serial();
    let count = 10_000;
    let input = [&[117][..], &varint(count)[..], &vec![126; count as usize]].concat();
    match decode_any(&input).unwrap() {
        Any::Array(items) => assert_eq!(items.len(), count as usize),
        other => panic!("unexpected {:?}", other),
    }
}

#[test]
fn readers_report_the_bytes_left() {
    let _serial = serial();
    let mut cursor = Cursor::new(&[1, 2, 3]);
    cursor.read_u8().unwrap();
    assert_eq!(cursor.remaining(), 2);

    let mut v1 = DecoderV1::from(&[1u8, 2, 3][..]);
    v1.read_u8().unwrap();
    assert_eq!(v1.remaining(), 2);

    let input = V2Frame::new(vec![1, 2, 3]).encode();
    let mut v2 = DecoderV2::new(Cursor::new(&input)).unwrap();
    v2.read_u8().unwrap();
    assert_eq!(v2.remaining(), 2);
}

fn sample_doc() -> Doc {
    let doc = Doc::with_client_id(1);
    let text = doc.get_or_insert_text("text");
    let array = doc.get_or_insert_array("array");
    let mut txn = doc.transact_mut();
    // inserting at the front keeps every character in a block of its own
    for _ in 0..2_000 {
        text.insert(&mut txn, 0, "u");
    }
    array.push_back(&mut txn, deep_value(Any::MAX_DECODE_DEPTH));
    let big: Vec<Any> = (0..10_000).map(|i| Any::from(i as f64)).collect();
    array.push_back(&mut txn, Any::Array(Arc::from(big)));
    drop(txn);
    doc
}

fn assert_same_content(left: &Doc, right: &Doc) {
    let left_text = left.get_or_insert_text("text");
    let right_text = right.get_or_insert_text("text");
    let left_array = left.get_or_insert_array("array");
    let right_array = right.get_or_insert_array("array");
    let (left_txn, right_txn) = (left.transact(), right.transact());
    assert_eq!(
        left_text.get_string(&left_txn),
        right_text.get_string(&right_txn)
    );
    assert_eq!(left_array.len(&left_txn), 2);
    assert_eq!(
        left_array.to_json(&left_txn),
        right_array.to_json(&right_txn)
    );
    assert_eq!(left_txn.state_vector(), right_txn.state_vector());
}

fn apply_full_state(source: &Doc, encode: Encoding) -> Doc {
    let target = Doc::with_client_id(2);
    let everything = StateVector::default();
    let update = {
        let txn = source.transact();
        match encode {
            Encoding::V1 => Update::decode_v1(&txn.encode_state_as_update_v1(&everything)),
            Encoding::V2 => Update::decode_v2(&txn.encode_state_as_update_v2(&everything)),
        }
    };
    target.transact_mut().apply_update(update.unwrap()).unwrap();
    target
}

enum Encoding {
    V1,
    V2,
}

#[test]
fn real_v1_update_round_trips() {
    let _serial = serial();
    let source = sample_doc();
    assert_same_content(&source, &apply_full_state(&source, Encoding::V1));
}

#[test]
fn real_v2_update_round_trips() {
    let _serial = serial();
    let source = sample_doc();
    assert_same_content(&source, &apply_full_state(&source, Encoding::V2));
}

/// A v2 update keeps block fields in separate streams, so with only text in it the main stream
/// holds far fewer bytes than the update has blocks. The bound limits what is reserved up front;
/// it must not refuse such a count.
#[test]
fn v2_update_with_more_blocks_than_main_stream_bytes_round_trips() {
    let _serial = serial();
    let source = Doc::with_client_id(1);
    let text = source.get_or_insert_text("text");
    {
        let mut txn = source.transact_mut();
        for _ in 0..2_000 {
            text.insert(&mut txn, 0, "u");
        }
    }
    let target = apply_full_state(&source, Encoding::V2);
    let target_text = target.get_or_insert_text("text");
    assert_eq!(
        target_text.get_string(&target.transact()),
        "u".repeat(2_000)
    );
}

#[test]
fn state_vector_round_trips() {
    let _serial = serial();
    let sv = sample_doc().transact().state_vector();
    assert_eq!(StateVector::decode_v1(&sv.encode_v1()).unwrap(), sv);
    assert_eq!(StateVector::decode_v2(&sv.encode_v2()).unwrap(), sv);
}
