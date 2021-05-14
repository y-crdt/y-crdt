use crate::block::{Block, ID};
use crate::transaction::Transaction;
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::utils::client_hasher::ClientHasher;
use crate::*;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::ops::Range;

// Note: use native Rust [Range](https://doc.rust-lang.org/std/ops/struct.Range.html)
// as it's left-inclusive/right-exclusive and defines the exact capabilities we care about here.

impl Encode for Range<u32> {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_ds_clock(self.start);
        encoder.write_ds_len(self.end - self.start);
    }
}

impl Decode for Range<u32> {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        let clock = decoder.read_ds_clock();
        let len = decoder.read_ds_len();
        clock..(clock + len)
    }
}

/// [IdRange] describes a single space of an [ID] clock values, belonging to the same client.
/// It can contain from a single continuous space, or multiple ones having "holes" between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdRange {
    /// A single continuous range of clocks.
    Continuous(Range<u32>),
    /// A multiple ranges containing clock values, separated from each other by other clock ranges
    /// not included in this [IdRange].
    Fragmented(Vec<Range<u32>>),
}

impl IdRange {
    pub fn with_capacity(capacity: usize) -> Self {
        IdRange::Fragmented(Vec::with_capacity(capacity))
    }

    /// Check if range is empty (doesn't cover any clock space).
    pub fn is_empty(&self) -> bool {
        match self {
            IdRange::Continuous(r) => r.start == r.end,
            IdRange::Fragmented(rs) => rs.is_empty(),
        }
    }

    /// Returns a range describing a continuous space of the [IdRange], starting from the beginning
    /// of the clock up to the end of current range or the first missing range (in case when current
    /// range is fragmented).
    ///
    /// Note: this function assumes that fragmented range already has been compacted.
    pub fn continuous(&self) -> Option<&Range<u32>> {
        match self {
            IdRange::Continuous(range) if range.start == 0 => Some(range),
            IdRange::Fragmented(ranges) => {
                let first = ranges.first()?;
                if first.start == 0 {
                    Some(first)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Check if given clock exists within current [IdRange].
    pub fn contains(&self, clock: u32) -> bool {
        match self {
            IdRange::Continuous(range) => range.contains(&clock),
            IdRange::Fragmented(ranges) => ranges.iter().any(|r| r.contains(&clock)),
        }
    }

    /// Iterate over ranges described by current [IdRange].
    pub fn iter(&self) -> IdRangeIter<'_> {
        let (range, inner) = match self {
            IdRange::Continuous(range) => (Some(range), None),
            IdRange::Fragmented(ranges) => (None, Some(ranges.iter())),
        };
        IdRangeIter { range, inner }
    }

    fn push(&mut self, range: Range<u32>) {
        match self {
            IdRange::Continuous(r) => {
                if r.end >= range.start {
                    r.end = range.end; // two ranges overlap, we can eagerly merge them
                } else {
                    *self = IdRange::Fragmented(vec![r.clone(), range])
                }
            }
            IdRange::Fragmented(ranges) => {
                ranges.push(range);
            }
        }
    }

    /// Alters current [IdRange] by compacting its internal implementation (in fragmented case).
    /// Example: fragmented space of [0,3), [3,5), [6,7) will be compacted into [0,5), [6,7).
    fn compact(&mut self) {
        if let IdRange::Fragmented(ranges) = self {
            if !ranges.is_empty() {
                ranges.sort_by(|a, b| a.start.cmp(&b.start));
                let mut new_len = 1;
                unsafe {
                    let len = ranges.len() as isize;
                    let head = ranges.as_mut_ptr();
                    let mut current = head.as_mut().unwrap();
                    let mut i = 1;
                    while i < len {
                        let next = head.offset(i).as_ref().unwrap();
                        if next.start <= current.end {
                            // merge next to current eg. curr=[0,5) & next=[3,6) => curr=[0,6)
                            current.end = next.end;
                        } else {
                            // current and next are disjoined eg. [0,5) & [6,9)

                            // move current pointer one index to the left: by using new_len we
                            // squash ranges possibly already merged to current
                            current = head.offset(new_len).as_mut().unwrap();

                            // make next a new current
                            current.start = next.start;
                            current.end = next.end;
                            new_len += 1;
                        }

                        i += 1;
                    }
                }

                if new_len == 1 {
                    *self = IdRange::Continuous(ranges.pop().unwrap())
                } else if ranges.len() != new_len as usize {
                    ranges.truncate(new_len as usize);
                }
            }
        }
    }
}

impl Encode for IdRange {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        match self {
            IdRange::Continuous(range) => {
                encoder.write_len(1);
                range.encode(encoder);
            }
            IdRange::Fragmented(ranges) => {
                encoder.write_len(ranges.len() as u32);
                for range in ranges.iter() {
                    range.encode(encoder);
                }
            }
        }
    }
}

impl Decode for IdRange {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        match decoder.read_len() {
            1 => {
                let range = Range::decode(decoder);
                IdRange::Continuous(range)
            }
            len => {
                let mut ranges = Vec::with_capacity(len as usize);
                let mut i = 0;
                while i < len {
                    ranges.push(Range::decode(decoder));
                    i += 1;
                }
                IdRange::Fragmented(ranges)
            }
        }
    }
}

pub struct IdRangeIter<'a> {
    inner: Option<std::slice::Iter<'a, Range<u32>>>,
    range: Option<&'a Range<u32>>,
}

