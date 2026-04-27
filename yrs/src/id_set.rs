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
use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smallvec::{smallvec, SmallVec};
use std::collections::HashMap;
use std::fmt::Formatter;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::ops::Range;
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

/// [IdRange] describes a set of elements, represented as ranges of `u32` clock values, created by
/// specific client, included in a corresponding [IdSet].
///
/// Internally backed by a [SmallVec] inlining a single [Range], so the common case of one
/// continuous range incurs no heap allocation.
///
/// Within the `IdRange` all ranges are values sorted by `start` of the range, overlapping or adjacent
/// ranges are merged together on insert.
#[repr(transparent)]
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct IdRange(SmallVec<[Range<u32>; 2]>);

impl std::ops::Deref for IdRange {
    type Target = [Range<u32>];
    #[inline]
    fn deref(&self) -> &[Range<u32>] {
        &self.0
    }
}

impl<'a> IntoIterator for &'a IdRange {
    type Item = &'a Range<u32>;
    type IntoIter = std::slice::Iter<'a, Range<u32>>;
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl IdRange {
    pub fn with_capacity(capacity: usize) -> Self {
        IdRange(SmallVec::with_capacity(capacity))
    }

    /// Inverts current [IdRange], returning another [IdRange] that contains all
    /// "holes" (ranges not included in current range). If current range is a continuous space
    /// starting from the initial clock (eg. [0..5)), then returned range will be empty.
    pub fn invert(&self) -> IdRange {
        let mut inv = SmallVec::new();
        let mut start = 0;
        for range in self.0.iter() {
            if range.start > start {
                inv.push(start..range.start);
            }
            start = range.end;
        }
        IdRange(inv)
    }

    /// Check if given clock exists within current [IdRange].
    pub fn contains_clock(&self, clock: u32) -> bool {
        self.0.iter().any(|r| r.contains(&clock))
    }

    fn push(&mut self, range: Range<u32>) {
        if range.start >= range.end {
            return;
        }

        // tail-fast push - if inserted range >= last element, we don't need binary search
        if let Some(last) = self.0.last_mut() {
            if range.start >= last.start {
                if range.start > last.end {
                    self.0.push(range);
                } else {
                    last.end = last.end.max(range.end); // inserted range is overlapping -> merge to the end
                }
                return;
            }
        }

        let mut start = range.start;
        let mut end = range.end;
        let mut lo = self.0.partition_point(|r| r.start < range.start);

        // Absorb the left neighbour if it touches or overlaps the new range.
        // `>= start` (not `> start`) so adjacents like [0..3) + [3..5) merge into [0..5).
        if lo > 0 && self.0[lo - 1].end >= start {
            lo -= 1;
            start = self.0[lo].start;
            end = end.max(self.0[lo].end);
        }

        // Walk forward absorbing every right neighbour that touches or overlaps.
        let mut hi = lo;
        while hi < self.0.len() && self.0[hi].start <= end {
            end = end.max(self.0[hi].end);
            hi += 1;
        }

        // Splice [lo..hi] with the single merged range.
        if lo == hi {
            self.0.insert(lo, start..end);
        } else {
            self.0[lo] = start..end;
            if hi - lo > 1 {
                self.0.drain(lo + 1..hi);
            }
        }
    }

    /// Merge `other` ID range into current one. Both inputs are assumed to be in canonical
    /// form (sorted, non-overlapping, non-adjacent); the result is too. Adjacent and
    /// overlapping ranges across the two inputs coalesce.
    pub fn merge(&mut self, other: IdRange) {
        if other.0.is_empty() {
            return;
        }
        if self.0.is_empty() {
            self.0 = other.0;
            return;
        }

        // Fast path: `other` lies entirely after `self`. Common when accumulating updates
        // in clock order — avoids the general two-way merge alloc.
        let self_last_idx = self.0.len() - 1;
        let self_last_end = self.0[self_last_idx].end;
        let other_first_start = other.0[0].start;
        if self_last_end < other_first_start {
            self.0.extend(other.0);
            return;
        }
        if self_last_end == other_first_start {
            // Adjacent at the boundary — merge first of `other` into last of `self`,
            // then append the rest.
            self.0[self_last_idx].end = self_last_end.max(other.0[0].end);
            let mut it = other.0.into_iter();
            it.next();
            self.0.extend(it);
            return;
        }

        // General two-way merge.
        let mut out = SmallVec::with_capacity(self.0.len() + other.0.len());
        let mut a = std::mem::take(&mut self.0).into_iter().peekable();
        let mut b = other.0.into_iter().peekable();

        loop {
            let pick_a = match (a.peek(), b.peek()) {
                (Some(ra), Some(rb)) => ra.start <= rb.start,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let next = if pick_a {
                a.next().unwrap()
            } else {
                b.next().unwrap()
            };
            match out.last_mut() {
                Some(last) if last.end >= next.start => {
                    last.end = last.end.max(next.end);
                }
                _ => out.push(next),
            }
        }

        self.0 = out;
    }

    /// Check if current [IdRange] is a subset of `other`. This means that all the elements
    /// described by the current [IdRange] can be found within the bounds of `other` [IdRange].
    /// If there are some clock values not found within the `other` this method will return false.
    pub fn subset_of(&self, other: &Self) -> bool {
        for range in self.iter() {
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

        for other_range in other.iter() {
            // Skip ranges that end before our current position
            if other_range.end <= current {
                continue;
            }

            // If there's a gap before this range, we're not fully covered
            if other_range.start > current {
                return false;
            }

            // This range covers from current to its end
            current = other_range.end;

            // If we've covered the entire range, we're done
            if current >= range.end {
                return true;
            }
        }

        // Check if we covered the entire range
        current >= range.end
    }

    /// Excludes `other` from the current [IdRange]: every clock covered by `other` is removed
    /// from `self`, leaving the parts of `self` that lie outside `other`.
    pub fn exclude(&mut self, other: &Self) {
        if other.0.is_empty() || self.0.is_empty() {
            return;
        }

        let mut result = SmallVec::new();
        let other = other.0.as_slice();
        let mut i = 0usize;

        for range in self.0.iter() {
            let mut start = range.start;
            let end = range.end;

            // Drop other-ranges entirely to the left of [s..e). Monotone across iterations.
            while i < other.len() && other[i].end <= start {
                i += 1;
            }

            // Cut the other range from [s..e)
            let mut j = i;
            while start < end && j < other.len() {
                let other_range = &other[j];
                if other_range.start >= end {
                    break; // remaining other-ranges lie past this self-range
                }
                if other_range.start > start {
                    result.push(start..other_range.start);
                }
                start = start.max(other_range.end);
                if other_range.end < end {
                    j += 1; // consumed other range; move on
                } else {
                    break; // other range extends over the current one — keep it for the next self-range
                }
            }
            if start < end {
                result.push(start..end);
            }
            i = j;
        }

        *self = IdRange(result);
    }

    /// Computes the intersection of the current [IdRange] with `other`, modifying the current
    /// range to contain only elements present in both ranges.
    pub fn intersect(&mut self, other: &Self) {
        if self.0.is_empty() || other.0.is_empty() {
            *self = IdRange::default();
            return;
        }

        let mut result = SmallVec::new();
        let other = other.0.as_slice();
        let mut i = 0usize;

        for range in self.0.iter() {
            while i < other.len() && other[i].end <= range.start {
                i += 1;
            }

            let mut j = i;
            while j < other.len() {
                let other_range = &other[j];
                if other_range.start >= range.end {
                    break;
                }
                let lo = range.start.max(other_range.start);
                let hi = range.end.min(other_range.end);
                if lo < hi {
                    result.push(lo..hi);
                }
                if other_range.end < range.end {
                    j += 1;
                } else {
                    break;
                }
            }
            i = j;
        }

        *self = IdRange(result);
    }

    fn encode_raw<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_var(self.0.len() as u32);
        for range in self.0.iter() {
            range.encode(encoder);
        }
    }
}

impl Encode for IdRange {
    #[inline]
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        self.encode_raw(encoder)
    }
}

impl Decode for IdRange {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let len: u32 = decoder.read_var()?;
        let mut ranges = SmallVec::with_capacity(len as usize);
        for _ in 0..len {
            ranges.push(Range::decode(decoder)?);
        }
        Ok(IdRange(ranges))
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

