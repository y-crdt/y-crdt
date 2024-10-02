use crate::block::{ClientID, ID};
use crate::block_store::BlockStore;
use crate::encoding::read::Error;
use crate::iter::TxnIterator;
use crate::slice::BlockSlice;
use crate::store::Store;
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::utils::client_hasher::ClientHasher;
use crate::ReadTxn;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::ops::{Range, RangeInclusive};
// Note: use native Rust [Range](https://doc.rust-lang.org/std/ops/struct.Range.html)
// as it's left-inclusive/right-exclusive and defines the exact capabilities we care about here.

impl Encode for Range<u32> {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_ds_clock(self.start);
        encoder.write_ds_len(self.end - self.start)
    }
}

impl Decode for Range<u32> {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let clock = decoder.read_ds_clock()?;
        let len = decoder.read_ds_len()?;
        Ok(clock..(clock + len))
    }
}

/// [IdRange] describes a single space of an [ID] clock values, belonging to the same client.
/// It can contain from a single continuous space, or multiple ones having "holes" between them.
#[derive(Clone, PartialEq, Eq, Hash)]
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
                    if r.start > range.end {
                        *self = IdRange::Fragmented(vec![range, r.clone()])
                    } else {
                        // two ranges overlap - merge them
                        r.end = range.end.max(r.end);
                        r.start = range.start.min(r.start);
                    }
                } else {
                    *self = IdRange::Fragmented(vec![r.clone(), range])
                }
            }
            IdRange::Fragmented(ranges) => {
                if ranges.is_empty() {
                    *self = IdRange::Continuous(range);
                } else {
                    let last_idx = ranges.len() - 1;
                    let last = &mut ranges[last_idx];
                    if !Self::try_join(last, &range) {
                        ranges.push(range);
                    }
                }
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

                let len = ranges.len() as isize;
                let head = ranges.as_mut_ptr();
                let mut current = unsafe { head.as_mut().unwrap() };
                let mut i = 1;
                while i < len {
                    let next = unsafe { head.offset(i).as_ref().unwrap() };
                    if !Self::try_join(current, next) {
                        // current and next are disjoined eg. [0,5) & [6,9)

                        // move current pointer one index to the left: by using new_len we
                        // squash ranges possibly already merged to current
                        current = unsafe { head.offset(new_len).as_mut().unwrap() };

                        // make next a new current
                        current.start = next.start;
                        current.end = next.end;
                        new_len += 1;
                    }

                    i += 1;
                }

                if new_len == 1 {
                    *self = IdRange::Continuous(ranges[0].clone())
                } else if ranges.len() != new_len as usize {
                    ranges.truncate(new_len as usize);
                }
            }
        }
    }

    fn is_squashed(&self) -> bool {
        match self {
            IdRange::Continuous(_) => true,
            IdRange::Fragmented(ranges) => {
                let mut i = ranges.iter();
                if let Some(r) = i.next() {
                    let mut prev_start = r.start;
                    let mut prev_end = r.end;
                    while let Some(r) = i.next() {
                        if r.start < prev_end {
                            return false;
                        }
                        prev_start = r.start;
                        prev_end = r.end;
                    }
                    true
                } else {
                    true
                }
            }
        }
    }

    fn merge(&mut self, other: IdRange) {
        let raw = std::mem::take(self);
        *self = match (raw, other) {
            (IdRange::Continuous(mut a), IdRange::Continuous(b)) => {
                let never_intersect = a.end < b.start || b.end < a.start;
                if never_intersect {
                    IdRange::Fragmented(vec![a, b])
                } else {
                    a.start = a.start.min(b.start);
                    a.end = a.end.max(b.end);
                    IdRange::Continuous(a)
                }
            }
            (IdRange::Fragmented(mut a), IdRange::Continuous(b)) => {
                a.push(b);
                IdRange::Fragmented(a)
            }
            (IdRange::Continuous(a), IdRange::Fragmented(b)) => {
                let mut v = b;
                v.push(a);
                IdRange::Fragmented(v)
            }
            (IdRange::Fragmented(mut a), IdRange::Fragmented(mut b)) => {
                a.append(&mut b);
                IdRange::Fragmented(a)
            }
        };
    }

    fn encode_raw<E: Encoder>(&self, encoder: &mut E) {
        match self {
            IdRange::Continuous(range) => {
                encoder.write_var(1u32);
                range.encode(encoder)
            }
            IdRange::Fragmented(ranges) => {
                encoder.write_var(ranges.len() as u32);
                for range in ranges.iter() {
                    range.encode(encoder);
                }
            }
        }
    }

    #[inline]
    fn try_join(a: &mut Range<u32>, b: &Range<u32>) -> bool {
        if Self::disjoint(a, b) {
            false
        } else {
            a.start = a.start.min(b.start);
            a.end = a.end.max(b.end);
            true
        }
    }

    #[inline]
    fn disjoint(a: &Range<u32>, b: &Range<u32>) -> bool {
        a.start > b.end || b.start > a.end
    }
}

