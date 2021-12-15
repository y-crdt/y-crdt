use criterion::*;
use rand::distributions::Alphanumeric;
use rand::prelude::StdRng;
use rand::{Rng, RngCore, SeedableRng};
use std::cell::Cell;
use yrs::Doc;

const N: usize = 6000;
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
        .map(|(i, str)| TextOp::Insert(0, str))
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
    let mut total_len = Cell::new(0u32);
    (0..size as u32)
        .into_iter()
        .map(|i| {
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
        .map(|i| ArrayOp::Insert(0, vec![rng.gen()]))
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
        let mut txn = doc.transact();
        let txt = txn.get_text("text");
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

fn array_benchmark<F>(c: &mut Criterion, name: &str, gen: F)
where
    F: FnOnce(&mut StdRng, usize) -> Vec<ArrayOp>,
{
    let input = {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let array = txn.get_array("text");
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

fn bench(c: &mut Criterion) {
    text_benchmark(c, "[B1.1] Append N characters", b1_1);
    text_benchmark(c, "[B1.2] Insert string of length N", b1_2);
    text_benchmark(c, "[B1.3] Prepend N characters", b1_3);
    text_benchmark(c, "[B1.4] Insert N characters at random positions", b1_4);
    text_benchmark(c, "[B1.5] Insert N words at random positions", b1_5);
    text_benchmark(c, "[B1.6] Insert string, then delete it", b1_6);
    text_benchmark(c, "[B1.7] Insert/Delete strings at random positions", b1_7);
    array_benchmark(c, "[B1.8] Append N numbers", b1_8);
    array_benchmark(c, "[B1.9] Insert Array of N numbers", b1_9);
    array_benchmark(c, "[B1.10] Prepend N numbers", b1_10);
    array_benchmark(c, "[B1.11] Insert N numbers at random positions", b1_11);
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench,
}
criterion_main!(benches);
