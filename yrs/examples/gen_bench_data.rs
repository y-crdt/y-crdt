use yrs::*;

use lib0::decoding::Cursor;
use rand::distributions::Alphanumeric;
use rand::prelude::ThreadRng;
use rand::{thread_rng, Rng};
use std::cell::Cell;
use std::fmt::Debug;
use std::fs::File;
use std::io::{Read, Write};
use yrs::updates::decoder::{Decode, Decoder, DecoderV1};
use yrs::updates::encoder::{Encode, Encoder, EncoderV1};
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

fn b1_1(size: usize) -> Vec<TextOp> {
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

fn b1_2(size: usize) -> Vec<TextOp> {
    let s: String = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(size as usize)
        .collect();
    vec![TextOp::Insert(0, s)]
}

fn b1_3(size: usize) -> Vec<TextOp> {
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

fn b1_4(size: usize) -> Vec<TextOp> {
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

fn gen_string(rng: &mut ThreadRng, min: usize, max: usize) -> String {
    let len = rng.gen_range(min, max);
    rng.sample_iter(&Alphanumeric).take(len).collect()
}

fn b1_5(size: usize) -> Vec<TextOp> {
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

fn b1_6(size: usize) -> Vec<TextOp> {
    let s: String = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(size as usize)
        .collect();
    let len = s.len() as u32;
    vec![TextOp::Insert(0, s), TextOp::Delete(0, len)]
}

fn b1_7(size: usize) -> Vec<TextOp> {
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

fn b1_8(size: usize) -> Vec<ArrayOp> {
    let mut rng = thread_rng();
    let ops: Vec<ArrayOp> = (0..size)
        .map(|i| ArrayOp::Insert(i as u32, vec![rng.gen()]))
        .collect();
    ops
}

fn b1_9(size: usize) -> Vec<ArrayOp> {
    let mut rng = thread_rng();
    let sample: Vec<u32> = (0..size).map(|_| rng.gen()).collect();
    vec![ArrayOp::Insert(0, sample)]
}

fn b1_10(size: usize) -> Vec<ArrayOp> {
    let mut rng = thread_rng();
    (0..size)
        .map(|i| ArrayOp::Insert(0, vec![rng.gen()]))
        .collect()
}

fn b1_11(size: usize) -> Vec<ArrayOp> {
    let mut rng = thread_rng();
    (0..size)
        .map(|i| {
            let idx = rng.gen_range(0, (i as u32).max(1));
            let values = vec![rng.gen()];
            ArrayOp::Insert(idx, values)
        })
        .collect()
}

fn generate<T: Encode + Decode + Eq + Debug>(fname: &str, data: Vec<T>) {
    let mut f = File::create(fname).unwrap();
    let mut encoder = EncoderV1::new();
    encode_vec(&data, &mut encoder);
    let i = encoder.to_vec();
    f.write_all(i.as_slice()).unwrap();
    drop(f);

    // verify
    let mut f = File::open(fname).unwrap();
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    assert_eq!(i, buf);

    let mut decoder = DecoderV1::new(Cursor::new(buf.as_slice()));
    let actual = decode_vec(&mut decoder);

    assert_eq!(data, actual);
}

fn main() {
    generate("./yrs/benches/input/b-1-1-6000.bin", b1_1(6000));
    generate("./yrs/benches/input/b-1-2-6000.bin", b1_2(6000));
    generate("./yrs/benches/input/b-1-3-6000.bin", b1_3(6000));
    generate("./yrs/benches/input/b-1-4-6000.bin", b1_4(6000));
    generate("./yrs/benches/input/b-1-5-6000.bin", b1_5(6000));
    generate("./yrs/benches/input/b-1-6-6000.bin", b1_6(6000));
    generate("./yrs/benches/input/b-1-7-6000.bin", b1_7(6000));
    generate("./yrs/benches/input/b-1-8-6000.bin", b1_8(6000));
    generate("./yrs/benches/input/b-1-9-6000.bin", b1_9(6000));
    generate("./yrs/benches/input/b-1-10-6000.bin", b1_10(6000));
    generate("./yrs/benches/input/b-1-11-6000.bin", b1_11(6000));
}
