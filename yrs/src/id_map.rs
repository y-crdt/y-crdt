use crate::block::{BlockRange, ClientID};
use crate::encoding::read::Error;
use crate::encoding::serde::{from_any, to_any};
use crate::ids::{IdMapInner, IdRanges, Merge};
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::{IdSet, ID};
use serde::de::DeserializeOwned;
use serde::Serialize;
use smallvec::SmallVec;
use std::collections::HashSet;
use std::hash::Hash;
use std::ops::{Deref, Range};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// ContentAttributes<A> — per-range attribute set with Merge support
// ---------------------------------------------------------------------------

/// A set of [`ContentAttribute<A>`] values attached to a range.
/// Implements [`Merge`] by unioning attributes.
#[derive(Eq, Default, Debug)]
pub struct ContentAttributes<A>(pub SmallVec<[ContentAttribute<A>; 2]>);

impl<A> Clone for ContentAttributes<A> {
    fn clone(&self) -> Self {
        ContentAttributes(self.0.clone())
    }
}

impl<A: PartialEq> PartialEq for ContentAttributes<A> {
    fn eq(&self, other: &Self) -> bool {
        if self.0.len() != other.0.len() {
            return false;
        }
        // Set equality — every element in self must be in other
        self.0.iter().all(|a| other.0.contains(a))
    }
}

impl<A: PartialEq + Eq + Hash + Clone> Merge for ContentAttributes<A> {
    fn merge(&mut self, other: &Self) {
        for attr in &other.0 {
            if !self.0.contains(attr) {
                self.0.push(attr.clone());
            }
        }
    }
}

impl<A> Deref for ContentAttributes<A> {
    type Target = [ContentAttribute<A>];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<A> ContentAttributes<A> {
    pub fn new() -> Self {
        ContentAttributes(SmallVec::new())
    }

    pub fn from_attrs(attrs: SmallVec<[ContentAttribute<A>; 2]>) -> Self {
        ContentAttributes(attrs)
    }
}

// ---------------------------------------------------------------------------
// IdMap<A>
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct IdMap<A: PartialEq + Eq + Hash + Clone> {
    attrs: HashSet<ContentAttribute<A>>,
    inner: IdMapInner<ContentAttributes<A>>,
}

impl<A: PartialEq + Eq + Hash + Clone> IdMap<A> {
    pub fn new() -> Self {
        IdMap {
            inner: IdMapInner::new(),
            attrs: Default::default(),
        }
    }
}

impl<A: PartialEq + Eq + Hash + Clone> Default for IdMap<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: PartialEq + Eq + Hash + Clone> IdMap<A> {
    pub fn from_set(id_set: IdSet, attrs: Vec<ContentAttribute<A>>) -> Self {
        let mut id_map = IdMap::new();
        let mut attrs: SmallVec<[ContentAttribute<A>; 2]> = attrs.into();
        attrs.dedup();
        id_map.ensure_attrs(&mut attrs);
        let content_attrs = ContentAttributes(attrs);
        for (client, ranges) in id_set.iter() {
            let mut id_ranges = IdRanges::new();
            for range in ranges.iter() {
                id_ranges.insert_with(range.clone(), content_attrs.clone());
            }
            id_map.inner.clients_mut().insert(*client, id_ranges);
        }
        id_map
    }

    pub fn iter(&self) -> Iter<'_, A> {
        Iter::new(self)
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn contains(&self, id: &ID) -> bool {
        self.inner.contains(id)
    }

