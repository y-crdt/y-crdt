use crate::block::{BlockRange, ClientID};
use crate::encoding::read::Error;
use crate::id_set::IdRange;
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::{IdSet, ID};
use serde::de::DeserializeOwned;
use serde::Serialize;
use smallvec::{smallvec, SmallVec};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::{Deref, Index, Range};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct IdMap<A> {
    attrs: HashSet<ContentAttribute<A>>,
    clients: HashMap<ClientID, AttrRanges<A>>,
}

impl<A> IdMap<A> {
    pub fn new() -> Self {
        IdMap {
            clients: Default::default(),
            attrs: Default::default(),
        }
    }
}

impl<A> Default for IdMap<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: PartialEq + Eq + Hash> IdMap<A> {
    pub fn from_set(id_set: IdSet, attrs: Vec<ContentAttribute<A>>) -> Self {
        let mut id_map = IdMap {
            clients: HashMap::with_capacity(id_set.len()),
            attrs: HashSet::with_capacity(attrs.len()),
        };
        let mut attrs: SmallVec<[ContentAttribute<A>; 2]> = attrs.into();
        attrs.dedup();
        id_map.ensure_attrs(&mut attrs);
        for (client, ranges) in id_set.iter() {
            let attr_ranges = AttrRanges(
                ranges
                    .iter()
                    .map(|range| AttrRange::new(range.clone()).with_attrs(attrs.clone()))
                    .collect(),
            );
            id_map.clients.insert(*client, attr_ranges);
        }
        id_map
    }

    pub fn iter(&self) -> Iter<'_, A> {
        Iter::new(self)
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    pub fn contains(&self, id: &ID) -> bool {
        if let Some(ranges) = self.clients.get(&id.client) {
            return Self::range_binary_search(ranges, id.clock).is_some();
        }
        false
    }

