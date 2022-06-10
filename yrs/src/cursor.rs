use crate::block::{Block, BlockPtr, ItemContent, ItemPosition, Prelim};
use crate::types::{BranchPtr, TypePtr, Value};
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::{OffsetKind, Transaction, ID};
use lib0::any::Any;
use std::cell::Cell;
use std::collections::HashSet;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};

pub trait Cursor {
    /// Advances cursor left (if `offset` is negative) or right by a number of elements specified
    /// in `offset` parameter.
    /// Returns a number of elements traversed this way.
    fn advance(&mut self, offset: isize) -> isize {
        if offset > 0 {
            self.forward(offset as usize) as isize
        } else {
            -(self.backward((-offset) as usize) as isize)
        }
    }

    /// Tries to setup a cursor position at user-given `index`.
    /// Returns true if successfully moved to given `index` position.
    /// If sequence was too short to reach given `index`, false is returned.
    fn seek(&mut self, index: usize) -> bool;

    /// Inserts given `values` at the current cursor position. Advances the cursor to the end of
    /// inserted buffer.
    fn insert<P: Prelim>(&mut self, txn: &mut Transaction, value: P);

    /// Removes a `len` of elements, starting at the current cursor position. Advances the cursor
    /// to the end of removed range. Returns number of elements removed.
    fn remove(&mut self, txn: &mut Transaction, len: usize);

    /// Moves cursor by `offset` elements to the right.
    /// Returns a number of elements, current cursor actually moved by.
    fn forward(&mut self, offset: usize) -> usize;

    /// Moves cursor by `offset` elements to the left.
    /// Returns a number of elements, current cursor actually moved by.
    fn backward(&mut self, offset: usize) -> usize;

    /// Returns a cursor range matching a range of elements, starting at the current cursor
    /// position and ending after `len` of elements.
    fn range(&self, len: usize, start_assoc: Assoc, end_assoc: Assoc, priority: u32)
        -> CursorRange;

    /// Moves a `range` of elements into a current cursor position.
    fn move_range(&mut self, txn: &mut Transaction, range: CursorRange);
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Move {
    pub range: CursorRange,

    /// We store which Items+ContentMove we override. Once we delete
    /// this ContentMove, we need to re-integrate the overridden items.
    ///
    /// This representation can be improved if we ever run into memory issues because of too many overrides.
    /// Ideally, we should probably just re-iterate the document and re-integrate all moved items.
    /// This is fast enough and reduces memory footprint significantly.
    pub(crate) overrides: HashSet<BlockPtr>,
}

impl Move {
    fn new(range: CursorRange) -> Self {
        Move {
            range,
            overrides: HashSet::new(),
        }
    }

    pub(crate) fn delete(&self, txn: &mut Transaction, item: BlockPtr) {
        let mut start = self.range.start.block();
        let end = self.range.end.block();
        while start != end && start.is_some() {
            if let Some(Block::Item(i)) = start.as_deref_mut() {
                if i.moved == Some(item) {
                    i.moved = None;
                }
                start = i.right;
            } else {
                break;
            }
        }

        fn reintegrate(mut ptr: BlockPtr, txn: &mut Transaction) {
            let ptr_copy = ptr.clone();
            if let Block::Item(item) = ptr.deref_mut() {
                let deleted = item.is_deleted();
                if let ItemContent::Move(content) = &mut item.content {
                    if deleted {
                        // potentially we can integrate the items that reIntegrateItem overrides
                        if !content.overrides.is_empty() {
                            for &inner in content.overrides.iter() {
                                reintegrate(inner, txn);
                            }
                        }
                    } else {
                        content.integrate_block(txn, ptr_copy)
                    }
                }
            }
        }

        if !self.overrides.is_empty() {
            for &ptr in self.overrides.iter() {
                reintegrate(ptr, txn);
            }
        }
    }