    pub fn attributions(&self, range: &BlockRange) -> Vec<AttrRange<A>> {
        let client = range.id.client;
        let clock = range.id.clock;
        let len = range.len;
        let mut result: Vec<AttrRange<A>> = Vec::new();

        if let Some(dr) = self.inner.get(&client) {
            if let Some(mut index) = dr.find_start(clock) {
                let entries = dr.as_slice();
                let mut prev_end = clock;
                while index < entries.len() {
                    let (ref entry_range, ref entry_attrs) = entries[index];
                    let mut r_start = entry_range.start;
                    let mut r_end = entry_range.end;
                    if r_start < clock {
                        r_start = clock;
                    }
                    if r_end > clock + len {
                        r_end = clock + len;
                    }
                    if r_start >= r_end {
                        break;
                    }
                    // Fill gap before this range
                    if prev_end < r_start {
                        result.push(AttrRange::new(prev_end..r_start));
                    }
                    prev_end = r_end;
                    result.push(AttrRange {
                        range: r_start..r_end,
                        attrs: entry_attrs.clone(),
                    });
                    index += 1;
                }
            }
        }

        if let Some(last) = result.last() {
            let end = last.range.end;
            if end < clock + len {
                result.push(AttrRange::new(end..(clock + len)));
            }
        } else {
            result.push(AttrRange::new(clock..(clock + len)));
        }

        result
    }

    pub fn insert(&mut self, range: BlockRange, attrs: Vec<ContentAttribute<A>>) {
        if attrs.is_empty() || range.len == 0 {
            return;
        }

        let mut attrs = attrs;
        self.ensure_attrs(&mut attrs);
        let content_attrs = ContentAttributes(attrs.into());
        self.inner
            .insert_range(range.id.client, range.id.clock..(range.id.clock + range.len), content_attrs);
    }

    pub fn remove(&mut self, range: &BlockRange) {
        let client = range.id.client;
        let clock = range.id.clock;
        let len = range.len;

        if len == 0 {
            return;
        }

        if let Some(r) = self.inner.get_mut(&client) {
            r.remove(clock..(clock + len));
            if r.is_empty() {
                self.inner.clients_mut().remove(&client);
            }
        }
    }

    pub fn merge_with(&mut self, other: Self) {
        self.inner.merge_with(&other.inner);
        // Merge the attr dedup sets
        for attr in other.attrs {
            self.attrs.insert(attr);
        }
    }

    pub fn merge_many(id_maps: &[Self]) -> Self
    where
        A: Clone,
    {
        let mut result = IdMap::new();
        for map in id_maps {
            result.inner.merge_with(&map.inner);
            for attr in &map.attrs {
                result.attrs.insert(attr.clone());
            }
        }
        result
    }

    pub fn intersect_with(&mut self, other: &Self) {
        self.inner.intersect_with(&other.inner);
    }

    pub fn diff_with<T: Merge>(&mut self, other: &IdMapInner<T>) {
        self.inner.diff_with(other);
    }

    pub fn filter<F>(&self, predicate: F) -> Self
    where
        F: Fn(&[ContentAttribute<A>]) -> bool,
        A: Clone,
    {
        let mut filtered = IdMap::new();
        for (client, ranges) in self.inner.iter() {
            let mut attr_ranges: SmallVec<[(Range<u32>, ContentAttributes<A>); 1]> = SmallVec::new();
            for (range, attrs) in ranges.iter() {
                if predicate(&attrs.0) {
                    let range_copy = (range.clone(), attrs.clone());
                    for attr in &attrs.0 {
                        filtered.attrs.insert(attr.clone());
                    }
                    attr_ranges.push(range_copy);
                }
            }
            if !attr_ranges.is_empty() {
                filtered.inner.clients_mut().insert(*client, IdRanges::from_raw(attr_ranges));
            }
        }
        filtered
    }

    /// Add `attrs` to local attribute set of current [IdMap]. If the attribute was already in set,
    /// reuse the already cached version.
    fn ensure_attrs(&mut self, attrs: &mut [ContentAttribute<A>]) {
        for a in attrs {
            if let Some(existing) = self.attrs.get(a).cloned() {
                *a = existing;
            } else {
                self.attrs.insert(a.clone());
            }
        }
    }

