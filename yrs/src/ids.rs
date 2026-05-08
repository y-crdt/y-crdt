use crate::block::{ClientID, ID};
use smallvec::SmallVec;
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::ops::Range;

/// Trait for per-range values that can be merged when ranges overlap.
/// Used to parameterize `IdRanges<T>` and `IdMapInner<T>` over different
/// value types (e.g. `()` for sets, `ContentAttributes<A>` for attributed maps).
pub trait Merge: Clone + PartialEq {
    /// Combine another value into this one. Called when two ranges overlap
    /// during merge (union) or intersect operations.
    fn merge(&mut self, other: &Self);
}

impl Merge for () {
    #[inline(always)]
    fn merge(&mut self, _other: &Self) {}
}

/// Per-client sorted, non-overlapping clock ranges with attached values.
///
/// Internally backed by a [`SmallVec`] of `(Range<u32>, T)` tuples.
/// Ranges are maintained sorted by start position; overlapping or adjacent
/// ranges with equal values are coalesced on insert.
#[derive(Clone, PartialEq, Eq)]
pub struct IdRanges<T>(SmallVec<[(Range<u32>, T); 1]>);

impl<T> Default for IdRanges<T> {
    #[inline]
    fn default() -> Self {
        IdRanges(SmallVec::new())
    }
}

/// Push a `(range, value)` entry onto `vec`, coalescing with the last
/// entry if they are adjacent/overlapping and have equal values.
#[inline]
fn push_coalesced<T: Merge, A: smallvec::Array<Item = (Range<u32>, T)>>(
    vec: &mut SmallVec<A>,
    range: Range<u32>,
    value: T,
) {
    if range.start >= range.end {
        return;
    }
    if let Some(last) = vec.last_mut() {
        if last.0.end >= range.start && last.1 == value {
            last.0.end = last.0.end.max(range.end);
            return;
        }
    }
    vec.push((range, value));
}

impl<T: Merge> IdRanges<T> {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        IdRanges(SmallVec::with_capacity(capacity))
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Check if a given clock value is covered by any range in this set.
    pub fn contains_clock(&self, clock: u32) -> bool {
        let idx = self.0.partition_point(|e| e.0.start <= clock);
        idx > 0 && self.0[idx - 1].0.end > clock
    }

    /// Find the index of the first entry whose range contains `clock`,
    /// or the first entry whose range starts after `clock`.
    /// Returns `None` if no such entry exists.
    pub fn find_start(&self, clock: u32) -> Option<usize> {
        if self.0.is_empty() {
            return None;
        }
        let mut left = 0;
        let mut right = self.0.len() - 1;
        while left <= right {
            let mid = (left + right) / 2;
            let range = &self.0[mid].0;
            if range.start <= clock {
                if clock < range.end {
                    return Some(mid);
                }
                left = mid + 1;
            } else {
                if mid == 0 {
                    break;
                }
                right = mid - 1;
            }
        }
        if left < self.0.len() {
            Some(left)
        } else {
            None
        }
    }

