use crate::block::{BlockRange, ClientID, ID};
use crate::block_store::BlockStore;
use crate::encoding::read::Error;
use crate::ids::{BlockSliceIter, IdMapInner, IdRanges};
use crate::iter::TxnIterator;
use crate::slice::BlockSlice;
use crate::store::Store;
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::ReadTxn;
use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smallvec::SmallVec;
use std::collections::btree_map::Entry;
use std::fmt::Formatter;
use std::hash::{Hash, Hasher};
use std::ops::Range;

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

/// [IdRange] describes a set of elements, represented as ranges of `u32` clock values, created by
/// a specific client, included in a corresponding [IdSet].
///
/// Internally backed by a [`SmallVec`] inlining a single `(Range<u32>, ())` pair.
/// Within the `IdRange` all ranges are sorted by `start`, overlapping or adjacent
/// ranges are merged together on insert.
pub type IdRange = IdRanges<()>;

impl IdRanges<()> {
    /// Check if current [IdRange] is a subset of `other`. This means that all the elements
    /// described by the current [IdRange] can be found within the bounds of `other` [IdRange].
    pub fn subset_of(&self, other: &Self) -> bool {
        for (range, _) in self.iter() {
            if !Self::is_range_covered(range, other) {
                return false;
            }
        }
        true
    }

    fn is_range_covered(range: &Range<u32>, other: &Self) -> bool {
        if range.is_empty() {
            return true;
        }

        let mut current = range.start;

        for (other_range, _) in other.iter() {
            if other_range.end <= current {
                continue;
            }
            if other_range.start > current {
                return false;
            }
            current = other_range.end;
            if current >= range.end {
                return true;
            }
        }

        current >= range.end
    }
}

impl Encode for IdRanges<()> {
    #[inline]
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_var(self.len() as u32);
        for (range, _) in self.iter() {
            range.encode(encoder);
        }
    }
}

impl Decode for IdRanges<()> {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let len: u32 = decoder.read_var()?;
        // Do NOT pre-allocate `len` entries up front: `len` is attacker-controlled (a few bytes
        // can declare a huge count), so `SmallVec::with_capacity(len)` is an allocation bomb that
        // aborts under a hard memory limit. `SmallVec::try_reserve` returns smallvec's own
        // `CollectionAllocErr` (not `std::collections::TryReserveError`), so it can't feed the
        // existing `Error::NotEnoughMemory` variant without widening the public error enum.
        // Instead let the `SmallVec` grow on push — each range consumes real bytes from the
        // decoder, so growth is bounded by the actual input length and it keeps its inline buffer
        // for the common small case. (State vector / block decoding use `try_reserve` directly.)
        let mut ranges = SmallVec::new();
        for _ in 0..len {
            ranges.push((Range::decode(decoder)?, ()));
        }
        Ok(IdRanges::from_raw(ranges))
    }
}

impl Hash for IdRanges<()> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for (range, _) in self.iter() {
            range.hash(state);
        }
    }
}

impl std::fmt::Display for IdRanges<()> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut i = self.ranges();
        write!(f, "[")?;
        if let Some(r) = i.next() {
            write!(f, "[{}..{})", r.start, r.end)?;
        }
        for r in i {
            write!(f, ", [{}..{})", r.start, r.end)?;
        }
        write!(f, "]")
    }
}

/// IdSet is a set describing ranges of blocks stored within the document.
///
/// Each block can be expressed as a `client_id, [start_clock..end_clock)` pair, where `client_id`
/// is a unique identifier of client which created given block, while `[start_clock..endclock)`
/// describe a clocks of the elements inserted into Yjs document. With that in mind, the [IdSet]
/// expresses a group of blocks in a very compact manner.
///
/// The most common use cases for [IdSet]s are:
/// - [IdSet] which is generated as part of the updates send between documents. It describes all
///   blocks deleted as part of an update.
/// - [crate::UndoManager] uses notion of stack items as a units of work, which can be undone/redone
///   together. Such stack item is a pair of (inserts, deletes), both of which are [IdSet]s.
#[derive(Default, Clone, PartialEq, Eq)]
pub struct IdSet(pub(crate) IdMapInner<()>);

/// Iterator over `(ClientID, Ranges)` pairs in an [`IdSet`].
pub struct Iter<'a>(std::collections::btree_map::Iter<'a, ClientID, IdRange>);

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a ClientID, Ranges<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, v)| (k, Ranges(v)))
    }
}

/// A view of the clock ranges for a single client in an [`IdSet`].
/// Provides iteration over `Range<u32>` values without exposing the
/// internal value type.
pub struct Ranges<'a>(&'a IdRange);

impl<'a> Ranges<'a> {
    /// Iterate over the clock ranges.
    pub fn iter(&self) -> RangesIter<'a> {
        RangesIter(self.0.inner().iter())
    }

    /// Check if a given clock value is covered by any range.
    pub fn contains_clock(&self, clock: u32) -> bool {
        self.0.contains_clock(clock)
    }

    /// Returns the number of disjoint ranges.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if there are no ranges.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn clock_start(&self) -> Option<u32> {
        self.0.clock_start()
    }

    pub(crate) fn clock_end(&self) -> Option<u32> {
        self.0.clock_end()
    }
}

impl<'a> IntoIterator for Ranges<'a> {
    type Item = &'a Range<u32>;
    type IntoIter = RangesIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        RangesIter(self.0.inner().iter())
    }
}

/// Iterator over `&Range<u32>` values within a single client's [`Ranges`].
pub struct RangesIter<'a>(std::slice::Iter<'a, (Range<u32>, ())>);

impl<'a> Iterator for RangesIter<'a> {
    type Item = &'a Range<u32>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(r, _)| r)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<'a> DoubleEndedIterator for RangesIter<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(r, _)| r)
    }
}

