use crate::block::{Block, BlockRange, BlockRef, ClientID, ItemPtr, ID};
use crate::slice::ItemSlice;
use crate::update::BlockSet;
use crate::utils::client_hasher::ClientHasher;
use crate::*;
use std::cell::UnsafeCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::ops::{Index, RangeInclusive};
use std::vec::Vec;

/// A resizable list of blocks inserted by a single client.
#[repr(transparent)]
#[derive(Default)]
pub(crate) struct ClientBlockList {
    inner: Vec<UnsafeCell<Block>>,
}

unsafe impl Send for ClientBlockList {}
unsafe impl Sync for ClientBlockList {}

impl ClientBlockList {
    pub fn last(&self) -> Option<BlockRef<'_>> {
        let cell = self.inner.last()?;
        Some(BlockRef::new(cell))
    }

    pub fn clock(&self) -> u32 {
        match self.last() {
            None => 0,
            Some(block) => block.as_ref().next_clock(),
        }
    }

    pub(crate) fn get(&self, index: usize) -> Option<BlockRef<'_>> {
        let cell = self.inner.get(index)?;
        Some(BlockRef::new(cell))
    }

    /// Given a block's identifier clock value, return an offset under which this block could be
    /// found using binary search algorithm, or a index under which this block should be inserted.
    pub(crate) fn find_index(&self, clock: u32) -> Option<usize> {
        let mut left = 0;
        let mut right = self.inner.len() - 1;
        let mut block = unsafe { &*self.inner[right].get() };
        let (mut start, mut end) = block.clock_range();
        if start == clock {
            // a common case is to just append a block at the end, so check first if we can do that
            Some(right)
        } else if clock > end {
            None // clock lands past the last block, so no block owns it.
        } else {
            let mut mid = ((clock / end) * right as u32) as usize;
            while left <= right {
                block = unsafe { &*self.inner[mid].get() };
                (start, end) = block.clock_range();
                if start <= clock {
                    if clock <= end {
                        return Some(mid);
                    }
                    left = mid + 1;
                } else {
                    right = mid - 1;
                }
                mid = (left + right) / 2;
            }

            None
        }
    }

    /// Attempts to find a Block which contains given clock sequence number within current block
    /// list. Clocks are considered to work in left-side inclusive way, meaning that block with
    /// an ID (<client-id>, 0) and length 2, with contain all elements with clock values
    /// corresponding to {0,1} but not 2.
    fn get_block(&self, clock: u32) -> Option<BlockRef<'_>> {
        let idx = self.find_index(clock)?;
        self.get(idx)
    }

    pub(crate) fn insert(&mut self, index: usize, cell: Block) {
        self.inner.insert(index, UnsafeCell::new(cell));
    }

    /// Pushes a new block at the end of this block list.
    fn push(&mut self, cell: Block) {
        self.inner.push(UnsafeCell::new(cell));
    }

    /// Returns a number of blocks stored within this list.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn iter(&self) -> ClientBlockListIter<'_> {
        ClientBlockListIter(self.inner.iter())
    }

    /// Squashes every block whose index is in one of `ranges` into its left neighbor where
    /// possible, in a single pass that compacts the list once. `ranges` must be sorted, must not
    /// overlap and must not include index 0.
    pub(crate) fn squash_left_in(&mut self, ranges: &[RangeInclusive<usize>]) {
        let (Some(first), Some(last)) = (ranges.first(), ranges.last()) else {
            return;
        };
        debug_assert!(*first.start() > 0, "the first block has no left neighbor");
        debug_assert!(
            ranges
                .windows(2)
                .all(|pair| pair[0].end() < pair[1].start()),
            "ranges are sorted and don't overlap"
        );
        let mut ranges = ranges.iter().peekable();
        // `kept` is the last block that stays in the list; blocks squashed into it are moved past
        // it and dropped at the end.
        let mut kept = *first.start() - 1;
        for i in *first.start()..=*last.end() {
            while ranges.next_if(|range| *range.end() < i).is_some() {}
            let is_in_ranges = ranges.peek().is_some_and(|range| range.contains(&i));
            if is_in_ranges && self.can_squash_into(kept, i) {
                self.squash_into(kept, i);
            } else {
                kept += 1;
                self.inner.swap(kept, i);
            }
        }
        self.inner.drain(kept + 1..=*last.end());
    }

    /// Attempts to squash block at a given `index` with a corresponding block on its left side.
    /// If this succeeds, block under a given `index` will be removed, and its contents will be
    /// squashed into its left neighbor. In such case a squash result will be returned in order to
    /// later on rewire left/right neighbor changes that may have occurred as a result of squashing
    /// and block removal.
    pub(crate) fn squash_left(&mut self, pos: usize) -> usize {
        let mut start = pos;
        while start > 0 && self.can_squash_into(start - 1, start) {
            start -= 1;
        }
        // Squash from left to right, so that each block's content is appended once. Squashing
        // from right to left would copy all the content squashed so far at every step.
        for i in start + 1..=pos {
            self.squash_into(start, i);
        }

        let merged = pos - start;
        if merged > 0 {
            self.inner.drain(start + 1..=pos);
        }
        merged
    }

    /// Squashes every block from `first` onwards into its left neighbor where possible, the same
    /// way as calling [Self::squash_left] on each of them from the last one down to `first`
    /// would, but with a single pass that compacts the list once.
    pub(crate) fn squash_left_from(&mut self, first: usize) {
        if first == 0 || first >= self.inner.len() {
            return;
        }
        let mut start = first;
        while start > 0 && self.can_squash_into(start - 1, start) {
            start -= 1;
        }

        // `kept` is the last block that stays in the list; blocks squashed into it are moved past
        // it and dropped at the end.
        let mut kept = start;
        for i in start + 1..self.inner.len() {
            if self.can_squash_into(kept, i) {
                self.squash_into(kept, i);
            } else {
                kept += 1;
                self.inner.swap(kept, i);
            }
        }
        self.inner.truncate(kept + 1);
    }

    fn can_squash_into(&self, left: usize, right: usize) -> bool {
        let left = &self[left];
        let right = &self[right];
        left.is_deleted() == right.is_deleted() && left.same_type(right) && left.can_squash(right)
    }

    fn squash_into(&mut self, left: usize, right: usize) {
        let left = unsafe { &mut *self.inner[left].get() };
        let right = unsafe { &*self.inner[right].get() };
        let squashed = left.try_squash(right);
        debug_assert!(squashed, "blocks were checked to be squashable");
        // update parent's map entry if necessary
        if let Block::Item(right) = right {
            if let Some(parent_sub) = &right.parent_sub {
                if let Some(mut parent) = right.parent.as_branch().copied() {
                    let e = parent.map.get_mut(parent_sub).unwrap();
                    if &**e == &**right {
                        *e = left.as_item().unwrap(); // already confirmed they're the same_type
                    }
                }
            }
        }
    }
}

