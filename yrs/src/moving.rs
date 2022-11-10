use crate::block::{Block, BlockPtr, ItemContent, Prelim};
use crate::block_iter::BlockIter;
use crate::transaction::TransactionMut;
use crate::types::BranchPtr;
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::{ReadTxn, WriteTxn, ID};
use lib0::error::Error;
use std::collections::HashSet;
use std::ops::{Deref, DerefMut};

/// Association type. If true, associate with right block. Otherwise with the left one.
pub type Assoc = bool;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Move {
    pub start: RelativePosition,
    pub end: RelativePosition,
    pub priority: i32,

    /// We store which Items+ContentMove we override. Once we delete
    /// this ContentMove, we need to re-integrate the overridden items.
    ///
    /// This representation can be improved if we ever run into memory issues because of too many overrides.
    /// Ideally, we should probably just re-iterate the document and re-integrate all moved items.
    /// This is fast enough and reduces memory footprint significantly.
    pub(crate) overrides: Option<HashSet<BlockPtr>>,
}

impl Move {
    pub fn new(start: RelativePosition, end: RelativePosition, priority: i32) -> Self {
        Move {
            start,
            end,
            priority,
            overrides: None,
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.start.id == self.end.id
    }

    pub(crate) fn get_moved_coords_mut<T: WriteTxn>(
        &self,
        txn: &mut T,
    ) -> (Option<BlockPtr>, Option<BlockPtr>) {
        let start = Self::get_item_ptr_mut(txn, &self.start.id, self.start.assoc);
        let end = Self::get_item_ptr_mut(txn, &self.end.id, self.end.assoc);
        (start, end)
    }

    fn get_item_ptr_mut<T: WriteTxn>(txn: &mut T, id: &ID, assoc: Assoc) -> Option<BlockPtr> {
        let store = txn.store_mut();
        if assoc {
            let slice = store.blocks.get_item_clean_start(id)?;
            if slice.adjacent() {
                Some(slice.as_ptr())
            } else {
                Some(store.materialize(slice))
            }
        } else {
            let slice = store.blocks.get_item_clean_end(id)?;
            let ptr = if slice.adjacent() {
                slice.as_ptr()
            } else {
                store.materialize(slice)
            };
            if let Block::Item(item) = ptr.deref() {
                item.right
            } else {
                None
            }
        }
    }

    pub(crate) fn get_moved_coords<T: ReadTxn>(
        &self,
        txn: &T,
    ) -> (Option<BlockPtr>, Option<BlockPtr>) {
        let start = Self::get_item_ptr(txn, &self.start.id, self.start.assoc);
        let end = Self::get_item_ptr(txn, &self.end.id, self.end.assoc);
        (start, end)
    }

    fn get_item_ptr<T: ReadTxn>(txn: &T, id: &ID, assoc: Assoc) -> Option<BlockPtr> {
        if assoc {
            let slice = txn.store().blocks.get_item_clean_start(id)?;
            debug_assert!(slice.adjacent()); //TODO: remove once confirmed that slice always fits block range
            Some(slice.as_ptr())
        } else {
            let slice = txn.store().blocks.get_item_clean_end(id)?;
            debug_assert!(slice.adjacent()); //TODO: remove once confirmed that slice always fits block range
            if let Block::Item(item) = slice.as_ptr().deref() {
                item.right
            } else {
                None
            }
        }
    }

    pub(crate) fn find_move_loop<T: ReadTxn>(
        &self,
        txn: &mut T,
        moved: BlockPtr,
        tracked_moved_items: &mut HashSet<BlockPtr>,
    ) -> bool {
        if tracked_moved_items.contains(&moved) {
            true
        } else {
            tracked_moved_items.insert(moved.clone());
            let (mut start, end) = self.get_moved_coords(txn);
            while let Some(Block::Item(item)) = start.as_deref() {
                if start == end {
                    break;
                }

                if !item.is_deleted() && item.moved == Some(moved) {
                    if let ItemContent::Move(m) = &item.content {
                        if m.find_move_loop(txn, start.unwrap(), tracked_moved_items) {
                            return true;
                        }
                    }
                }

                start = item.right;
            }

            false
        }
    }

    fn push_override(&mut self, ptr: BlockPtr) {
        let e = self.overrides.get_or_insert_with(HashSet::default);
        e.insert(ptr);
    }

    pub(crate) fn integrate_block(&mut self, txn: &mut TransactionMut, item: BlockPtr) {
        let (init, end) = self.get_moved_coords_mut(txn);
        let mut max_priority = 0i32;
        let adapt_priority = self.priority < 0;
        let mut start = init;
        while start != end && start.is_some() {
            let start_ptr = start.unwrap().clone();
            if let Some(Block::Item(start_item)) = start.as_deref_mut() {
                let mut prev_move = start_item.moved;
                let next_prio = if let Some(Block::Item(m)) = prev_move.as_deref() {
                    if let ItemContent::Move(next) = &m.content {
                        next.priority
                    } else {
                        -1
                    }
                } else {
                    -1
                };

                #[inline]
                fn is_lower(a: &ID, b: &ID) -> bool {
                    a.client < b.client || (a.client == b.client && a.clock < b.clock)
                }

                if adapt_priority
                    || next_prio < self.priority
                    || (prev_move.is_some()
                        && next_prio == self.priority
                        && is_lower(prev_move.unwrap().id(), item.id()))
                {
                    if let Some(moved_ptr) = prev_move.clone() {
                        if let Block::Item(item) = moved_ptr.deref() {
                            if let ItemContent::Move(m) = &item.content {
                                if m.is_collapsed() {
                                    moved_ptr.delete_as_cleanup(txn, adapt_priority);
                                }
                            }
                        }
                        self.push_override(moved_ptr);
                        if Some(start_ptr) != init {
                            // only add this to mergeStructs if this is not the first item
                            txn.merge_blocks.push(start_item.id);
                        }
                    }
                    max_priority = max_priority.max(next_prio);
                    // was already moved
                    let prev_move = start_item.moved;
                    if let Some(prev_move) = prev_move {
                        if !txn.prev_moved.contains_key(&prev_move) && txn.has_added(prev_move.id())
                        {
                            // only override prevMoved if the prevMoved item is not new
                            // we need to know which item previously moved an item
                            txn.prev_moved.insert(start_ptr, prev_move);
                        }
                    }
                    start_item.moved = Some(item);
                    if !start_item.is_deleted() {
                        if let ItemContent::Move(m) = &start_item.content {
                            if m.find_move_loop(txn, start_ptr, &mut HashSet::from([item])) {
                                item.delete_as_cleanup(txn, adapt_priority);
                                return;
                            }
                        }
                    }
                } else if let Some(Block::Item(moved_item)) = prev_move.as_deref_mut() {
                    if let ItemContent::Move(m) = &mut moved_item.content {
                        m.push_override(item);
                    }
                }
                start = start_item.right;
            } else {
                break;
            }
        }

        if adapt_priority {
            self.priority = max_priority + 1;
        }
    }

    pub(crate) fn delete(&self, txn: &mut TransactionMut, item: BlockPtr) {
        let (mut start, end) = self.get_moved_coords(txn);
        while start != end && start.is_some() {
            if let Some(start_ptr) = start {
                if let Block::Item(i) = start_ptr.clone().deref_mut() {
                    if i.moved == Some(item) {
                        if let Some(&prev_moved) = txn.prev_moved.get(&start_ptr) {
                            if txn.has_added(item.id()) {
                                if prev_moved == item {
                                    // Edge case: Item has been moved by this move op and it has been created & deleted in the same transaction (hence no effect that should be emitted by the change computation)
                                    txn.prev_moved.remove(&start_ptr);
                                }
                            }
                        } else {
                            // Normal case: item has been moved by this move and it has not been created & deleted in the same transaction
                            txn.prev_moved.insert(start_ptr, item);
                        }
                        i.moved = None;
                    }
                    start = i.right;
                    continue;
                }
            }
            break;
        }

        fn reintegrate(mut ptr: BlockPtr, txn: &mut TransactionMut) {
            let ptr_copy = ptr.clone();
            if let Block::Item(item) = ptr.deref_mut() {
                let deleted = item.is_deleted();
                if let ItemContent::Move(content) = &mut item.content {
                    if deleted {
                        // potentially we can integrate the items that reIntegrateItem overrides
                        if let Some(overrides) = &content.overrides {
                            for &inner in overrides.iter() {
                                reintegrate(inner, txn);
                            }
                        }
                    } else {
                        content.integrate_block(txn, ptr_copy)
                    }
                }
            }
        }

        if let Some(overrides) = &self.overrides {
            for &ptr in overrides {
                reintegrate(ptr, txn);
            }
        }
    }
}

impl Encode for Move {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        let is_collapsed = self.is_collapsed();
        let flags = {
            let mut b = 0;
            if is_collapsed {
                b |= 0b0000_0001
            }
            if self.start.assoc {
                b |= 0b0000_0010
            }
            if self.end.assoc {
                b |= 0b0000_0100
            }
            b |= self.priority << 6;
            b
        };
        encoder.write_var(flags);
        encoder.write_var(self.start.id.client);
        encoder.write_var(self.start.id.clock);
        if !is_collapsed {
            encoder.write_var(self.end.id.client);
            encoder.write_var(self.end.id.clock);
        }
    }
}