    pub(crate) fn integrate_block(&mut self, txn: &mut Transaction, item: BlockPtr) {
        let mut start = self.range.start.block();
        let end = self.range.end.block();
        let mut max_priority = 0;
        let adapt_priority = self.priority < 0;
        while start != end && start.is_some() {
            let start_ptr = start.unwrap().clone();
            if let Some(Block::Item(start_item)) = start.as_deref_mut() {
                let mut curr_moved = start_item.moved;
                let next_prio = if let Some(Block::Item(m)) = curr_moved.as_deref() {
                    if let ItemContent::Move(next) = &m.content {
                        next.priority
                    } else {
                        0
                    }
                } else {
                    0
                };

                if adapt_priority
                    || next_prio < self.priority
                    || (curr_moved.is_some()
                        && next_prio == self.priority
                        && (*curr_moved.unwrap().id() < *item.id()))
                {
                    if let Some(moved_ptr) = curr_moved.clone() {
                        self.overrides.insert(moved_ptr);
                    }
                    max_priority = max_priority.max(next_prio);
                    // was already moved
                    let prev_move = start_item.moved;
                    if let Some(prev_move) = prev_move {
                        if !txn.prev_moved.contains_key(&prev_move)
                            && prev_move.id().clock < txn.before_state.get(&prev_move.id().client)
                        {
                            // only override prevMoved if the prevMoved item is not new
                            // we need to know which item previously moved an item
                            txn.prev_moved.insert(start_ptr, prev_move);
                        }
                    }
                    start_item.moved = Some(item);
                    if !start_item.is_deleted() {
                        if let ItemContent::Move(m) = &start_item.content {
                            if m.find_move_loop(start_ptr, &mut HashSet::from([item])) {
                                item.delete_as_cleanup(txn);
                                return;
                            }
                        }
                    }
                } else if let Some(Block::Item(moved_item)) = curr_moved.as_deref_mut() {
                    if let ItemContent::Move(m) = &mut moved_item.content {
                        m.overrides.insert(item);
                    }
                }
                start = start_item.right;
            } else {
                break;
            }
        }

        if adapt_priority {
            self.range.priority = max_priority + 1;
        }
    }
}

impl From<CursorRange> for Move {
    fn from(range: CursorRange) -> Self {
        Move::new(range)
    }
}

impl Deref for Move {
    type Target = CursorRange;

    fn deref(&self) -> &Self::Target {
        &self.range
    }
}

impl std::fmt::Display for Move {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "move(")?;

        std::fmt::Display::fmt(&self.range, f)?;

        if !self.overrides.is_empty() {
            write!(f, ", overrides: [")?;
            let mut i = self.overrides.iter();
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

/// Associativity of cursor ranges. Whenever cursor range defines a range of items living in a block
/// we need to think about potential concurrent blocks being inserted at the edges of that range.
/// Is such concurrent scenario happens, should range include newly created blocks?
#[repr(u8)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Assoc {
    /// With left associativity, all blocks that where concurrently inserted on the **left** side of
    /// a range edge, will be included into range.
    Left,
    /// With left associativity, all blocks that where concurrently inserted on the **right** side
    /// of a range edge, will be included into range.
    Right,
}

impl Encode for Assoc {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        match self {
            Assoc::Left => encoder.write_ivar(-1),
            Assoc::Right => encoder.write_ivar(1),
        }
    }
}

impl Decode for Assoc {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        if decoder.read_ivar() >= 0 {
            Assoc::Right
        } else {
            Assoc::Left
        }
    }
}

/// Descriptor of a continuous range of blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorRange {
    /// Start of a cursor range.
    pub start: CursorRangeEdge,
    /// End of a cursor range.
    pub end: CursorRangeEdge,
    /// Priority that this cursor range has over other concurrent ranges.
    priority: u32,
}

impl CursorRange {
    fn new(start: CursorRangeEdge, end: CursorRangeEdge, priority: u32) -> Self {
        CursorRange {
            start,
            end,
            priority,
        }
    }