    /// Access the inner [IdMapInner] (crate-internal).
    pub(crate) fn inner(&self) -> &IdMapInner<ContentAttributes<A>> {
        &self.inner
    }
}

impl<A: PartialEq + Eq + Hash + Clone> PartialEq for IdMap<A> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

pub struct Iter<'a, A: PartialEq + Eq + Hash + Clone> {
    inner: std::collections::btree_map::Iter<'a, ClientID, IdRanges<ContentAttributes<A>>>,
    current_client: Option<ClientID>,
    current_iter: Option<std::slice::Iter<'a, (Range<u32>, ContentAttributes<A>)>>,
}

impl<'a, A: PartialEq + Eq + Hash + Clone> Iter<'a, A> {
    fn new(id_map: &'a IdMap<A>) -> Self {
        Iter {
            inner: id_map.inner.clients().iter(),
            current_client: None,
            current_iter: None,
        }
    }
}

impl<'a, A: PartialEq + Eq + Hash + Clone> Iterator for Iter<'a, A> {
    type Item = (ClientID, AttrRange<A>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(iter) = self.current_iter.as_mut() {
                if let Some((range, attrs)) = iter.next() {
                    return Some((
                        self.current_client.unwrap(),
                        AttrRange {
                            range: range.clone(),
                            attrs: attrs.clone(),
                        },
                    ));
                }
            }
            let (client, ranges) = self.inner.next()?;
            self.current_client = Some(*client);
            self.current_iter = Some(ranges.iter());
        }
    }
}

impl<A: PartialEq + Eq + Hash + Clone> From<IdMap<A>> for IdSet {
    fn from(value: IdMap<A>) -> Self {
        let mut set = IdSet::default();
        for (client, ranges) in value.inner.clients() {
            let range = set.range_mut(*client);
            for (r, _) in ranges.iter() {
                range.insert(r.clone());
            }
        }
        set
    }
}

impl<A: Serialize + PartialEq + Eq + Hash + Clone> Encode for IdMap<A> {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_var(self.inner.len() as u32);
        let mut last_written_client_id: u64 = 0;
        let mut visited_attributions: std::collections::HashMap<*const ContentAttributeInner<A>, usize> =
            std::collections::HashMap::new();
        let mut visited_attr_names: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();

        // Ensure deterministic order (BTreeMap already sorted)
        for (client_id, ranges) in self.inner.iter() {
            encoder.reset_ds_cur_val();
            let diff = client_id.get() - last_written_client_id;
            encoder.write_var(diff);
            last_written_client_id = client_id.get();
            encoder.write_var(ranges.len() as u32);

            for (range, attrs) in ranges.iter() {
                encoder.write_ds_clock(range.start);
                encoder.write_ds_len(range.end - range.start);
                encoder.write_var(attrs.0.len() as u32);

                for attr in &attrs.0 {
                    let ptr = Arc::as_ptr(&attr.0);
                    if let Some(&attr_id) = visited_attributions.get(&ptr) {
                        encoder.write_var(attr_id as u32);
                    } else {
                        let new_attr_id = visited_attributions.len();
                        visited_attributions.insert(ptr, new_attr_id);
                        encoder.write_var(new_attr_id as u32);

                        let name = &attr.0.name;
                        if let Some(&name_id) = visited_attr_names.get(name.as_str()) {
                            encoder.write_var(name_id as u32);
                        } else {
                            let new_name_id = visited_attr_names.len();
                            encoder.write_var(new_name_id as u32);
                            encoder.write_string(name);
                            visited_attr_names.insert(name, new_name_id);
                        }

                        let any = to_any(&attr.0.value).unwrap();
                        encoder.write_any(&any);
                    }
                }
            }
        }
    }
}

