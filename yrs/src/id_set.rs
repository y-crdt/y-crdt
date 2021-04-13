
use crate::*;
use crate::block::Block;

#[derive(Default)]
struct IdRange {
    clock: u32,
    len: u32
}

#[derive(Default)]
pub struct IdSet {
    clients: HashMap::<u64, Vec<IdRange>, BuildHasherDefault<ClientHasher>>,
}

impl IdSet {
    pub fn new () -> Self {
        Self::default()
    }
    pub fn iterate_blocks (&self, tr: &mut Transaction, f: fn(Block)) {

    }
}