    fn is_collapsed(&self) -> bool {
        self.start.id == self.end.id
    }

    fn contains(&self, id: &ID) -> bool {
        *id >= self.start.id && *id <= self.end.id
    }

    fn repair(&self, txn: &mut Transaction) {
        self.start.repair(txn);
        self.end.repair(txn);
    }

    pub(crate) fn find_move_loop(
        &self,
        moved: BlockPtr,
        tracked_moved_items: &mut HashSet<BlockPtr>,
    ) -> bool {
        if tracked_moved_items.contains(&moved) {
            true
        } else {
            tracked_moved_items.insert(moved.clone());
            let mut start = self.start.block();
            let end = self.end.block();
            while let Some(start_ptr) = start {
                match start_ptr.deref() {
                    Block::Item(item) if Some(start_ptr) != end => {
                        if !item.is_deleted() && item.moved == Some(moved) {
                            if let ItemContent::Move(m) = &item.content {
                                if m.range.find_move_loop(start_ptr, tracked_moved_items) {
                                    return true;
                                }
                            }
                        }

                        start = item.right;
                    }
                    _ => break,
                }
            }

            false
        }
    }
}

impl Encode for CursorRange {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        let is_collapsed = self.is_collapsed();
        encoder.write_u8(if is_collapsed { 1 } else { 0 });
        self.start.encode(encoder);
        if !is_collapsed {
            self.end.encode(encoder);
        }
        encoder.write_uvar(self.priority);
    }
}

impl Decode for CursorRange {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        let is_collapsed = decoder.read_u8() != 0;
        let start = CursorRangeEdge::decode(decoder);
        let end = if is_collapsed {
            let mut end = start.clone();
            end.assoc = Assoc::Left;
            end
        } else {
            CursorRangeEdge::decode(decoder)
        };
        let priority = decoder.read_uvar();
        CursorRange::new(start, end, priority)
    }
}

impl Prelim for CursorRange {
    fn into_content(self, txn: &mut Transaction) -> (ItemContent, Option<Self>) {
        self.repair(txn);
        (ItemContent::Move(self.into()), None)
    }

    fn integrate(self, txn: &mut Transaction, inner_ref: BranchPtr) {}
}

impl Into<Box<Move>> for CursorRange {
    fn into(self) -> Box<Move> {
        Box::new(Move::new(self))
    }
}

impl std::fmt::Display for CursorRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.start)?;
        if self.start != self.end {
            write!(f, "..{}", self.end)?;
        }
        if self.priority != 0 {
            write!(f, ", prio: {}", self.priority)?;
        }
        Ok(())
    }
}

/// Identifies a range boundary of a [CursorRange]. Due to concurrent updates edges of ranges need
/// to provide an extra information about the associativity bounds of an edge, which can be used to
/// include or exclude concurrently inserted blocks within the cursor range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorRangeEdge {
    /// Cursor range edge block ID. If associativity if [Assoc::Left], then ID points at the end of
    /// of a block to include. If it's [Assoc::Right], the ID points at the beginning of a block.
    id: ID,
    assoc: Assoc,
    ptr: Cell<Option<BlockPtr>>,
}

const RELATIVE_POSITION_TAG_ITEM: u8 = 0;

impl CursorRangeEdge {
    fn new(id: ID, assoc: Assoc, ptr: Option<BlockPtr>) -> Self {
        CursorRangeEdge {
            id,
            assoc,
            ptr: Cell::new(ptr),
        }
    }

    fn from_ptr(ptr: BlockPtr, assoc: Assoc) -> Self {
        let id = match assoc {
            Assoc::Right => *ptr.id(),
            Assoc::Left => ptr.last_id(),
        };
        CursorRangeEdge {
            id,
            assoc,
            ptr: Cell::new(Some(ptr)),
        }
    }

    pub fn id(&self) -> &ID {
        &self.id
    }