impl<A: DeserializeOwned + PartialEq + Eq + Hash + Clone> Decode for IdMap<A> {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let mut id_map = IdMap::new();
        let num_clients: u32 = decoder.read_var()?;
        let mut visited_attributions: Vec<ContentAttribute<A>> = Vec::new();
        let mut visited_attr_names: Vec<String> = Vec::new();
        let mut last_client_id: u64 = 0;

        for _ in 0..num_clients {
            decoder.reset_ds_cur_val();
            let diff: u64 = decoder.read_var()?;
            let client = last_client_id + diff;
            last_client_id = client;
            let num_ranges: u32 = decoder.read_var()?;
            let mut entries: SmallVec<[(Range<u32>, ContentAttributes<A>); 1]> = SmallVec::new();

            for _ in 0..num_ranges {
                let range_clock = decoder.read_ds_clock()?;
                let range_len = decoder.read_ds_len()?;
                let attrs_len: u32 = decoder.read_var()?;
                let mut attrs: SmallVec<[ContentAttribute<A>; 2]> = SmallVec::new();

                for _ in 0..attrs_len {
                    let attr_id: usize = decoder.read_var()?;
                    if attr_id >= visited_attributions.len() {
                        let attr_name_id: usize = decoder.read_var()?;
                        if attr_name_id >= visited_attr_names.len() {
                            let name = decoder.read_string()?.to_string();
                            visited_attr_names.push(name);
                        }
                        let any = decoder.read_any()?;
                        let value: A = from_any(&any)?;
                        visited_attributions.push(ContentAttribute::new(
                            visited_attr_names[attr_name_id].clone(),
                            value,
                        ));
                    }
                    attrs.push(visited_attributions[attr_id].clone());
                }

                entries.push((range_clock..(range_clock + range_len), ContentAttributes(attrs)));
            }

            id_map
                .inner
                .clients_mut()
                .insert(ClientID::new(client), IdRanges::from_raw(entries));
        }

        for attr in visited_attributions {
            id_map.attrs.insert(attr);
        }

        Ok(id_map)
    }
}

// ---------------------------------------------------------------------------
// AttrRange<A> — public API type for attributions results
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub struct AttrRange<A> {
    pub range: Range<u32>,
    pub attrs: ContentAttributes<A>,
}

impl<A> AttrRange<A> {
    pub fn new(range: Range<u32>) -> Self {
        AttrRange {
            range,
            attrs: ContentAttributes::new(),
        }
    }

    pub fn with_attrs(self, attrs: SmallVec<[ContentAttribute<A>; 2]>) -> Self {
        AttrRange {
            range: self.range,
            attrs: ContentAttributes(attrs),
        }
    }
}

impl<A> Clone for AttrRange<A> {
    fn clone(&self) -> Self {
        AttrRange {
            range: self.range.clone(),
            attrs: self.attrs.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// ContentAttribute<A>
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct ContentAttribute<A>(Arc<ContentAttributeInner<A>>);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
struct ContentAttributeInner<A> {
    name: String,
    value: A,
}

impl<A> ContentAttribute<A> {
    pub fn new<S: Into<String>>(name: S, value: A) -> Self {
        ContentAttribute(Arc::new(ContentAttributeInner {
            name: name.into(),
            value,
        }))
    }

    pub fn name(&self) -> &str {
        &self.0.name
    }

    pub fn value(&self) -> &A {
        &self.0.value
    }
}

impl<A> Clone for ContentAttribute<A> {
    fn clone(&self) -> Self {
        ContentAttribute(self.0.clone())
    }
}

impl<A: std::fmt::Display> std::fmt::Display for ContentAttribute<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "(\"{}\": {})", self.0.name, self.0.value)
    }
}

#[cfg(test)]
mod test {
    use crate::block::{BlockRange, ClientID};
    use crate::id_map::{ContentAttribute, IdMap};
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::Encode;
    use crate::{IdSet, ID};
    use serde::Serialize;
    use std::hash::Hash;

