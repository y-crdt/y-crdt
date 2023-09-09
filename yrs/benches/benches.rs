use criterion::*;
use rand::distributions::Alphanumeric;
use rand::prelude::StdRng;
use rand::{Rng, RngCore, SeedableRng};
use std::cell::Cell;
use std::collections::HashMap;
use yrs::updates::decoder::Decode;
use yrs::{Array, Doc, Map, MapRef, Text, TextRef, Transact, TransactionMut, Update};
use yrs::encoding::read::{Cursor, Read};

const N: usize = 6000;
const SQRT_N: usize = 77 * 20;
const SEED: u64 = 0xdeadbeaf;

enum TextOp {
    Insert(u32, String),
    Delete(u32, u32),
}

enum ArrayOp {
    Insert(u32, Vec<u32>),
    Delete(u32, u32),
}

fn b1_1<R: RngCore>(rng: &mut R, size: usize) -> Vec<TextOp> {
    let sample: Vec<_> = rng
        .sample_iter(&Alphanumeric)
        .take(size)
        .map(|c| c.to_string())
        .collect();

    (0..size as u32)
        .into_iter()
        .zip(sample)
        .map(|(i, str)| TextOp::Insert(i, str))
        .collect()
}

fn b1_2<R: RngCore>(rng: &mut R, size: usize) -> Vec<TextOp> {
    let s: String = rng.sample_iter(&Alphanumeric).take(size as usize).collect();
    vec![TextOp::Insert(0, s)]
}

fn b1_3<R: RngCore>(rng: &mut R, size: usize) -> Vec<TextOp> {
    let sample: Vec<_> = rng
        .sample_iter(&Alphanumeric)
        .take(size)
        .map(|c| c.to_string())
        .collect();

    (0..size as u32)
        .into_iter()
        .zip(sample)
        .map(|(_, str)| TextOp::Insert(0, str))
        .collect()
}

fn b1_4<R: RngCore>(rng: &mut R, size: usize) -> Vec<TextOp> {
    let sample: Vec<_> = rng
        .sample_iter(&Alphanumeric)
        .take(size)
        .map(|c| c.to_string())
        .collect();

    (0..size as u32)
        .into_iter()
        .zip(sample)
        .map(|(i, str)| {
            let idx = rng.gen_range(0, i.max(1));
            TextOp::Insert(idx, str)
        })
        .collect()
}

fn gen_string<R: RngCore>(rng: &mut R, min: usize, max: usize) -> String {
    let len = rng.gen_range(min, max);
    rng.sample_iter(&Alphanumeric).take(len).collect()
}

fn b1_5<R: RngCore>(rng: &mut R, size: usize) -> Vec<TextOp> {
    (0..size as u32)
        .into_iter()
        .map(|i| {
            let str = gen_string(rng, 2, 10);
            let idx = rng.gen_range(0, i.max(1));
            TextOp::Insert(idx, str)
        })
        .collect()
}

fn b1_6<R: RngCore>(rng: &mut R, size: usize) -> Vec<TextOp> {
    let s: String = rng.sample_iter(&Alphanumeric).take(size as usize).collect();
    let len = s.len() as u32;
    vec![TextOp::Insert(0, s), TextOp::Delete(0, len)]
}

fn b1_7<R: RngCore>(rng: &mut R, size: usize) -> Vec<TextOp> {
    let total_len = Cell::new(0u32);
    (0..size as u32)
        .into_iter()
        .map(|_| {
            let total = total_len.get();
            let idx = if total == 0 {
                0
            } else {
                rng.gen_range(0, total)
            };
            if total == idx || rng.gen_bool(0.5) {
                let str = {
                    let len = rng.gen_range(2, 10);
                    total_len.set(total + len);
                    rng.sample_iter(&Alphanumeric).take(len as usize).collect()
                };
                TextOp::Insert(idx, str)
            } else {
                let hi = (total - idx).min(9);
                let len = if hi == 1 { 1 } else { rng.gen_range(1, hi) };
                total_len.set(total - len);
                TextOp::Delete(idx, len)
            }
        })
        .collect()
}

fn b1_8<R: RngCore>(rng: &mut R, size: usize) -> Vec<ArrayOp> {
    let ops: Vec<ArrayOp> = (0..size)
        .map(|i| ArrayOp::Insert(i as u32, vec![rng.gen()]))
        .collect();
    ops
}

fn b1_9<R: RngCore>(rng: &mut R, size: usize) -> Vec<ArrayOp> {
    let sample: Vec<u32> = (0..size).map(|_| rng.gen()).collect();
    vec![ArrayOp::Insert(0, sample)]
}