    pub(crate) fn block(&self) -> Option<BlockPtr> {
        self.ptr.get()
    }

    fn repair(&self, txn: &mut Transaction) {
        let ptr = if self.assoc == Assoc::Right {
            txn.store_mut().blocks.get_item_clean_start(&self.id)
        } else if let Some(Block::Item(item)) = txn
            .store_mut()
            .blocks
            .get_item_clean_end(&self.id)
            .as_deref()
        {
            item.right
        } else {
            None
        };
        self.ptr.set(ptr);
    }
}

impl Encode for CursorRangeEdge {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_u8(RELATIVE_POSITION_TAG_ITEM);
        encoder.write_uvar(self.id.client);
        encoder.write_uvar(self.id.clock);
        self.assoc.encode(encoder)
    }
}

impl Decode for CursorRangeEdge {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        let tag = decoder.read_u8();
        if tag == RELATIVE_POSITION_TAG_ITEM {
            let client = decoder.read_uvar();
            let clock = decoder.read_uvar();
            let id = ID::new(client, clock);
            let assoc = if decoder.has_content() {
                Assoc::decode(decoder)
            } else {
                Assoc::Right
            };
            Self::new(id, assoc, None)
        } else {
            panic!("CursorRangeEdge::decode - unknown tag: {}", tag);
        }
    }
}

impl std::fmt::Display for CursorRangeEdge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.assoc == Assoc::Left {
            write!(f, "<")?;
        }
        write!(f, "{}", self.id)?;
        if self.assoc == Assoc::Right {
            write!(f, ">")?;
        }
        Ok(())
    }
}

pub trait ContentReader {
    fn read_content(&mut self, content: &ItemContent, offset: usize) -> usize;
}

impl<'a> ContentReader for &'a mut [Value] {
    fn read_content(&mut self, content: &ItemContent, mut offset: usize) -> usize {
        if self.is_empty() {
            return 0;
        }
        match content {
            ItemContent::Move(_) | ItemContent::Deleted(_) | ItemContent::Format(_, _) => 0,
            ItemContent::Any(v) => {
                let mut i = 0;
                while i < self.len() && offset < v.len() {
                    self[i] = v[offset].clone().into();
                    i += 1;
                    offset += 1;
                }
                i
            }
            ItemContent::Binary(v) => {
                self[0] = Value::Any(Any::Buffer(v.clone().into_boxed_slice()));
                1
            }
            ItemContent::Doc(_, v) => {
                self[0] = Value::Any(v.as_ref().clone());
                1
            }
            ItemContent::Embed(v) => {
                self[0] = Value::Any(v.as_ref().clone());
                1
            }
            ItemContent::Type(c) => {
                let branch_ref = BranchPtr::from(c);
                self[0] = branch_ref.into();
                1
            }
            ItemContent::JSON(v) => {
                let mut i = 0;
                while i < self.len() && offset < v.len() {
                    self[i] = Value::Any(Any::String(v[offset].clone().into_boxed_str()));
                    i += 1;
                    offset += 1;
                }
                i
            }
            ItemContent::String(v) => {
                let mut iter = v.chars().skip(offset);
                let mut i = 0;
                while let Some(c) = iter.next() {
                    if i < self.len() {
                        self[i] = Value::Any(Any::String(c.to_string().into_boxed_str()));
                    } else {
                        break;
                    }
                    i += 1;
                }
                i
            }
        }
    }
}