impl Index<usize> for ClientBlockList {
    type Output = Block;

    fn index(&self, index: usize) -> &Self::Output {
        unsafe { &*self.inner[index].get() }
    }
}

pub(crate) struct ClientBlockListIter<'a>(std::slice::Iter<'a, UnsafeCell<Block>>);

impl<'a> Iterator for ClientBlockListIter<'a> {
    type Item = BlockRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let cell = self.0.next()?;
        Some(BlockRef::new(cell))
    }
}

/// Block store is a collection of all blocks known to a document owning instance of this type.
/// Blocks are organized per client ID and contain a resizable list of all blocks inserted by that
/// client.
#[derive(Default)]
pub(crate) struct BlockStore {
    clients: HashMap<ClientID, ClientBlockList, BuildHasherDefault<ClientHasher>>,
    pub(crate) skips: IdSet,
}

pub(crate) type Iter<'a> = std::collections::hash_map::Iter<'a, ClientID, ClientBlockList>;
pub(crate) type IterMut<'a> = std::collections::hash_map::IterMut<'a, ClientID, ClientBlockList>;

impl BlockStore {
    /// Checks if block store is empty. Empty block store doesn't contain any blocks, neither active
    /// nor tombstoned.
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    pub fn is_missing(&self, id: &ID) -> bool {
        id.clock >= self.get_clock(&id.client) || self.skips.contains(id)
    }