fn b1_10<R: RngCore>(rng: &mut R, size: usize) -> Vec<ArrayOp> {
    (0..size)
        .map(|_| ArrayOp::Insert(0, vec![rng.gen()]))
        .collect()
}

fn b1_11<R: RngCore>(rng: &mut R, size: usize) -> Vec<ArrayOp> {
    (0..size)
        .map(|i| {
            let idx = rng.gen_range(0, (i as u32).max(1));
            let values = vec![rng.gen()];
            ArrayOp::Insert(idx, values)
        })
        .collect()
}

fn text_benchmark<F>(c: &mut Criterion, name: &str, gen: F)
where
    F: FnOnce(&mut StdRng, usize) -> Vec<TextOp>,
{
    let input = {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("text");
        let mut rng = StdRng::seed_from_u64(SEED);
        let ops = gen(&mut rng, N);
        (doc, txt, ops)
    };

    c.bench_with_input(
        BenchmarkId::new(name, input.2.len()),
        &input,
        |b, (doc, text, ops)| {
            b.iter(|| {
                for op in ops.iter() {
                    let mut txn = doc.transact_mut();
                    match op {
                        TextOp::Insert(idx, txt) => text.insert(&mut txn, *idx, txt),
                        TextOp::Delete(idx, len) => text.remove_range(&mut txn, *idx, *len),
                    }
                }
            });
        },
    );
}