impl ContentReader for Value {
    fn read_content(&mut self, content: &ItemContent, offset: usize) -> usize {
        match content {
            ItemContent::Move(_) | ItemContent::Deleted(_) | ItemContent::Format(_, _) => 0,
            ItemContent::Any(v) => {
                if let Some(v) = v.get(offset) {
                    *self = v.clone().into();
                    1
                } else {
                    0
                }
            }
            ItemContent::Binary(v) => {
                *self = Value::Any(Any::Buffer(v.clone().into_boxed_slice()));
                1
            }
            ItemContent::Doc(_, v) => {
                *self = Value::Any(v.as_ref().clone());
                1
            }
            ItemContent::Embed(v) => {
                *self = Value::Any(v.as_ref().clone());
                1
            }
            ItemContent::Type(c) => {
                let branch_ref = BranchPtr::from(c);
                *self = branch_ref.into();
                1
            }
            ItemContent::JSON(v) => {
                if let Some(v) = v.get(offset) {
                    *self = Value::Any(Any::String(v.clone().into_boxed_str()));
                    1
                } else {
                    0
                }
            }
            ItemContent::String(v) => {
                let mut iter = v.chars().skip(offset);
                if let Some(c) = iter.next() {
                    *self = Value::Any(Any::String(c.to_string().into_boxed_str()));
                    c.len_utf16()
                } else {
                    0
                }
            }
        }
    }
}

impl ContentReader for String {
    fn read_content(&mut self, content: &ItemContent, offset: usize) -> usize {
        match content {
            ItemContent::String(chunk) => {
                let encoding = OffsetKind::Utf16;
                let len = chunk.len(encoding);
                if offset == 0 {
                    self.push_str(chunk.as_str());
                    len
                } else {
                    let (_, right) = chunk.split_at(offset, encoding);
                    self.push_str(right);
                    len - offset
                }
            }
            _ => 0,
        }
    }
}

pub struct ArrayCursor {
    iter: MoveIter,
    branch: BranchPtr,
    current_block: Option<BlockPtr>,
    current_block_offset: usize,
    index: usize,
}

impl ArrayCursor {
    pub(crate) fn new(branch: BranchPtr) -> Self {
        let iter = MoveIter::new(branch.start);
        ArrayCursor {
            branch,
            iter,
            current_block: None,
            current_block_offset: 0,
            index: 0,
        }
    }

    pub fn reset(&mut self) {
        self.iter = MoveIter::new(self.branch.start);
        self.current_block = None;
        self.current_block_offset = 0;
        self.index = 0;
    }

    /// Tries to fill the `buf` slice with cursor content, advancing cursor position to the right,
    /// until end of cursor is reached or the buffer is fully filled.
    /// Returns a number of elements read.
    pub fn read(&mut self, buf: &mut [Value]) -> usize {
        let mut read = 0;
        if let Some(Block::Item(item)) = self.current_block.as_deref() {
            if !item.is_deleted() {
                let mut slice = &mut buf[read..];
                let r = slice.read_content(&item.content, self.current_block_offset);
                read += r;
                if r == 0 || read == buf.len() {
                    // it was enough to read contents of the current block
                    self.current_block_offset += read;
                    return read;
                }
            }
        }

        loop {
            self.current_block = self.iter.next();
            self.current_block_offset = 0;
            if let Some(Block::Item(item)) = self.current_block.as_deref() {
                if !item.is_deleted() {
                    let mut slice = &mut buf[read..];
                    let r = slice.read_content(&item.content, self.current_block_offset);
                    read += r;
                    if r == 0 || read == buf.len() {
                        self.current_block_offset += read;
                        return read;
                    }
                }
            } else {
                self.current_block = None;
                break;
            }
        }
        read
    }
}

impl Cursor for ArrayCursor {
    fn seek(&mut self, index: usize) -> bool {
        self.reset();
        self.forward(index) == index
    }

    fn forward(&mut self, offset: usize) -> usize {
        let mut remaining = offset;

        if let Some(Block::Item(item)) = self.current_block.as_deref() {
            self.current_block_offset += offset;
            if self.current_block_offset < item.len() as usize {
                // we moved forward but we're still within current block
                return offset;
            }
        }

        // pass as many blocks as necessary in order to reach the offset
        loop {
            self.current_block = self.iter.next();
            self.current_block_offset = 0;
            if let Some(Block::Item(item)) = self.current_block.as_deref() {
                if !item.is_deleted() && item.is_countable() {
                    let item_len = item.len() as usize;
                    if remaining < item_len {
                        self.current_block_offset = remaining;
                        remaining = 0;
                        break;
                    } else {
                        remaining -= item_len;
                    }
                }
            } else {
                self.current_block = None;
                break;
            }
        }
        offset - remaining
    }