    pub fn attributions(&self, range: &BlockRange) -> Vec<AttrRange<A>> {
        let client = range.id.client;
        let clock = range.id.clock;
        let len = range.len;
        let mut result: Vec<AttrRange<A>> = Vec::new();

        if let Some(dr) = self.clients.get(&client) {
            if let Some(mut index) = dr.find_start(clock) {
                let mut prev_end = clock;
                while index < dr.len() {
                    let mut r = dr[index].clone();
                    if r.range.start < clock {
                        r.range = clock..r.range.end;
                    }
                    if r.range.end > clock + len {
                        r.range = r.range.start..(clock + len);
                    }
                    if r.range.start >= r.range.end {
                        break;
                    }
                    // Fill gap before this range
                    if prev_end < r.range.start {
                        result.push(AttrRange::new(prev_end..r.range.start));
                    }
                    prev_end = r.range.end;
                    result.push(r);
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

    pub fn insert<I>(&mut self, range: BlockRange, attrs: Vec<ContentAttribute<A>>) {
        if attrs.is_empty() {
            return;
        }

        let mut attrs = attrs;
        self.ensure_attrs(&mut attrs);
        let attr_range =
            AttrRange::new(range.id.clock..(range.id.clock + range.len)).with_attrs(attrs.into());
        match self.clients.entry(range.id.client) {
            Entry::Occupied(mut e) => e.get_mut().insert(attr_range),
            Entry::Vacant(e) => {
                e.insert(AttrRanges(smallvec![attr_range]));
            }
        }
    }

    pub fn remove(&mut self, range: &BlockRange) {
        let client = range.id.client;
        let clock = range.id.clock;
        let len = range.len;

        if len == 0 {
            return;
        }

        let mut remove_client = false;
        if let Some(dr) = self.clients.get_mut(&client) {
            if let Some(mut index) = dr.find_start(clock) {
                let ids = &mut self.clients.get_mut(&client).unwrap().0;
                while index < ids.len() && ids[index].range.start < clock + len {
                    let ar = &mut ids[index];
                    if ar.range.start < clock + len {
                        if ar.range.start < clock {
                            // Range starts before delete region — trim it to end at clock
                            ar.range = ar.range.start..clock;
                            if clock + len < ar.range.end {
                                // Delete doesn't cover the whole range — keep the tail
                                let mut tail = ar.clone();
                                tail.range = (clock + len)..ar.range.end;
                                ids.insert(index + 1, tail);
                            }
                            index += 1;
                        } else if clock + len < ar.range.end {
                            // Delete ends before this range ends — keep tail
                            ar.range = (clock + len)..ar.range.end;
                            index += 1;
                        } else if ids.len() == 1 {
                            // Only range for this client — remove entire client entry
                            remove_client = true;
                            break;
                        } else {
                            // Fully covered by delete — remove this range
                            ids.remove(index);
                            // Don't increment: next element shifted into current position
                        }
                    } else {
                        break;
                    }
                }
            }
        }

        if remove_client {
            self.clients.remove(&client);
        }
    }

    pub fn merge_with(&mut self, other: Self) {
        todo!()
    }

    pub fn merge_many(id_maps: &[Self]) -> Self {
        todo!()
    }

    pub fn intersect_with(&mut self, other: &Self) {
        todo!()
    }

    pub fn filter<F>(&self, predicate: F) -> Self
    where
        F: Fn(&[ContentAttribute<A>]) -> bool,
    {
        /*
        const filtered = createIdMap()
        idmap.clients.forEach((ranges, client) => {
          /**
           * @type {Array<AttrRange<Attrs>>}
           */
          const attrRanges = []
          ranges.getIds().forEach((range) => {
            if (predicate(range.attrs)) {
              const rangeCpy = range.copyWith(range.clock, range.len)
              attrRanges.push(rangeCpy)
              rangeCpy.attrs.forEach(attr => {
                filtered.attrs.add(attr)
                filtered.attrsH.set(attr.hash(), attr)
              })
            }
          })
          if (attrRanges.length > 0) {
            filtered.clients.set(client, new AttrRanges(attrRanges))
          }
        })
        return filtered
               */
        todo!()
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

    fn range_binary_search(dis: &[AttrRange<A>], clock: u32) -> Option<usize> {
        let mut left = 0;
        let mut right = dis.len() - 1;
        while left <= right {
            let mid_idx = (left + right) / 2;
            let attr_range = &dis[mid_idx];
            if attr_range.range.start <= clock {
                if clock < attr_range.range.end {
                    return Some(mid_idx);
                }
                left = mid_idx + 1;
            } else {
                right = mid_idx - 1;
            }
        }
        None
    }
}

impl<A: PartialEq + Eq + Hash> PartialEq for IdMap<A> {
    fn eq(&self, other: &Self) -> bool {
        self.attrs == other.attrs && self.clients == other.clients
    }
}

pub struct Iter<'a, A> {
    _b: &'a IdMap<A>,
}

impl<'a, A> Iter<'a, A> {
    fn new(id_map: &'a IdMap<A>) -> Self {
        todo!()
    }
}

impl<'a, A> Iterator for Iter<'a, A> {
    type Item = (ClientID, AttrRange<A>);

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

pub trait Diff<T> {
    fn diff_with(&mut self, other: &T);
}

impl<A> Diff<IdSet> for IdMap<A> {
    fn diff_with(&mut self, other: &IdSet) {}
}

impl<A> Diff<IdMap<A>> for IdMap<A> {
    fn diff_with(&mut self, other: &IdMap<A>) {}
}

impl<A> From<IdMap<A>> for IdSet {
    fn from(value: IdMap<A>) -> Self {
        let mut set = IdSet::default();
        for (client, ranges) in value.clients {
            let range = set.range_mut(client);
            for attr_range in ranges.0 {
                range.insert(attr_range.range);
            }
        }
        set
    }
}

impl<A: Serialize> Encode for IdMap<A> {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_var(self.clients.len());
        let mut last_written_client_id = 0;
        let mut visited_attributions = HashMap::new();
        let mut visited_attr_names = HashMap::new();

        // Ensure that the ids are written in a deterministic order (smaller clientids first)
        let mut keys = self.clients.keys().copied().collect::<Vec<_>>();
        keys.sort();
        for client_id in keys {
            let ranges = self.clients.get(&client_id).unwrap();
            todo!()
        }

        /*

        encoding.writeVarUint(encoder.restEncoder, idmap.clients.size)
        let lastWrittenClientId = 0
        /**
         * @type {Map<ContentAttribute<Attr>, number>}
         */
        const visitedAttributions = map.create()
        /**
         * @type {Map<string, number>}
         */
        const visitedAttrNames = map.create()
        // Ensure that the ids are written in a deterministic order (smaller clientids first)
        array.from(idmap.clients.entries())
          .sort((a, b) => a[0] - b[0])
          .forEach(([client, _idRanges]) => {
            const attrRanges = _idRanges.getIds()
            encoder.resetIdSetCurVal()
            const diff = client - lastWrittenClientId
            encoding.writeVarUint(encoder.restEncoder, diff)
            lastWrittenClientId = client
            const len = attrRanges.length
            encoding.writeVarUint(encoder.restEncoder, len)
            for (let i = 0; i < len; i++) {
              const item = attrRanges[i]
              const attrs = item.attrs
              const attrLen = attrs.length
              encoder.writeIdSetClock(item.clock)
              encoder.writeIdSetLen(item.len)
              encoding.writeVarUint(encoder.restEncoder, attrLen)
              for (let j = 0; j < attrLen; j++) {
                const attr = attrs[j]
                const attrId = visitedAttributions.get(attr)
                if (attrId != null) {
                  encoding.writeVarUint(encoder.restEncoder, attrId)
                } else {
                  const newAttrId = visitedAttributions.size
                  visitedAttributions.set(attr, newAttrId)
                  encoding.writeVarUint(encoder.restEncoder, newAttrId)
                  const attrNameId = visitedAttrNames.get(attr.name)
                  // write attr.name
                  if (attrNameId != null) {
                    encoding.writeVarUint(encoder.restEncoder, attrNameId)
                  } else {
                    const newAttrNameId = visitedAttrNames.size
                    encoding.writeVarUint(encoder.restEncoder, newAttrNameId)
                    encoding.writeVarString(encoder.restEncoder, attr.name)
                    visitedAttrNames.set(attr.name, newAttrNameId)
                  }
                  encoding.writeAny(encoder.restEncoder, /** @type {any} */ (attr.val))
                }
              }
            }
          })
               */
    }
}

impl<A: DeserializeOwned> Decode for IdMap<A> {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        /*
        const idmap = new IdMap()
        const numClients = decoding.readVarUint(decoder.restDecoder)
        /**
         * @type {Array<ContentAttribute<any>>}
         */
        const visitedAttributions = []
        /**
         * @type {Array<string>}
         */
        const visitedAttrNames = []
        let lastClientId = 0
        for (let i = 0; i < numClients; i++) {
          decoder.resetDsCurVal()
          const client = lastClientId + decoding.readVarUint(decoder.restDecoder)
          lastClientId = client
          const numberOfDeletes = decoding.readVarUint(decoder.restDecoder)
          /**
           * @type {Array<AttrRange<any>>}
           */
          const attrRanges = []
          for (let i = 0; i < numberOfDeletes; i++) {
            const rangeClock = decoder.readDsClock()
            const rangeLen = decoder.readDsLen()
            /**
             * @type {Array<ContentAttribute<any>>}
             */
            const attrs = []
            const attrsLen = decoding.readVarUint(decoder.restDecoder)
            for (let j = 0; j < attrsLen; j++) {
              const attrId = decoding.readVarUint(decoder.restDecoder)
              if (attrId >= visitedAttributions.length) {
                // attrId not known yet
                const attrNameId = decoding.readVarUint(decoder.restDecoder)
                if (attrNameId >= visitedAttrNames.length) {
                  visitedAttrNames.push(decoding.readVarString(decoder.restDecoder))
                }
                visitedAttributions.push(new ContentAttribute(visitedAttrNames[attrNameId], decoding.readAny(decoder.restDecoder)))
              }
              attrs.push(visitedAttributions[attrId])
            }
            attrRanges.push(new AttrRange(rangeClock, rangeLen, attrs))
          }
          idmap.clients.set(client, new AttrRanges(attrRanges))
        }
        visitedAttributions.forEach(attr => {
          idmap.attrs.add(attr)
          idmap.attrsH.set(attr.hash(), attr)
        })
        return idmap
               */
        todo!()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AttrRanges<A>(SmallVec<[AttrRange<A>; 1]>);

impl<A> AttrRanges<A> {
    #[inline]
    pub fn new(attrs: SmallVec<[AttrRange<A>; 1]>) -> Self {
        Self(attrs)
    }

    pub fn insert(&mut self, range: AttrRange<A>) {
        todo!()
    }

    pub fn get_ids(&self) -> IdRange {
        let mut ranges = SmallVec::with_capacity(self.0.len());
        for attr_range in &self.0 {
            ranges.push(attr_range.range.clone());
        }
        IdRange::new(ranges)
    }

    pub fn find_start(&self, clock: u32) -> Option<usize> {
        let mut left = 0;
        let mut right = self.0.len() - 1;
        while left <= right {
            let mid_idx = (left + right) / 2;
            let a = &self.0[mid_idx];
            if a.range.start <= clock {
                if clock < a.range.end {
                    return Some(mid_idx);
                }
                left = mid_idx + 1;
            } else {
                right = mid_idx - 1;
            }
        }
        if left < self.0.len() {
            Some(left)
        } else {
            None
        }
    }
}

impl<A> Deref for AttrRanges<A> {
    type Target = [AttrRange<A>];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct AttrRange<A> {
    range: Range<u32>,
    attrs: SmallVec<[ContentAttribute<A>; 2]>,
}

impl<A> AttrRange<A> {
    pub fn new(range: Range<u32>) -> Self {
        AttrRange {
            range,
            attrs: Default::default(),
        }
    }

    pub fn with_attrs(self, attrs: SmallVec<[ContentAttribute<A>; 2]>) -> Self {
        AttrRange {
            range: self.range,
            attrs,
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

#[derive(Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct ContentAttribute<A>(Arc<ContentAttributeInner<A>>);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
struct ContentAttributeInner<A> {
    name: String,
    value: A,
}

impl<A> ContentAttribute<A> {
    pub fn new(name: String, value: A) -> Self {
        ContentAttribute(Arc::new(ContentAttributeInner { name, value }))
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
    use crate::id_map::{ContentAttribute, Diff, IdMap};
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::Encode;
    use crate::{Any, IdSet, ID};
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
        const CLIENTS: u32 = 4;
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
                        composed.insert(BlockRange::new(ID::new(client_id, clock), len), a.attrs);
                    }
                }
            }
        }
        assert_eq!(merged, composed);
    }

    #[test]
    fn repeat_random_diffing() {
        const CLIENTS: u32 = 4;
        const CLOCK_RANGE: u32 = 5;
        let attrs = vec![1, 2, 3];
        let mut id_map_1 = random_id_map(CLIENTS, CLOCK_RANGE, attrs.clone());
        let id_map_2 = random_id_map(CLIENTS, CLOCK_RANGE, attrs.clone());
        let mut merged = IdMap::merge_many(&[id_map_1.clone(), id_map_2.clone()]);
        id_map_1.diff_with(&id_map_2);
        merged.diff_with(&id_map_2);
        assert_eq!(merged, id_map_1);

        let copy = IdMap::decode_v1(&id_map_1.encode_v1()).unwrap();
        assert_eq!(id_map_1, copy);
    }

    #[test]
    fn repeat_random_diffing_2() {
        const CLIENTS: u32 = 4;
        const CLOCK_RANGE: u32 = 100;
        let attrs = vec![1, 2, 3];
        let mut id_map_1 = random_id_map(CLIENTS, CLOCK_RANGE, attrs.clone());
        let mut id_map_2 = random_id_map(CLIENTS, CLOCK_RANGE, attrs.clone());
        let id_exclude = random_id_set(CLIENTS, CLOCK_RANGE);
        let mut merged = IdMap::merge_many(&[id_map_1.clone(), id_map_2.clone()]);
        let mut merge_excluded = merged.clone();
        merge_excluded.diff_with(&id_exclude);

        id_map_1.diff_with(&id_exclude);
        id_map_2.diff_with(&id_exclude);

        let excluded_merged = IdMap::merge_many(&[id_map_1.clone(), id_map_2.clone()]);
        assert_eq!(merge_excluded, excluded_merged);

        let copy = IdMap::decode_v1(&merge_excluded.encode_v1()).unwrap();
        assert_eq!(merge_excluded, copy);
    }

    #[test]
    fn repeat_random_deletes() {
        const CLIENTS: u32 = 1;
        const CLOCK_RANGE: u32 = 100;
        let mut id_map: IdMap<i32> = random_id_map(CLIENTS, CLOCK_RANGE, vec![]);
        let client = *id_map.clients.keys().next().unwrap();
        let clock = fastrand::u32(0..CLOCK_RANGE);
        let len = fastrand::u32(0..((CLOCK_RANGE - clock) as f64 * 1.2).round() as u32); // allow exceeding range to cover more edge cases
        let mut ds_map = IdMap::new();
        ds_map.insert(BlockRange::new(ID::new(client, clock), len), vec![]);
        let mut diffed = id_map.clone();
        diffed.diff_with(&ds_map);
        id_map.remove(&BlockRange::new(ID::new(client, clock), len));

        for i in 0..len {
            assert!(id_map.contains(&ID::new(client, clock + i)));
        }

        assert_eq!(id_map, diffed);
    }

    #[test]
    fn repeat_random_intersects() {
        const CLIENTS: u32 = 4;
        const CLOCK_RANGE: u32 = 100;
        let mut ids1: IdMap<Any> = random_id_map(CLIENTS, CLOCK_RANGE, vec![1.into()]);
        let ids2: IdMap<Any> = random_id_map(CLIENTS, CLOCK_RANGE, vec!["two".into()]);
        let mut intersected = ids1.clone();
        intersected.intersect_with(&ids2);

        for client in 0..CLIENTS {
            for clock in 0..CLOCK_RANGE {
                let id = ID::new(client as ClientID, clock);
                assert_eq!(
                    ids1.contains(&id) && ids2.contains(&id),
                    intersected.contains(&id),
                    "if both maps contain ID, so should intersection"
                );

                let range1 = ids1.attributions(&BlockRange::new(id, 1)).first().cloned();
                let range2 = ids2.attributions(&BlockRange::new(id, 1)).first().cloned();

                let expected_attrs: Vec<_> = [range1, range2]
                    .into_iter()
                    .filter_map(|x| x.clone())
                    .collect();
                let attrs = if let Some(attr) = intersected
                    .attributions(&BlockRange::new(id, 1))
                    .first()
                    .cloned()
                {
                    vec![attr]
                } else {
                    Vec::new()
                };
                assert_eq!(attrs, expected_attrs);
            }
        }

        let mut diffed1 = ids1.clone();
        diffed1.diff_with(&ids2);
        ids1.diff_with(&intersected);
        assert_eq!(ids1, diffed1);
    }

    #[ignore]
    #[test]
    fn attribution_encoding() {
        //TODO: to make this test pass, we need to update transaction with notion of inset/delete id sets

        /*

        /**
         * @todo debug why this approach needs 30 bytes per item
         * @todo it should be possible to only use a single idmap and, in each attr entry, encode the diff
         * to the previous entries (e.g. remove a,b, insert c,d)
         */
        const attributions = createIdMap()
        const currentTime = time.getUnixTime()
        const ydoc = new YY.Doc()
        ydoc.on('afterTransaction', tr => {
          ids.insertIntoIdMap(attributions, ids.createIdMapFromIdSet(tr.insertSet, [createContentAttribute('insert', 'userX'), createContentAttribute('insertAt', currentTime)]))
          ids.insertIntoIdMap(attributions, ids.createIdMapFromIdSet(tr.deleteSet, [createContentAttribute('delete', 'userX'), createContentAttribute('deleteAt', currentTime)]))
        })
        const ytext = ydoc.get()
        const N = 10000
        t.measureTime(`time to attribute ${N / 1000}k changes`, () => {
          for (let i = 0; i < N; i++) {
            if (i % 2 > 0 && ytext.length > 0) {
              const pos = prng.int31(tc.prng, 0, ytext.length)
              const delLen = prng.int31(tc.prng, 0, ytext.length - pos)
              ytext.delete(pos, delLen)
            } else {
              ytext.insert(prng.int31(tc.prng, 0, ytext.length), prng.word(tc.prng))
            }
          }
        })
        t.measureTime('time to encode attributions map', () => {
          /**
           * @todo I can optimize size by encoding only the differences to the prev item.
           */
          const encAttributions = ids.encodeIdMap(attributions)
          t.info('encoded size: ' + encAttributions.byteLength)
          t.info('size per change: ' + math.floor((encAttributions.byteLength / N) * 100) / 100 + ' bytes')
        })
               */
        todo!()
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

    fn random_id_set(clients: u32, clock_range: u32) -> IdSet {
        const MAX_OPS: u32 = 5;
        let num_ops = (clients * clock_range) / MAX_OPS;
        let mut id_set = IdSet::new();
        for _ in 0..num_ops {
            let client = fastrand::u32(0..(clients - 1)) as ClientID;
            let clock_start = fastrand::u32(0..clock_range);
            let len = fastrand::u32(0..(clock_range - clock_start));
            id_set.insert(ID::new(client, clock_start), len);
        }
        if id_set.len() == clients as usize && clients > 1 && fastrand::bool() {
            // idset.clients.delete(prng.uint32(gen, 0, clients))
        }
        id_set
    }

    fn random_id_map<T: Hash + Clone + PartialEq + Serialize>(
        clients: u32,
        clock_range: u32,
        attr_choices: Vec<T>,
    ) -> IdMap<T> {
        const MAX_OP_LEN: u32 = 5;
        let num_ops = ((clients * clock_range) / MAX_OP_LEN).max(1);
        let mut id_map = IdMap::new();
        for i in 0..num_ops {
            let client = fastrand::u32(0..(clients - 1)) as ClientID;
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
        let attr_len = attr_choices.len();
        let enc_len = id_map.encode_v1().len();
        //println!(
        //    "created IdMap with {num_ops} ranges and {attr_len} different attributes. Encoded size: {enc_len}",
        //);
        id_map
    }
}