impl<'a> Iterator for IdRangeIter<'a> {
    type Item = &'a Range<u32>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(inner) = &mut self.inner {
            inner.next()
        } else {
            self.range.take()
        }
    }
}

/// DeleteSet is a temporary object that is created when needed.
/// - When created in a transaction, it must only be accessed after sorting and merging.
///   - This DeleteSet is sent to other clients.
/// - We do not create a DeleteSet when we send a sync message. The DeleteSet message is created
///   directly from StructStore.
/// - We read a DeleteSet as a apart of sync/update message. In this case the DeleteSet is already
///   sorted and merged.
#[derive(Default, Debug)]
pub struct IdSet(HashMap<u64, IdRange, BuildHasherDefault<ClientHasher>>);

pub(crate) type Iter<'a> = std::collections::hash_map::Iter<'a, u64, IdRange>;

//TODO: I'd say we should split IdSet and DeleteSet into two structures. While DeleteSet can be
// implemented in terms of IdSet, it has more specific methods (related to deletion process), while
// IdSet could contain wider area of use cases.
impl IdSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn iter(&self) -> Iter<'_> {
        self.0.iter()
    }

    /// Check if current [IdSet] contains given `id`.
    pub fn contains(&self, id: &ID) -> bool {
        if let Some(ranges) = self.0.get(&id.client) {
            ranges.contains(id.clock)
        } else {
            false
        }
    }

    pub fn apply_ranges<F>(&self, transaction: &mut Transaction, f: &F)
    where
        F: Fn(&Block) -> (),
    {
        // equivalent of JS: Y.iterateDeletedStructs
        for (client, ranges) in self.0.iter() {
            if transaction.store.blocks.contains_client(client) {
                for range in ranges.iter() {
                    transaction.iterate_structs(client, range, f);
                }
            }
        }
    }

    pub fn compact(&mut self) {
        for block in self.0.values_mut() {
            block.compact();
        }
    }

    pub fn insert(&mut self, id: ID, len: u32) {
        let range = id.clock..(id.clock + len);
        match self.0.entry(id.client) {
            Entry::Occupied(r) => {
                r.into_mut().push(range);
            }
            Entry::Vacant(e) => {
                e.insert(IdRange::Continuous(range));
            }
        }
    }

    pub fn insert_range(&mut self, client: u64, range: IdRange) {
        self.0.insert(client, range);
    }
}

impl Encode for IdSet {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_len(self.0.len() as u32);
        for (&client_id, block) in self.0.iter() {
            encoder.reset_ds_cur_val();
            encoder.write_client(client_id);
            block.encode(encoder);
        }
    }
}

impl Decode for IdSet {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        let mut set = Self::new();
        let client_len = decoder.read_len();
        let mut i = 0;
        while i < client_len {
            decoder.reset_ds_cur_val();
            let client = decoder.read_client();
            let range = IdRange::decode(decoder);
            set.0.insert(client, range);
            i += 1;
        }
        set
    }
}

/// [DeleteSet] contains information about all blocks (described by clock ranges) that have been
/// subjected to delete process.
pub(crate) struct DeleteSet(IdSet);

impl DeleteSet {
    /// Creates a [DeleteSet] by reading all deleted blocks and including their clock ranges into
    /// the delete set itself.
    pub fn from(store: &BlockStore) -> Self {
        let mut set = DeleteSet(IdSet::new());
        for (&client, blocks) in store.iter() {
            let mut deletes = IdRange::with_capacity(blocks.list.len());
            let mut i = 0;
            while i < blocks.list.len() {
                let block = &blocks.list[i];
                if block.is_deleted() {
                    let start = block.id().clock;
                    let end = start + block.len();
                    deletes.push(start..end);
                }
                i += 1;
            }

            if !deletes.is_empty() {
                set.0.insert_range(client, deletes);
            }
        }
        set
    }

    pub fn is_deleted(&self, id: &ID) -> bool {
        self.0.contains(id)
    }
}

#[cfg(test)]
mod test {
    use crate::id_set::IdRange;

    #[test]
    fn id_range_compact() {
        let mut r = IdRange::Fragmented(vec![(0..3), (3..5), (6..7)]);
        r.compact();
        assert_eq!(r, IdRange::Fragmented(vec![(0..5), (6..7)]));
    }
}