    fn backward(&mut self, offset: usize) -> usize {
        let mut remaining = offset;

        if let Some(Block::Item(item)) = self.current_block.as_deref() {
            if self.current_block_offset >= offset {
                // we moved back but we're still within current block
                self.current_block_offset -= offset;
                return offset;
            }
        }

        // pass as many blocks as necessary in order to reach the offset
        loop {
            self.current_block = self.iter.next_back();
            self.current_block_offset = 0;
            if let Some(Block::Item(item)) = self.current_block.as_deref() {
                if !item.is_deleted() && item.is_countable() {
                    let item_len = item.len() as usize;
                    if remaining < item_len {
                        self.current_block_offset = item_len - remaining;
                        remaining = 0;
                        break;
                    } else {
                        remaining -= item_len;
                    }
                }
            } else {
                self.current_block = None;
                break;
            }
        }
        offset - remaining
    }

    fn insert<P: Prelim>(&mut self, txn: &mut Transaction, value: P) {
        let (left, right) = if let Some(ptr) = self.current_block.clone() {
            let right = txn.store.blocks.split_block(
                ptr,
                self.current_block_offset as u32,
                OffsetKind::Utf16,
            );
            (self.current_block, right)
        } else {
            (None, None)
        };
        let pos = ItemPosition {
            parent: TypePtr::Branch(self.branch),
            left,
            right,
            index: self.current_block_offset as u32,
            current_attrs: None,
        };
        let ptr = txn.create_item(&pos, value, None);
        self.current_block = Some(ptr);
        self.current_block_offset = ptr.len() as usize;
    }

    fn range(
        &self,
        len: usize,
        start_assoc: Assoc,
        end_assoc: Assoc,
        priority: u32,
    ) -> CursorRange {
        todo!()
    }

    fn move_range(&mut self, txn: &mut Transaction, range: CursorRange) {
        todo!()
    }

    fn remove(&mut self, txn: &mut Transaction, len: usize) {
        let mut remaining = len as u32;
        while remaining > 0 {
            if let Some(mut ptr) = self.iter.next() {
                let len = ptr.len();
                if remaining >= len {
                    txn.delete(ptr);
                    remaining -= len;
                } else {
                    txn.store.blocks.split_block_inner(ptr, remaining);
                    txn.delete(ptr);
                    remaining = 0;
                }
            } else {
                break;
            }
        }

        if remaining != 0 {
            panic!("Requested removal slice exceeded length of an array.")
        }
    }
}

impl Iterator for ArrayCursor {
    type Item = Value;

    fn next(&mut self) -> Option<Self::Item> {
        let mut buf: [Value; 1] = [unsafe { MaybeUninit::uninit().assume_init() }; 1];
        if self.read(&mut buf) != 0 {
            Some(std::mem::take(&mut buf[0]))
        } else {
            None
        }
    }
}

/// Block iterator that allows to traverse over blocks (no matter of their type, countability or
/// deletion state) back and forth, with respect to potential block moves (see: move feature)
/// that may have happened.
///
/// It aims for maintaining "move transparency", meaning that moved items are iterated over using
/// their move destination positions, while blocks with [ItemContent::Move] are skipped over.
#[derive(Debug, Clone)]
struct MoveIter {
    current: Option<BlockPtr>,
    move_stack: MoveStackRef,
}

impl MoveIter {
    fn new(start: Option<BlockPtr>) -> Self {
        MoveIter {
            current: start,
            move_stack: MoveStackRef::default(),
        }
    }
}

impl Iterator for MoveIter {
    type Item = BlockPtr;

