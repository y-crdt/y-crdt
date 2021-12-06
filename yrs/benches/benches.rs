use criterion::*;
use rand::distributions::Alphanumeric;
use rand::prelude::ThreadRng;
use rand::{thread_rng, Rng};
use std::cell::Cell;
use yrs::{Array, Doc, Text};

enum TextOp {
    Insert(u32, String),
    Delete(u32, u32),
}

enum ArrayOp {
    Insert(u32, Vec<i32>),
    Delete(u32, u32),
}

fn text_bench(doc: &Doc, text: &Text, ops: &Vec<TextOp>) {
    for op in ops.iter() {
        let mut txn = doc.transact();
        match op {
            TextOp::Insert(idx, txt) => text.insert(&mut txn, *idx, txt),
            TextOp::Delete(idx, len) => text.remove_range(&mut txn, *idx, *len),
        }
    }
}

fn array_bench(doc: &Doc, array: &Array, ops: &Vec<ArrayOp>) {
    for op in ops.iter() {
        let mut txn = doc.transact();
        match op {
            ArrayOp::Insert(idx, values) => array.insert_range(&mut txn, *idx, values.clone()),
            ArrayOp::Delete(idx, len) => array.remove_range(&mut txn, *idx, *len),
        }
    }
}

fn b1_1(c: &mut Criterion) {
    let mut group = c.benchmark_group("[B1.1] Append N characters");

    fn gen_ops(size: usize) -> Vec<TextOp> {
        let sample: Vec<_> = thread_rng()
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

    for size in [1000, 3000, 6000] {
        let input = {
            let doc = Doc::new();
            let mut txn = doc.transact();
            let txt = txn.get_text("text");
            (doc, txt, gen_ops(size as usize))
        };

        group.throughput(Throughput::Elements(size));
        group.bench_with_input(
            format!("[B1.1] Append {} characters", size),
            &input,
            |b, (doc, text, ops)| {
                b.iter(|| text_bench(doc, text, ops));
            },
        );
    }
    group.finish();
}

fn b1_2(c: &mut Criterion) {
    let mut group = c.benchmark_group("[B1.2] Insert string of length N");

    for size in [1000, 3000, 6000] {
        let input = {
            let doc = Doc::new();
            let mut txn = doc.transact();
            let txt = txn.get_text("text");
            let s: String = thread_rng()
                .sample_iter(&Alphanumeric)
                .take(size as usize)
                .collect();
            (doc, txt, vec![TextOp::Insert(0, s)])
        };

        group.throughput(Throughput::Elements(size));
        group.bench_with_input(
            format!("[B1.2] Insert string of length {}", size),
            &input,
            |b, (doc, txt, ops)| {
                b.iter(|| text_bench(doc, txt, ops));
            },
        );
    }
    group.finish();
}

fn b1_3(c: &mut Criterion) {
    let mut group = c.benchmark_group("[B1.3] Prepend N characters");

    fn gen_ops(size: usize) -> Vec<TextOp> {
        let sample: Vec<_> = thread_rng()
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

    for size in [1000, 3000, 6000] {
        let input = {
            let doc = Doc::new();
            let mut txn = doc.transact();
            let txt = txn.get_text("text");
            (doc, txt, gen_ops(size as usize))
        };

        group.throughput(Throughput::Elements(size));
        group.bench_with_input(
            format!("[B1.3] Prepend {} characters", size),
            &input,
            |b, (doc, text, ops)| {
                b.iter(|| text_bench(doc, text, ops));
            },
        );
    }
    group.finish();
}

fn b1_4(c: &mut Criterion) {
    let mut group = c.benchmark_group("[B1.4] Insert N characters at random positions");

    fn gen_ops(size: usize) -> Vec<TextOp> {
        let mut rng = thread_rng();
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

    for size in [1000, 3000, 6000] {
        let input = {
            let doc = Doc::new();
            let mut txn = doc.transact();
            let txt = txn.get_text("text");
            (doc, txt, gen_ops(size as usize))
        };

        group.throughput(Throughput::Elements(size));
        group.bench_with_input(
            format!("[B1.4] Insert {} characters at random positions", size),
            &input,
            |b, (doc, text, ops)| {
                b.iter(|| text_bench(doc, text, ops));
            },
        );
    }
    group.finish();
}

fn b1_5(c: &mut Criterion) {
    let mut group = c.benchmark_group("[B1.5] Insert N words at random positions");

    fn gen_string(rng: &mut ThreadRng, min: usize, max: usize) -> String {
        let len = rng.gen_range(min, max);
        rng.sample_iter(&Alphanumeric).take(len).collect()
    }

    fn gen_ops(size: usize) -> Vec<TextOp> {
        let mut rng = thread_rng();
        (0..size as u32)
            .into_iter()
            .map(|i| {
                let str = gen_string(&mut rng, 2, 10);
                let idx = rng.gen_range(0, i.max(1));
                TextOp::Insert(idx, str)
            })
            .collect()
    }

    for size in [1000, 3000, 6000] {
        let input = {
            let doc = Doc::new();
            let mut txn = doc.transact();
            let txt = txn.get_text("text");
            (doc, txt, gen_ops(size as usize))
        };

        group.throughput(Throughput::Elements(size));
        group.bench_with_input(
            format!("[B1.5] Insert {} words at random positions", size),
            &input,
            |b, (doc, text, ops)| {
                b.iter(|| text_bench(doc, text, ops));
            },
        );
    }
    group.finish();
}

fn b1_6(c: &mut Criterion) {
    let mut group = c.benchmark_group("[B1.6] Insert string, then delete it");

    for size in [1000, 3000, 6000] {
        let input = {
            let doc = Doc::new();
            let mut txn = doc.transact();
            let txt = txn.get_text("text");
            let s: String = thread_rng()
                .sample_iter(&Alphanumeric)
                .take(size as usize)
                .collect();
            let len = s.len() as u32;
            (doc, txt, vec![TextOp::Insert(0, s), TextOp::Delete(0, len)])
        };

        group.throughput(Throughput::Elements(size));
        group.bench_with_input(
            format!("[B1.6] Insert string of length {}, then delete it", size),
            &input,
            |b, (doc, text, ops)| {
                b.iter(|| text_bench(doc, text, ops));
            },
        );
    }
    group.finish();
}

fn b1_7(c: &mut Criterion) {
    let mut group = c.benchmark_group("[B1.7] Insert/Delete strings at random positions");

    fn gen_ops(size: usize) -> Vec<TextOp> {
        let mut rng = thread_rng();
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

    for size in [1000, 3000, 6000] {
        let input = {
            let doc = Doc::new();
            let mut txn = doc.transact();
            let txt = txn.get_text("text");
            (doc, txt, gen_ops(size as usize))
        };

        group.throughput(Throughput::Elements(size));
        group.bench_with_input(
            format!("[B1.7] Insert/Delete {} strings at random positions", size),
            &input,
            |b, (doc, text, ops)| {
                b.iter(|| text_bench(doc, text, ops));
            },
        );
    }
    group.finish();
}

fn b1_8(c: &mut Criterion) {
    let mut group = c.benchmark_group("[B1.8] Append N numbers");

    for size in [1000, 3000, 6000] {
        let input = {
            let mut rng = thread_rng();
            let doc = Doc::new();
            let mut txn = doc.transact();
            let array = txn.get_array("text");
            let ops: Vec<ArrayOp> = (0..size)
                .map(|i| ArrayOp::Insert(i, vec![rng.gen()]))
                .collect();
            (doc, array, ops)
        };

        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(
            format!("[B1.8] Append {} numbers", size),
            &input,
            |b, (doc, array, ops)| {
                b.iter(|| array_bench(doc, array, ops));
            },
        );
    }
    group.finish();
}

fn b1_9(c: &mut Criterion) {
    let mut group = c.benchmark_group("[B1.9] Insert Array of N numbers");

    for size in [1000, 3000, 6000] {
        let input = {
            let mut rng = thread_rng();
            let doc = Doc::new();
            let mut txn = doc.transact();
            let array = txn.get_array("text");
            let sample: Vec<i32> = (0..size).map(|_| rng.gen()).collect();
            (doc, array, vec![ArrayOp::Insert(0, sample)])
        };

        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(
            format!("[B1.9] Insert Array of {} numbers", size),
            &input,
            |b, (doc, array, ops)| {
                b.iter(|| array_bench(doc, array, ops));
            },
        );
    }
    group.finish();
}

fn b1_10(c: &mut Criterion) {
    let mut group = c.benchmark_group("[B1.10] Prepend N numbers");

    for size in [1000, 3000, 6000] {
        let input = {
            let mut rng = thread_rng();
            let doc = Doc::new();
            let mut txn = doc.transact();
            let array = txn.get_array("text");
            let ops: Vec<ArrayOp> = (0..size)
                .map(|i| ArrayOp::Insert(0, vec![rng.gen()]))
                .collect();
            (doc, array, ops)
        };

        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(
            format!("[B1.10] Prepend {} numbers", size),
            &input,
            |b, (doc, array, ops)| {
                b.iter(|| array_bench(doc, array, ops));
            },
        );
    }
    group.finish();
}

fn b1_11(c: &mut Criterion) {
    let mut group = c.benchmark_group("[B1.11] Insert N numbers at random positions");

    for size in [1000, 3000, 6000] {
        let input = {
            let mut rng = thread_rng();
            let doc = Doc::new();
            let mut txn = doc.transact();
            let array = txn.get_array("text");
            let ops: Vec<ArrayOp> = (0..size)
                .map(|i| {
                    let idx = rng.gen_range(0, i.max(1));
                    let values = vec![rng.gen()];
                    ArrayOp::Insert(idx, values)
                })
                .collect();
            (doc, array, ops)
        };

        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(
            format!("[B1.11] Insert {} numbers at random positions", size),
            &input,
            |b, (doc, array, ops)| {
                b.iter(|| array_bench(doc, array, ops));
            },
        );
    }
    group.finish();
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

criterion_group!(benches, bench);
criterion_main!(benches);
