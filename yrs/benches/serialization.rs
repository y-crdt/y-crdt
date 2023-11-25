use criterion::{black_box, criterion_group, criterion_main, Criterion, SamplingMode};
use yrs::Any;
use yrs::encoding::read::{Cursor, Read};
use yrs::encoding::write::Write;

const BENCHMARK_SIZE: u32 = 100000;

fn bench_encoding(c: &mut Criterion) {
    let mut encoding_group = c.benchmark_group("encoding");
    encoding_group.sampling_mode(SamplingMode::Flat);

    encoding_group.bench_function("var_int (64 bit)", |b| {
        b.iter(|| {
            let mut encoder = Vec::with_capacity(BENCHMARK_SIZE as usize * 8);
            for i in 0..(BENCHMARK_SIZE as i64) {
                encoder.write_var(i);
            }
            let mut decoder = Cursor::from(&encoder);
            for i in 0..(BENCHMARK_SIZE as i64) {
                let num: i64 = decoder.read_var().unwrap();
                assert_eq!(num, i);
            }
        })
    });

    encoding_group.bench_function("var_uint (32 bit)", |b| {
        b.iter(|| {
            let mut encoder = Vec::with_capacity(BENCHMARK_SIZE as usize * 8);
            for i in 0..BENCHMARK_SIZE {
                encoder.write_var(i);
            }
            let mut decoder = Cursor::from(&encoder);
            for i in 0..BENCHMARK_SIZE {
                let num: u32 = decoder.read_var().unwrap();
                assert_eq!(num, i);
            }
        })
    });

    encoding_group.bench_function("uint32", |b| {
        b.iter(|| {
            let mut encoder = Vec::with_capacity(BENCHMARK_SIZE as usize * 8);
            for i in 0..BENCHMARK_SIZE {
                encoder.write_u32(i);
            }
            let mut decoder = Cursor::from(&encoder);
            for i in 0..BENCHMARK_SIZE {
                let num: u32 = decoder.read_u32().unwrap();
                assert_eq!(num, i);
            }
        })
    });

    encoding_group.bench_function("var_uint (64 bit)", |b| {
        b.iter(|| {
            let mut encoder = Vec::with_capacity(BENCHMARK_SIZE as usize * 8);
            for i in 0..(BENCHMARK_SIZE as u64) {
                encoder.write_var(i);
            }
            let mut decoder = Cursor::from(&encoder);
            for i in 0..(BENCHMARK_SIZE as u64) {
                let num: u64 = decoder.read_var().unwrap();
                assert_eq!(num, i);
            }
        })
    });

    encoding_group.bench_function("uint64", |b| {
        b.iter(|| {
            let mut encoder = Vec::with_capacity(BENCHMARK_SIZE as usize * 8);
            for i in 0..(BENCHMARK_SIZE as u64) {
                encoder.write_u64(i)
            }
            let mut decoder = Cursor::from(&encoder);
            for i in 0..(BENCHMARK_SIZE as u64) {
                let num = decoder.read_u64().unwrap();
                assert_eq!(num, i);
            }
        })
    });
}

fn bench_serialization(c: &mut Criterion) {
    use std::collections::HashMap;

    let any = Any::from(HashMap::from([
        ("bool".into(), Any::from(true)),
        ("int".into(), Any::from(1)),
        ("negativeInt".into(), Any::from(-1)),
        ("maxInt".into(), Any::from(i64::MAX)),
        ("minInt".into(), Any::from(i64::MIN)),
        ("realNumber".into(), Any::from(-123.2387f64)),
        ("maxNumber".into(), Any::from(f64::MIN)),
        ("minNumber".into(), Any::from(f64::MAX)),
        ("null".into(), Any::Null),
        (
            "map".into(),
            Any::from(HashMap::from([
                ("bool".into(), Any::from(true)),
                ("int".into(), Any::from(1)),
                ("negativeInt".into(), Any::from(-1)),
                ("maxInt".into(), Any::from(i64::MAX)),
                ("minInt".into(), Any::from(i64::MIN)),
                ("realNumber".into(), Any::from(-123.2387f64)),
                ("maxNumber".into(), Any::from(f64::MIN)),
                ("minNumber".into(), Any::from(f64::MAX)),
                ("null".into(), Any::Null),
            ])),
        ),
        (
            "key6".into(),
            Any::from(vec![
                Any::from(true),
                Any::from(1),
                Any::from(-1),
                Any::from(i64::MAX),
                Any::from(i64::MIN),
                Any::from(-123.2387f64),
                Any::from(f64::MIN),
                Any::from(f64::MAX),
                Any::Null,
            ]),
        ),
    ]));

    let any_json = serde_json::to_string(&any).unwrap();

    let mut serde_group = c.benchmark_group("serde");
    serde_group.bench_function("Any-JSON roundtrip", |b| {
        b.iter(|| {
            black_box(serde_json::from_str::<Any>(&serde_json::to_string(&any).unwrap()).unwrap());
        })
    });

    serde_group.bench_function("Any serialize", |b| {
        b.iter(|| {
            black_box(serde_json::to_string(&any).unwrap());
        })
    });

    serde_group.bench_function("Any deserialize", |b| {
        b.iter(|| {
            black_box(serde_json::to_string(&any).unwrap());
        })
    });

    serde_group.finish();

    let mut custom_group = c.benchmark_group("custom json serialization");

    custom_group.bench_function("Any-JSON roundtrip", |b| {
        b.iter(|| {
            let mut roundtrip = String::new();
            any.to_json(&mut roundtrip);

            black_box(Any::from_json(&roundtrip));
        })
    });

    custom_group.bench_function("Any serialize", |b| {
        b.iter(|| {
            let mut roundtrip = String::new();
            black_box(any.to_json(&mut roundtrip));
        })
    });

    custom_group.bench_function("Any deserialize", |b| {
        b.iter(|| {
            black_box(Any::from_json(&any_json));
        })
    });

    custom_group.finish();
}

criterion_group!(serialization, bench_encoding, bench_serialization);
criterion_main!(serialization);