    fn next(&mut self) -> Option<Self::Item> {
        let mut current = self.current.clone();
        let mut retry = false;
        self.current = if let Some(Block::Item(item)) = current.clone().as_deref() {
            if let Some(move_destination) = item.moved {
                if Some(move_destination) != self.move_stack.current_move() {
                    // skip over this block, it was move elsewhere
                    retry = true;
                    item.right
                } else if item.right == self.move_stack.current_end() {
                    // we reached the end of current move scope, pop back and continue
                    let stack = self.move_stack.as_mut();
                    let block = stack.destination;
                    stack.pop();
                    if let Some(Block::Item(item)) = block.as_deref() {
                        item.right
                    } else {
                        None
                    }
                } else if let ItemContent::Move(moved) = &item.content {
                    // this item represents new moved pointer
                    let stack = self.move_stack.as_mut();
                    stack.descend(current.unwrap(), &moved);
                    todo!()
                } else {
                    item.right
                }
            } else {
                item.right
            }
        } else {
            None
        };
        if retry {
            self.next() // this should become tail recursive call
        } else {
            current
        }
    }
}

impl DoubleEndedIterator for MoveIter {
    fn next_back(&mut self) -> Option<Self::Item> {
        let current = self.current.take();
        self.current = if let Some(Block::Item(item)) = current.clone().as_deref() {
            item.left
        } else {
            None
        };
        current
    }
}

/// Convenient wrapper around [MoveStack] that allows to lazily initialize it when needed.
#[repr(transparent)]
#[derive(Debug, Clone, Default)]
struct MoveStackRef(Option<Box<MoveStack>>);

impl MoveStackRef {
    #[inline]
    fn is_initialized(&self) -> bool {
        self.0.is_some()
    }

    #[inline]
    fn try_get(&self) -> Option<&MoveStack> {
        self.0.as_deref()
    }

    #[inline]
    fn try_get_mut(&mut self) -> Option<&mut MoveStack> {
        self.0.as_deref_mut()
    }

    fn current_move(&self) -> Option<BlockPtr> {
        let stack = self.try_get()?;
        stack.destination
    }

    fn current_start(&self) -> Option<BlockPtr> {
        let stack = self.try_get()?;
        stack.start
    }

    fn current_end(&self) -> Option<BlockPtr> {
        let stack = self.try_get()?;
        stack.start
    }
}

impl AsMut<MoveStack> for MoveStackRef {
    fn as_mut(&mut self) -> &mut MoveStack {
        self.0.get_or_insert_with(|| Box::new(MoveStack::default()))
    }
}

#[derive(Debug, Clone, Default)]
struct MoveStack {
    destination: Option<BlockPtr>,
    start: Option<BlockPtr>,
    end: Option<BlockPtr>,
    stack: Vec<StackItem>,
}

impl MoveStack {
    fn pop(&mut self) {
        let mut start = None;
        let mut end = None;
        let mut moved = None;
        if let Some(stack_item) = self.stack.pop() {
            moved = Some(stack_item.destination);
            start = stack_item.start;
            end = stack_item.end;

            //let moved_item = stack_item.destination.as_item().unwrap();
            //if let ItemContent::Move(m) = &moved_item.content {
            //    if m.range.start.assoc == Assoc::Right && (m.range.start.within_range(start))
            //        || (m.range.end.within_range(end))
            //    {
            //        let (s, e) = m.get_moved_coords(txn);
            //        start = s;
            //        end = e;
            //    }
            //}
        }
        self.destination = moved;
        self.start = start;
        self.end = end;
    }

    fn descend(&mut self, destination: BlockPtr, m: &Move) {
        if let Some(destination) = self.destination {
            let item = StackItem::new(self.start, self.end, destination);
            self.stack.push(item);
        }

        self.destination = Some(destination);
    }
}

#[derive(Debug, Clone)]
struct StackItem {
    start: Option<BlockPtr>,
    end: Option<BlockPtr>,
    destination: BlockPtr,
}

