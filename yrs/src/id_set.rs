use crate::block::{Block, ID};
use crate::block_store::BlockStore;
use crate::store::Store;
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::utils::client_hasher::ClientHasher;
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

    /// Inverts current [IdRange], returning another [IdRange] that contains all
    /// "holes" (ranges not included in current range). If current range is a continuous space
    /// starting from the initial clock (eg. [0..5)), then returned range will be empty.
    pub fn invert(&self) -> IdRange {
        match self {
            IdRange::Continuous(range) => IdRange::Continuous(0..range.start),
            IdRange::Fragmented(ranges) => {
                let mut inv = Vec::new();
                let mut start = 0;
                for range in ranges.iter() {
                    if range.start > start {
                        inv.push(start..range.start);
                    }
                    start = range.end;
                }
                match inv.len() {
                    0 => IdRange::Continuous(0..0),
                    1 => IdRange::Continuous(inv[0].clone()),
                    _ => IdRange::Fragmented(inv),
                }
            }
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
    fn squash(&mut self) {
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

/// Implement this to efficiently let IdRange iterator work in descending order
impl<'a> DoubleEndedIterator for IdRangeIter<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if let Some(inner) = &mut self.inner {
            inner.next_back()
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
#[derive(Default, Debug, Clone, PartialEq, Eq)]
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

    /// Checks if current ID set contains any data.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty() || self.0.values().all(|r| r.is_empty())
    }

    /// Compacts an internal ranges representation.
    pub fn squash(&mut self) {
        for block in self.0.values_mut() {
            block.squash();
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

    /// Inserts a new ID `range` corresponding with a given `client`.
    pub fn insert_range(&mut self, client: u64, range: IdRange) {
        self.0.insert(client, range);
    }

    /// Merges another ID set into a current one, combining their information about observed ID
    /// ranges and squashing them if necessary.
    pub fn merge(&mut self, other: Self) {
        other
            .0
            .into_iter()
            .for_each(|(client, range)| match self.0.entry(client) {
                Entry::Occupied(mut e) => {
                    let r = e.get_mut();
                    match (r, range) {
                        (IdRange::Continuous(r1), IdRange::Continuous(r2)) => {
                            if r1.end >= r2.start && r1.start <= r2.start {
                                r1.end = r2.end;
                            } else {
                                let new = IdRange::Fragmented(vec![r1.clone(), r2.clone()]);
                                e.replace_entry(new);
                            }
                        }
                        (IdRange::Fragmented(rs), IdRange::Continuous(r)) => {
                            rs.push(r);
                        }
                        (IdRange::Continuous(r), IdRange::Fragmented(rs)) => {
                            let mut v = rs;
                            v.push(r.clone());
                            let new = IdRange::Fragmented(v);
                            e.replace_entry(new);
                        }
                        (IdRange::Fragmented(rs1), IdRange::Fragmented(mut rs2)) => {
                            rs1.append(&mut rs2);
                        }
                    }
                }
                Entry::Vacant(e) => {
                    e.insert(range);
                }
            });
        self.squash()
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
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DeleteSet(IdSet);

impl From<IdSet> for DeleteSet {
    fn from(id_set: IdSet) -> Self {
        DeleteSet(id_set)
    }
}

impl<'a> From<&'a BlockStore> for DeleteSet {
    /// Creates a [DeleteSet] by reading all deleted blocks and including their clock ranges into
    /// the delete set itself.
    fn from(store: &'a BlockStore) -> Self {
        let mut set = DeleteSet(IdSet::new());
        for (&client, blocks) in store.iter() {
            let mut deletes = IdRange::with_capacity(blocks.len());
            for block in blocks.iter() {
                if block.is_deleted() {
                    let start = block.id().clock;
                    let end = start + block.len();
                    deletes.push(start..end);
                }
            }

            if !deletes.is_empty() {
                set.0.insert_range(client, deletes);
            }
        }
        set
    }
}

impl Default for DeleteSet {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DeleteSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for IdSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{{")?;
        for (k, v) in self.0.iter() {
            writeln!(f, "\t{}:{}", k, v)?;
        }
        writeln!(f, "}}")
    }
}

impl std::fmt::Display for IdRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdRange::Continuous(r) => write!(f, "[{}..{})", r.start, r.end),
            IdRange::Fragmented(r) => {
                write!(f, "[")?;
                for r in r.iter() {
                    write!(f, " [{}..{})", r.start, r.end)?;
                }
                write!(f, " ]")
            }
        }
    }
}

impl DeleteSet {
    /// Creates a new empty delete set instance.
    pub fn new() -> Self {
        DeleteSet(IdSet::new())
    }

    /// Inserts an information about delete block (identified by `id` and having a specified length)
    /// inside of a current delete set.
    pub fn insert(&mut self, id: ID, len: u32) {
        self.0.insert(id, len)
    }

    /// Checks if delete set contains any clock ranges.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Checks if given block `id` is considered deleted from the perspective of current delete set.
    pub fn is_deleted(&self, id: &ID) -> bool {
        self.0.contains(id)
    }

    /// Returns an iterator over all client-range pairs registered in this delete set.
    pub fn iter(&self) -> Iter<'_> {
        self.0.iter()
    }

    /// Merges another delete set into a current one, combining their information about deleted
    /// clock ranges.
    pub fn merge(&mut self, other: Self) {
        self.0.merge(other.0)
    }

    /// Squashes the contents of a current delete set. This operation means, that in case when
    /// delete set contains any overlapping ranges within, they will be squashed together to
    /// optimize the space and make future encoding more compact.
    pub fn squash(&mut self) {
        self.0.squash()
    }

    pub(crate) fn try_squash_with(&mut self, store: &mut Store) {
        // try to merge deleted / gc'd items
        for (client, range) in self.iter() {
            if let Some(mut blocks) = store.blocks.get_mut(client) {
                for r in range.iter().rev() {
                    // start with merging the item next to the last deleted item
                    let mut si = (blocks.len() - 1)
                        .min(1 + blocks.find_pivot(r.end - 1).unwrap_or_default());
                    let mut block = &blocks[si];
                    while si > 0 && block.id().clock >= r.start {
                        if let Some(compaction) = blocks.squash_left(si) {
                            if let Some(right) = compaction.new_right {
                                right.fix_pivot((right.pivot().max(1) - 1) as u32);
                                if let Block::Item(item) =
                                    store.blocks.get_block_mut(&right).unwrap()
                                {
                                    item.left = Some(compaction.replacement);
                                }
                                blocks = store.blocks.get_mut(client).unwrap();
                            }
                        }
                        si -= 1;
                        block = &blocks[si];
                    }
                }
            }
        }
    }
}

impl Decode for DeleteSet {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        DeleteSet(IdSet::decode(decoder))
    }
}

impl Encode for DeleteSet {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        self.0.encode(encoder)
    }
}