impl Decode for Move {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let flags: i32 = decoder.read_var()?;
        let is_collapsed = flags & 0b0000_0001 != 0;
        let start_assoc = flags & 0b0000_0010 != 0;
        let end_assoc = flags & 0b0000_0100 != 0;
        //TODO use BIT3 & BIT4 to indicate the case `null` is the start/end
        // BIT5 is reserved for future extensions
        let priority = flags >> 6;
        let start_id = ID::new(decoder.read_var()?, decoder.read_var()?);
        let end_id = if is_collapsed {
            start_id
        } else {
            ID::new(decoder.read_var()?, decoder.read_var()?)
        };
        let start = RelativePosition::create(start_id, start_assoc);
        let end = RelativePosition::create(end_id, end_assoc);
        Ok(Move::new(start, end, priority))
    }
}

impl Prelim for Move {
    #[inline]
    fn into_content(self, _: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        (ItemContent::Move(Box::new(self)), None)
    }

    #[inline]
    fn integrate(self, _: &mut TransactionMut, _inner_ref: BranchPtr) {}
}

impl std::fmt::Display for Move {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "move(")?;
        write!(f, "{}", self.start)?;
        if self.start != self.end {
            write!(f, "..{}", self.end)?;
        }
        if self.priority != 0 {
            write!(f, ", prio: {}", self.priority)?;
        }
        if let Some(overrides) = self.overrides.as_ref() {
            write!(f, ", overrides: [")?;
            let mut i = overrides.iter();
            if let Some(b) = i.next() {
                write!(f, "{}", b.id())?;
            }
            while let Some(b) = i.next() {
                write!(f, ", {}", b.id())?;
            }
            write!(f, "]")?;
        }
        write!(f, ")")
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct RelativePosition {
    pub id: ID,
    /// If true - associate to the right block. Otherwise associate to the left one.
    pub assoc: Assoc,
}