impl StackItem {
    fn new(start: Option<BlockPtr>, end: Option<BlockPtr>, destination: BlockPtr) -> Self {
        StackItem {
            start,
            end,
            destination,
        }
    }
}

#[cfg(test)]
mod test {
    use crate::block::ItemContent;
    use crate::cursor::{Assoc, MoveIter};
    use crate::{Doc, ID};

    #[test]
    fn move_iter_next() {
        let doc = Doc::with_client_id(1);
        let txt = doc.transact().get_text("text");

        txt.push(&mut doc.transact(), "hello");
        txt.push(&mut doc.transact(), " world");
        txt.insert(&mut doc.transact(), 5, " wonderful");
        txt.remove_range(&mut doc.transact(), 0, 6);

        let mut i = MoveIter::new(txt.as_ref().start);

        let block = i.next().unwrap(); // <1#0> -> deleted: 'hello'
        assert_eq!(*block.id(), ID::new(1, 0));
        assert_eq!(block.len(), 5);
        assert!(block.is_deleted());

        let block = i.next().unwrap(); // <1#11> -> deleted: ' '
        assert_eq!(*block.id(), ID::new(1, 11));
        assert_eq!(block.len(), 1);
        assert!(block.is_deleted());

        let block = i.next().unwrap(); // <1#12> -> 'wonderful'
        assert_eq!(*block.id(), ID::new(1, 12));
        assert_eq!(block.len(), 9);
        assert!(!block.is_deleted());

        let block = i.next().unwrap(); // <1#5> -> ' world'
        assert_eq!(*block.id(), ID::new(1, 5));
        assert_eq!(block.len(), 6);
        assert!(!block.is_deleted());

        assert_eq!(i.next(), None);
    }

    #[test]
    fn move_iter_over_moved_blocks() {
        const ID: u64 = 1;
        let doc = Doc::with_client_id(ID);
        let array = doc.transact().get_array("array");
        array.insert_range(&mut doc.transact(), 0, [1, 2, 3, 4]);
        array.move_range_to(&mut doc.transact(), 2, Assoc::Right, 3, Assoc::Left, 1);

        let mut i = MoveIter::new(array.as_ref().start);

        let block = i.next().unwrap();
        let item = block.as_item().unwrap(); // <1#0> -> 1
        assert_eq!(item.id, ID::new(ID, 0));
        assert_eq!(item.content, ItemContent::Any(vec![1.into()]));

        let block = i.next().unwrap();
        let item = block.as_item().unwrap(); // <1#2> -> [3,4]
        assert_eq!(item.id, ID::new(ID, 2));
        assert_eq!(item.content, ItemContent::Any(vec![3.into(), 4.into()]));

        let block = i.next().unwrap();
        let item = block.as_item().unwrap(); // <1#1> -> 2
        assert_eq!(item.id, ID::new(ID, 1));
        assert_eq!(item.content, ItemContent::Any(vec![2.into()]));

        assert_eq!(i.next(), None);
    }

    #[test]
    fn move_iter_next_back() {
        let doc = Doc::with_client_id(1);
        let map = doc.transact().get_map("map");

        map.insert(&mut doc.transact(), "key", 1);
        map.insert(&mut doc.transact(), "key", 2);
        map.insert(&mut doc.transact(), "key", 3);

        let mut i = MoveIter::new(map.as_ref().map.get("key").cloned());

        let block = i.next_back().unwrap(); // <1#2> -> 'key' => 3
        assert_eq!(*block.id(), ID::new(1, 2));
        assert_eq!(block.len(), 1);
        assert!(!block.is_deleted());

        let block = i.next_back().unwrap(); // <1#0> -> deleted: 2 items
        assert_eq!(*block.id(), ID::new(1, 0));
        assert_eq!(block.len(), 2);
        assert!(block.is_deleted());

        assert_eq!(i.next_back(), None);
    }
}