    #[test]
    fn am_merge_filter_out_empty_items() {
        let attrs = vec![42];
        let a = construct([(0, 1, 0, attrs)]);
        let b = construct([]);
        assert_eq!(a, b);
    }

    #[test]
    fn am_merge_filter_out_empty_items_2() {
        let attrs = vec![42];
        let a = construct([(0, 1, 0, attrs.clone()), (0, 2, 0, attrs)]);
        let b = construct([]);
        assert_eq!(a, b);
    }

    #[test]
    fn am_merge_filter_out_empty_items_end() {
        let attrs = vec![42];
        let a = construct([(0, 1, 1, attrs.clone()), (0, 2, 0, attrs.clone())]);
        let b = construct([(0, 1, 1, attrs.clone())]);
        assert_eq!(a, b);
    }

    #[test]
    fn am_merge_filter_out_empty_items_4_middle() {
        let attrs = vec![42];
        let a = construct([
            (0, 1, 1, attrs.clone()),
            (0, 2, 0, attrs.clone()),
            (0, 3, 1, attrs.clone()),
        ]);
        let b = construct([(0, 1, 1, attrs.clone()), (0, 3, 1, attrs.clone())]);
        assert_eq!(a, b);
    }

    #[test]
    fn am_merge_filter_out_empty_items_5_start() {
        let attrs = vec![42];
        let a = construct([
            (0, 1, 0, attrs.clone()),
            (0, 2, 1, attrs.clone()),
            (0, 3, 1, attrs.clone()),
        ]);
        let b = construct([(0, 2, 1, attrs.clone()), (0, 3, 1, attrs.clone())]);
        assert_eq!(a, b);
    }

    #[test]
    fn am_merge_merges_overlapping_id_ranges() {
        let attrs = vec![42];
        let a = construct([(0, 1, 2, attrs.clone()), (0, 0, 2, attrs.clone())]);
        let b = construct([(0, 0, 3, attrs.clone())]);
        assert_eq!(a, b);
    }

    #[test]
    fn am_merge_construct_without_hole() {
        let attrs = vec![42];
        let a = construct([(0, 1, 2, attrs.clone()), (0, 3, 1, attrs.clone())]);
        let b = construct([(0, 1, 3, attrs.clone())]);
        assert_eq!(a, b);
    }

    #[test]
    fn am_doesnt_merge_overlapping_id_ranges_with_different_attributes() {
        let a = construct([(0, 1, 2, vec![1]), (0, 0, 2, vec![2])]);
        let b = construct([
            (0, 0, 1, vec![2]),
            (0, 1, 1, vec![1, 2]),
            (0, 2, 1, vec![1]),
        ]);
        assert_eq!(a, b);
    }

    #[test]
    fn repeat_merging_mutliple_maps() {
        const CLIENTS: u64 = 4;
        const CLOCK_RANGE: u32 = 5;
        let mut sets = Vec::new();
        for _ in 0..3 {
            sets.push(random_id_map(CLIENTS, CLOCK_RANGE, vec![1, 2, 3]));
        }

        let merged = IdMap::merge_many(&sets);
        sets.reverse();
        let merge_reversed = IdMap::merge_many(&sets);
        assert_eq!(merged, merge_reversed);

        let mut composed = IdMap::new();
        for i_client in 0..CLIENTS {
            for i_clock in 0..(CLOCK_RANGE + 42) {
                let client_id = ClientID::new(i_client as u64);
                let id = ID::new(client_id, i_clock);
                let merged_contains = merged.contains(&id);
                let exists = sets.iter().any(|set| set.contains(&id));
                assert_eq!(exists, merged_contains);

                let merged_attrs = merged.attributions(&BlockRange::new(id, 1));
                for a in merged_attrs {
                    if !a.attrs.is_empty() {
                        let clock = a.range.start;
                        let len = a.range.end - a.range.start;
                        composed.insert(
                            BlockRange::new(ID::new(client_id, clock), len),
                            a.attrs.0.to_vec(),
                        );
                    }
                }
            }
        }
        assert_eq!(merged, composed);
    }