#[cfg(test)]
mod test {
    use crate::id_set::{IdRange, IdSet};
    use crate::updates::decoder::{Decode, DecoderV1};
    use crate::updates::encoder::{Encode, Encoder, EncoderV1};
    use crate::ID;
    use std::fmt::Debug;

    #[test]
    fn id_range_compact() {
        let mut r = IdRange::Fragmented(vec![(0..3), (3..5), (6..7)]);
        r.squash();
        assert_eq!(r, IdRange::Fragmented(vec![(0..5), (6..7)]));
    }

    #[test]
    fn id_range_invert() {
        assert!(IdRange::Continuous(0..3).invert().is_empty());

        assert_eq!(
            IdRange::Continuous(3..5).invert(),
            IdRange::Continuous(0..3)
        );

        assert_eq!(
            IdRange::Fragmented(vec![0..3, 4..5]).invert(),
            IdRange::Continuous(3..4)
        );

        assert_eq!(
            IdRange::Fragmented(vec![3..4, 7..9]).invert(),
            IdRange::Fragmented(vec![0..3, 4..7])
        );
    }

    #[test]
    fn id_range_contains() {
        assert!(!IdRange::Continuous(1..3).contains(0));
        assert!(IdRange::Continuous(1..3).contains(1));
        assert!(IdRange::Continuous(1..3).contains(2));
        assert!(!IdRange::Continuous(1..3).contains(3));

        assert!(!IdRange::Fragmented(vec![1..3, 4..5]).contains(0));
        assert!(IdRange::Fragmented(vec![1..3, 4..5]).contains(1));
        assert!(IdRange::Fragmented(vec![1..3, 4..5]).contains(2));
        assert!(!IdRange::Fragmented(vec![1..3, 4..5]).contains(3));
        assert!(IdRange::Fragmented(vec![1..3, 4..5]).contains(4));
        assert!(!IdRange::Fragmented(vec![1..3, 4..5]).contains(5));
        assert!(!IdRange::Fragmented(vec![1..3, 4..5]).contains(6));
    }

    #[test]
    fn id_range_push() {
        let mut range = IdRange::Continuous(0..0);

        range.push(0..4);
        assert_eq!(range, IdRange::Continuous(0..4));

        range.push(4..6);
        assert_eq!(range, IdRange::Continuous(0..6));

        range.push(7..9);
        assert_eq!(range, IdRange::Fragmented(vec![0..6, 7..9]));
    }

    #[test]
    fn id_range_encode_decode() {
        roundtrip(&IdRange::Continuous(0..4));
        roundtrip(&IdRange::Fragmented(vec![1..4, 5..8]));
    }

    #[test]
    fn id_set_encode_decode() {
        let mut set = IdSet::new();
        set.insert(ID::new(124, 0), 1);
        set.insert(ID::new(1337, 0), 12);
        set.insert(ID::new(124, 1), 3);

        roundtrip(&set);
    }

    fn roundtrip<T>(value: &T)
    where
        T: Encode + Decode + PartialEq + Debug,
    {
        let mut encoder = EncoderV1::new();
        value.encode(&mut encoder);
        let buf = encoder.to_vec();
        let mut decoder = DecoderV1::from(buf.as_slice());
        let decoded = T::decode(&mut decoder);

        assert_eq!(value, &decoded);
    }
}