impl Default for IdRange {
    fn default() -> Self {
        IdRange::Continuous(0..0)
    }
}

impl Encode for IdRange {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        if self.is_squashed() {
            self.encode_raw(encoder)
        } else {
            let mut clone = self.clone();
            clone.squash();
            clone.encode_raw(encoder);
        }
    }
}

impl Decode for IdRange {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        match decoder.read_var::<u32>()? {
            1 => {
                let range = Range::decode(decoder)?;
                Ok(IdRange::Continuous(range))
            }
            len => {
                let mut ranges = Vec::with_capacity(len as usize);
                let mut i = 0;
                while i < len {
                    ranges.push(Range::decode(decoder)?);
                    i += 1;
                }
                Ok(IdRange::Fragmented(ranges))
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
#[derive(Default, Clone, PartialEq, Eq)]
pub struct IdSet(HashMap<ClientID, IdRange, BuildHasherDefault<ClientHasher>>);

pub(crate) type Iter<'a> = std::collections::hash_map::Iter<'a, ClientID, IdRange>;

//TODO: I'd say we should split IdSet and DeleteSet into two structures. While DeleteSet can be
// implemented in terms of IdSet, it has more specific methods (related to deletion process), while
// IdSet could contain wider area of use cases.
impl IdSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns number of clients stored;
    pub fn len(&self) -> usize {
        self.0.len()
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
    pub fn insert_range(&mut self, client: ClientID, range: IdRange) {
        self.0.insert(client, range);
    }

    /// Merges another ID set into a current one, combining their information about observed ID
    /// ranges and squashing them if necessary.
    pub fn merge(&mut self, other: Self) {
        other.0.into_iter().for_each(|(client, range)| {
            if let Some(r) = self.0.get_mut(&client) {
                r.merge(range)
            } else {
                self.0.insert(client, range);
            }
        });
        self.squash()
    }

    pub fn get(&self, client_id: &ClientID) -> Option<&IdRange> {
        self.0.get(client_id)
    }
}

impl Encode for IdSet {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_var(self.0.len() as u32);
        for (&client_id, block) in self.0.iter() {
            encoder.reset_ds_cur_val();
            encoder.write_var(client_id);
            block.encode(encoder);
        }
    }
}

impl Decode for IdSet {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let mut set = Self::new();
        let client_len: u32 = decoder.read_var()?;
        let mut i = 0;
        while i < client_len {
            decoder.reset_ds_cur_val();
            let client: u32 = decoder.read_var()?;
            let range = IdRange::decode(decoder)?;
            set.0.insert(client as ClientID, range);
            i += 1;
        }
        Ok(set)
    }
}

impl Hash for IdSet {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for (client, range) in self.0.iter() {
            client.hash(state);
            range.hash(state);
        }
    }
}

/// [DeleteSet] contains information about all blocks (described by clock ranges) that have been
/// subjected to delete process.
#[derive(Clone, Eq, PartialEq, Hash)]
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
                    let (start, end) = block.clock_range();
                    deletes.push(start..(end + 1));
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

impl std::fmt::Debug for DeleteSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}
impl std::fmt::Display for DeleteSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::fmt::Debug for IdSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}
impl std::fmt::Display for IdSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("");
        for (k, v) in self.iter() {
            s.field(&k.to_string(), v);
        }
        s.finish()
    }
}