impl<'a> ExactSizeIterator for RangesIter<'a> {}

impl IdSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_iter<I1, I2>(iter: I1) -> Self
    where
        I1: IntoIterator<Item = (ClientID, I2)>,
        I2: IntoIterator<Item = Range<u32>>,
    {
        let mut set = Self::new();
        for (client_id, ranges) in iter {
            let range = IdRanges::from_ranges(ranges);
            set.0.clients_mut().insert(client_id, range);
        }
        set
    }

    /// Returns number of clients stored.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn client_ids<'a>(&'a self) -> impl Iterator<Item = ClientID> + 'a {
        self.0.clients().keys().copied()
    }

    pub fn iter(&self) -> Iter<'_> {
        Iter(self.0.clients().iter())
    }

    pub(crate) fn iter_blocks<'a>(&'a self, blocks: &'a BlockStore) -> BlockSliceIter<'a, ()> {
        self.0.iter_blocks(blocks)
    }

    pub fn range_mut(&mut self, client_id: ClientID) -> &mut IdRange {
        self.0.entry(client_id).or_default()
    }

    /// Check if current [IdSet] contains given `id`.
    pub fn contains(&self, id: &ID) -> bool {
        self.0.contains(id)
    }

    /// Checks if current ID set contains any data.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn insert(&mut self, id: ID, len: u32) {
        self.0
            .entry(id.client)
            .or_default()
            .insert(id.clock..(id.clock + len));
    }

    /// Inserts a new ID `range` corresponding with a given `client`.
    pub fn insert_range(&mut self, client: ClientID, range: IdRange) {
        match self.0.clients_mut().entry(client) {
            std::collections::btree_map::Entry::Occupied(mut e) => e.get_mut().merge(&range),
            std::collections::btree_map::Entry::Vacant(e) => {
                e.insert(range);
            }
        }
    }

    /// Merges another ID set into a current one, combining their information about observed ID
    /// ranges. Per-client [IdRange] merge maintains the canonical invariant on its own, so no
    /// trailing squash is needed.
    pub fn merge_with(&mut self, other: Self) {
        self.0.merge_with(&other.0);
    }

    /// Merges another ID set with a current one, returning a new set as a result. Information
    /// about observed ID ranges will be combined. Per-client [IdRange] merge maintains
    /// the canonical invariant on its own, so no trailing squash is needed.
    pub fn merge(&self, other: &Self) -> Self {
        Self(self.0.merge(&other.0))
    }

    /// Removes from `self` every clock range covered by `other`. Per-client [IdRange]s are
    /// excluded individually; clients absent from `other` are left untouched.
    pub fn diff_with(&mut self, other: &Self) {
        self.0.diff_with(&other.0);
    }

    /// Removes from `self` every clock range covered by `other`, returning new set as a result.
    /// Per-client [IdRange]s are excluded individually; clients absent from `other`
    /// are left untouched.
    pub fn diff(&self, other: &Self) -> Self {
        Self(self.0.diff(&other.0))
    }

    /// Replaces `self` with the per-client intersection against `other`. Clients present only
    /// in `self` (or only in `other`) are dropped.
    pub fn intersect_with(&mut self, other: &Self) {
        self.0.intersect_with(&other.0);
    }

    /// Returns a new ID set, which holds [IdRange]s which are present in both current set and
    /// the other one. Ranges not present in both of the inputs at the same time will be removed.
    pub fn intersect(&self, other: &Self) -> Self {
        Self(self.0.intersect(&other.0))
    }

    /// Remove a given range of elements from the current [IdSet]. If the client's [IdRange]
    /// becomes empty as a result, the client entry is dropped from the set entirely.
    pub fn remove_range(&mut self, range: &BlockRange) {
        if let Entry::Occupied(mut e) = self.0.entry(range.client) {
            let ranges = e.get_mut();
            ranges.remove(range.clock_range());
            if ranges.is_empty() {
                e.remove();
            }
        }
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
            encoder.write_var(client_id.get());
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
            let client: u64 = decoder.read_var()?;
            let range = IdRange::decode(decoder)?;
            set.0.clients_mut().insert(ClientID::new(client), range);
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

impl Serialize for IdSet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(&self.encode_v1())
    }
}

impl<'de> Deserialize<'de> for IdSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct IdSetVisitor;
        impl<'de> Visitor<'de> for IdSetVisitor {
            type Value = IdSet;

            fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
                write!(formatter, "IdSet")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                IdSet::decode_v1(v).map_err(E::custom)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut bytes = match seq.size_hint() {
                    None => Vec::new(),
                    Some(capacity) => Vec::with_capacity(capacity),
                };
                while let Some(x) = seq.next_element()? {
                    bytes.push(x);
                }
                use serde::de::Error;
                IdSet::decode_v1(&bytes).map_err(A::Error::custom)
            }
        }

        deserializer.deserialize_bytes(IdSetVisitor)
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
        for (k, v) in self.0.iter() {
            s.field(&k.to_string(), v);
        }
        s.finish()
    }
}

/// An extension over [IdSet] that defines methods specific to work with block store deletions.
pub(crate) trait DeleteSet {
    /// Creates a [IdSet] by reading all deleted blocks and including their clock ranges into
    /// the delete set itself.
    fn from_store(store: &BlockStore) -> Self;

    fn try_squash_with(&mut self, store: &mut Store);

    fn blocks(&self) -> Blocks<'_>;
}