    pub fn from_iter<I1, I2>(iter: I1) -> Self
    where
        I1: IntoIterator<Item = (ClientID, I2)>,
        I2: IntoIterator<Item = Range<u32>>,
    {
        let mut set = Self::new();
        for (client_id, range) in iter {
            let range = IdRange(range.into_iter().collect());
            set.0.insert(client_id, range);
        }
        set
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
            ranges.contains_clock(id.clock)
        } else {
            false
        }
    }

    /// Checks if current ID set contains any data.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty() || self.0.values().all(|r| r.is_empty())
    }

    pub fn insert(&mut self, id: ID, len: u32) {
        self.0
            .entry(id.client)
            .or_default()
            .push(id.clock..(id.clock + len));
    }

    /// Inserts a new ID `range` corresponding with a given `client`.
    pub fn insert_range(&mut self, client: ClientID, range: IdRange) {
        self.0.insert(client, range);
    }

    /// Merges another ID set into a current one, combining their information about observed ID
    /// ranges. Per-client [IdRange] merge maintains the canonical invariant on its own, so no
    /// trailing squash is needed.
    pub fn merge(&mut self, other: Self) {
        other.0.into_iter().for_each(|(client, range)| {
            if let Some(r) = self.0.get_mut(&client) {
                r.merge(range)
            } else {
                self.0.insert(client, range);
            }
        });
    }

    /// Removes from `self` every clock range covered by `other`. Per-client [IdRange]s are
    /// excluded individually; clients absent from `other` are left untouched.
    pub fn exclude(&mut self, other: &Self) {
        self.0.retain(|client, ranges| {
            if let Some(other_ranges) = other.0.get(client) {
                ranges.exclude(other_ranges);
            }
            !ranges.is_empty()
        });
    }

    /// Replaces `self` with the per-client intersection against `other`. Clients present only
    /// in `self` (or only in `other`) are dropped.
    pub fn intersect(&mut self, other: &Self) {
        self.0.retain(|client, ranges| match other.0.get(client) {
            Some(other_ranges) => {
                ranges.intersect(other_ranges);
                !ranges.is_empty()
            }
            None => false,
        });
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

/// [DeleteSet] contains information about all blocks (described by clock ranges) that have been
/// subjected to delete process.
#[repr(transparent)]
#[derive(Clone, Eq, PartialEq, Hash, Default, Serialize, Deserialize)]
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
        let mut i = self.iter();
        write!(f, "[")?;
        if let Some(r) = i.next() {
            write!(f, "[{}..{})", r.start, r.end)?;
        }
        while let Some(r) = i.next() {
            write!(f, ", [{}..{})", r.start, r.end)?;
        }
        write!(f, "]")
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

    /// Removes from `self` every clock range covered by `other`. Clients whose ranges become
    /// empty are dropped.
    pub fn exclude(&mut self, other: &Self) {
        self.0.exclude(&other.0)
    }

    /// Restricts `self` to the per-client intersection with `other`. Clients absent from
    /// `other` (or whose intersection is empty) are dropped.
    pub fn intersect(&mut self, other: &Self) {
        self.0.intersect(&other.0)
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
    range_iter: Option<std::slice::Iter<'ds, Range<u32>>>,
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
    use smallvec::smallvec;
    use std::collections::HashSet;
    use std::fmt::Debug;

    #[test]
    fn id_range_merge_continous() {
        // `b` entirely within `a`
        let mut a = IdRange(smallvec![0..5]);
        a.merge(IdRange(smallvec![2..4]));
        assert_eq!(a, IdRange(smallvec![0..5]));

        // the tail of `a` crosses the head of `b`
        let mut a = IdRange(smallvec![0..5]);
        a.merge(IdRange(smallvec![4..9]));
        assert_eq!(a, IdRange(smallvec![0..9]));

        // `b` is immediately adjacent to the end of `a`
        let mut a = IdRange(smallvec![0..5]);
        a.merge(IdRange(smallvec![5..9]));
        assert_eq!(a, IdRange(smallvec![0..9]));

        // `a` does not intersect with `b`
        let mut a = IdRange(smallvec![0..4]);
        a.merge(IdRange(smallvec![6..9]));
        assert_eq!(a, IdRange(smallvec![0..4, 6..9]));
    }

    #[test]
    fn id_range_merge_canonical() {
        // Empty into non-empty — self unchanged.
        let mut a = IdRange(smallvec![0..3]);
        a.merge(IdRange::default());
        assert_eq!(a, IdRange(smallvec![0..3]));

        // Non-empty into empty — adopts other.
        let mut a = IdRange::default();
        a.merge(IdRange(smallvec![1..5]));
        assert_eq!(a, IdRange(smallvec![1..5]));

        // Other has lower start than self — sorted output.
        let mut a = IdRange(smallvec![5..7]);
        a.merge(IdRange(smallvec![1..3]));
        assert_eq!(a, IdRange(smallvec![1..3, 5..7]));

        // Two-way merge with overlapping middles.
        let mut a = IdRange(smallvec![0..2, 4..6, 10..12]);
        a.merge(IdRange(smallvec![1..5, 11..15]));
        // [0..2] + [1..5] -> [0..5], absorbs [4..6] -> [0..6]; [10..12] + [11..15] -> [10..15]
        assert_eq!(a, IdRange(smallvec![0..6, 10..15]));

        // Cross-source adjacency must merge.
        let mut a = IdRange(smallvec![0..3, 10..12]);
        a.merge(IdRange(smallvec![3..5, 12..14]));
        assert_eq!(a, IdRange(smallvec![0..5, 10..14]));

        // Interleaved disjoint ranges stay disjoint and sorted.
        let mut a = IdRange(smallvec![0..2, 6..8]);
        a.merge(IdRange(smallvec![3..5, 9..11]));
        assert_eq!(a, IdRange(smallvec![0..2, 3..5, 6..8, 9..11]));
    }

    #[test]
    fn id_range_compact() {
        // Same canonical end-state as the old squash-based test, now produced incrementally
        // by merge-on-insert: adjacent [0..3) + [3..5) collapses, [6..7) stays separate.
        let mut r = IdRange::default();
        r.push(0..3);
        r.push(3..5);
        r.push(6..7);
        assert_eq!(r, IdRange(smallvec![0..5, 6..7]));
    }

    #[test]
    fn id_range_invert() {
        assert!(IdRange(smallvec![0..3]).invert().is_empty());

        assert_eq!(IdRange(smallvec![3..5]).invert(), IdRange(smallvec![0..3]));

        assert_eq!(
            IdRange(smallvec![0..3, 4..5]).invert(),
            IdRange(smallvec![3..4])
        );

        assert_eq!(
            IdRange(smallvec![3..4, 7..9]).invert(),
            IdRange(smallvec![0..3, 4..7])
        );
    }

    #[test]
    fn id_range_contains() {
        assert!(!IdRange(smallvec![1..3]).contains_clock(0));
        assert!(IdRange(smallvec![1..3]).contains_clock(1));
        assert!(IdRange(smallvec![1..3]).contains_clock(2));
        assert!(!IdRange(smallvec![1..3]).contains_clock(3));

        assert!(!IdRange(smallvec![1..3, 4..5]).contains_clock(0));
        assert!(IdRange(smallvec![1..3, 4..5]).contains_clock(1));
        assert!(IdRange(smallvec![1..3, 4..5]).contains_clock(2));
        assert!(!IdRange(smallvec![1..3, 4..5]).contains_clock(3));
        assert!(IdRange(smallvec![1..3, 4..5]).contains_clock(4));
        assert!(!IdRange(smallvec![1..3, 4..5]).contains_clock(5));
        assert!(!IdRange(smallvec![1..3, 4..5]).contains_clock(6));
    }

    #[test]
    fn id_range_push() {
        let mut range = IdRange(smallvec![]);

        range.push(0..4);
        assert_eq!(range, IdRange(smallvec![0..4]));

        range.push(4..6);
        assert_eq!(range, IdRange(smallvec![0..6]));

        range.push(7..9);
        assert_eq!(range, IdRange(smallvec![0..6, 7..9]));
    }

    #[test]
    fn id_range_push_out_of_order() {
        let mut range = IdRange::default();
        range.push(5..7);
        range.push(1..3);
        range.push(3..5);
        // Adjacency-chain: [3..5) bridges [1..3) and [5..7) into a single [1..7).
        assert_eq!(range, IdRange(smallvec![1..7]));
    }

    #[test]
    fn id_range_push_skips_empty() {
        let mut range = IdRange::default();
        range.push(5..5);
        assert_eq!(range, IdRange::default());
        range.push(0..3);
        range.push(7..7);
        assert_eq!(range, IdRange(smallvec![0..3]));
    }

    #[test]
    fn id_range_push_chain_absorb() {
        let mut range = IdRange::default();
        range.push(0..2);
        range.push(4..6);
        range.push(8..10);
        range.push(1..9);
        // The single push spans three existing ranges — all collapse into one.
        assert_eq!(range, IdRange(smallvec![0..10]));
    }

    #[test]
    fn id_range_push_adjacency_merge() {
        let mut range = IdRange::default();
        range.push(0..3);
        range.push(3..5);
        // Adjacent (touching at 3) — must merge, not stay split.
        assert_eq!(range, IdRange(smallvec![0..5]));
    }

    #[test]
    fn id_range_push_front() {
        let mut range = IdRange::default();
        range.push(5..7);
        range.push(1..3);
        // Front insert preserves order; no adjacency to merge.
        assert_eq!(range, IdRange(smallvec![1..3, 5..7]));
    }

    #[test]
    fn id_range_push_subset_of_last() {
        // Tail-fast path must not shrink the last range when the new range is a
        // subset of it (range.start >= last.start AND range.end <= last.end).
        let mut range = IdRange::default();
        range.push(0..10);
        range.push(5..7);
        assert_eq!(range, IdRange(smallvec![0..10]));

        // Boundary: range.end == last.end — also a subset, must be a no-op.
        let mut range = IdRange::default();
        range.push(0..10);
        range.push(3..10);
        assert_eq!(range, IdRange(smallvec![0..10]));

        // Boundary: identical to last.
        let mut range = IdRange::default();
        range.push(0..10);
        range.push(0..10);
        assert_eq!(range, IdRange(smallvec![0..10]));
    }

    #[test]
    fn id_range_subset_of() {
        assert!(IdRange(smallvec![1..2]).subset_of(&IdRange(smallvec![1..2])));
        assert!(IdRange(smallvec![1..2]).subset_of(&IdRange(smallvec![0..2])));
        assert!(IdRange(smallvec![1..2]).subset_of(&IdRange(smallvec![1..3])));

        assert!(IdRange(smallvec![1..2, 3..4]).subset_of(&IdRange(smallvec![1..4])));
        assert!(IdRange(smallvec![1..2, 3..4, 5..6]).subset_of(&IdRange(smallvec![1..2, 3..6])));
        assert!(IdRange(smallvec![3..4, 5..6]).subset_of(&IdRange(smallvec![0..1, 3..6])));
        assert!(IdRange(smallvec![3..4, 5..6]).subset_of(&IdRange(smallvec![3..4, 5..6])));

        assert!(!IdRange(smallvec![1..3]).subset_of(&IdRange(smallvec![0..2])));
        assert!(!IdRange(smallvec![1..3]).subset_of(&IdRange(smallvec![1..2])));
        assert!(!IdRange(smallvec![1..3]).subset_of(&IdRange(smallvec![2..3])));
        assert!(!IdRange(smallvec![1..3]).subset_of(&IdRange(smallvec![2..4])));

        assert!(!IdRange(smallvec![1..2, 3..4]).subset_of(&IdRange(smallvec![1..3])));
        assert!(!IdRange(smallvec![1..2, 3..6]).subset_of(&IdRange(smallvec![1..2, 3..5])));
        assert!(!IdRange(smallvec![1..2, 3..6]).subset_of(&IdRange(smallvec![1..2, 4..7])));
    }

    #[test]
    fn id_range_exclude() {
        // exclude from the left side
        let mut a = IdRange(smallvec![0..4]);
        a.exclude(&IdRange(smallvec![0..2]));
        assert_eq!(a, IdRange(smallvec![2..4]));

        // exclude from the right side
        let mut a = IdRange(smallvec![0..4]);
        a.exclude(&IdRange(smallvec![2..4]));
        assert_eq!(a, IdRange(smallvec![0..2]));

        // exclude in the middle - splitting the continuous block in two
        let mut a = IdRange(smallvec![0..4]);
        a.exclude(&IdRange(smallvec![1..3]));
        assert_eq!(a, IdRange(smallvec![0..1, 3..4]));

        // exclude fragmented block - splitting single range into >2 ranges
        let mut a = IdRange(smallvec![0..10]);
        a.exclude(&IdRange(smallvec![1..3, 4..5]));
        assert_eq!(a, IdRange(smallvec![0..1, 3..4, 5..10]));

        // exclude continuous range overlapping with more than one range
        let mut a = IdRange(smallvec![0..4, 5..6, 7..10]);
        a.exclude(&IdRange(smallvec![3..8]));
        assert_eq!(a, IdRange(smallvec![0..3, 8..10]));

        // exclude two fragmented ranges with partially overlapping boundaries
        let mut a = IdRange(smallvec![0..4, 7..10]);
        a.exclude(&IdRange(smallvec![3..5, 6..9]));
        assert_eq!(a, IdRange(smallvec![0..3, 9..10]));

        // exclude fragmented ranges, when one gets split into 2+, and another overlaps at the end
        let mut a = IdRange(smallvec![0..4, 7..10]);
        a.exclude(&IdRange(smallvec![2..3, 5..6, 9..10]));
        assert_eq!(a, IdRange(smallvec![0..2, 3..4, 7..9]));
    }

    #[test]
    fn id_range_intersect() {
        // Basic continuous range intersection
        let mut a = IdRange(smallvec![0..10]);
        a.intersect(&IdRange(smallvec![5..15]));
        assert_eq!(a, IdRange(smallvec![5..10]));

        // Fragmented intersecting with continuous
        let mut a = IdRange(smallvec![0..5, 10..15]);
        a.intersect(&IdRange(smallvec![3..12]));
        assert_eq!(a, IdRange(smallvec![3..5, 10..12]));

        // No overlap - empty result
        let mut a = IdRange(smallvec![0..5]);
        a.intersect(&IdRange(smallvec![10..15]));
        assert_eq!(a, IdRange::default());

        // Multiple ranges with multiple intersections
        let mut a = IdRange(smallvec![1..4, 6..9]);
        a.intersect(&IdRange(smallvec![2..7]));
        assert_eq!(a, IdRange(smallvec![2..4, 6..7]));

        // Complete overlap
        let mut a = IdRange(smallvec![2..8]);
        a.intersect(&IdRange(smallvec![0..10]));
        assert_eq!(a, IdRange(smallvec![2..8]));

        // Partial overlap on both sides
        let mut a = IdRange(smallvec![5..15]);
        a.intersect(&IdRange(smallvec![0..10]));
        assert_eq!(a, IdRange(smallvec![5..10]));

        // Fragmented with fragmented
        let mut a = IdRange(smallvec![0..5, 10..15, 20..25]);
        a.intersect(&IdRange(smallvec![3..12, 22..30]));
        assert_eq!(a, IdRange(smallvec![3..5, 10..12, 22..25]));

        // Exact match
        let mut a = IdRange(smallvec![5..10]);
        a.intersect(&IdRange(smallvec![5..10]));
        assert_eq!(a, IdRange(smallvec![5..10]));

        // Single element overlap
        let mut a = IdRange(smallvec![0..5]);
        a.intersect(&IdRange(smallvec![4..10]));
        assert_eq!(a, IdRange(smallvec![4..5]));

        // Half-open boundary touch — must not yield an empty range.
        let mut a = IdRange(smallvec![0..5]);
        a.intersect(&IdRange(smallvec![5..10]));
        assert_eq!(a, IdRange::default());
    }

    #[test]
    fn id_range_encode_decode() {
        roundtrip(&IdRange(smallvec![0..4]));
        roundtrip(&IdRange(smallvec![1..4, 5..8]));
    }

    #[test]
    fn id_set_exclude() {
        // Clients only in `self` are untouched; clients only in `other` are ignored.
        let mut a = IdSet::from_iter([(1, [0..10]), (2, [0..5])]);
        let b = IdSet::from_iter([(2, [0..3]), (3, [0..100])]);
        a.exclude(&b);
        assert_eq!(a, IdSet::from_iter([(1, [0..10]), (2, [3..5])]));

        // Per-client exclude carves the right shape (split into two).
        let mut a = IdSet::from_iter([(1, [0..10])]);
        let b = IdSet::from_iter([(1, [3..7])]);
        a.exclude(&b);
        assert_eq!(a, IdSet::from_iter([(1, [0..3, 7..10])]));

        // Clients whose ranges become empty are dropped entirely.
        let mut a = IdSet::from_iter([(1, [0..5]), (2, [0..5])]);
        let b = IdSet::from_iter([(1, [0..5])]);
        a.exclude(&b);
        assert_eq!(a, IdSet::from_iter([(2, [0..5])]));

        // Excluding an empty other is a no-op.
        let mut a = IdSet::from_iter([(1, [0..10])]);
        a.exclude(&IdSet::new());
        assert_eq!(a, IdSet::from_iter([(1, [0..10])]));
    }

    #[test]
    fn id_set_intersect() {
        // Clients present only in `self` are dropped.
        let mut a = IdSet::from_iter([(1, [0..10]), (2, [0..5])]);
        let b = IdSet::from_iter([(1, [5..15])]);
        a.intersect(&b);
        assert_eq!(a, IdSet::from_iter([(1, [5..10])]));

        // Clients present only in `other` are ignored.
        let mut a = IdSet::from_iter([(1, [0..10])]);
        let b = IdSet::from_iter([(1, [3..7]), (2, [0..100])]);
        a.intersect(&b);
        assert_eq!(a, IdSet::from_iter([(1, [3..7])]));

        // Clients whose intersection is empty (disjoint ranges) are dropped.
        let mut a = IdSet::from_iter([(1, [0..5]), (2, [0..5])]);
        let b = IdSet::from_iter([(1, [10..20]), (2, [1..4])]);
        a.intersect(&b);
        assert_eq!(a, IdSet::from_iter([(2, [1..4])]));

        // Intersecting with an empty other yields empty.
        let mut a = IdSet::from_iter([(1, [0..10])]);
        a.intersect(&IdSet::new());
        assert!(a.is_empty());
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