impl std::fmt::Debug for IdRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
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

    /// Returns number of clients stored;
    pub fn len(&self) -> usize {
        self.0.len()
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

    pub fn range(&self, client_id: &ClientID) -> Option<&IdRange> {
        self.0.get(client_id)
    }

    pub(crate) fn try_squash_with(&mut self, store: &mut Store) {
        // try to merge deleted / gc'd items
        for (&client, range) in self.iter() {
            let blocks = store.blocks.get_client_blocks_mut(client);
            for r in range.iter().rev() {
                // start with merging the item next to the last deleted item
                let mut si =
                    (blocks.len() - 1).min(1 + blocks.find_pivot(r.end - 1).unwrap_or_default());
                let mut block = &blocks[si];

                let mut valid_range = usize::MAX..usize::MIN;

                while si > 0 && block.clock_start() >= r.start {
                    valid_range.start = valid_range.start.min(si);
                    valid_range.end = valid_range.end.max(si);
                    si -= 1;
                    block = &blocks[si];
                }

                if valid_range.start != usize::MAX && valid_range.end != usize::MIN {
                    blocks.squash_left_range_compaction(valid_range.start..=valid_range.end);
                }
            }
        }
    }

    pub(crate) fn deleted_blocks(&self) -> DeletedBlocks {
        DeletedBlocks::new(self)
    }
}

impl Decode for DeleteSet {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        Ok(DeleteSet(IdSet::decode(decoder)?))
    }
}

impl Encode for DeleteSet {
    #[inline]
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        self.0.encode(encoder)
    }
}

pub(crate) struct DeletedBlocks<'ds> {
    ds_iter: Iter<'ds>,
    current_range: Option<&'ds Range<u32>>,
    current_client_id: Option<ClientID>,
    range_iter: Option<IdRangeIter<'ds>>,
    current_index: Option<usize>,
}

impl<'ds> DeletedBlocks<'ds> {
    pub(crate) fn new(ds: &'ds DeleteSet) -> Self {
        let ds_iter = ds.iter();
        DeletedBlocks {
            ds_iter,
            current_client_id: None,
            current_range: None,
            range_iter: None,
            current_index: None,
        }
    }
}

impl<'ds> TxnIterator for DeletedBlocks<'ds> {
    type Item = BlockSlice;

    fn next<T: ReadTxn>(&mut self, txn: &T) -> Option<Self::Item> {
        if let Some(r) = self.current_range {
            let mut block = if let Some(idx) = self.current_index.as_mut() {
                if let Some(block) = txn
                    .store()
                    .blocks
                    .get_client(&self.current_client_id?)
                    .unwrap()
                    .get(*idx)
                {
                    *idx += 1;
                    block.as_slice()
                } else {
                    self.current_range = None;
                    self.current_index = None;
                    return self.next(txn);
                }
            } else {
                // first block for a particular client
                let list = txn
                    .store()
                    .blocks
                    .get_client(&self.current_client_id?)
                    .unwrap();
                if let Some(idx) = list.find_pivot(r.start) {
                    let mut block = list[idx].as_slice();
                    let clock = block.clock_start();

                    // check if we don't need to cut first block
                    if clock < r.start {
                        block.trim_start(r.start - clock);
                    }
                    self.current_index = Some(idx + 1);
                    block
                } else {
                    self.current_range = None;
                    self.current_index = None;
                    return self.next(txn);
                }
            };

            // check if this is the last block for a current client
            let clock = block.clock_start();
            let block_len = block.len();
            if clock > r.end {
                // move to the next range
                self.current_range = None;
                self.current_index = None;
                return self.next(txn);
            } else if clock < r.end && clock + block_len > r.end {
                // we need to cut the last block
                block.trim_end(clock + block_len - r.end);
                self.current_range = None;
                self.current_index = None;
            }

            if clock + block_len >= r.end {
                self.current_range = None;
                self.current_index = None;
            }

            Some(block)
        } else {
            let range_iter = if let Some(iter) = self.range_iter.as_mut() {
                iter
            } else {
                let (client_id, range) = self.ds_iter.next()?;
                self.current_client_id = Some(client_id.clone());
                self.current_index = None;
                self.range_iter = Some(range.iter());
                self.range_iter.as_mut().unwrap()
            };
            self.current_range = match range_iter.next() {
                None => {
                    let (client_id, range) = self.ds_iter.next()?;
                    self.current_client_id = Some(client_id.clone());
                    self.current_index = None;
                    let mut iter = range.iter();
                    let range = iter.next();
                    self.range_iter = Some(iter);
                    range
                }
                other => other,
            };
            return self.next(txn);
        }
    }
}

