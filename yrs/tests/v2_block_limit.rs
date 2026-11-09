//! `Update::decode_v2_with_block_limit`. A separate test binary because it installs a counting
//! global allocator: a refused update returns `Err` however late it was refused, so only the
//! memory it took shows whether the claim was checked before the blocks were decoded.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use yrs::encoding::read::Error;
use yrs::encoding::write::Write;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, GetString, ReadTxn, StateVector, Text, Transact, Update};

struct PeakAllocator;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn track(size: usize) {
    let live = LIVE.fetch_add(size, Ordering::SeqCst) + size;
    PEAK.fetch_max(live, Ordering::SeqCst);
}

unsafe impl GlobalAlloc for PeakAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        track(layout.size());
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        track(new_size);
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOCATOR: PeakAllocator = PeakAllocator;

/// The allocator counters are shared by the whole process.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn varint(value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    out.write_var(value);
    out
}

/// A v2 update of GC blocks of length 1. Their fields come from run-length streams, so entries
/// of any size take the same few bytes of input.
fn gc_blocks(per_client: &[u64]) -> Vec<u8> {
    let total: u64 = per_client.iter().sum();
    let mut len = vec![0x40 | 1]; // length 1, with a repeat count following
    len.extend(varint(total - 2));
    // client id 1 for every entry: a run needs at least two values
    let client = match per_client.len() {
        1 => vec![1],
        n => [vec![0x40 | 1], varint(n as u64 - 2)].concat(),
    };
    let streams: [&[u8]; 9] = [&[], &client, &[], &[], &[0], &[0], &[], &[], &len];

    let mut input = vec![0]; // feature flag
    for stream in streams {
        input.extend(varint(stream.len() as u64));
        input.extend_from_slice(stream);
    }
    input.extend(varint(per_client.len() as u64));
    let mut clock = 0;
    for &blocks in per_client {
        input.extend(varint(blocks));
        input.extend(varint(clock));
        clock += blocks;
    }
    input.push(0); // empty delete set
    input
}

fn is_refused(result: Result<Update, Error>) -> bool {
    matches!(result, Err(Error::UnexpectedValue))
}

/// A document whose update has `blocks` blocks: inserting at the front leaves nothing to merge.
fn doc_with_blocks(blocks: usize) -> Doc {
    let doc = Doc::with_client_id(1);
    let text = doc.get_or_insert_text("text");
    let mut txn = doc.transact_mut();
    for _ in 0..blocks {
        text.insert(&mut txn, 0, "u");
    }
    drop(txn);
    doc
}

fn state_v2(doc: &Doc) -> Vec<u8> {
    doc.transact()
        .encode_state_as_update_v2(&StateVector::default())
}

#[test]
fn a_few_bytes_claiming_millions_of_blocks_are_refused_before_decoding() {
    let _serial = serial();
    let input = gc_blocks(&[2_000_000]);
    assert!(input.len() < 32);

    let before = LIVE.load(Ordering::SeqCst);
    PEAK.store(before, Ordering::SeqCst);
    let result = Update::decode_v2_with_block_limit(&input, 1_000);
    let peak = PEAK.load(Ordering::SeqCst) - before;

    assert!(is_refused(result));
    assert!(peak < 64 * 1024, "took {} bytes before refusing", peak);
}

#[test]
fn limit_is_exact() {
    let _serial = serial();
    let input = gc_blocks(&[1_000]);
    assert!(Update::decode_v2_with_block_limit(&input, 1_000).is_ok());
    assert!(is_refused(Update::decode_v2_with_block_limit(&input, 999)));
}

#[test]
fn limit_applies_to_the_total_across_clients() {
    let _serial = serial();
    let input = gc_blocks(&[600, 600]);
    assert!(Update::decode_v2_with_block_limit(&input, 1_200).is_ok());
    assert!(is_refused(Update::decode_v2_with_block_limit(
        &input, 1_199
    )));
}

#[test]
fn zero_limit_accepts_an_empty_update() {
    let _serial = serial();
    let bytes = Update::new().encode_v2();
    assert!(Update::decode_v2_with_block_limit(&bytes, 0).is_ok());
}

#[test]
fn real_update_within_the_limit_applies() {
    let _serial = serial();
    let source = doc_with_blocks(2_000);
    let text = source.get_or_insert_text("text");
    text.remove_range(&mut source.transact_mut(), 0, 100);
    let bytes = state_v2(&source);

    let update = Update::decode_v2_with_block_limit(&bytes, 1_000_000).unwrap();
    let target = Doc::with_client_id(2);
    target.transact_mut().apply_update(update).unwrap();

    let target_text = target.get_or_insert_text("text");
    assert_eq!(
        target_text.get_string(&target.transact()),
        "u".repeat(1_900)
    );
}

#[test]
fn real_update_over_the_limit_is_refused() {
    let _serial = serial();
    let bytes = state_v2(&doc_with_blocks(2_000));
    assert!(Update::decode_v2_with_block_limit(&bytes, 2_000).is_ok());
    assert!(is_refused(Update::decode_v2_with_block_limit(
        &bytes, 1_999
    )));
}

#[test]
fn malformed_input_is_still_an_error() {
    let _serial = serial();
    let mut input = gc_blocks(&[1_000]);
    input.pop(); // the delete set is gone
    assert!(Update::decode_v2_with_block_limit(&input, 1_000).is_err());
    assert!(Update::decode_v2_with_block_limit(&[], 1_000).is_err());
}

/// Valid updates of any size have to keep decoding through the existing entry point.
#[test]
fn decode_v2_has_no_limit() {
    let _serial = serial();
    assert!(Update::decode_v2(&gc_blocks(&[200_000])).is_ok());
}
