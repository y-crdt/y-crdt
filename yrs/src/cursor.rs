use crate::block::{Block, BlockPtr, ItemContent, ItemPosition, Prelim};
use crate::types::{BranchPtr, TypePtr, Value};
use crate::{OffsetKind, Transaction, ID};
use lib0::any::Any;
use std::cell::Cell;
use std::cmp::Ordering;
use std::mem::MaybeUninit;
use std::ops::Deref;

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
    fn move_range(&mut self, txn: &mut Transaction, range: &CursorRange);
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

/// Descriptor of a continuous range of blocks.
pub struct CursorRange {
    /// Start of a cursor range.
    start: CursorRangeEdge,
    /// End of a cursor range.
    end: CursorRangeEdge,
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
}

pub struct CursorRangeEdge {
    id: ID,
    assoc: Assoc,
    ptr: Cell<BlockPtr>,
}

impl CursorRangeEdge {
    fn new(ptr: BlockPtr, assoc: Assoc) -> Self {
        let id = ptr.id().clone();
        CursorRangeEdge {
            id,
            assoc,
            ptr: Cell::new(ptr),
        }
    }

    pub fn id(&self) -> &ID {
        &self.id
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
    iter: BlockIterator,
    branch: BranchPtr,
    current_block: Option<BlockPtr>,
    current_block_offset: usize,
}

impl ArrayCursor {
    fn new(branch: BranchPtr) -> Self {
        let iter = BlockIterator::new(branch.start);
        ArrayCursor {
            branch,
            iter,
            current_block: None,
            current_block_offset: 0,
        }
    }

    fn reset(&mut self) {
        self.iter = BlockIterator::new(self.branch.start);
        self.current_block = None;
        self.current_block_offset = 0;
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

    fn move_range(&mut self, txn: &mut Transaction, range: &CursorRange) {
        todo!()
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

/// Block iterator allows to traverse over blocks (no matter of their type, countability or
/// deletion state) back and forth, with respect to potential block moves (see: move feature)
/// that may have happened.
#[derive(Debug, Clone)]
struct BlockIterator {
    current: Option<BlockPtr>,
    move_stack: Option<Box<MoveStack>>,
}

impl BlockIterator {
    fn new(start: Option<BlockPtr>) -> Self {
        BlockIterator {
            current: start,
            move_stack: None,
        }
    }

    fn move_stack_mut(&mut self) -> &mut MoveStack {
        self.move_stack
            .get_or_insert_with(|| Box::new(MoveStack::new()))
    }
}

impl Iterator for BlockIterator {
    type Item = BlockPtr;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.current.take();
        self.current = if let Some(Block::Item(item)) = current.clone().as_deref() {
            if let Some(move_target) = item.moved {
                todo!()
            }
            item.right
        } else {
            None
        };
        current
    }
}

impl DoubleEndedIterator for BlockIterator {
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

#[derive(Debug, Clone)]
struct MoveStack {
    current: Option<BlockPtr>,
    current_start: Option<BlockPtr>,
    current_end: Option<BlockPtr>,
    stack: Vec<StackItem>,
}

impl MoveStack {
    fn new() -> Self {
        MoveStack {
            stack: Vec::new(),
            current: None,
            current_start: None,
            current_end: None,
        }
    }
}

impl Default for MoveStack {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
struct StackItem {
    start: Option<BlockPtr>,
    end: Option<BlockPtr>,
    moved_to: BlockPtr,
}

impl StackItem {
    fn new(start: Option<BlockPtr>, end: Option<BlockPtr>, moved_to: BlockPtr) -> Self {
        StackItem {
            start,
            end,
            moved_to,
        }
    }
}

#[cfg(test)]
mod test {
    use crate::cursor::BlockIterator;
    use crate::{Doc, ID};

    #[test]
    fn block_iter_next() {
        let doc = Doc::with_client_id(1);
        let txt = doc.transact().get_text("text");

        txt.push(&mut doc.transact(), "hello");
        txt.push(&mut doc.transact(), " world");
        txt.insert(&mut doc.transact(), 5, " wonderful");
        txt.remove_range(&mut doc.transact(), 0, 6);

        let mut i = BlockIterator::new(txt.as_ref().start);

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
    fn block_iter_next_back() {
        let doc = Doc::with_client_id(1);
        let map = doc.transact().get_map("map");

        map.insert(&mut doc.transact(), "key", 1);
        map.insert(&mut doc.transact(), "key", 2);
        map.insert(&mut doc.transact(), "key", 3);

        let mut i = BlockIterator::new(map.as_ref().map.get("key").cloned());

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
