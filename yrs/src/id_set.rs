use crate::block::{Block, ID};
use crate::transaction::Transaction;
use crate::updates::decoder::Decoder;
use crate::updates::encoder::Encoder;
use crate::utils::client_hasher::ClientHasher;
use crate::*;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;

#[derive(Default, Copy, Clone)]
pub(crate) struct IdRange {
    pub clock: u32,
    pub len: u32,
}

impl IdRange {
    pub fn new(clock: u32, len: u32) -> Self {
        IdRange { clock, len }
    }

    pub fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_ds_clock(self.clock);
        encoder.write_ds_len(self.len);
    }

    pub fn decode<D: Decoder>(decoder: &mut D) -> Self {
        let clock = decoder.read_ds_clock();
        let len = decoder.read_ds_len();
        IdRange { clock, len }
    }
}

/// DeleteSet is a temporary object that is created when needed.
/// - When created in a transaction, it must only be accessed after sorting and merging.
///   - This DeleteSet is sent to other clients.
/// - We do not create a DeleteSet when we send a sync message. The DeleteSet message is created
///   directly from StructStore.
/// - We read a DeleteSet as a apart of sync/update message. In this case the DeleteSet is already
///   sorted and merged.
#[derive(Default)]
pub struct IdSet {
    clients: HashMap<u64, Vec<IdRange>, BuildHasherDefault<ClientHasher>>,
}

pub(crate) type Iter<'a> = std::collections::hash_map::Iter<'a, u64, Vec<IdRange>>;

impl IdSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from(store: &BlockStore) -> Self {
        let mut set = Self::new();
        for (&client, blocks) in store.iter() {
            let mut ranges = Vec::with_capacity(blocks.list.len());
            let mut i = 0;
            while i < blocks.list.len() {
                let block = &blocks.list[i];
                if block.is_deleted() {
                    let clock = block.id().clock;
                    let mut len = block.len();
                    if i + 1 < blocks.list.len() {
                        let mut next = &blocks.list[i + 1];
                        while i + 1 < blocks.list.len() && next.is_deleted() {
                            len += next.len();
                            i += 1;
                            next = &blocks.list[i + 1];
                        }
                    }
                    ranges.push(IdRange::new(clock, len as u32));
                }

                i += 1;
            }

            if !ranges.is_empty() {
                set.clients.insert(client, ranges);
            }
        }
        set
    }

    pub(crate) fn iter(&self) -> Iter<'_> {
        self.clients.iter()
    }

    pub fn apply_ranges<F>(&self, transaction: &mut Transaction, f: &F)
    where
        F: Fn(&Block) -> (),
    {
        // equivalent of JS: Y.iterateDeletedStructs
        for (client, ranges) in self.clients.iter() {
            if transaction.store.blocks.contains_client(client) {
                for range in ranges.iter() {
                    transaction.iterate_structs(client, range.clock, range.len, f);
                }
            }
        }
    }

    fn find_pivot(block: &[IdRange], clock: u32) -> Option<usize> {
        let mut left = 0;
        let mut right = block.len() - 1;
        while left <= right {
            let mid_idx = (left + right) / 2;
            let mid = &block[mid_idx];
            if mid.clock <= clock {
                if clock < mid.clock + mid.len {
                    return Some(mid_idx);
                }
                left = mid_idx + 1;
            } else {
                right = mid_idx - 1;
            }
        }
        None
    }

    pub fn is_deleted(&self, id: &ID) -> bool {
        if let Some(block) = self.clients.get(&id.client) {
            Self::find_pivot(block.as_slice(), id.clock).is_some()
        } else {
            false
        }
    }

    pub fn sort_and_merge(&mut self) {
        for block in self.clients.values_mut() {
            block.sort_by(|a, b| a.clock.cmp(&b.clock));
            // merge items without filtering or splicing the array
            // i is the current pointer
            // j refers to the current insert position for the pointed item
            // try to merge dels[i] into dels[j-1] or set dels[j]=dels[i]
            let mut i = 1;
            let mut j = 1;
            while i < block.len() {
                let right = block[i]; // copying 8B element is very cheap
                let left = &mut block[j - 1];
                if left.clock + left.len >= right.clock {
                    left.len = left.len.max(right.clock + right.len - left.clock);
                } else {
                    if j < i {
                        block[j] = right.clone();
                    }
                    j += 1;
                }
                i += 1;
            }

            if j < block.len() {
                block.shrink_to(j);
            }
        }
    }

    pub fn insert(&mut self, id: ID, len: u32) {
        let block = self.clients.entry(id.client).or_default();
        block.push(IdRange::new(id.clock, len));
    }

    pub fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_len(self.clients.len() as u32);
        for (&client_id, block) in self.clients.iter() {
            encoder.reset_ds_cur_val();
            encoder.write_client(client_id);
            encoder.write_len(block.len() as u32);
            for item in block.iter() {
                item.encode(encoder);
            }
        }
    }

    pub fn decode<D: Decoder>(decoder: &mut D) -> Self {
        let mut set = Self::new();
        let client_len = decoder.read_len();
        let mut i = 0;
        while i < client_len {
            decoder.reset_ds_cur_val();
            let client = decoder.read_client();
            let block_len = decoder.read_len() as usize;
            if block_len > 0 {
                let block = set
                    .clients
                    .entry(client)
                    .or_insert_with(|| Vec::with_capacity(block_len));

                let mut j = 0;
                while j < block_len {
                    block.push(IdRange::decode(decoder));
                    j += 1;
                }
            }
            i += 1;
        }
        set
    }
}

#[cfg(test)]
mod test {}