    #[test]
    fn repeat_random_diffing() {
        const CLIENTS: u64 = 4;
        const CLOCK_RANGE: u32 = 5;
        let attrs = vec![1, 2, 3];
        let mut id_map_1 = random_id_map(CLIENTS, CLOCK_RANGE, attrs.clone());
        let id_map_2 = random_id_map(CLIENTS, CLOCK_RANGE, attrs.clone());
        let mut merged = IdMap::merge_many(&[id_map_1.clone(), id_map_2.clone()]);
        id_map_1.diff_with(&id_map_2.inner);
        merged.diff_with(&id_map_2.inner);
        assert_eq!(merged, id_map_1);

        let copy = IdMap::decode_v1(&id_map_1.encode_v1()).unwrap();
        assert_eq!(id_map_1, copy);
    }

    #[test]
    fn repeat_random_diffing_2() {
        const CLIENTS: u64 = 4;
        const CLOCK_RANGE: u32 = 100;
        let attrs = vec![1, 2, 3];
        let mut id_map_1 = random_id_map(CLIENTS, CLOCK_RANGE, attrs.clone());
        let mut id_map_2 = random_id_map(CLIENTS, CLOCK_RANGE, attrs.clone());
        let id_exclude = random_id_set(CLIENTS, CLOCK_RANGE);
        let mut merged = IdMap::merge_many(&[id_map_1.clone(), id_map_2.clone()]);
        let mut merge_excluded = merged.clone();
        merge_excluded.diff_with(id_exclude.inner());

        id_map_1.diff_with(id_exclude.inner());
        id_map_2.diff_with(id_exclude.inner());

        let excluded_merged = IdMap::merge_many(&[id_map_1.clone(), id_map_2.clone()]);
        assert_eq!(merge_excluded, excluded_merged);

        let copy = IdMap::decode_v1(&merge_excluded.encode_v1()).unwrap();
        assert_eq!(merge_excluded, copy);
    }

    #[test]
    fn repeat_random_deletes() {
        const CLIENTS: u64 = 2;
        const CLOCK_RANGE: u32 = 100;
        let mut id_map: IdMap<i32> = random_id_map(CLIENTS, CLOCK_RANGE, vec![1]);
        let client = *id_map.inner.clients().keys().next().unwrap();
        let clock = fastrand::u32(0..CLOCK_RANGE);
        let len = fastrand::u32(1..((CLOCK_RANGE - clock) as f64 * 1.2).round().max(2.0) as u32);
        // Build an IdSet representing the delete range (diff_with works on clock positions only)
        let mut ds = IdSet::new();
        ds.insert(ID::new(client, clock), len);
        let mut diffed = id_map.clone();
        diffed.diff_with(ds.inner());
        id_map.remove(&BlockRange::new(ID::new(client, clock), len));

        // After removal, the removed clocks should NOT be contained
        for i in 0..len.min(CLOCK_RANGE - clock) {
            assert!(!id_map.contains(&ID::new(client, clock + i)),
                "clock {} should have been removed", clock + i);
        }

        assert_eq!(id_map, diffed);
    }