    /// Insert a `(range, value)` entry. Overlapping existing entries are split
    /// at boundaries and their values merged via [`Merge::merge`]. Adjacent
    /// entries with equal values are coalesced.
    ///
    /// For `IdRanges<()>` (i.e. `IdRange`), prefer the convenience method
    /// [`IdRanges::insert`] which omits the unit value.
    pub fn insert_with(&mut self, range: Range<u32>, value: T) {
        if range.start >= range.end {
            return;
        }

        // Tail-fast path: if the new range starts at or after the last entry,
        // no binary search needed. Common when accumulating updates in clock order.
        if let Some(last) = self.0.last_mut() {
            if range.start >= last.0.start {
                if range.start > last.0.end {
                    // Disjoint after last — just push
                    self.0.push((range, value));
                    return;
                }
                // Overlaps or adjacent with last entry
                if last.1 == value {
                    // Same value — extend
                    last.0.end = last.0.end.max(range.end);
                    return;
                }
                if range.start == last.0.end {
                    // Adjacent but different value — just push (no overlap to merge)
                    self.0.push((range, value));
                    return;
                }
                // Partial overlap with different value — fall through to general path
            }
        }

        // Find first potentially overlapping entry
        let mut lo = self.0.partition_point(|e| e.0.start < range.start);

        // Check if the previous entry extends into our range
        if lo > 0 && self.0[lo - 1].0.end >= range.start {
            lo -= 1;
        }

        // Find the end of overlapping entries
        let mut hi = lo;
        while hi < self.0.len() && self.0[hi].0.start <= range.end {
            hi += 1;
        }

        // No overlap — simple insert, then coalesce with neighbors
        if lo == hi {
            self.0.insert(lo, (range, value));
            // Coalesce with right neighbor
            if lo + 1 < self.0.len()
                && self.0[lo].0.end >= self.0[lo + 1].0.start
                && self.0[lo].1 == self.0[lo + 1].1
            {
                self.0[lo].0.end = self.0[lo].0.end.max(self.0[lo + 1].0.end);
                self.0.remove(lo + 1);
            }
            // Coalesce with left neighbor
            if lo > 0
                && self.0[lo - 1].0.end >= self.0[lo].0.start
                && self.0[lo - 1].1 == self.0[lo].1
            {
                self.0[lo - 1].0.end = self.0[lo - 1].0.end.max(self.0[lo].0.end);
                self.0.remove(lo);
            }
            return;
        }

        // Build replacement entries by walking through the overlapping region.
        let new_start = range.start;
        let new_end = range.end;

        let mut replacement: SmallVec<[(Range<u32>, T); 4]> = SmallVec::new();
        let mut cursor = self.0[lo].0.start.min(new_start);

        for i in lo..hi {
            let (ref entry_range, ref entry_value) = self.0[i];

            // Gap before this entry, covered only by new value
            if cursor >= new_start && cursor < entry_range.start {
                push_coalesced(
                    &mut replacement,
                    cursor..entry_range.start.min(new_end),
                    value.clone(),
                );
            }

            // Prefix of existing entry before new range
            if entry_range.start < new_start {
                push_coalesced(
                    &mut replacement,
                    entry_range.start..new_start,
                    entry_value.clone(),
                );
            }

            // Overlapping portion — merge values
            let overlap_start = entry_range.start.max(new_start);
            let overlap_end = entry_range.end.min(new_end);
            if overlap_start < overlap_end {
                let mut merged = entry_value.clone();
                merged.merge(&value);
                push_coalesced(&mut replacement, overlap_start..overlap_end, merged);
            }

            // Suffix of existing entry after new range
            if entry_range.end > new_end {
                push_coalesced(
                    &mut replacement,
                    new_end..entry_range.end,
                    entry_value.clone(),
                );
            }

            cursor = entry_range.end;
        }

        // Remaining gap after all overlapping entries, covered only by new value
        if cursor < new_end {
            push_coalesced(&mut replacement, cursor..new_end, value);
        }

        // Splice replacement into self: drain [lo..hi) and insert replacement entries.
        // For a single replacement entry (common for T=()), this is just a drain + assign.
        let repl_len = replacement.len();
        self.0.drain(lo..hi);
        // Reserve and insert
        self.0.reserve(repl_len);
        for (i, entry) in replacement.into_iter().enumerate() {
            self.0.insert(lo + i, entry);
        }

        // Coalesce at splice boundaries
        let splice_end = lo + repl_len;
        if splice_end < self.0.len() && splice_end > 0 {
            let prev = splice_end - 1;
            if self.0[prev].0.end >= self.0[splice_end].0.start
                && self.0[prev].1 == self.0[splice_end].1
            {
                self.0[prev].0.end = self.0[prev].0.end.max(self.0[splice_end].0.end);
                self.0.remove(splice_end);
            }
        }
        if lo > 0 && lo < self.0.len() {
            if self.0[lo - 1].0.end >= self.0[lo].0.start && self.0[lo - 1].1 == self.0[lo].1 {
                self.0[lo - 1].0.end = self.0[lo - 1].0.end.max(self.0[lo].0.end);
                self.0.remove(lo);
            }
        }
    }

