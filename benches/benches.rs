use criterion::{criterion_group, criterion_main, Criterion};
use yrs::*;

const ITERATIONS: u32 = 1000000;

fn ytext_insert() {
    let doc = Doc::new();
    let t = doc.get_type("");
    let mut tr = doc.transact();
    t.insert(&mut tr, 0, 'x');
    for _ in 1..ITERATIONS {
        t.insert(&mut tr, 0, 'a')
    }
}

pub struct TestItem {
    pub id: ID,
    pub left: Option<StructPtr>,
    pub right: Option<StructPtr>,
    pub origin: Option<ID>,
    pub right_origin: Option<ID>,
    pub content: char,
    pub parent: TypePtr
}

fn gen_vec_perf () {
    let mut vec: Vec<TestItem> = Vec::new();
    for _ in 0..(ITERATIONS) {
        vec.push(TestItem {
            id: ID { client: 0, clock: 0 },
            left: None,
            right: None,
            origin: None,
            right_origin: None,
            content: 'a',
            parent: TypePtr::Named(1),
        });
    }
}

fn gen_vec_perf_pred () {
    let mut vec: Vec<TestItem> = Vec::with_capacity(ITERATIONS as usize);
    for _ in 0..(ITERATIONS) {
        vec.push(TestItem {
            id: ID { client: 0, clock: 0 },
            left: None,
            right: None,
            origin: None,
            right_origin: None,
            content: 'a',
            parent: TypePtr::Named(1),
        });
    }
}


const MULT_STRUCT_SIZE: u32 = 7;

fn gen_vec_perf_optimal () {
    let mut vec: Vec<u64> = Vec::new();
    for i in 0..((ITERATIONS * MULT_STRUCT_SIZE) as u64) {
        vec.push(i);
    }
}

fn gen_vec_perf_pred_optimal () {
    let mut vec: Vec<u64> = Vec::with_capacity((ITERATIONS * MULT_STRUCT_SIZE) as usize);
    for i in 0..((ITERATIONS * MULT_STRUCT_SIZE) as u64) {
        vec.push(i);
    }
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("ytext insert", |b| b.iter(|| ytext_insert()));
    c.bench_function("gen vec perf", |b| b.iter(|| gen_vec_perf()));
    c.bench_function("gen vec perf pred", |b| b.iter(|| gen_vec_perf_pred()));
    c.bench_function("gen vec perf optimal", |b| b.iter(|| gen_vec_perf_optimal()));
    c.bench_function("gen vec perf pred optimal", |b| b.iter(|| gen_vec_perf_pred_optimal()));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
