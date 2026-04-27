use criterion::*;
use yrs::{DeleteSet, ID};

const CLIENT_A: u64 = 1;
const CLIENT_B: u64 = 2;

// Base set: 10 ranges for one client, spaced with gaps.
// Pattern: [0..5), [10..15), [20..25), ..., [90..95).
fn build_base_10() -> DeleteSet {
    let mut ds = DeleteSet::new();
    for i in 0..10u32 {
        ds.insert(ID::new(CLIENT_A, i * 10), 5);
    }
    ds
}

// Scenario 1: starting from a 10-range set, insert one new range that overlaps the middle
fn bench_insert_one_then_squash(c: &mut Criterion) {
    c.bench_function("id_set/insert_1_then_squash", |b| {
        b.iter_batched(
            build_base_10,
            |mut ds| {
                // [47..53) overlaps [40..45) tail and [50..55) head — bridges them.
                ds.insert(ID::new(CLIENT_A, 47), 6);
                ds
            },
            BatchSize::SmallInput,
        );
    });
}

// Scenario 2: starting from a 10-range set, insert five new ranges
// (mix of disjoint, adjacent, overlapping, and extending past the tail).
fn bench_insert_five_then_squash(c: &mut Criterion) {
    c.bench_function("id_set/insert_5_then_squash", |b| {
        b.iter_batched(
            build_base_10,
            |mut ds| {
                // [7..11)   — bridges [0..5) and [10..15) via the gap.
                // [23..27)  — overlaps [20..25).
                // [47..53)  — bridges [40..45) and [50..55).
                // [75..83)  — overlaps [70..75) and [80..85).
                // [100..110)— extends past the tail.
                ds.insert(ID::new(CLIENT_A, 7), 4);
                ds.insert(ID::new(CLIENT_A, 23), 4);
                ds.insert(ID::new(CLIENT_A, 47), 6);
                ds.insert(ID::new(CLIENT_A, 75), 8);
                ds.insert(ID::new(CLIENT_A, 100), 10);
                ds
            },
            BatchSize::SmallInput,
        );
    });
}

// Scenario 3: merge two DeleteSets in three flavours.
//
// disjoint_same_client: same client id; ranges in B sit far past the
// ranges in A — same per-client list to walk, but no overlap.
//
// unique_clients: A and B carry different client ids — merge degenerates
// to a HashMap union with no per-IdRange merging.
//
// overlapping: same client id; every range in B overlaps a range in A,
// forcing a real merge per range.
fn bench_merge(c: &mut Criterion) {
    let build_b_disjoint = || {
        let mut ds = DeleteSet::new();
        for i in 0..10u32 {
            ds.insert(ID::new(CLIENT_A, 200 + i * 10), 5);
        }
        ds
    };

    let build_b_unique = || {
        let mut ds = DeleteSet::new();
        for i in 0..10u32 {
            ds.insert(ID::new(CLIENT_B, i * 10), 5);
        }
        ds
    };

    let build_b_overlapping = || {
        let mut ds = DeleteSet::new();
        for i in 0..10u32 {
            // A has [i*10 .. i*10 + 5); B has [i*10 + 3 .. i*10 + 8).
            ds.insert(ID::new(CLIENT_A, i * 10 + 3), 5);
        }
        ds
    };

    let mut g = c.benchmark_group("id_set/merge");

    g.bench_function("disjoint_same_client", |b| {
        b.iter_batched(
            || (build_base_10(), build_b_disjoint()),
            |(mut a, b)| {
                a.merge_with(b);
                a
            },
            BatchSize::SmallInput,
        );
    });

    g.bench_function("unique_clients", |b| {
        b.iter_batched(
            || (build_base_10(), build_b_unique()),
            |(mut a, b)| {
                a.merge_with(b);
                a
            },
            BatchSize::SmallInput,
        );
    });

    g.bench_function("overlapping", |b| {
        b.iter_batched(
            || (build_base_10(), build_b_overlapping()),
            |(mut a, b)| {
                a.merge_with(b);
                a
            },
            BatchSize::SmallInput,
        );
    });

    g.finish();
}