    pub fn contains(&self, id: &ID) -> bool {
        if let Some(clients) = self.clients.get(&id.client) {
            id.clock < clients.clock()
        } else {
            false
        }
    }

    pub fn push(&mut self, block: Block) {
        let id = block.id();
        match self.clients.entry(id.client) {
            Entry::Occupied(mut e) => {
                let list = e.get_mut();
                match list.last() {
                    Some(last) if last.as_ref().next_clock() != block.clock_start() => {
                        // this replaces an integrated skip
                        let clock_start = block.clock_start();
                        let mut index = list.find_index(clock_start).unwrap();
                        let skip = unsafe { &*list.inner[index].get() };
                        let diff_start = clock_start - skip.clock_start();
                        let diff_end = skip.next_clock() - block.next_clock();
                        if diff_start > 0 {
                            list.insert(index, Block::Skip(BlockRange::new(skip.id(), diff_start)));
                            index += 1;
                        }
                        if diff_end > 0 {
                            let mut id = block.id();
                            id.clock += block.len();
                            let skip = Block::Skip(BlockRange::new(id, diff_end));
                            list.inner.insert(index + 1, UnsafeCell::new(skip));
                        }
                        self.skips.remove_range(&block.range());
                        list.inner[index] = UnsafeCell::new(block);
                    }
                    _ => list.inner.push(UnsafeCell::new(block)),
                }
            }
            Entry::Vacant(e) => {
                let list = e.insert(ClientBlockList::default());
                list.push(block);
            }
        }
    }

