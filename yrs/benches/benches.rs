use criterion::*;
use lib0::decoding::Cursor;
use rand::distributions::Alphanumeric;
use rand::prelude::ThreadRng;
use rand::{thread_rng, Rng};
use std::cell::Cell;
use std::fs::File;
use std::io::Read;
use yrs::updates::decoder::{Decode, Decoder, DecoderV1};
use yrs::updates::encoder::{Encode, Encoder};
use yrs::{Array, Doc, Text};

#[derive(PartialEq, Eq, Debug)]
enum TextOp {
    Insert(u32, String),
    Delete(u32, u32),
}

impl Encode for TextOp {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        match self {
            TextOp::Insert(idx, content) => {
                encoder.write_u32(1);
                encoder.write_u32(*idx);
                encoder.write_string(content.as_str());
            }
            TextOp::Delete(idx, len) => {
                encoder.write_u32(2);
                encoder.write_u32(*idx);
                encoder.write_u32(*len);
            }
        }
    }
}

impl Decode for TextOp {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        match decoder.read_u32() {
            1 => {
                let idx = decoder.read_u32();
                let content = decoder.read_string().to_string();
                TextOp::Insert(idx, content)
            }
            2 => {
                let idx = decoder.read_u32();
                let len = decoder.read_u32();
                TextOp::Delete(idx, len)
            }
            a => panic!("unrecognized TextOp case: {}", a),
        }
    }
}

#[derive(PartialEq, Eq, Debug)]
enum ArrayOp {
    Insert(u32, Vec<u32>),
    Delete(u32, u32),
}

impl Encode for ArrayOp {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        match self {
            ArrayOp::Insert(idx, content) => {
                encoder.write_u32(1);
                encoder.write_u32(*idx);
                encoder.write_u32(content.len() as u32);
                for i in content.iter() {
                    encoder.write_u32(*i);
                }
            }
            ArrayOp::Delete(idx, len) => {
                encoder.write_u32(2);
                encoder.write_u32(*idx);
                encoder.write_u32(*len);
            }
        }
    }
}

impl Decode for ArrayOp {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        match decoder.read_u32() {
            1 => {
                let idx = decoder.read_u32();
                let mut i = decoder.read_u32() as usize;
                let mut content = Vec::with_capacity(i);
                while i > 0 {
                    content.push(decoder.read_u32());
                    i -= 1;
                }
                ArrayOp::Insert(idx, content)
            }
            2 => {
                let idx = decoder.read_u32();
                let len = decoder.read_u32();
                ArrayOp::Delete(idx, len)
            }
            _ => panic!("unrecognized TextOp case"),
        }
    }
}

fn encode_vec<T: Encode, E: Encoder>(vec: &Vec<T>, encoder: &mut E) {
    encoder.write_u32(vec.len() as u32);
    for value in vec.iter() {
        value.encode(encoder);
    }
}

fn decode_vec<T: Decode, D: Decoder>(decoder: &mut D) -> Vec<T> {
    let mut i = decoder.read_u32() as usize;
    let mut vec = Vec::with_capacity(i);
    while i > 0 {
        vec.push(T::decode(decoder));
        i -= 1;
    }
    vec
}

fn load<T: Decode>(src: &str) -> Vec<T> {
    let mut f = File::open(src).unwrap();
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    let mut decoder = DecoderV1::new(Cursor::new(buf.as_slice()));
    decode_vec(&mut decoder)
}

fn text_benchmark(c: &mut Criterion, name: &str, src: &str) {
    let input = {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("text");
        let ops = load(src);
        (doc, txt, ops)
    };

    c.bench_with_input(
        BenchmarkId::new(name, input.2.len()),
        &input,
        |b, (doc, text, ops)| {
            b.iter(|| {
                for op in ops.iter() {
                    let mut txn = doc.transact();
                    match op {
                        TextOp::Insert(idx, txt) => text.insert(&mut txn, *idx, txt),
                        TextOp::Delete(idx, len) => text.remove_range(&mut txn, *idx, *len),
                    }
                }
            });
        },
    );
}

fn array_benchmark(c: &mut Criterion, name: &str, src: &str) {
    let input = {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let array = txn.get_array("text");
        let ops = load(src);
        (doc, array, ops)
    };

    c.bench_with_input(
        BenchmarkId::new(name, input.2.len()),
        &input,
        |b, (doc, array, ops)| {
            b.iter(|| {
                for op in ops.iter() {
                    let mut txn = doc.transact();
                    match op {
                        ArrayOp::Insert(idx, values) => {
                            array.insert_range(&mut txn, *idx, values.clone())
                        }
                        ArrayOp::Delete(idx, len) => array.remove_range(&mut txn, *idx, *len),
                    }
                }
            });
        },
    );
}

fn b1_1(c: &mut Criterion) {
    text_benchmark(
        c,
        "[B1.1] Append 6000 characters",
        "./yrs/benches/input/b-1-1-6000.bin",
    );
}

fn b1_2(c: &mut Criterion) {
    text_benchmark(
        c,
        "[B1.2] Insert string of length 6000",
        "./yrs/benches/input/b-1-2-6000.bin",
    );
}

fn b1_3(c: &mut Criterion) {
    text_benchmark(
        c,
        "[B1.3] Prepend 6000 characters",
        "./yrs/benches/input/b-1-3-6000.bin",
    );
}

fn b1_4(c: &mut Criterion) {
    text_benchmark(
        c,
        "[B1.4] Insert 6000 characters at random positions",
        "./yrs/benches/input/b-1-4-6000.bin",
    );
}

fn b1_5(c: &mut Criterion) {
    text_benchmark(
        c,
        "[B1.5] Insert 6000 words at random positions",
        "./yrs/benches/input/b-1-5-6000.bin",
    );
}

fn b1_6(c: &mut Criterion) {
    text_benchmark(
        c,
        "[B1.6] Insert string, then delete it",
        "./yrs/benches/input/b-1-6-6000.bin",
    );
}

fn b1_7(c: &mut Criterion) {
    text_benchmark(
        c,
        "[B1.7] Insert/Delete strings at random positions",
        "./yrs/benches/input/b-1-7-6000.bin",
    );
}

fn b1_8(c: &mut Criterion) {
    array_benchmark(
        c,
        "[B1.8] Append 6000 numbers",
        "./yrs/benches/input/b-1-8-6000.bin",
    );
}

fn b1_9(c: &mut Criterion) {
    array_benchmark(
        c,
        "[B1.9] Insert Array of 6000 numbers",
        "./yrs/benches/input/b-1-9-6000.bin",
    );
}

fn b1_10(c: &mut Criterion) {
    array_benchmark(
        c,
        "[B1.10] Prepend 6000 numbers",
        "./yrs/benches/input/b-1-10-6000.bin",
    );
}

fn b1_11(c: &mut Criterion) {
    array_benchmark(
        c,
        "[B1.11] Insert 6000 numbers at random positions",
        "./yrs/benches/input/b-1-11-6000.bin",
    );
}

fn bench(c: &mut Criterion) {
    b1_1(c);
    b1_2(c);
    b1_3(c);
    b1_4(c);
    b1_5(c);
    b1_6(c);
    b1_7(c);
    b1_8(c);
    b1_9(c);
    b1_10(c);
    b1_11(c);
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench,
}
criterion_main!(benches);