impl DeleteSet for IdSet {
    fn from_store(store: &BlockStore) -> Self {
        let mut set = IdSet::new();
        for (&client, blocks) in store.iter() {
            let mut deletes = IdRange::with_capacity(blocks.len());
            for block in blocks.iter() {
                let block = block.as_ref();
                if block.is_deleted() {
                    let (start, end) = block.clock_range();
                    deletes.insert(start..(end + 1));
                }
            }

            if !deletes.is_empty() {
                set.0.clients_mut().insert(client, deletes);
            }
        }
        set
    }

    fn try_squash_with(&mut self, store: &mut Store) {
        #[cfg(test)]
        if test::PER_RANGE_COMPACTION.with(|on| on.get()) {
            return try_squash_with_per_range(self, store);
        }
        // try to merge deleted / gc'd items, with all ranges of a client in one pass over its
        // blocks
        for (&client, range) in self.0.iter() {
            let blocks = store.blocks.get_client_blocks_mut(client);
            blocks.squash_deleted_ranges(range.iter().rev().map(|(r, _)| r.clone()));
        }
    }

    fn blocks(&self) -> Blocks<'_> {
        Blocks::new(self)
    }
}

/// What [DeleteSet::try_squash_with] did before it squashed all ranges of a client in one pass:
/// one `ClientBlockList::squash_left_range_compaction` per range. Kept for tests, which compare
/// the two.
#[cfg(test)]
fn try_squash_with_per_range(set: &IdSet, store: &mut Store) {
    for (&client, range) in set.0.iter() {
        squash_per_range(store.blocks.get_client_blocks_mut(client), range);
    }
}