    /// Remove all clock positions in `range` from this set. Existing entries
    /// that partially overlap are trimmed; fully covered entries are removed.
    /// Values on surviving pieces are preserved (cloned when split).
    pub fn remove(&mut self, range: Range<u32>) {
        if range.start >= range.end || self.0.is_empty() {
            return;
        }

        // First entry whose range overlaps (i.e. entry.end > range.start)
        let mut i = self.0.partition_point(|e| e.0.end <= range.start);
        if i >= self.0.len() {
            return;
        }

        // Special case: a single existing entry strictly contains `range` — split it
        if self.0[i].0.start < range.start && self.0[i].0.end > range.end {
            let right_range = range.end..self.0[i].0.end;
            let right_value = self.0[i].1.clone();
            self.0[i].0.end = range.start;
            self.0.insert(i + 1, (right_range, right_value));
            return;
        }

        // Trim leftmost entry if it straddles range.start
        if self.0[i].0.start < range.start {
            self.0[i].0.end = range.start;
            i += 1;
        }

        // Find entries fully covered
        let mut j = i;
        while j < self.0.len() && self.0[j].0.end <= range.end {
            j += 1;
        }

        // Trim trailing entry if it straddles range.end
        if j < self.0.len() && self.0[j].0.start < range.end {
            self.0[j].0.start = range.end;
        }

        // Drop fully-covered entries
        if j > i {
            self.0.drain(i..j);
        }
    }

    /// Merge `other` into `self` (set union) in O(n+m). Overlapping portions
    /// get their values combined via [`Merge::merge`]. Adjacent entries with
    /// equal values are coalesced.
    pub fn merge(&mut self, other: &Self) {
        if other.0.is_empty() {
            return;
        }
        if self.0.is_empty() {
            self.0 = other.0.clone();
            return;
        }

        let a = std::mem::take(&mut self.0);
        let b = &other.0;
        let mut result: SmallVec<[(Range<u32>, T); 1]> = SmallVec::with_capacity(a.len() + b.len());

        let mut ai = 0usize;
        let mut bi = 0usize;
        // Effective start positions — tracks partial consumption of current entries.
        let mut a_cur = a[0].0.start;
        let mut b_cur = b[0].0.start;

        while ai < a.len() || bi < b.len() {
            let a_avail = ai < a.len();
            let b_avail = bi < b.len();

            // Only one side remains — drain it
            if !b_avail {
                push_coalesced(&mut result, a_cur..a[ai].0.end, a[ai].1.clone());
                ai += 1;
                for i in ai..a.len() {
                    push_coalesced(&mut result, a[i].0.clone(), a[i].1.clone());
                }
                break;
            }
            if !a_avail {
                push_coalesced(&mut result, b_cur..b[bi].0.end, b[bi].1.clone());
                bi += 1;
                for i in bi..b.len() {
                    push_coalesced(&mut result, b[i].0.clone(), b[i].1.clone());
                }
                break;
            }

            let a_end = a[ai].0.end;
            let b_end = b[bi].0.end;

            // No overlap — emit the one that ends first
            if a_end <= b_cur {
                push_coalesced(&mut result, a_cur..a_end, a[ai].1.clone());
                ai += 1;
                a_cur = if ai < a.len() { a[ai].0.start } else { 0 };
                continue;
            }
            if b_end <= a_cur {
                push_coalesced(&mut result, b_cur..b_end, b[bi].1.clone());
                bi += 1;
                b_cur = if bi < b.len() { b[bi].0.start } else { 0 };
                continue;
            }

            // Overlap exists — emit prefix, overlap, then advance the shorter one
            if a_cur < b_cur {
                push_coalesced(&mut result, a_cur..b_cur, a[ai].1.clone());
            } else if b_cur < a_cur {
                push_coalesced(&mut result, b_cur..a_cur, b[bi].1.clone());
            }

            let overlap_start = a_cur.max(b_cur);
            let overlap_end = a_end.min(b_end);
            let mut merged = a[ai].1.clone();
            merged.merge(&b[bi].1);
            push_coalesced(&mut result, overlap_start..overlap_end, merged);

            // Advance the entry that ends first; partially consume the other
            if a_end < b_end {
                ai += 1;
                a_cur = if ai < a.len() { a[ai].0.start } else { 0 };
                b_cur = overlap_end;
            } else if b_end < a_end {
                bi += 1;
                b_cur = if bi < b.len() { b[bi].0.start } else { 0 };
                a_cur = overlap_end;
            } else {
                ai += 1;
                bi += 1;
                a_cur = if ai < a.len() { a[ai].0.start } else { 0 };
                b_cur = if bi < b.len() { b[bi].0.start } else { 0 };
            }
        }

        self.0 = result;
    }