    #[test]
    fn repeat_random_intersects() {
        const CLIENTS: u64 = 4;
        const CLOCK_RANGE: u32 = 100;
        let ids1: IdMap<String> = random_id_map(CLIENTS, CLOCK_RANGE, vec!["one".into()]);
        let ids2: IdMap<String> = random_id_map(CLIENTS, CLOCK_RANGE, vec!["two".into()]);
        let mut intersected = ids1.clone();
        intersected.intersect_with(&ids2);

        for client in 0..CLIENTS {
            let client = ClientID::new(client as u64);
            for clock in 0..CLOCK_RANGE {
                let id = ID::new(client, clock);
                assert_eq!(
                    ids1.contains(&id) && ids2.contains(&id),
                    intersected.contains(&id),
                    "if both maps contain ID, so should intersection"
                );

                // For clocks in the intersection, verify attrs are merged from both
                if ids1.contains(&id) && ids2.contains(&id) {
                    let attr1 = ids1.attributions(&BlockRange::new(id, 1)).first().cloned().unwrap();
                    let attr2 = ids2.attributions(&BlockRange::new(id, 1)).first().cloned().unwrap();
                    let intersect_attr = intersected.attributions(&BlockRange::new(id, 1)).first().cloned().unwrap();

                    // The intersected attrs should contain all attrs from both sides
                    for a in attr1.attrs.iter() {
                        assert!(intersect_attr.attrs.contains(a),
                            "intersected attribution should contain attrs from first map");
                    }
                    for a in attr2.attrs.iter() {
                        assert!(intersect_attr.attrs.contains(a),
                            "intersected attribution should contain attrs from second map");
                    }
                }
            }
        }

        let mut diffed1 = ids1.clone();
        diffed1.diff_with(&ids2.inner);
        let mut ids1_copy = ids1.clone();
        ids1_copy.diff_with(&intersected.inner);
        assert_eq!(ids1_copy, diffed1);
    }

    #[ignore]
    #[test]
    fn attribution_encoding() {
        //TODO: to make this test pass, we need to update transaction with notion of insert/delete id sets
    }

    fn construct<const N: usize>(ops: [(u64, u32, u32, Vec<u32>); N]) -> IdMap<u32> {
        let mut id_map = IdMap::new();

        for (client, clock, len, attrs) in ops {
            let range = BlockRange::new(ID::new(ClientID::new(client), clock), len);
            let attrs: Vec<_> = attrs
                .iter()
                .map(|i| ContentAttribute::new("", *i))
                .collect();
            id_map.insert(range, attrs);
        }

        id_map
    }

    fn random_id_set(clients: u64, clock_range: u32) -> IdSet {
        const MAX_OPS: u32 = 5;
        let num_ops = (clients as u32 * clock_range) / MAX_OPS;
        let mut id_set = IdSet::new();
        for _ in 0..num_ops {
            let client = ClientID::new(fastrand::u64(0..(clients - 1)));
            let clock_start = fastrand::u32(0..clock_range);
            let len = fastrand::u32(0..(clock_range - clock_start));
            id_set.insert(ID::new(client, clock_start), len);
        }
        if id_set.len() == clients as usize && clients > 1 && fastrand::bool() {
            // idset.clients.delete(prng.uint32(gen, 0, clients))
        }
        id_set
    }

    fn random_id_map<T: Hash + Clone + PartialEq + Eq + Serialize>(
        clients: u64,
        clock_range: u32,
        attr_choices: Vec<T>,
    ) -> IdMap<T> {
        const MAX_OP_LEN: u32 = 5;
        let num_ops = ((clients as u32 * clock_range) / MAX_OP_LEN).max(1);
        let mut id_map = IdMap::new();
        for _i in 0..num_ops {
            let client = ClientID::new(fastrand::u64(0..(clients - 1)));
            let clock_start = fastrand::u32(0..clock_range);
            let len = fastrand::u32(0..(clock_range - clock_start));
            let mut attrs = vec![ContentAttribute::new(
                "",
                fastrand::choice(&attr_choices).unwrap().clone(),
            )];
            // maybe add another attr
            if fastrand::bool() {
                let attr =
                    ContentAttribute::new("", fastrand::choice(&attr_choices).unwrap().clone());
                if attr != attrs[0] {
                    attrs.push(attr);
                }
            }
            let range = BlockRange::new(ID::new(client, clock_start), len);
            id_map.insert(range, attrs);
        }
        let _attr_len = attr_choices.len();
        let _enc_len = id_map.encode_v1().len();
        id_map
    }
}