/// The per-range compaction of one client's blocks.
#[cfg(test)]
fn squash_per_range(blocks: &mut crate::block_store::ClientBlockList, range: &IdRange) {
    for (r, _) in range.iter().rev() {
        // start with merging the item next to the last deleted item
        let mut si = (blocks.len() - 1).min(1 + blocks.find_index(r.end - 1).unwrap_or_default());
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

pub(crate) struct Blocks<'ds> {
    ds_iter: std::collections::btree_map::Iter<'ds, ClientID, IdRange>,
    current_range: Option<Range<u32>>,
    current_client_id: Option<ClientID>,
    range_iter: Option<std::slice::Iter<'ds, (Range<u32>, ())>>,
    current_index: Option<usize>,
}

impl<'ds> Blocks<'ds> {
    pub(crate) fn new(ds: &'ds IdSet) -> Self {
        let ds_iter = ds.0.clients().iter();
        Blocks {
            ds_iter,
            current_client_id: None,
            current_range: None,
            range_iter: None,
            current_index: None,
        }
    }
}

impl<'ds> TxnIterator for Blocks<'ds> {
    type Item = BlockSlice;

    fn next<T: ReadTxn>(&mut self, txn: &T) -> Option<Self::Item> {
        if let Some(r) = self.current_range.clone() {
            let mut block = if let Some(idx) = self.current_index.as_mut() {
                if let Some(block) = txn
                    .store()
                    .blocks
                    .get_client(&self.current_client_id?)
                    .unwrap()
                    .get(*idx)
                {
                    *idx += 1;
                    block.as_ref().as_slice()
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
                if let Some(idx) = list.find_index(r.start) {
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
                self.current_client_id = Some(*client_id);
                self.current_index = None;
                self.range_iter = Some(range.inner().iter());
                self.range_iter.as_mut().unwrap()
            };
            self.current_range = match range_iter.next() {
                None => {
                    let (client_id, range) = self.ds_iter.next()?;
                    self.current_client_id = Some(*client_id);
                    self.current_index = None;
                    let mut iter = range.inner().iter();
                    let entry = iter.next();
                    self.range_iter = Some(iter);
                    entry.map(|(r, _)| r.clone())
                }
                Some((r, _)) => Some(r.clone()),
            };
            return self.next(txn);
        }
    }
}

#[cfg(test)]
pub(crate) mod test {
    use crate::block::{Block, BlockRange, ClientID, ItemContent};
    use crate::block_store::ClientBlockList;
    use crate::id_set::{squash_per_range, DeleteSet, IdRange, IdSet};
    use crate::ids::IdRanges;
    use crate::iter::TxnIterator;
    use crate::slice::BlockSlice;
    use crate::test_utils::exchange_updates;
    use crate::types::ToJson;
    use crate::undo::UndoManager;
    use crate::updates::decoder::{Decode, DecoderV1};
    use crate::updates::encoder::{Encode, Encoder, EncoderV1};
    use crate::{
        Array, ArrayRef, GetString, Map, MapPrelim, MapRef, Out, StateVector, TextPrelim, TextRef,
        Update,
    };
    use crate::{Doc, Options, ReadTxn, Text, Transact, ID};
    use fastrand::Rng;
    use std::cell::Cell;
    use std::collections::HashSet;
    use std::fmt::Debug;

    /// Helper to build an IdRange from a list of ranges.
    fn id_range(ranges: impl IntoIterator<Item = Range<u32>>) -> IdRange {
        IdRanges::from_ranges(ranges)
    }

    use std::ops::Range;

    #[test]
    fn decode_rejects_range_length_amplification() {
        // Regression: an id set (used for delete sets carried by updates) must not let a few
        // bytes declare a huge range count and trigger an eager multi-hundred-MB allocation
        // (allocation bomb / DoS). Growing the `SmallVec` on push instead of pre-allocating
        // `len` entries bounds the allocation by the actual input length, so an oversized count
        // fails with a clean decode error instead of aborting under a hard memory limit.
        // Bytes: client_len = 1, client = 1, range_count = ~250M (0xFF 0xFF 0xFF 0x7A), no data.
        let adversarial = [0x01u8, 0x01, 0xFF, 0xFF, 0xFF, 0x7A];
        let result = IdSet::decode_v1(&adversarial);
        assert!(
            result.is_err(),
            "an oversized id range length must be rejected, not eagerly allocated"
        );
    }

    #[test]
    fn id_range_merge_continous() {
        let mut a = id_range([0..5]);
        a.merge(&id_range([2..4]));
        assert_eq!(a, id_range([0..5]));

        let mut a = id_range([0..5]);
        a.merge(&id_range([4..9]));
        assert_eq!(a, id_range([0..9]));

        let mut a = id_range([0..5]);
        a.merge(&id_range([5..9]));
        assert_eq!(a, id_range([0..9]));

        let mut a = id_range([0..4]);
        a.merge(&id_range([6..9]));
        assert_eq!(a, id_range([0..4, 6..9]));
    }

    #[test]
    fn id_range_merge_canonical() {
        let mut a = id_range([0..3]);
        a.merge(&IdRange::default());
        assert_eq!(a, id_range([0..3]));

        let mut a = IdRange::default();
        a.merge(&id_range([1..5]));
        assert_eq!(a, id_range([1..5]));

        let mut a = id_range([5..7]);
        a.merge(&id_range([1..3]));
        assert_eq!(a, id_range([1..3, 5..7]));

        let mut a = id_range([0..2, 4..6, 10..12]);
        a.merge(&id_range([1..5, 11..15]));
        assert_eq!(a, id_range([0..6, 10..15]));

        let mut a = id_range([0..3, 10..12]);
        a.merge(&id_range([3..5, 12..14]));
        assert_eq!(a, id_range([0..5, 10..14]));

        let mut a = id_range([0..2, 6..8]);
        a.merge(&id_range([3..5, 9..11]));
        assert_eq!(a, id_range([0..2, 3..5, 6..8, 9..11]));
    }

    #[test]
    fn id_range_compact() {
        let mut r = IdRange::default();
        r.insert(0..3);
        r.insert(3..5);
        r.insert(6..7);
        assert_eq!(r, id_range([0..5, 6..7]));
    }

    #[test]
    fn id_range_contains() {
        assert!(!id_range([1..3]).contains_clock(0));
        assert!(id_range([1..3]).contains_clock(1));
        assert!(id_range([1..3]).contains_clock(2));
        assert!(!id_range([1..3]).contains_clock(3));

        assert!(!id_range([1..3, 4..5]).contains_clock(0));
        assert!(id_range([1..3, 4..5]).contains_clock(1));
        assert!(id_range([1..3, 4..5]).contains_clock(2));
        assert!(!id_range([1..3, 4..5]).contains_clock(3));
        assert!(id_range([1..3, 4..5]).contains_clock(4));
        assert!(!id_range([1..3, 4..5]).contains_clock(5));
        assert!(!id_range([1..3, 4..5]).contains_clock(6));
    }

    #[test]
    fn id_range_push() {
        let mut range = IdRange::default();

        range.insert(0..4);
        assert_eq!(range, id_range([0..4]));

        range.insert(4..6);
        assert_eq!(range, id_range([0..6]));

        range.insert(7..9);
        assert_eq!(range, id_range([0..6, 7..9]));
    }

    #[test]
    fn id_range_push_out_of_order() {
        let mut range = IdRange::default();
        range.insert(5..7);
        range.insert(1..3);
        range.insert(3..5);
        assert_eq!(range, id_range([1..7]));
    }

    #[test]
    fn id_range_push_skips_empty() {
        let mut range = IdRange::default();
        range.insert(5..5);
        assert_eq!(range, IdRange::default());
        range.insert(0..3);
        range.insert(7..7);
        assert_eq!(range, id_range([0..3]));
    }

    #[test]
    fn id_range_push_chain_absorb() {
        let mut range = IdRange::default();
        range.insert(0..2);
        range.insert(4..6);
        range.insert(8..10);
        range.insert(1..9);
        assert_eq!(range, id_range([0..10]));
    }

    #[test]
    fn id_range_push_adjacency_merge() {
        let mut range = IdRange::default();
        range.insert(0..3);
        range.insert(3..5);
        assert_eq!(range, id_range([0..5]));
    }

    #[test]
    fn id_range_push_front() {
        let mut range = IdRange::default();
        range.insert(5..7);
        range.insert(1..3);
        assert_eq!(range, id_range([1..3, 5..7]));
    }

    #[test]
    fn id_range_remove() {
        let mut r = id_range([0..10]);
        r.remove(5..5);
        assert_eq!(r, id_range([0..10]));

        let mut r = id_range([0..5]);
        r.remove(10..15);
        assert_eq!(r, id_range([0..5]));

        let mut r = id_range([0..5, 10..15]);
        r.remove(5..10);
        assert_eq!(r, id_range([0..5, 10..15]));

        let mut r = id_range([0..10]);
        r.remove(3..7);
        assert_eq!(r, id_range([0..3, 7..10]));

        let mut r = id_range([0..10]);
        r.remove(0..3);
        assert_eq!(r, id_range([3..10]));

        let mut r = id_range([0..10]);
        r.remove(7..10);
        assert_eq!(r, id_range([0..7]));

        let mut r = id_range([0..10]);
        r.remove(0..10);
        assert_eq!(r, IdRange::default());

        let mut r = id_range([0..3, 6..10, 12..15]);
        r.remove(2..13);
        assert_eq!(r, id_range([0..2, 13..15]));

        let mut r = id_range([0..3, 6..10, 12..15]);
        r.remove(0..15);
        assert_eq!(r, IdRange::default());

        let mut r = id_range([0..5, 10..15]);
        r.remove(12..100);
        assert_eq!(r, id_range([0..5, 10..12]));

        let mut r = id_range([5..10]);
        r.remove(0..7);
        assert_eq!(r, id_range([7..10]));
    }

    #[test]
    fn id_set_remove() {
        let mut set = IdSet::from_iter([(ClientID::new(1), [0..10])]);
        set.remove_range(&BlockRange::new(ID::new(ClientID::new(2), 0), 5));
        assert_eq!(set, IdSet::from_iter([(ClientID::new(1), [0..10])]));

        let mut set = IdSet::from_iter([(ClientID::new(1), [0..10])]);
        set.remove_range(&BlockRange::new(ID::new(ClientID::new(1), 3), 4));
        assert_eq!(set, IdSet::from_iter([(ClientID::new(1), [0..3, 7..10])]));

        let mut set = IdSet::from_iter([(ClientID::new(1), [0..10]), (ClientID::new(2), [0..5])]);
        set.remove_range(&BlockRange::new(ID::new(ClientID::new(1), 0), 10));
        assert_eq!(set, IdSet::from_iter([(ClientID::new(2), [0..5])]));

        let mut set = IdSet::from_iter([(ClientID::new(1), [0..10])]);
        set.remove_range(&BlockRange::new(ID::new(ClientID::new(1), 5), 0));
        assert_eq!(set, IdSet::from_iter([(ClientID::new(1), [0..10])]));
    }

    #[test]
    fn id_range_push_subset_of_last() {
        let mut range = IdRange::default();
        range.insert(0..10);
        range.insert(5..7);
        assert_eq!(range, id_range([0..10]));

        let mut range = IdRange::default();
        range.insert(0..10);
        range.insert(3..10);
        assert_eq!(range, id_range([0..10]));

        let mut range = IdRange::default();
        range.insert(0..10);
        range.insert(0..10);
        assert_eq!(range, id_range([0..10]));
    }

    #[test]
    fn id_range_subset_of() {
        assert!(id_range([1..2]).subset_of(&id_range([1..2])));
        assert!(id_range([1..2]).subset_of(&id_range([0..2])));
        assert!(id_range([1..2]).subset_of(&id_range([1..3])));

        assert!(id_range([1..2, 3..4]).subset_of(&id_range([1..4])));
        assert!(id_range([1..2, 3..4, 5..6]).subset_of(&id_range([1..2, 3..6])));
        assert!(id_range([3..4, 5..6]).subset_of(&id_range([0..1, 3..6])));
        assert!(id_range([3..4, 5..6]).subset_of(&id_range([3..4, 5..6])));

        assert!(!id_range([1..3]).subset_of(&id_range([0..2])));
        assert!(!id_range([1..3]).subset_of(&id_range([1..2])));
        assert!(!id_range([1..3]).subset_of(&id_range([2..3])));
        assert!(!id_range([1..3]).subset_of(&id_range([2..4])));

        assert!(!id_range([1..2, 3..4]).subset_of(&id_range([1..3])));
        assert!(!id_range([1..2, 3..6]).subset_of(&id_range([1..2, 3..5])));
        assert!(!id_range([1..2, 3..6]).subset_of(&id_range([1..2, 4..7])));
    }

    #[test]
    fn id_range_exclude() {
        let mut a = id_range([0..4]);
        a.exclude(&id_range([0..2]));
        assert_eq!(a, id_range([2..4]));

        let mut a = id_range([0..4]);
        a.exclude(&id_range([2..4]));
        assert_eq!(a, id_range([0..2]));

        let mut a = id_range([0..4]);
        a.exclude(&id_range([1..3]));
        assert_eq!(a, id_range([0..1, 3..4]));

        let mut a = id_range([0..10]);
        a.exclude(&id_range([1..3, 4..5]));
        assert_eq!(a, id_range([0..1, 3..4, 5..10]));

        let mut a = id_range([0..4, 5..6, 7..10]);
        a.exclude(&id_range([3..8]));
        assert_eq!(a, id_range([0..3, 8..10]));

        let mut a = id_range([0..4, 7..10]);
        a.exclude(&id_range([3..5, 6..9]));
        assert_eq!(a, id_range([0..3, 9..10]));

        let mut a = id_range([0..4, 7..10]);
        a.exclude(&id_range([2..3, 5..6, 9..10]));
        assert_eq!(a, id_range([0..2, 3..4, 7..9]));
    }

    #[test]
    fn id_range_intersect() {
        let mut a = id_range([0..10]);
        a.intersect(&id_range([5..15]));
        assert_eq!(a, id_range([5..10]));

        let mut a = id_range([0..5, 10..15]);
        a.intersect(&id_range([3..12]));
        assert_eq!(a, id_range([3..5, 10..12]));

        let mut a = id_range([0..5]);
        a.intersect(&id_range([10..15]));
        assert_eq!(a, IdRange::default());

        let mut a = id_range([1..4, 6..9]);
        a.intersect(&id_range([2..7]));
        assert_eq!(a, id_range([2..4, 6..7]));

        let mut a = id_range([2..8]);
        a.intersect(&id_range([0..10]));
        assert_eq!(a, id_range([2..8]));

        let mut a = id_range([5..15]);
        a.intersect(&id_range([0..10]));
        assert_eq!(a, id_range([5..10]));

        let mut a = id_range([0..5, 10..15, 20..25]);
        a.intersect(&id_range([3..12, 22..30]));
        assert_eq!(a, id_range([3..5, 10..12, 22..25]));

        let mut a = id_range([5..10]);
        a.intersect(&id_range([5..10]));
        assert_eq!(a, id_range([5..10]));

        let mut a = id_range([0..5]);
        a.intersect(&id_range([4..10]));
        assert_eq!(a, id_range([4..5]));

        let mut a = id_range([0..5]);
        a.intersect(&id_range([5..10]));
        assert_eq!(a, IdRange::default());
    }

    #[test]
    fn id_range_encode_decode() {
        roundtrip(&id_range([0..4]));
        roundtrip(&id_range([1..4, 5..8]));
    }

    #[test]
    fn id_set_exclude() {
        let mut a = IdSet::from_iter([(ClientID::new(1), [0..10]), (ClientID::new(2), [0..5])]);
        let b = IdSet::from_iter([(ClientID::new(2), [0..3]), (ClientID::new(3), [0..100])]);
        a.diff_with(&b);
        assert_eq!(
            a,
            IdSet::from_iter([(ClientID::new(1), [0..10]), (ClientID::new(2), [3..5])])
        );

        let mut a = IdSet::from_iter([(ClientID::new(1), [0..10])]);
        let b = IdSet::from_iter([(ClientID::new(1), [3..7])]);
        a.diff_with(&b);
        assert_eq!(a, IdSet::from_iter([(ClientID::new(1), [0..3, 7..10])]));

        let mut a = IdSet::from_iter([(ClientID::new(1), [0..5]), (ClientID::new(2), [0..5])]);
        let b = IdSet::from_iter([(ClientID::new(1), [0..5])]);
        a.diff_with(&b);
        assert_eq!(a, IdSet::from_iter([(ClientID::new(2), [0..5])]));

        let mut a = IdSet::from_iter([(ClientID::new(1), [0..10])]);
        a.diff_with(&IdSet::new());
        assert_eq!(a, IdSet::from_iter([(ClientID::new(1), [0..10])]));
    }

    #[test]
    fn id_set_intersect() {
        let mut a = IdSet::from_iter([(ClientID::new(1), [0..10]), (ClientID::new(2), [0..5])]);
        let b = IdSet::from_iter([(ClientID::new(1), [5..15])]);
        a.intersect_with(&b);
        assert_eq!(a, IdSet::from_iter([(ClientID::new(1), [5..10])]));

        let mut a = IdSet::from_iter([(ClientID::new(1), [0..10])]);
        let b = IdSet::from_iter([(ClientID::new(1), [3..7]), (ClientID::new(2), [0..100])]);
        a.intersect_with(&b);
        assert_eq!(a, IdSet::from_iter([(ClientID::new(1), [3..7])]));

        let mut a = IdSet::from_iter([(ClientID::new(1), [0..5]), (ClientID::new(2), [0..5])]);
        let b = IdSet::from_iter([(ClientID::new(1), [10..20]), (ClientID::new(2), [1..4])]);
        a.intersect_with(&b);
        assert_eq!(a, IdSet::from_iter([(ClientID::new(2), [1..4])]));

        let mut a = IdSet::from_iter([(ClientID::new(1), [0..10])]);
        a.intersect_with(&IdSet::new());
        assert!(a.is_empty());
    }

    #[test]
    fn id_set_encode_decode() {
        let mut set = IdSet::new();
        set.insert(ID::new(ClientID::new(124), 0), 1);
        set.insert(ID::new(ClientID::new(1337), 0), 12);
        set.insert(ID::new(ClientID::new(124), 1), 3);

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
        o.client_id = ClientID::new(1);
        o.skip_gc = true;
        let d1 = Doc::with_options(o.clone());
        let t1 = d1.get_or_insert_text("test");

        o.client_id = ClientID::new(2);
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
            let mut deleted = s.delete_set.blocks();
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
            (true, ID::new(ClientID::new(1), 0), 1, "a".to_owned()),
            (true, ID::new(ClientID::new(1), 4), 1, "a".to_owned()),
            (true, ID::new(ClientID::new(1), 7), 1, "b".to_owned()),
            (true, ID::new(ClientID::new(2), 1), 2, "cc".to_owned()),
        ]);

        assert_eq!(blocks, expected);
    }

    #[test]
    fn deleted_blocks2() {
        let mut ds = IdSet::new();
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        txt.push(&mut doc.transact_mut(), "testab");
        ds.insert(ID::new(ClientID::new(1), 5), 1);
        let txn = doc.transact_mut();
        let mut i = ds.blocks();
        let ptr = i.next(&txn).unwrap();
        let start = ptr.clock_start();
        let end = ptr.clock_end();
        assert_eq!(start, 5);
        assert_eq!(end, 5);
        assert!(i.next(&txn).is_none());
    }

    thread_local! {
        /// Makes [DeleteSet::try_squash_with] compact one deleted range at a time, as it did
        /// before it compacted all ranges of a client in one pass.
        pub(crate) static PER_RANGE_COMPACTION: Cell<bool> = Cell::new(false);
    }

    /// Each client's blocks as `(clock, length, kind)`, the whole state encoded, and the content
    /// of the root types.
    type Snapshot = (Vec<(ClientID, Vec<(u32, u32, String)>)>, Vec<u8>, String);

    fn snapshot(doc: &Doc) -> Snapshot {
        let (t, a, m) = roots(doc);
        let txn = doc.transact();
        let mut clients: Vec<_> = txn
            .store()
            .blocks
            .iter()
            .map(|(client, blocks)| {
                let blocks = blocks
                    .iter()
                    .map(|block| {
                        let block = block.as_ref();
                        let kind = match block {
                            Block::GC(_) => "gc".to_owned(),
                            Block::Skip(_) => "skip".to_owned(),
                            Block::Item(item) => format!(
                                "item deleted: {}, keep: {}, {:?}",
                                item.is_deleted(),
                                item.info.is_keep(),
                                std::mem::discriminant(&item.content)
                            ),
                        };
                        (block.clock_start(), block.len(), kind)
                    })
                    .collect();
                (*client, blocks)
            })
            .collect();
        clients.sort_by_key(|(client, _)| *client);
        let state = txn.encode_state_as_update_v1(&StateVector::default());
        let content = format!(
            "{} {} {}",
            t.get_string(&txn),
            // through serde_json's map, whose keys are sorted
            serde_json::to_value(a.to_json(&txn)).unwrap(),
            serde_json::to_value(m.to_json(&txn)).unwrap(),
        );
        (clients, state, content)
    }

    /// A few insertions into the text, the array, the map and the texts nested in them, in one
    /// transaction.
    fn insert_some(doc: &Doc, rng: &mut Rng) {
        let (t, a, m) = roots(doc);
        let mut txn = doc.transact_mut();
        for _ in 0..rng.usize(1..=6) {
            match rng.u8(..6) {
                0 | 1 => {
                    let at = rng.u32(..=t.len(&txn));
                    t.insert(&mut txn, at, &"abcdef"[..rng.usize(1..=4)]);
                }
                2 => {
                    let at = rng.u32(..=a.len(&txn));
                    match rng.u8(..4) {
                        0 => {
                            a.insert(&mut txn, at, TextPrelim::new("xyz"));
                        }
                        1 => {
                            a.insert(&mut txn, at, MapPrelim::from([("k", rng.i64(..9))]));
                        }
                        _ => {
                            let values: Vec<i64> = (0..rng.i64(1..=4)).collect();
                            a.insert_range(&mut txn, at, values);
                        }
                    }
                }
                3 => {
                    let key = format!("k{}", rng.u8(..5));
                    if rng.bool() {
                        m.insert(&mut txn, key, TextPrelim::new("uvw"));
                    } else {
                        m.insert(&mut txn, key, rng.i64(..9));
                    }
                }
                _ => {
                    if let Some(nested) = nested_text(&txn, &a, &m, rng) {
                        let at = rng.u32(..=nested.len(&txn));
                        nested.insert(&mut txn, at, &"pqrs"[..rng.usize(1..=3)]);
                    }
                }
            }
        }
    }

    /// Deletions in one transaction, many of them separate: single elements, short ranges,
    /// every other element of a text, the first and last elements, map entries (with what is
    /// nested in them), and text nested in the array and the map. Returns the transaction's
    /// update.
    fn delete_many(doc: &Doc, rng: &mut Rng, origin: Option<&str>) -> Vec<u8> {
        let (t, a, m) = roots(doc);
        let mut txn = match origin {
            Some(origin) => doc.transact_mut_with(origin),
            None => doc.transact_mut(),
        };
        for _ in 0..rng.usize(1..=12) {
            match rng.u8(..8) {
                0 | 1 => {
                    let len = t.len(&txn);
                    if len > 0 {
                        let at = rng.u32(..len);
                        let n = rng.u32(1..=3).min(len - at);
                        t.remove_range(&mut txn, at, n);
                    }
                }
                2 => {
                    // every other character, from the end
                    let len = t.len(&txn);
                    let mut at = len;
                    while at >= 2 {
                        at -= 2;
                        t.remove_range(&mut txn, at, 1);
                    }
                }
                3 => {
                    // the first or the last
                    let len = t.len(&txn);
                    if len > 0 {
                        let at = if rng.bool() { 0 } else { len - 1 };
                        t.remove_range(&mut txn, at, 1);
                    }
                }
                4 => {
                    let len = a.len(&txn);
                    if len > 0 {
                        let at = rng.u32(..len);
                        let n = rng.u32(1..=2).min(len - at);
                        a.remove_range(&mut txn, at, n);
                    }
                }
                5 => {
                    let key = format!("k{}", rng.u8(..5));
                    m.remove(&mut txn, &key);
                }
                _ => {
                    if let Some(nested) = nested_text(&txn, &a, &m, rng) {
                        let len = nested.len(&txn);
                        if len > 0 {
                            let at = rng.u32(..len);
                            nested.remove_range(&mut txn, at, 1);
                        }
                    }
                }
            }
        }
        txn.encode_update_v1()
    }

    fn roots(doc: &Doc) -> (TextRef, ArrayRef, MapRef) {
        (
            doc.get_or_insert_text("t"),
            doc.get_or_insert_array("a"),
            doc.get_or_insert_map("m"),
        )
    }

    /// A text nested in the array or the map, if there is one where `rng` looks.
    fn nested_text<T: ReadTxn>(
        txn: &T,
        a: &ArrayRef,
        m: &MapRef,
        rng: &mut Rng,
    ) -> Option<TextRef> {
        let value = if rng.bool() {
            let len = a.len(txn);
            if len == 0 {
                return None;
            }
            a.get(txn, rng.u32(..len))?
        } else {
            m.get(txn, &format!("k{}", rng.u8(..5)))?
        };
        match value {
            Out::YText(text) => Some(text),
            _ => None,
        }
    }

    /// Three clients insert and delete, and sync now and then; a fourth document receives each
    /// deleting transaction as an update. What the first client deletes with the origin
    /// "tracked" is kept by an undo manager, not collected. Some runs do not collect at all.
    /// Returns a snapshot of the editing document and of the receiving one after each step, and
    /// the first document.
    fn compaction_run(seed: u64, steps: usize) -> (Vec<Snapshot>, Doc) {
        let mut rng = Rng::with_seed(seed);
        let skip_gc = rng.u8(..6) == 0;
        let doc = |client| {
            let mut options = Options::with_client_id(ClientID::new(client));
            options.skip_gc = skip_gc;
            Doc::with_options(options)
        };
        let docs = [doc(1), doc(2), doc(3)];
        let receiver = doc(4);
        let mut undo: UndoManager<()> = UndoManager::new();
        undo.include_origin("tracked");
        let (t, a, m) = roots(&docs[0]);
        undo.expand_scope(&docs[0], &t);
        undo.expand_scope(&docs[0], &a);
        undo.expand_scope(&docs[0], &m);

        let mut snapshots = Vec::new();
        for _ in 0..steps {
            let i = rng.usize(..docs.len());
            let doc = &docs[i];
            match rng.u8(..10) {
                0..=4 => insert_some(doc, &mut rng),
                5..=8 => {
                    // the receiver has everything the document has before the deletions
                    let sv = receiver.transact().state_vector();
                    let missing = doc.transact().encode_diff_v1(&sv);
                    let mut txn = receiver.transact_mut();
                    txn.apply_update(Update::decode_v1(&missing).unwrap())
                        .unwrap();
                    drop(txn);

                    let origin = if i == 0 && rng.bool() {
                        Some("tracked")
                    } else {
                        None
                    };
                    let update = delete_many(doc, &mut rng, origin);
                    let mut txn = receiver.transact_mut();
                    txn.apply_update(Update::decode_v1(&update).unwrap())
                        .unwrap();
                }
                _ => exchange_updates(&[&docs[0], &docs[1], &docs[2]]),
            }
            snapshots.push(snapshot(doc));
            snapshots.push(snapshot(&receiver));
        }
        let [first, _, _] = docs;
        (snapshots, first)
    }

    /// The deleted ranges of a transaction are compacted in one pass per client
    /// (`ClientBlockList::squash_deleted_ranges`). The block lists, the encoded state and the
    /// content must be exactly those that compacting one range at a time gives.
    #[test]
    fn compacting_deleted_ranges_in_one_pass_is_the_same_as_one_at_a_time() {
        const SEEDS: u64 = 300;
        const STEPS: usize = 40;
        for seed in 0..SEEDS {
            let (one_pass, _) = compaction_run(seed, STEPS);
            PER_RANGE_COMPACTION.with(|on| on.set(true));
            let (per_range, _) = compaction_run(seed, STEPS);
            PER_RANGE_COMPACTION.with(|on| on.set(false));
            for (step, (a, b)) in one_pass.iter().zip(per_range.iter()).enumerate() {
                assert_eq!(a, b, "seed {seed}, snapshot {step}");
            }
            assert_eq!(one_pass.len(), per_range.len());
        }
    }
    /// The same for delete sets that no transaction makes: random ranges over every client's
    /// clocks, which may start and end inside a block, compacted directly on the documents of
    /// the test above.
    #[test]
    fn compacting_any_deleted_ranges_in_one_pass_is_the_same_as_one_at_a_time() {
        const SEEDS: u64 = 300;
        const STEPS: usize = 40;
        for seed in 0..SEEDS {
            let (_, one_pass) = compaction_run(seed, STEPS);
            let (_, per_range) = compaction_run(seed, STEPS);
            assert_eq!(snapshot(&one_pass), snapshot(&per_range), "seed {seed}");

            let mut rng = Rng::with_seed(seed);
            let mut ranges = IdSet::new();
            let clients: Vec<(ClientID, u32)> = one_pass
                .transact()
                .store()
                .blocks
                .iter()
                .map(|(client, blocks)| (*client, blocks.clock()))
                .collect();
            for (client, clock) in clients {
                let mut at = rng.u32(..3);
                while at < clock {
                    let len = rng.u32(1..=4).min(clock - at);
                    ranges.insert(ID::new(client, at), len);
                    at += len + rng.u32(1..=4);
                }
            }

            ranges
                .clone()
                .try_squash_with(one_pass.transact_mut().store_mut());
            PER_RANGE_COMPACTION.with(|on| on.set(true));
            ranges
                .clone()
                .try_squash_with(per_range.transact_mut().store_mut());
            PER_RANGE_COMPACTION.with(|on| on.set(false));
            assert_eq!(snapshot(&one_pass), snapshot(&per_range), "seed {seed}");
        }
    }
    /// The same on block lists of GC and skip blocks alone, built directly, with GC blocks next
    /// to each other that commits would have merged: a range can then end inside a block that
    /// the compaction of a later range squashed the next blocks into.
    #[test]
    fn compacting_ranges_of_gc_blocks_in_one_pass_is_the_same_as_one_at_a_time() {
        const SEEDS: u64 = 3000;
        let client = ClientID::new(1);
        let kinds = |blocks: &ClientBlockList| -> Vec<(u32, u32, bool)> {
            blocks
                .iter()
                .map(|block| {
                    let block = block.as_ref();
                    let gc = matches!(block, Block::GC(_));
                    (block.clock_start(), block.len(), gc)
                })
                .collect()
        };
        for seed in 0..SEEDS {
            let mut rng = Rng::with_seed(seed);
            let mut spec = Vec::new();
            let mut clock = 0;
            for _ in 0..rng.usize(1..40) {
                let len = rng.u32(1..=4);
                spec.push((clock, len, rng.u8(..5) != 0));
                clock += len;
            }
            let list = || {
                let mut list = ClientBlockList::default();
                for &(clock, len, gc) in spec.iter() {
                    let range = BlockRange::new(ID::new(client, clock), len);
                    let block = if gc {
                        Block::GC(range)
                    } else {
                        Block::Skip(range)
                    };
                    list.insert(list.len(), block);
                }
                list
            };
            let mut ranges = IdRange::default();
            let mut at = rng.u32(..3);
            while at < clock {
                let len = rng.u32(1..=4).min(clock - at);
                ranges.insert(at..at + len);
                at += len + rng.u32(1..=4);
            }

            let mut one_pass = list();
            one_pass.squash_deleted_ranges(ranges.iter().rev().map(|(r, _)| r.clone()));
            let mut per_range = list();
            squash_per_range(&mut per_range, &ranges);
            assert_eq!(kinds(&one_pass), kinds(&per_range), "seed {seed}");
        }
    }
}