    /// Remove from `self` every clock position covered by `other`.
    /// Values from `other` are ignored — only clock positions matter.
    /// Values on surviving pieces of `self` are preserved.
    pub fn exclude<U>(&mut self, other: &IdRanges<U>) {
        if other.0.is_empty() || self.0.is_empty() {
            return;
        }

        let mut result: SmallVec<[(Range<u32>, T); 1]> = SmallVec::new();
        let other = other.0.as_slice();
        let mut i = 0usize;

        for (ref range, ref value) in self.0.iter() {
            let mut start = range.start;
            let end = range.end;

            // Skip other-entries entirely to the left
            while i < other.len() && other[i].0.end <= start {
                i += 1;
            }

            // Cut other ranges from [start..end)
            let mut j = i;
            while start < end && j < other.len() {
                let other_range = &other[j].0;
                if other_range.start >= end {
                    break;
                }
                if other_range.start > start {
                    result.push((start..other_range.start, value.clone()));
                }
                start = start.max(other_range.end);
                if other_range.end < end {
                    j += 1;
                } else {
                    break;
                }
            }
            if start < end {
                result.push((start..end, value.clone()));
            }
            i = j;
        }

        self.0 = result;
    }

    /// Replace `self` with the intersection against `other`. Only clock
    /// positions present in both are kept. Values are combined via
    /// [`Merge::merge`].
    pub fn intersect(&mut self, other: &Self) {
        if self.0.is_empty() || other.0.is_empty() {
            self.0 = SmallVec::new();
            return;
        }

        let mut result: SmallVec<[(Range<u32>, T); 1]> = SmallVec::new();
        let other = other.0.as_slice();
        let mut i = 0usize;

        for (ref range, ref value) in self.0.iter() {
            while i < other.len() && other[i].0.end <= range.start {
                i += 1;
            }

            let mut j = i;
            while j < other.len() {
                let (ref other_range, ref other_value) = other[j];
                if other_range.start >= range.end {
                    break;
                }
                let lo = range.start.max(other_range.start);
                let hi = range.end.min(other_range.end);
                if lo < hi {
                    let mut merged = value.clone();
                    merged.merge(other_value);
                    // Coalesce with last result entry if adjacent and equal
                    if let Some(last) = result.last_mut() {
                        if last.0.end == lo && last.1 == merged {
                            last.0.end = hi;
                        } else {
                            result.push((lo..hi, merged));
                        }
                    } else {
                        result.push((lo..hi, merged));
                    }
                }
                if other_range.end < range.end {
                    j += 1;
                } else {
                    break;
                }
            }
            i = j;
        }

        self.0 = result;
    }

    /// Returns iterator over `(Range<u32>, &T)` pairs.
    pub fn iter(&self) -> std::slice::Iter<'_, (Range<u32>, T)> {
        self.0.iter()
    }

    /// Returns an iterator over clock ranges only (ignoring values).
    pub fn ranges(&self) -> impl Iterator<Item = &Range<u32>> {
        self.0.iter().map(|e| &e.0)
    }

    /// Returns a reference to the entry at `idx`.
    #[inline]
    pub fn get(&self, idx: usize) -> Option<&(Range<u32>, T)> {
        self.0.get(idx)
    }

    /// Access inner entries as a slice.
    #[inline]
    pub fn as_slice(&self) -> &[(Range<u32>, T)] {
        &self.0
    }

    /// Direct access to the inner SmallVec (crate-internal).
    #[inline]
    pub(crate) fn inner(&self) -> &SmallVec<[(Range<u32>, T); 1]> {
        &self.0
    }

    /// Direct mutable access to the inner SmallVec (crate-internal).
    #[inline]
    pub(crate) fn inner_mut(&mut self) -> &mut SmallVec<[(Range<u32>, T); 1]> {
        &mut self.0
    }

    /// Construct from raw SmallVec (crate-internal, assumes sorted/non-overlapping).
    #[inline]
    pub(crate) fn from_raw(raw: SmallVec<[(Range<u32>, T); 1]>) -> Self {
        IdRanges(raw)
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for IdRanges<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[")?;
        let mut first = true;
        for (range, value) in &self.0 {
            if !first {
                write!(f, ", ")?;
            }
            write!(f, "[{}..{}, {:?}]", range.start, range.end, value)?;
            first = false;
        }
        write!(f, "]")
    }
}