// Scenario 4: exclude one DeleteSet from another in three flavours, mirroring `bench_merge`.
fn bench_exclude(c: &mut Criterion) {
    let build_b_disjoint = || {
        let mut ds = DeleteSet::new();
        for i in 0..10u32 {
            ds.insert(ID::new(CLIENT_A, 200 + i * 10), 5);
        }
        ds
    };

    let build_b_unique = || {
        let mut ds = DeleteSet::new();
        for i in 0..10u32 {
            ds.insert(ID::new(CLIENT_B, i * 10), 5);
        }
        ds
    };

    let build_b_overlapping = || {
        let mut ds = DeleteSet::new();
        for i in 0..10u32 {
            // A has [i*10 .. i*10 + 5); B has [i*10 + 3 .. i*10 + 8) — carves the right edge of every range in A.
            ds.insert(ID::new(CLIENT_A, i * 10 + 3), 5);
        }
        ds
    };

    let mut g = c.benchmark_group("id_set/exclude");

    g.bench_function("disjoint_same_client", |b| {
        b.iter_batched(
            || (build_base_10(), build_b_disjoint()),
            |(mut a, b)| {
                a.diff_with(&b);
                a
            },
            BatchSize::SmallInput,
        );
    });

    g.bench_function("unique_clients", |b| {
        b.iter_batched(
            || (build_base_10(), build_b_unique()),
            |(mut a, b)| {
                a.diff_with(&b);
                a
            },
            BatchSize::SmallInput,
        );
    });

    g.bench_function("overlapping", |b| {
        b.iter_batched(
            || (build_base_10(), build_b_overlapping()),
            |(mut a, b)| {
                a.diff_with(&b);
                a
            },
            BatchSize::SmallInput,
        );
    });

    g.finish();
}

// Scenario 5: intersect two DeleteSets in three flavours.
fn bench_intersect(c: &mut Criterion) {
    let build_b_disjoint = || {
        let mut ds = DeleteSet::new();
        for i in 0..10u32 {
            ds.insert(ID::new(CLIENT_A, 200 + i * 10), 5);
        }
        ds
    };

    let build_b_unique = || {
        let mut ds = DeleteSet::new();
        for i in 0..10u32 {
            ds.insert(ID::new(CLIENT_B, i * 10), 5);
        }
        ds
    };

    let build_b_overlapping = || {
        let mut ds = DeleteSet::new();
        for i in 0..10u32 {
            // B's [i*10 + 3 .. i*10 + 8) overlaps the right portion of every range in A.
            ds.insert(ID::new(CLIENT_A, i * 10 + 3), 5);
        }
        ds
    };

    let mut g = c.benchmark_group("id_set/intersect");

    g.bench_function("disjoint_same_client", |b| {
        b.iter_batched(
            || (build_base_10(), build_b_disjoint()),
            |(mut a, b)| {
                a.intersect_with(&b);
                a
            },
            BatchSize::SmallInput,
        );
    });

    g.bench_function("unique_clients", |b| {
        b.iter_batched(
            || (build_base_10(), build_b_unique()),
            |(mut a, b)| {
                a.intersect_with(&b);
                a
            },
            BatchSize::SmallInput,
        );
    });

    g.bench_function("overlapping", |b| {
        b.iter_batched(
            || (build_base_10(), build_b_overlapping()),
            |(mut a, b)| {
                a.intersect_with(&b);
                a
            },
            BatchSize::SmallInput,
        );
    });

    g.finish();
}

criterion_group! {
    name = id_set_benches;
    config = Criterion::default();
    targets = bench_insert_one_then_squash, bench_insert_five_then_squash, bench_merge, bench_exclude, bench_intersect,
}
criterion_main!(id_set_benches);
