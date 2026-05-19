use crate::block::{Block, BlockRef, ClientID, ItemPtr, ID};
use crate::slice::ItemSlice;
use crate::types::TypePtr;
use crate::utils::client_hasher::ClientHasher;
use crate::*;
use std::cell::UnsafeCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::ops::{Index, Range, RangeInclusive};
use std::vec::Vec;

/// A resizable list of blocks inserted by a single client.
#[repr(transparent)]
#[derive(Default)]
pub(crate) struct ClientBlockList {
    inner: Vec<UnsafeCell<Block>>,
}

struct SquashBlockRange {
    range: Range<usize>,
    gc_block: bool,
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

    /// Pushes a new block at the end of this block list.
    fn push(&mut self, cell: Block) {
        self.inner.push(UnsafeCell::new(cell));
    }

    /// Inserts a new block at a given `index` position within this block list. This method may
    /// panic if `index` is greater than a length of the list.
    pub(crate) fn insert(&mut self, index: usize, cell: Block) {
        self.inner.insert(index, UnsafeCell::new(cell));
    }

    /// Returns a number of blocks stored within this list.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn iter(&self) -> ClientBlockListIter<'_> {
        ClientBlockListIter(self.inner.iter())
    }

    /// Attempts to squash multiple blocks within the given range of indices.
    /// For each block in the `indices_range`, it will check if the block can be squashed with its left neighbor.
    /// If consecutive blocks are squashable, they are tracked in a range and processed in bulk to compact
    /// the list efficiently. The function supports both GC and Block cells.
    ///
    /// - For GC blocks: If blocks are consecutive, the range is extended and squashing is deferred until
    ///   all squashable blocks are identified.
    ///
    /// - For Block cells: The function attempts to squash the contents of the right block into the left block.
    ///   If successful, it tracks the blocks to be removed and rewires references in the parent node if necessary.
    ///   Block cells currently don't support range compaction due to the complexity of squashing Blocks.
    ///
    /// The function processes all blocks in reverse order (from the end of the range to the start),
    /// compacts the list by removing squashed blocks, and updates references for any parent-child relationships
    /// affected by the squashing.
    ///
    /// # Arguments
    /// * `indices_range` - A range of indices, where each index represents a block in the list to be examined
    ///   for squashing. The range must be non-empty (`start` must be <= `end`).
    ///
    /// # Panics
    /// * Panics if `indices_range.start()` is greater than `indices_range.end()`.
    ///
    pub(crate) fn squash_left_range_compaction(&mut self, indices_range: RangeInclusive<usize>) {
        assert!(indices_range.start() <= indices_range.end());
        let mut squash_intervals: Vec<SquashBlockRange> = Vec::new();

        for right_index in indices_range.rev() {
            let (l, r) = self.inner.split_at_mut(right_index);
            let left = unsafe { &mut *l[l.len() - 1].get() };
            let right = unsafe { &mut *r[0].get() };

            match (left, right) {
                (Block::GC(_), Block::GC(_)) => {
                    let mut extended = false;
                    match squash_intervals.last_mut() {
                        Some(last_range) if last_range.gc_block => {
                            // Extend if consecutive
                            if last_range.range.start - 1 == right_index {
                                last_range.range.start = right_index;
                                extended = true;
                            }
                        }
                        _ => {}
                    }

                    if !extended {
                        // Add new range if no consecutive block found
                        squash_intervals.push(SquashBlockRange {
                            range: Range {
                                start: right_index,
                                end: right_index,
                            },
                            gc_block: true,
                        });
                    }
                }
                (Block::Item(left), Block::Item(right)) => {
                    let mut left = ItemPtr::from(left);
                    let right = ItemPtr::from(right);
                    if left.try_squash(right) {
                        // Merge right into left Blocks one by one.
                        squash_intervals.push(SquashBlockRange {
                            range: Range {
                                start: right_index,
                                end: right_index,
                            },
                            gc_block: false,
                        });
                    }
                }
                _ => { /* cannot squash incompatible types */ }
            }
        }

        for squash_range in &squash_intervals {
            let start_idx = squash_range.range.start;
            let end_idx = squash_range.range.end;
            assert!(start_idx <= end_idx);

            let (left_slice, right_slice) = self.inner.split_at_mut(end_idx);

            // The start_idx - 1 element is the one want to squash into.
            let left = unsafe { &mut *left_slice[start_idx - 1].get() };
            let right = unsafe { &*right_slice[0].get() };

            match (left, right) {
                (Block::GC(left), Block::GC(right)) => {
                    left.len = right.clock - left.clock + right.len;
                }
                (Block::Item(left), Block::Item(right)) => {
                    let left = ItemPtr::from(left);
                    let right = ItemPtr::from(right.as_ref());
                    if let Some(key) = right.parent_sub.as_deref() {
                        if let TypePtr::Branch(mut parent) = right.parent {
                            if let Some(e) = parent.map.get_mut(key) {
                                if right == *e {
                                    *e = ItemPtr::from(left);
                                }
                            }
                        }
                    }
                }
                _ => { /* cannot squash incompatible types */ }
            }

            // Finally, remove the BlockCells in bulk.
            self.inner.drain(start_idx..=end_idx);
        }
    }

    /// Attempts to squash block at a given `index` with a corresponding block on its left side.
    /// If this succeeds, block under a given `index` will be removed, and its contents will be
    /// squashed into its left neighbor. In such case a squash result will be returned in order to
    /// later on rewire left/right neighbor changes that may have occurred as a result of squashing
    /// and block removal.
    pub(crate) fn squash_left(&mut self, index: usize) {
        let (l, r) = self.inner.split_at_mut(index);
        let left = unsafe { &mut *l[index - 1].get() };
        let right = unsafe { &mut *r[0].get() };
        match (left, right) {
            (Block::GC(left), Block::GC(right)) => {
                left.len = right.clock - left.clock + right.len;
                self.inner.remove(index);
            }
            (Block::Item(left), Block::Item(right)) => {
                let mut left = ItemPtr::from(left);
                let right = ItemPtr::from(right);
                if left.try_squash(right) {
                    if let Some(key) = right.parent_sub.as_deref() {
                        if let TypePtr::Branch(mut parent) = right.parent {
                            if let Some(e) = parent.map.get_mut(key) {
                                if right == *e {
                                    *e = ItemPtr::from(left);
                                }
                            }
                        }
                    }
                    self.inner.remove(index);
                }
            }
            _ => { /* cannot squash incompatible types */ }
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
}

pub(crate) type Iter<'a> = std::collections::hash_map::Iter<'a, ClientID, ClientBlockList>;
pub(crate) type IterMut<'a> = std::collections::hash_map::IterMut<'a, ClientID, ClientBlockList>;

impl BlockStore {
    /// Checks if block store is empty. Empty block store doesn't contain any blocks, neither active
    /// nor tombstoned.
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
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
                list.push(block);
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
        let map = self
            .clients
            .iter()
            .map(|(client_id, list)| (*client_id, list.clock()))
            .collect();
        StateVector::new(map)
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