impl IdRanges<()> {
    /// Convenience: insert a bare clock range (no value needed for `T = ()`).
    pub fn insert(&mut self, range: Range<u32>) {
        self.insert_with(range, ());
    }

    /// Construct from an iterator of `Range<u32>` values.
    /// Each range is inserted individually, so overlapping / adjacent ranges
    /// are coalesced automatically.
    pub fn from_ranges(ranges: impl IntoIterator<Item = Range<u32>>) -> Self {
        let mut r = IdRanges::new();
        for range in ranges {
            r.insert(range);
        }
        r
    }
}

/// Generic map from [`ClientID`] to [`IdRanges<T>`], using a [`BTreeMap`] for
/// sorted client iteration. This is the shared core of [`IdSet`] and `IdMap<A>`.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct IdMapInner<T: Merge>(BTreeMap<ClientID, IdRanges<T>>);

impl<T: Merge> Default for IdMapInner<T> {
    #[inline]
    fn default() -> Self {
        IdMapInner(BTreeMap::new())
    }
}

impl<T: Merge> IdMapInner<T> {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if the map contains no clock ranges.
    ///
    /// Invariant: empty [`IdRanges`] entries are never stored in the map,
    /// so this is a simple check on the map length.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ClientID, &IdRanges<T>)> {
        self.0.iter()
    }

    pub fn get(&self, client_id: &ClientID) -> Option<&IdRanges<T>> {
        self.0.get(client_id)
    }

    pub(crate) fn entry(
        &mut self,
        client_id: ClientID,
    ) -> std::collections::btree_map::Entry<'_, ClientID, IdRanges<T>> {
        self.0.entry(client_id)
    }

    pub fn contains(&self, id: &ID) -> bool {
        if let Some(ranges) = self.0.get(&id.client) {
            ranges.contains_clock(id.clock)
        } else {
            false
        }
    }

    /// Insert a range with a value for a given client.
    pub fn insert_range(&mut self, client_id: ClientID, range: Range<u32>, value: T) {
        self.0
            .entry(client_id)
            .or_default()
            .insert_with(range, value);
    }

    /// Merge another map into this one (set union). Per-client ranges are
    /// merged via [`IdRanges::merge`].
    pub fn merge_with(&mut self, other: &Self) {
        for (client, other_ranges) in &other.0 {
            match self.0.entry(*client) {
                Entry::Occupied(mut e) => e.get_mut().merge(other_ranges),
                Entry::Vacant(e) => {
                    e.insert(other_ranges.clone());
                }
            }
        }
    }

    /// Return a new map that is the union of `self` and `other`.
    pub fn merge(&self, other: &Self) -> Self {
        let mut result = self.clone();
        result.merge_with(other);
        result
    }

    /// Remove from `self` every clock position covered by `other`.
    /// The value type `U` of `other` is ignored — only clock positions matter.
    pub fn diff_with<U: Merge>(&mut self, other: &IdMapInner<U>) {
        self.0.retain(|client, ranges| {
            if let Some(other_ranges) = other.0.get(client) {
                ranges.exclude(other_ranges);
            }
            !ranges.is_empty()
        });
    }

    /// Return a new map with clock positions from `self` that are not in `other`.
    pub fn diff<U: Merge>(&self, other: &IdMapInner<U>) -> Self {
        let mut result = self.clone();
        result.diff_with(other);
        result
    }

    /// Replace `self` with the per-client intersection against `other`.
    /// Clients present only in one side are dropped. Values are merged
    /// via [`Merge::merge`].
    pub fn intersect_with(&mut self, other: &Self) {
        self.0.retain(|client, ranges| match other.0.get(client) {
            Some(other_ranges) => {
                ranges.intersect(other_ranges);
                !ranges.is_empty()
            }
            None => false,
        });
    }

    /// Return a new map with only clock positions present in both.
    pub fn intersect(&self, other: &Self) -> Self {
        let mut result = self.clone();
        result.intersect_with(other);
        result
    }

    /// Maps values inside a current ID map using provided function, creating a new ID map as
    /// a result. New values are remapped and squashed together into continuous ID range if possible.
    pub fn map<F, U>(&self, f: F) -> IdMapInner<U>
    where
        F: Fn(&T) -> U,
        U: Merge,
    {
        let mut result = IdMapInner::new();
        for (client, ranges) in self.0.iter() {
            let mut new_ranges: IdRanges<U> = IdRanges::with_capacity(ranges.len());
            for (range, value) in ranges.iter() {
                new_ranges.insert_with(range.clone(), f(value));
            }
            result.0.insert(*client, new_ranges);
        }

        result
    }

    /// Direct access to the inner BTreeMap (crate-internal).
    #[inline]
    pub(crate) fn clients(&self) -> &BTreeMap<ClientID, IdRanges<T>> {
        &self.0
    }

    /// Direct mutable access to the inner BTreeMap (crate-internal).
    #[inline]
    pub(crate) fn clients_mut(&mut self) -> &mut BTreeMap<ClientID, IdRanges<T>> {
        &mut self.0
    }
}