    /// Returns an iterator over the client and block lists pairs known to a current block store.
    pub fn iter(&self) -> Iter<'_> {
        self.clients.iter()
    }

    /// Returns an iterator over the client and mutable block lists pairs known to a current block store.
    pub fn iter_mut(&mut self) -> IterMut<'_> {
        self.clients.iter_mut()
    }

    /// Returns a state vector, which is a compact representation of the state of blocks integrated
    /// into a current block store. This state vector can later be encoded and send to a remote
    /// peers in order to calculate differences between two stored and produce a compact update,
    /// that can be applied in order to fill missing update information.
    pub fn get_state_vector(&self) -> StateVector {
        let mut map: HashMap<ClientID, u32, _> = self
            .clients
            .iter()
            .map(|(client_id, list)| (*client_id, list.clock()))
            .collect();

        for (client, ranges) in self.skips.iter() {
            if let Some(clock) = ranges.clock_start() {
                map.insert(*client, clock);
            }
        }

        StateVector::new(map)
    }

    pub(crate) fn known_state(&self, ss: &BlockSet) -> IdSet {
        let mut known_state = IdSet::default();
        for (client, _) in ss.clients.iter() {
            if let Some(store_structs) = self.clients.get(client) {
                let last = store_structs.last().unwrap().as_ref();
                known_state.insert(ID::new(*client, 0), last.next_clock());
                // remove known items from ss
                if let Some(skips) = self.skips.get(client) {
                    for (skip, _) in skips.iter() {
                        known_state.remove_range(&BlockRange::new(
                            ID::new(*client, skip.start),
                            skip.end - skip.start,
                        ));
                    }
                }
            }
        }
        known_state
    }

    pub(crate) fn get_client(&self, client_id: &ClientID) -> Option<&ClientBlockList> {
        self.clients.get(client_id)
    }

    pub(crate) fn get_client_mut(&mut self, client_id: &ClientID) -> Option<&mut ClientBlockList> {
        self.clients.get_mut(client_id)
    }

    /// Returns immutable reference to a block, given its pointer. Returns `None` if not such
    /// block could be found.
    pub(crate) fn get_block(&self, id: &ID) -> Option<BlockRef<'_>> {
        let clients = self.clients.get(&id.client)?;
        clients.get_block(id.clock)
    }

    pub(crate) fn get_item(&self, id: &ID) -> Option<ItemPtr> {
        let mut cell = self.get_block(id)?;
        let item = cell.as_item_mut()?;
        Some(ItemPtr::from(&*item))
    }

    /// Returns a block slice that represents a range of data within a particular block containing
    /// provided [ID], starting from that [ID] until the end of the block.
    ///
    /// Example: *for a block `A:1..=5` and id `A:3`, the returned slice will represent `A:3..=5`*.
    pub(crate) fn get_item_clean_start(&self, id: &ID) -> Option<ItemSlice> {
        let ptr = self.get_item(id)?;
        let offset = id.clock - ptr.id().clock;
        Some(ItemSlice::new(ptr, offset, ptr.len() - 1))
    }

    /// Returns a block slice that represents a range of data within a particular block containing
    /// provided [ID], starting from the beginning of the block until the that [ID] (inclusive).
    ///
    /// Example: *for a block `A:1..=5` and id `A:3`, the returned slice will represent `A:1..=3`*.
    pub(crate) fn get_item_clean_end(&self, id: &ID) -> Option<ItemSlice> {
        let ptr = self.get_item(id)?;
        let block_id = ptr.id();
        let offset = id.clock - block_id.clock;
        Some(ItemSlice::new(ptr, 0, offset))
    }

    /// Returns the last observed clock sequence number for a given `client`. This is exclusive
    /// value meaning it describes a clock value of the beginning of the next block that's about
    /// to be inserted. You cannot use that clock value to find any existing block content.
    pub fn get_clock(&self, client: &ClientID) -> u32 {
        if let Some(list) = self.clients.get(client) {
            list.clock()
        } else {
            0
        }
    }

    /// Returns a mutable reference to block list for the given `client`. In case when no such list
    /// existed, a new one will be created and returned.
    pub(crate) fn get_client_blocks_mut(&mut self, client: ClientID) -> &mut ClientBlockList {
        self.clients
            .entry(client)
            .or_insert_with(ClientBlockList::default)
    }

    /// Given block pointer, tries to split it, returning a true, if block was split in result of
    /// calling this action, and false otherwise.
    pub fn split_block(
        &mut self,
        mut block: ItemPtr,
        offset: u32,
        encoding: OffsetKind,
    ) -> Option<ItemPtr> {
        let id = block.id().clone();
        let blocks = self.clients.get_mut(&id.client)?;
        let index = blocks.find_index(id.clock)?;
        let mut right = block.splice(offset, encoding)?;
        let right_ptr = ItemPtr::from(&mut right);
        blocks.insert(index + 1, right.into());

        Some(right_ptr)
    }

    pub(crate) fn split_block_inner(&mut self, block: ItemPtr, offset: u32) -> Option<ItemPtr> {
        self.split_block(block, offset, OffsetKind::Utf16)
    }
}

impl std::fmt::Debug for ClientBlockList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for ClientBlockList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.inner.iter()).finish()
    }
}

impl std::fmt::Debug for BlockStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for BlockStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("");
        for (k, v) in self.clients.iter() {
            s.field(&k.to_string(), v);
        }
        s.finish()
    }
}

#[cfg(test)]
mod test {
    use crate::block::ClientID;
    use crate::updates::decoder::Decode;
    use crate::{Doc, GetString, ReadTxn, Text, Transact, Update};

    const CLIENT: u64 = 7;

    fn write_var(buf: &mut Vec<u8>, mut value: u64) {
        while value >= 0x80 {
            buf.push((value as u8 & 0x7f) | 0x80);
            value >>= 7;
        }
        buf.push(value as u8);
    }