#[cfg(test)]
mod test {
    use crate::block::ItemContent;
    use crate::id_set::{IdRange, IdSet};
    use crate::iter::TxnIterator;
    use crate::slice::BlockSlice;
    use crate::test_utils::exchange_updates;
    use crate::updates::decoder::{Decode, DecoderV1};
    use crate::updates::encoder::{Encode, Encoder, EncoderV1};
    use crate::{DeleteSet, Doc, Options, ReadTxn, Text, Transact, ID};
    use std::collections::HashSet;
    use std::fmt::Debug;

    #[test]
    fn id_range_merge_continous() {
        // `b` entirely within `a`
        let mut a = IdRange::Continuous(0..5);
        a.merge(IdRange::Continuous(2..4));
        assert_eq!(a, IdRange::Continuous(0..5));

        // the tail of `a` crosses the head of `b`
        let mut a = IdRange::Continuous(0..5);
        a.merge(IdRange::Continuous(4..9));
        assert_eq!(a, IdRange::Continuous(0..9));

        // `b` is immediately adjacent to the end of `a`
        let mut a = IdRange::Continuous(0..5);
        a.merge(IdRange::Continuous(5..9));
        assert_eq!(a, IdRange::Continuous(0..9));

        // `a` does not intersect with `b`
        let mut a = IdRange::Continuous(0..4);
        a.merge(IdRange::Continuous(6..9));
        assert_eq!(a, IdRange::Fragmented(vec![0..4, 6..9]));
    }

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
        let decoded = T::decode(&mut decoder).unwrap();

        assert_eq!(value, &decoded);
    }

    #[test]
    fn deleted_blocks() {
        let mut o = Options::default();
        o.client_id = 1;
        o.skip_gc = true;
        let d1 = Doc::with_options(o.clone());
        let t1 = d1.get_or_insert_text("test");

        o.client_id = 2;
        let d2 = Doc::with_options(o);
        let t2 = d2.get_or_insert_text("test");

        t1.insert(&mut d1.transact_mut(), 0, "aaaaa");
        t1.insert(&mut d1.transact_mut(), 0, "bbb");

        exchange_updates(&[&d1, &d2]);

        t2.insert(&mut d2.transact_mut(), 4, "cccc");

        exchange_updates(&[&d1, &d2]);

        // t1: 'bbbaccccaaaa'
        t1.remove_range(&mut d1.transact_mut(), 2, 2); // => 'bbccccaaaa'
        t1.remove_range(&mut d1.transact_mut(), 3, 1); // => 'bbcccaaaa'
        t1.remove_range(&mut d1.transact_mut(), 3, 1); // => 'bbccaaaa'
        t1.remove_range(&mut d1.transact_mut(), 7, 1); // => 'bbccaaa'

        let blocks = {
            let mut txn = d1.transact_mut();
            let s = txn.snapshot();

            let mut blocks = HashSet::new();

            let mut i = 0;
            let mut deleted = s.delete_set.deleted_blocks();
            while let Some(BlockSlice::Item(b)) = deleted.next(&txn) {
                let item = txn.store.materialize(b);
                if let ItemContent::String(str) = &item.content {
                    let t = (
                        item.is_deleted(),
                        item.id,
                        item.len(),
                        str.as_str().to_string(),
                    );
                    blocks.insert(t);
                }
                i += 1;
                if i == 5 {
                    break;
                }
            }
            blocks
        };

        let expected = HashSet::from([
            (true, ID::new(1, 0), 1, "a".to_owned()),
            (true, ID::new(1, 4), 1, "a".to_owned()),
            (true, ID::new(1, 7), 1, "b".to_owned()),
            (true, ID::new(2, 1), 2, "cc".to_owned()),
        ]);

        assert_eq!(blocks, expected);
    }

    #[test]
    fn deleted_blocks2() {
        let mut ds = DeleteSet::new();
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        txt.push(&mut doc.transact_mut(), "testab");
        ds.insert(ID::new(1, 5), 1);
        let txn = doc.transact_mut();
        let mut i = ds.deleted_blocks();
        let ptr = i.next(&txn).unwrap();
        let start = ptr.clock_start();
        let end = ptr.clock_end();
        assert_eq!(start, 5);
        assert_eq!(end, 5);
        assert!(i.next(&txn).is_none());
    }
}
