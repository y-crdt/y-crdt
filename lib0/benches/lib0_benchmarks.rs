use criterion::{criterion_group, criterion_main, Criterion, SamplingMode};
use lib0::decoding::Decoder;
use lib0::encoding::Encoder;

const BENCHMARK_SIZE: u32 = 100000;

fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("encoding");
    group.sampling_mode(SamplingMode::Flat);

    group.bench_function("var_int (64 bit)", |b| {
        b.iter(|| {
            let mut encoder = Encoder::new();
            for i in 0..(BENCHMARK_SIZE as i64) {
                encoder.write_var_int(i);
            }
            let mut decoder = Decoder::new(&encoder.buf);
            for i in 0..(BENCHMARK_SIZE as i64) {
                let num: i64 = decoder.read_var_int();
                assert_eq!(num, i);
            }
        })
    });

    group.bench_function("var_uint (32 bit)", |b| {
        b.iter(|| {
            let mut encoder = Encoder::new();
            for i in 0..BENCHMARK_SIZE {
                encoder.write_var_uint(i);
            }
            let mut decoder = Decoder::new(&encoder.buf);
            for i in 0..BENCHMARK_SIZE {
                let num: u32 = decoder.read_var_uint();
                assert_eq!(num, i);
            }
        })
    });

    group.bench_function("uint32", |b| {
        b.iter(|| {
            let mut encoder = Encoder::new();
            for i in 0..BENCHMARK_SIZE {
                encoder.write_uint32(i);
            }
            let mut decoder = Decoder::new(&encoder.buf);
            for i in 0..BENCHMARK_SIZE {
                let num: u32 = decoder.read_uint32();
                assert_eq!(num, i);
            }
        })
    });

    group.bench_function("var_uint (64 bit)", |b| {
        b.iter(|| {
            let mut encoder = Encoder::new();
            for i in 0..(BENCHMARK_SIZE as u64) {
                encoder.write_var_uint(i);
            }
            let mut decoder = Decoder::new(&encoder.buf);
            for i in 0..(BENCHMARK_SIZE as u64) {
                let num: u64 = decoder.read_var_uint();
                assert_eq!(num, i);
            }
        })
    });

    group.bench_function("uint64", |b| {
        b.iter(|| {
            let mut encoder = Encoder::new();
            for i in 0..(BENCHMARK_SIZE as u64) {
                encoder.write_big_uint64(i)
            }
            let mut decoder = Decoder::new(&encoder.buf);
            for i in 0..(BENCHMARK_SIZE as u64) {
                let num = decoder.read_big_uint64();
                assert_eq!(num, i);
            }
        })
    });

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