fn array_benchmark<F>(c: &mut Criterion, name: &str, gen: F)
where
    F: FnOnce(&mut StdRng, usize) -> Vec<ArrayOp>,
{
    let input = {
        let doc = Doc::new();
        let array = doc.get_or_insert_array("text");
        let mut rng = StdRng::seed_from_u64(SEED);
        let ops = gen(&mut rng, N);
        (doc, array, ops)
    };

    c.bench_with_input(
        BenchmarkId::new(name, input.2.len()),
        &input,
        |b, (doc, array, ops)| {
            b.iter(|| {
                for op in ops.iter() {
                    let mut txn = doc.transact_mut();
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

fn concurrent_text_benchmark<F>(c: &mut Criterion, name: &str, gen: F)
where
    F: FnOnce(&mut StdRng, usize) -> Vec<(TextOp, TextOp)>,
{
    let input = {
        let d1 = Doc::new();
        let t1 = d1.get_or_insert_text("text");

        let d2 = Doc::new();
        let t2 = d2.get_or_insert_text("text");

        let mut rng = StdRng::seed_from_u64(SEED);
        let ops = gen(&mut rng, N);
        (d1, t1, d2, t2, ops)
    };

    fn apply(txn: &mut TransactionMut, txt: &TextRef, op: &TextOp) {
        match op {
            TextOp::Insert(idx, content) => txt.insert(txn, *idx, content),
            TextOp::Delete(idx, len) => txt.remove_range(txn, *idx, *len),
        }
    }

    c.bench_with_input(
        BenchmarkId::new(name, N),
        &input,
        |b, (d1, t1, d2, t2, ops)| {
            b.iter(|| {
                for (o1, o2) in ops {
                    let mut txn1 = d1.transact_mut();
                    apply(&mut txn1, t1, o1);
                    let u1 = txn1.encode_update_v1();

                    let mut txn2 = d2.transact_mut();
                    apply(&mut txn2, t2, o2);
                    let u2 = txn2.encode_update_v1();

                    txn1.apply_update(Update::decode_v1(u2.as_slice()).unwrap());
                    txn2.apply_update(Update::decode_v1(u1.as_slice()).unwrap());
                }
            });
        },
    );
}

fn b2_1<R: RngCore>(rng: &mut R, size: usize) -> Vec<(TextOp, TextOp)> {
    let s1 = rng.sample_iter(&Alphanumeric).take(size as usize).collect();
    let s2 = rng.sample_iter(&Alphanumeric).take(size as usize).collect();
    let a = TextOp::Insert(0, s1);
    let b = TextOp::Insert(0, s2);
    vec![(a, b)]
}

fn b2_2<R: RngCore>(rng: &mut R, size: usize) -> Vec<(TextOp, TextOp)> {
    (0..size as u32)
        .into_iter()
        .map(|i| {
            let ca: char = rng.sample_iter(&Alphanumeric).next().unwrap();
            let ia = rng.gen_range(0, i.max(1));

            let cb: char = rng.sample_iter(&Alphanumeric).next().unwrap();
            let ib = rng.gen_range(0, i.max(1));

            (
                TextOp::Insert(ia, ca.to_string()),
                TextOp::Insert(ib, cb.to_string()),
            )
        })
        .collect()
}

fn b2_3<R: RngCore>(rng: &mut R, size: usize) -> Vec<(TextOp, TextOp)> {
    let total_len1 = Cell::new(0u32);
    let total_len2 = Cell::new(0u32);
    (0..size as u32)
        .into_iter()
        .map(|_| {
            let t1 = total_len1.get();
            let i1 = rng.gen_range(0, t1.max(1));
            let s1 = gen_string(rng, 3, 9);
            total_len1.set(t1 + s1.len() as u32);

            let t2 = total_len2.get();
            let i2 = rng.gen_range(0, t2.max(1));
            let s2 = gen_string(rng, 3, 9);
            total_len2.set(t2 + s2.len() as u32);

            (TextOp::Insert(i1, s1), TextOp::Insert(i2, s2))
        })
        .collect()
}

fn b2_4<R: RngCore>(rng: &mut R, size: usize) -> Vec<(TextOp, TextOp)> {
    let mut total_len1 = Cell::new(0u32);
    let mut total_len2 = Cell::new(0u32);

    fn make_op<R: RngCore>(rng: &mut R, total: &mut Cell<u32>) -> TextOp {
        let t = total.get();
        let idx = rng.gen_range(0, t.max(1));
        if t == idx || rng.gen_bool(0.5) {
            let str = gen_string(rng, 3, 9);
            total.set(t + str.len() as u32);
            TextOp::Insert(idx, str)
        } else {
            let hi = (t - idx).min(9);
            let len = if hi == 1 { 1 } else { rng.gen_range(1, hi) };
            total.set(t - len);
            TextOp::Delete(idx, len)
        }
    }

    (0..size as u32)
        .into_iter()
        .map(|_| {
            let o1 = make_op(rng, &mut total_len1);
            let o2 = make_op(rng, &mut total_len2);

            (o1, o2)
        })
        .collect()
}

fn n_concurrent_map_benchmark<F>(c: &mut Criterion, name: &str, f: F)
where
    F: Fn(&MapRef, &mut TransactionMut, usize),
{
    let input: Vec<_> = (0..SQRT_N)
        .into_iter()
        .map(|i| {
            let doc = Doc::new();
            let map = doc.get_or_insert_map("map");
            let update = {
                let mut txn = doc.transact_mut();
                f(&map, &mut txn, i);
                txn.encode_update_v1()
            };
            (doc, update)
        })
        .collect();

    c.bench_with_input(BenchmarkId::new(name, SQRT_N), &input, |b, input| {
        b.iter(|| {
            let mut iter = input.into_iter();
            let (doc, _) = iter.next().unwrap();
            let mut txn = doc.transact_mut();
            while let Some((_, update)) = iter.next() {
                txn.apply_update(Update::decode_v1(update.as_slice()).unwrap());
            }
        });
    });
}

fn b3_1(map: &MapRef, txn: &mut TransactionMut, i: usize) {
    map.insert(txn, "v", i as u32);
}

fn b3_2(map: &MapRef, txn: &mut TransactionMut, i: usize) {
    let mut o = HashMap::with_capacity(2);
    o.insert("name".to_string(), i.to_string());
    o.insert("address".to_string(), "here".to_string());

    map.insert(txn, "v", o);
}

fn b3_3(map: &MapRef, txn: &mut TransactionMut, i: usize) {
    let mut str = String::with_capacity(i * SQRT_N);
    for _ in 0..SQRT_N {
        str.push_str(i.to_string().as_str());
    }
    map.insert(txn, "v", str);
}

fn b3_4(c: &mut Criterion, name: &str) {
    let input: Vec<_> = (0..SQRT_N)
        .into_iter()
        .map(|i| {
            let doc = Doc::new();
            let array = doc.get_or_insert_array("array");
            let update = {
                let mut txn = doc.transact_mut();
                array.insert(&mut txn, 0, i.to_string());
                txn.encode_update_v1()
            };
            (doc, update)
        })
        .collect();

    c.bench_with_input(BenchmarkId::new(name, SQRT_N), &input, |b, input| {
        b.iter(|| {
            let mut iter = input.into_iter();
            let (doc, _) = iter.next().unwrap();
            let mut txn = doc.transact_mut();
            while let Some((_, update)) = iter.next() {
                txn.apply_update(Update::decode_v1(update.as_slice()).unwrap());
            }
        });
    });
}

fn b4_1(c: &mut Criterion, name: &str) {
    let doc = Doc::new();
    let txt = doc.get_or_insert_text("text");
    let input = read_input("./assets/bench-input/b4-editing-trace.bin");

    c.bench_with_input(
        BenchmarkId::new(name, input.len()),
        &(doc, txt, input),
        |b, (doc, txt, input)| {
            b.iter(|| {
                for i in input {
                    let mut txn = doc.transact_mut();
                    match i {
                        TextOp::Insert(idx, chunk) => txt.insert(&mut txn, *idx, chunk),
                        TextOp::Delete(idx, len) => txt.remove_range(&mut txn, *idx, *len),
                    }
                }
            });
        },
    );
}

fn b4_2(c: &mut Criterion, name: &str) {
    let doc = Doc::new();
    let txt = doc.get_or_insert_text("text");
    let mut buf = Vec::with_capacity(400 * 1024);
    let mut f = std::fs::File::open("./assets/bench-input/b4-update.bin").unwrap();
    std::io::Read::read_to_end(&mut f, &mut buf).unwrap();

    c.bench_with_input(
        BenchmarkId::new(name, buf.len()),
        &(doc, txt, buf),
        |b, (doc, _txt, buf)| {
            b.iter(|| {
                let mut txn = doc.transact_mut();
                txn.apply_update(Update::decode_v1(buf.as_slice()).unwrap());
            });
        },
    );
}

fn read_input(fpath: &str) -> Vec<TextOp> {
    use std::fs::File;
    use yrs::updates::decoder::DecoderV1;

    let mut f = File::open(fpath).unwrap();
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut f, &mut buf).unwrap();
    let mut decoder = DecoderV1::new(Cursor::new(buf.as_slice()));
    let len: usize = decoder.read_var().unwrap();
    let mut result = Vec::with_capacity(len);
    for _ in 0..len {
        let op = {
            match decoder.read_var().unwrap() {
                1u32 => {
                    let idx = decoder.read_var().unwrap();
                    let chunk = decoder.read_string().unwrap();
                    TextOp::Insert(idx, chunk.to_string())
                }
                2u32 => {
                    let idx = decoder.read_var().unwrap();
                    let len = decoder.read_var().unwrap();
                    TextOp::Delete(idx, len)
                }
                other => panic!("unrecognized TextOp tag type: {}", other),
            }
        };
        result.push(op);
    }
    result
}

fn bench(c: &mut Criterion) {
    text_benchmark(c, "[B1.1] Append N characters", b1_1);
    text_benchmark(c, "[B1.2] Insert string of length N", b1_2);
    text_benchmark(c, "[B1.3] Prepend N characters", b1_3);
    text_benchmark(c, "[B1.4] Insert N characters at random positions", b1_4);
    text_benchmark(c, "[B1.5] Insert N words at random positions", b1_5);
    //text_benchmark(c, "[B1.6] Insert string, then delete it", b1_6);
    text_benchmark(c, "[B1.7] Insert/Delete strings at random positions", b1_7);
    array_benchmark(c, "[B1.8] Append N numbers", b1_8);
    array_benchmark(c, "[B1.9] Insert Array of N numbers", b1_9);
    array_benchmark(c, "[B1.10] Prepend N numbers", b1_10);
    array_benchmark(c, "[B1.11] Insert N numbers at random positions", b1_11);

    concurrent_text_benchmark(
        c,
        "[B2.1] Concurrently insert string of length N at index 0",
        b2_1,
    );
    concurrent_text_benchmark(
        c,
        "[B2.2] Concurrently insert N characters at random positions",
        b2_2,
    );
    concurrent_text_benchmark(
        c,
        "[B2.3] Concurrently insert N words at random positions",
        b2_3,
    );
    concurrent_text_benchmark(c, "[B2.4] Concurrently insert & delete", b2_4);

    n_concurrent_map_benchmark(
        c,
        "[B3.1] 20√N clients concurrently set number in Map",
        b3_1,
    );
    n_concurrent_map_benchmark(
        c,
        "[B3.2] 20√N clients concurrently set Object in Map",
        b3_2,
    );
    n_concurrent_map_benchmark(
        c,
        "[B3.3] 20√N clients concurrently set String in Map",
        b3_3,
    );
    b3_4(c, "[B3.4] 20√N clients concurrently insert text in Array");
    b4_2(c, "[B4.2] Apply real-world document snapshot of size");
    b4_1(c, "[B4.1] Apply real-world editing dataset");
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench,
}
criterion_main!(benches);