impl RelativePosition {
    pub(crate) fn create(id: ID, assoc: Assoc) -> Self {
        RelativePosition { id, assoc }
    }

    pub(crate) fn from_type_index(
        txn: &mut TransactionMut,
        branch: BranchPtr,
        mut index: u32,
        assoc: Assoc,
    ) -> Option<Self> {
        if !assoc {
            if index == 0 {
                return None;
            }
            index -= 1;
        }

        let mut walker = BlockIter::new(branch);
        if !walker.try_forward(txn, index) {
            panic!("Block iter couldn't move forward");
        }
        if walker.finished() {
            if !assoc {
                let ptr = walker.next_item()?;
                let id = ptr.last_id();
                Some(Self::create(id, assoc))
            } else {
                None
            }
        } else {
            let ptr = walker.next_item()?;
            let mut id = ptr.id().clone();
            id.clock += walker.rel();
            Some(Self::create(id, assoc))
        }
    }

    pub(crate) fn within_range(&self, ptr: Option<BlockPtr>) -> bool {
        if !self.assoc {
            return false;
        } else if let Some(Block::Item(item)) = ptr.as_deref() {
            match item.left {
                Some(ptr) => ptr.last_id() != self.id,
                None => false,
            }
        } else {
            true
        }
    }
}

impl std::fmt::Display for RelativePosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.assoc {
            write!(f, "<")?;
        }
        write!(f, "{}", self.id)?;
        if self.assoc {
            write!(f, ">")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AbsolutePosition {
    branch: BranchPtr,
    index: u32,
    assoc: Assoc,
}

impl AbsolutePosition {
    fn new(branch: BranchPtr, index: u32, assoc: Assoc) -> Self {
        AbsolutePosition {
            branch,
            index,
            assoc,
        }
    }
}