impl<T: Merge + std::fmt::Debug> std::fmt::Debug for IdMapInner<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("IdMapInner");
        for (client, ranges) in &self.0 {
            s.field(&client.to_string(), ranges);
        }
        s.finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // ---- IdRanges<()> tests (equivalent to old IdRange) ----

    #[test]
    fn insert_non_overlapping() {
        let mut r = IdRanges::<()>::new();
        r.insert(5..8);
        r.insert(0..3);
        r.insert(10..12);
        assert_eq!(r.as_slice(), &[(0..3, ()), (5..8, ()), (10..12, ())]);
    }

    #[test]
    fn insert_adjacent_coalesces() {
        let mut r = IdRanges::<()>::new();
        r.insert(0..3);
        r.insert(3..5);
        assert_eq!(r.as_slice(), &[(0..5, ())]);
    }

    #[test]
    fn insert_overlapping_coalesces() {
        let mut r = IdRanges::<()>::new();
        r.insert(0..5);
        r.insert(3..8);
        assert_eq!(r.as_slice(), &[(0..8, ())]);
    }

    #[test]
    fn insert_contained_coalesces() {
        let mut r = IdRanges::<()>::new();
        r.insert(0..10);
        r.insert(3..7);
        assert_eq!(r.as_slice(), &[(0..10, ())]);
    }

    #[test]
    fn insert_spanning_multiple_coalesces() {
        let mut r = IdRanges::<()>::new();
        r.insert(0..3);
        r.insert(5..8);
        r.insert(10..12);
        r.insert(2..11);
        assert_eq!(r.as_slice(), &[(0..12, ())]);
    }

    #[test]
    fn insert_empty_range_noop() {
        let mut r = IdRanges::<()>::new();
        r.insert(5..5);
        assert!(r.is_empty());
    }

    #[test]
    fn remove_middle_splits() {
        let mut r = IdRanges::<()>::new();
        r.insert(0..10);
        r.remove(3..7);
        assert_eq!(r.as_slice(), &[(0..3, ()), (7..10, ())]);
    }

    #[test]
    fn remove_prefix() {
        let mut r = IdRanges::<()>::new();
        r.insert(0..10);
        r.remove(0..5);
        assert_eq!(r.as_slice(), &[(5..10, ())]);
    }

    #[test]
    fn remove_suffix() {
        let mut r = IdRanges::<()>::new();
        r.insert(0..10);
        r.remove(5..10);
        assert_eq!(r.as_slice(), &[(0..5, ())]);
    }

    #[test]
    fn remove_entire() {
        let mut r = IdRanges::<()>::new();
        r.insert(0..10);
        r.remove(0..10);
        assert!(r.is_empty());
    }

    #[test]
    fn remove_beyond() {
        let mut r = IdRanges::<()>::new();
        r.insert(0..5);
        r.remove(10..15);
        assert_eq!(r.as_slice(), &[(0..5, ())]);
    }

    #[test]
    fn merge_two_ranges() {
        let mut a = IdRanges::<()>::new();
        a.insert(0..3);
        a.insert(7..10);

        let mut b = IdRanges::<()>::new();
        b.insert(2..8);

        a.merge(&b);
        assert_eq!(a.as_slice(), &[(0..10, ())]);
    }

    #[test]
    fn exclude_partial() {
        let mut a = IdRanges::<()>::new();
        a.insert(0..10);

        let mut b = IdRanges::<()>::new();
        b.insert(3..7);

        a.exclude(&b);
        assert_eq!(a.as_slice(), &[(0..3, ()), (7..10, ())]);
    }

    #[test]
    fn intersect_partial() {
        let mut a = IdRanges::<()>::new();
        a.insert(0..10);

        let mut b = IdRanges::<()>::new();
        b.insert(3..15);

        a.intersect(&b);
        assert_eq!(a.as_slice(), &[(3..10, ())]);
    }

    #[test]
    fn intersect_no_overlap() {
        let mut a = IdRanges::<()>::new();
        a.insert(0..3);

        let mut b = IdRanges::<()>::new();
        b.insert(5..8);

        a.intersect(&b);
        assert!(a.is_empty());
    }

    #[test]
    fn contains_clock_basic() {
        let mut r = IdRanges::<()>::new();
        r.insert(0..5);
        r.insert(10..15);
        assert!(r.contains_clock(0));
        assert!(r.contains_clock(4));
        assert!(!r.contains_clock(5));
        assert!(!r.contains_clock(7));
        assert!(r.contains_clock(10));
        assert!(r.contains_clock(14));
        assert!(!r.contains_clock(15));
    }

    // ---- Attributed IdRanges tests ----

    #[derive(Clone, PartialEq, Eq, Debug)]
    struct Attrs(Vec<&'static str>);

    impl Merge for Attrs {
        fn merge(&mut self, other: &Self) {
            for a in &other.0 {
                if !self.0.contains(a) {
                    self.0.push(a);
                    self.0.sort();
                }
            }
        }
    }

    #[test]
    fn insert_attributed_overlapping_splits() {
        let mut r = IdRanges::<Attrs>::new();
        r.insert_with(0..5, Attrs(vec!["a"]));
        r.insert_with(3..8, Attrs(vec!["b"]));
        assert_eq!(
            r.as_slice(),
            &[
                (0..3, Attrs(vec!["a"])),
                (3..5, Attrs(vec!["a", "b"])),
                (5..8, Attrs(vec!["b"])),
            ]
        );
    }

    #[test]
    fn insert_attributed_same_attrs_coalesces() {
        let mut r = IdRanges::<Attrs>::new();
        r.insert_with(0..5, Attrs(vec!["a"]));
        r.insert_with(3..8, Attrs(vec!["a"]));
        assert_eq!(r.as_slice(), &[(0..8, Attrs(vec!["a"]))]);
    }

    #[test]
    fn insert_attributed_contained() {
        let mut r = IdRanges::<Attrs>::new();
        r.insert_with(0..10, Attrs(vec!["a"]));
        r.insert_with(3..7, Attrs(vec!["b"]));
        assert_eq!(
            r.as_slice(),
            &[
                (0..3, Attrs(vec!["a"])),
                (3..7, Attrs(vec!["a", "b"])),
                (7..10, Attrs(vec!["a"])),
            ]
        );
    }

    #[test]
    fn insert_attributed_spanning_gap() {
        let mut r = IdRanges::<Attrs>::new();
        r.insert_with(2..4, Attrs(vec!["a"]));
        r.insert_with(6..8, Attrs(vec!["b"]));
        r.insert_with(3..7, Attrs(vec!["c"]));
        assert_eq!(
            r.as_slice(),
            &[
                (2..3, Attrs(vec!["a"])),
                (3..4, Attrs(vec!["a", "c"])),
                (4..6, Attrs(vec!["c"])),
                (6..7, Attrs(vec!["b", "c"])),
                (7..8, Attrs(vec!["b"])),
            ]
        );
    }

    #[test]
    fn remove_attributed_splits() {
        let mut r = IdRanges::<Attrs>::new();
        r.insert_with(0..10, Attrs(vec!["a"]));
        r.remove(3..7);
        assert_eq!(
            r.as_slice(),
            &[(0..3, Attrs(vec!["a"])), (7..10, Attrs(vec!["a"])),]
        );
    }

    #[test]
    fn exclude_attributed() {
        let mut a = IdRanges::<Attrs>::new();
        a.insert_with(0..10, Attrs(vec!["a"]));

        let mut b = IdRanges::<Attrs>::new();
        b.insert_with(3..7, Attrs(vec!["ignored"]));

        a.exclude(&b);
        assert_eq!(
            a.as_slice(),
            &[(0..3, Attrs(vec!["a"])), (7..10, Attrs(vec!["a"])),]
        );
    }

    #[test]
    fn intersect_attributed() {
        let mut a = IdRanges::<Attrs>::new();
        a.insert_with(0..10, Attrs(vec!["a"]));

        let mut b = IdRanges::<Attrs>::new();
        b.insert_with(3..15, Attrs(vec!["b"]));

        a.intersect(&b);
        assert_eq!(a.as_slice(), &[(3..10, Attrs(vec!["a", "b"]))]);
    }

    // ---- IdMapInner tests ----

    #[test]
    fn id_map_inner_merge() {
        let mut a = IdMapInner::<()>::new();
        a.insert_range(ClientID::new(1), 0..5, ());
        a.insert_range(ClientID::new(2), 0..3, ());

        let mut b = IdMapInner::<()>::new();
        b.insert_range(ClientID::new(1), 3..8, ());
        b.insert_range(ClientID::new(3), 0..4, ());

        a.merge_with(&b);
        assert!(a.contains(&ID::new(ClientID::new(1), 7)));
        assert!(a.contains(&ID::new(ClientID::new(2), 2)));
        assert!(a.contains(&ID::new(ClientID::new(3), 3)));
    }

    #[test]
    fn id_map_inner_diff() {
        let mut a = IdMapInner::<()>::new();
        a.insert_range(ClientID::new(1), 0..10, ());

        let mut b = IdMapInner::<()>::new();
        b.insert_range(ClientID::new(1), 3..7, ());

        a.diff_with(&b);
        assert!(a.contains(&ID::new(ClientID::new(1), 2)));
        assert!(!a.contains(&ID::new(ClientID::new(1), 5)));
        assert!(a.contains(&ID::new(ClientID::new(1), 8)));
    }

    #[test]
    fn id_map_inner_intersect() {
        let mut a = IdMapInner::<()>::new();
        a.insert_range(ClientID::new(1), 0..10, ());
        a.insert_range(ClientID::new(2), 0..5, ());

        let mut b = IdMapInner::<()>::new();
        b.insert_range(ClientID::new(1), 5..15, ());

        a.intersect_with(&b);
        assert!(!a.contains(&ID::new(ClientID::new(1), 3)));
        assert!(a.contains(&ID::new(ClientID::new(1), 7)));
        assert!(!a.contains(&ID::new(ClientID::new(2), 2)));
    }

    #[test]
    fn id_map_inner_diff_generic_u() {
        let mut a = IdMapInner::<Attrs>::new();
        a.insert_range(ClientID::new(1), 0..10, Attrs(vec!["a"]));

        let mut b = IdMapInner::<()>::new();
        b.insert_range(ClientID::new(1), 3..7, ());

        a.diff_with(&b);
        assert!(a.contains(&ID::new(ClientID::new(1), 2)));
        assert!(!a.contains(&ID::new(ClientID::new(1), 5)));
        assert!(a.contains(&ID::new(ClientID::new(1), 8)));
    }
}