    /// Builds a v1 update with `count` one-character items of `CLIENT` starting at clock 0. The
    /// first one starts the "text" root; `origins(clock)` gives the origin and right origin of
    /// every other one. `deleted` lists `(clock, len)` ranges of `CLIENT` to delete.
    fn one_char_items(
        count: u64,
        origins: impl Fn(u64) -> (Option<u64>, Option<u64>),
        deleted: &[(u64, u64)],
    ) -> Vec<u8> {
        let mut buf = vec![1];
        write_var(&mut buf, count);
        write_var(&mut buf, CLIENT);
        write_var(&mut buf, 0);
        buf.extend_from_slice(&[4, 1, 4]);
        buf.extend_from_slice(b"text");
        buf.extend_from_slice(&[1, b'a']);
        for clock in 1..count {
            let (origin, right_origin) = origins(clock);
            let mut info = 4;
            if origin.is_some() {
                info |= 0x80;
            }
            if right_origin.is_some() {
                info |= 0x40;
            }
            buf.push(info);
            for id in [origin, right_origin].iter().flatten() {
                write_var(&mut buf, CLIENT);
                write_var(&mut buf, *id);
            }
            buf.extend_from_slice(&[1, b'a']);
        }
        if deleted.is_empty() {
            buf.push(0);
        } else {
            buf.push(1);
            write_var(&mut buf, CLIENT);
            write_var(&mut buf, deleted.len() as u64);
            for &(clock, len) in deleted {
                write_var(&mut buf, clock);
                write_var(&mut buf, len);
            }
        }
        buf
    }

    /// Applies `update` to a new doc and returns its text and the number of blocks `CLIENT`
    /// holds afterwards.
    fn apply(update: &[u8]) -> (String, usize) {
        let doc = Doc::new();
        let text = doc.get_or_insert_text("text");
        doc.transact_mut()
            .apply_update(Update::decode_v1(update).unwrap())
            .unwrap();
        let txn = doc.transact();
        let blocks = txn
            .store()
            .blocks
            .get_client(&ClientID::new(CLIENT))
            .unwrap()
            .len();
        (text.get_string(&txn), blocks)
    }

    #[test]
    fn squash_long_run_in_one_update() {
        // Squashing a run right to left used to copy everything squashed so far into the block
        // on its left at every step: memory and time grew with the square of the run's length.
        let count = 10_000;
        let update = one_char_items(count, |clock| (Some(clock - 1), None), &[]);
        let (text, blocks) = apply(&update);
        assert_eq!(text, "a".repeat(count as usize));
        assert_eq!(blocks, 1);
    }

    #[test]
    fn squash_many_short_runs_in_one_update() {
        // Pairs of items, each pair inserted after the first item: the two in a pair squash, the
        // pairs don't. Squashing each run used to shift the rest of the block list.
        let pairs = 5_000;
        let update = one_char_items(
            1 + 2 * pairs,
            |clock| {
                let pair_start = clock - (clock - 1) % 2;
                let previous_pair = (pair_start > 1).then(|| pair_start - 2);
                match clock == pair_start {
                    true => (Some(0), previous_pair),
                    false => (Some(pair_start), previous_pair),
                }
            },
            &[],
        );
        let (text, blocks) = apply(&update);
        assert_eq!(text.len(), 1 + 2 * pairs as usize);
        assert_eq!(blocks, 1 + pairs as usize);
    }

    #[test]
    fn squash_run_deleted_in_same_update() {
        // A run deleted by the same update is squashed with the delete set, which used to work
        // right to left too and shift the block list once per squashed pair.
        let run = 5_000;
        let update = one_char_items(
            2 * run,
            |clock| match clock < run {
                true => (Some(clock - 1), None),
                false => (Some(run - 1), (clock > run).then(|| clock - 1)),
            },
            &[(0, run)],
        );
        let (text, blocks) = apply(&update);
        assert_eq!(text, "a".repeat(run as usize));
        assert_eq!(blocks, 1 + run as usize);
    }
}
