#![allow(dead_code)]
/*
   ORIGINAL CODE AUTHORED BY Seph Gentle
   available at: https://github.com/josephg/editing-traces
*/

use flate2::bufread::GzDecoder;
use serde::Deserialize;
use std::fs::File;
use std::io::{BufReader, Read};

/// This file contains some simple helpers for loading test data. Its used by benchmarking and
/// testing code.

/// (position, delete length, insert content).
#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct TestPatch(pub usize, pub usize, pub String);

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct TestTxn {
    // time: String, // ISO String. Unused.
    pub patches: Vec<TestPatch>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct TestData {
    #[serde(default)]
    pub using_byte_positions: bool,

    #[serde(rename = "startContent")]
    pub start_content: String,
    #[serde(rename = "endContent")]
    pub end_content: String,

    pub txns: Vec<TestTxn>,
}

impl TestData {
    pub fn len(&self) -> usize {
        self.txns.iter().map(|txn| txn.patches.len()).sum::<usize>()
    }

    pub fn is_empty(&self) -> bool {
        !self.txns.iter().any(|txn| !txn.patches.is_empty())
    }

    /// This method returns a clone of the testing data using byte offsets instead of codepoint
    /// indexes.
    pub fn chars_to_bytes(&self) -> Self {
        assert_eq!(false, self.using_byte_positions);

        let mut r = ropey::Rope::new();

        Self {
            using_byte_positions: true,
            start_content: self.start_content.clone(),
            end_content: self.end_content.clone(),
            txns: self
                .txns
                .iter()
                .map(|txn| {
                    TestTxn {
                        patches: txn
                            .patches
                            .iter()
                            .map(|TestPatch(pos_chars, del_chars, ins)| {
                                let pos_bytes = r.char_to_byte(*pos_chars);
                                // if *pos_chars != pos_bytes {
                                //     println!("Converted position {} to {}", *pos_chars, pos_bytes);
                                // }
                                let del_bytes = if *del_chars > 0 {
                                    let del_end_bytes = r.char_to_byte(pos_chars + *del_chars);
                                    r.remove(*pos_chars..*pos_chars + *del_chars);
                                    del_end_bytes - pos_bytes
                                } else {
                                    0
                                };
                                if !ins.is_empty() {
                                    r.insert(*pos_chars, ins);
                                }

                                TestPatch(pos_bytes, del_bytes, ins.clone())
                            })
                            .collect(),
                    }
                })
                .collect(),
        }
    }

    pub fn patches(&self) -> impl Iterator<Item = &TestPatch> {
        self.txns.iter().flat_map(|txn| txn.patches.iter())
    }
}

/// Load the testing data at the specified file. If the filename ends in .gz, it will be
/// transparently uncompressed.
///
/// This method panics if the file does not exist, or is corrupt. It'd be better to have a try_
/// variant of this method, but given this is mostly for benchmarking and testing, I haven't felt
/// the need to write that code.
pub fn load_testing_data(filename: &str) -> TestData {
    // let start = SystemTime::now();
    // let mut file = File::open("benchmark_data/automerge-paper.json.gz").unwrap();
    let file = File::open(filename).unwrap();

    let mut reader = BufReader::new(file);
    // We could pass the GzDecoder straight to serde, but it makes it way slower to parse for
    // some reason.
    let mut raw_json = vec![];

    if filename.ends_with(".gz") {
        let mut reader = GzDecoder::new(reader);
        reader.read_to_end(&mut raw_json).unwrap();
    } else {
        reader.read_to_end(&mut raw_json).unwrap();
    }

    // println!("uncompress time {}", start.elapsed().unwrap().as_millis());

    // let start = SystemTime::now();
    let data: TestData = serde_json::from_reader(raw_json.as_slice()).unwrap();
    // println!("JSON parse time {}", start.elapsed().unwrap().as_millis());

    data
}
