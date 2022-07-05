use crate::block::{Block, BlockPtr, ItemContent, ItemPosition, Prelim};
use crate::store::Store;
use crate::types::{Branch, BranchPtr, TypePtr, Value};
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::{OffsetKind, Transaction, ID};
use lib0::any::Any;
use lib0::error::Error;
use std::cell::Cell;
use std::collections::HashSet;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

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
    fn range(&mut self, len: usize, start_assoc: Assoc, end_assoc: Assoc) -> CursorRange;

    /// Moves the `range` of elements into a current cursor position.
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
    pub fn new(range: CursorRange) -> Self {
        Move {
            range,
            overrides: HashSet::new(),
        }
    }

    pub(crate) fn delete(&self, txn: &mut Transaction, item: BlockPtr) {
        let mut start = self.range.start_block();
        let end = self.range.end_block();
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
        let mut start = self.range.start_block();
        let end = self.range.end_block();
        let mut max_priority = 0;
        let adapt_priority = self.priority < 0;
        while let Some(Block::Item(start_item)) = start.clone().as_deref_mut() {
            let start_ptr = start.unwrap().clone();
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

            if start == end {
                break;
            } else {
                start = start_item.right;
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
            Assoc::Left => encoder.write_var(-1),
            Assoc::Right => encoder.write_var(1),
        }
    }
}

impl Decode for Assoc {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let value: i32 = decoder.read_var()?;
        if value >= 0 {
            Ok(Assoc::Right)
        } else {
            Ok(Assoc::Left)
        }
    }
}

/// Descriptor of a continuous range of blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorRange {
    /// Start of a cursor range.
    start: Option<CursorRangeEdge>,
    /// End of a cursor range.
    end: Option<CursorRangeEdge>,
    /// Shared type that owns this branch.
    parent: TypePtr,
    /// Priority that this cursor range has over other concurrent ranges.
    priority: i32,
}

impl CursorRange {
    fn new(
        start: Option<CursorRangeEdge>,
        end: Option<CursorRangeEdge>,
        parent: TypePtr,
        priority: i32,
    ) -> Self {
        CursorRange {
            start,
            end,
            parent,
            priority,
        }
    }

    pub fn start(&self) -> Option<&CursorRangeEdge> {
        self.start.as_ref()
    }

    pub(crate) fn start_block(&self) -> Option<BlockPtr> {
        if let Some(edge) = self.start.as_ref() {
            edge.ptr.get()
        } else {
            None
        }
    }

    pub fn end(&self) -> Option<&CursorRangeEdge> {
        self.end.as_ref()
    }

    pub(crate) fn end_block(&self) -> Option<BlockPtr> {
        if let Some(edge) = self.start.as_ref() {
            edge.ptr.get()
        } else {
            None
        }
    }

    pub fn is_collapsed(&self) -> bool {
        match (self.start(), self.end()) {
            (Some(s), Some(e)) => s.id == e.id,
            _ => false,
        }
    }

    fn contains(&self, id: &ID) -> bool {
        let id = *id;
        if let Some(edge) = self.start.as_ref() {
            if id < edge.id {
                return false;
            }
        }
        if let Some(edge) = self.end.as_ref() {
            if id > edge.id {
                return false;
            }
        }

        true
    }

    pub(crate) fn repair(&mut self, store: &mut Store) {
        if let Some(edge) = self.start.as_ref() {
            edge.repair(store);
        }
        if let Some(edge) = self.end.as_ref() {
            edge.repair(store);
        }
        let parent = match std::mem::take(&mut self.parent) {
            TypePtr::Unknown => {
                // pick the parent from start/end
                let edge = self.start.as_ref().or(self.end.as_ref()).unwrap();
                if let Some(Block::Item(item)) = edge.ptr.get().as_deref() {
                    item.parent.clone()
                } else {
                    TypePtr::Unknown
                }
            }
            TypePtr::Named(name) => {
                if let Some(branch) = store.get_type(name.as_ref()) {
                    TypePtr::Branch(branch)
                } else {
                    TypePtr::Named(name)
                }
            }
            TypePtr::ID(id) => {
                if let Some(Block::Item(item)) = store.blocks.get_block(&id).as_deref() {
                    if let ItemContent::Type(branch) = &item.content {
                        TypePtr::Branch(BranchPtr::from(branch))
                    } else {
                        TypePtr::ID(id)
                    }
                } else {
                    TypePtr::ID(id)
                }
            }
            other => other,
        };
        self.parent = parent;
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
            let mut start = self.start_block();
            let end = self.end_block();
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

    fn encode_type_ptr<E: Encoder>(type_ptr: &TypePtr, store: Option<&Store>, encoder: &mut E) {
        match type_ptr {
            TypePtr::Branch(ptr) => {
                if let Some(item) = ptr.item {
                    encoder.write_u8(RELATIVE_POSITION_TAG_TYPE_ID);
                    let id = item.id();
                    encoder.write_var(id.client);
                    encoder.write_var(id.clock);
                } else {
                    encoder.write_u8(RELATIVE_POSITION_TAG_TYPE_NAME);
                    let name = store.unwrap().get_type_key(*ptr).unwrap();
                    encoder.write_string(name.as_ref());
                }
            }
            TypePtr::Named(name) => {
                encoder.write_u8(RELATIVE_POSITION_TAG_TYPE_NAME);
                encoder.write_string(name.as_ref());
            }
            TypePtr::ID(id) => {
                encoder.write_u8(RELATIVE_POSITION_TAG_TYPE_ID);
                encoder.write_var(id.client);
                encoder.write_var(id.clock);
            }
            TypePtr::Unknown => panic!("Cursor range parent is unknown"),
        }
    }

    pub(crate) fn encode<E: Encoder>(&self, encoder: &mut E, store: Option<&Store>) {
        let is_collapsed = self.is_collapsed();
        encoder.write_u8(if is_collapsed { 1 } else { 0 });
        if let Some(edge) = self.start.as_ref() {
            edge.encode(encoder);
        } else {
            Self::encode_type_ptr(&self.parent, store, encoder);
        }
        if !is_collapsed {
            if let Some(edge) = self.end.as_ref() {
                edge.encode(encoder);
            } else {
                Self::encode_type_ptr(&self.parent, store, encoder);
            }
        }
        encoder.write_var(self.priority as u32);
    }

    pub(crate) fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let is_collapsed = decoder.read_u8()? != 0;
        let mut parent = TypePtr::Unknown;
        let start = match CursorRangeEdge::decode(decoder)? {
            Ok(edge) => Some(edge),
            Err(tag) => {
                parent = match tag {
                    RELATIVE_POSITION_TAG_TYPE_ID => {
                        let id = ID::new(decoder.read_var()?, decoder.read_var()?);
                        TypePtr::ID(id)
                    }
                    RELATIVE_POSITION_TAG_TYPE_NAME => {
                        let name = decoder.read_string()?;
                        TypePtr::Named(Rc::from(name))
                    }
                    other => panic!("Unsupported relative position tag: {}", other),
                };
                None
            }
        };
        let end = if is_collapsed {
            if let Some(mut end) = start.clone() {
                end.assoc = Assoc::Left;
                Some(end)
            } else {
                None
            }
        } else {
            match CursorRangeEdge::decode(decoder)? {
                Ok(edge) => Some(edge),
                Err(tag) => {
                    match tag {
                        RELATIVE_POSITION_TAG_TYPE_ID => {
                            let id = ID::new(decoder.read_var()?, decoder.read_var()?);
                            assert_eq!(TypePtr::ID(id), parent);
                        }
                        RELATIVE_POSITION_TAG_TYPE_NAME => {
                            let name = decoder.read_string()?;
                            assert_eq!(TypePtr::Named(name.into()), parent);
                        }
                        other => panic!("Unsupported relative position tag: {}", other),
                    };
                    None
                }
            }
        };
        let priority: u32 = decoder.read_var()?;
        Ok(CursorRange::new(start, end, parent, priority as i32))
    }
}

impl Prelim for CursorRange {
    fn into_content(mut self, txn: &mut Transaction) -> (ItemContent, Option<Self>) {
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
        if let Some(edge) = self.start.as_ref() {
            write!(f, "{}", edge)?;
        }
        if self.start != self.end {
            write!(f, "..")?;
        }
        if let Some(edge) = self.end.as_ref() {
            write!(f, "{}", edge)?;
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

/// Serialization tag used whenever [CursorEdgeRange] was given for a encoded [CursorRange].
const RELATIVE_POSITION_TAG_ITEM: u8 = 0;

/// Serialization tag used whenever [CursorEdgeRange] was absent for a encoded [CursorRange],
/// which itself refers to a root-level shared type.
const RELATIVE_POSITION_TAG_TYPE_NAME: u8 = 1;

/// Serialization tag used whenever [CursorEdgeRange] was absent for a encoded [CursorRange],
/// which itself refers to a nested shared type.
const RELATIVE_POSITION_TAG_TYPE_ID: u8 = 2;

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

    /// Repair process triggered before [Move] block containing current cursor range edge is
    /// integrated. Cursor range boundaries may be places inside of a blocks they refer to. This
    /// method splits these blocks by the given boundary.
    fn repair(&self, store: &mut Store) {
        let ptr = if self.assoc == Assoc::Right {
            store.blocks.get_item_clean_start(&self.id)
        } else if let Some(Block::Item(item)) = store.blocks.get_item_clean_end(&self.id).as_deref()
        {
            item.right
        } else {
            None
        };
        self.ptr.set(ptr);
    }

    fn decode<D: Decoder>(decoder: &mut D) -> Result<Result<Self, u8>, Error> {
        let tag = decoder.read_u8()?;
        if tag == RELATIVE_POSITION_TAG_ITEM {
            let item = Self::decode_item(decoder)?;
            Ok(Ok(item))
        } else {
            Ok(Err(tag))
        }
    }

    fn decode_item<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let client = decoder.read_var()?;
        let clock = decoder.read_var()?;
        let id = ID::new(client, clock);
        let assoc = match decoder.read_var::<i32>() {
            Ok(value) if value >= 0 => Assoc::Right,
            Ok(_value) => Assoc::Left,
            Err(Error::EndOfBuffer) => Assoc::Right,
            Err(e) => {
                return Err(e);
            }
        };
        Ok(Self::new(id, assoc, None))
    }

    fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_u8(RELATIVE_POSITION_TAG_ITEM);
        encoder.write_var(self.id.client);
        encoder.write_var(self.id.clock);
        self.assoc.encode(encoder)
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

/// Content reader trait used to read contents of a block.
pub trait ContentReader {
    /// Request to read a `content` into a current reader, starting at given position (if `content`
    /// has multiple elements).
    ///
    /// Returns a number of elements read into a current reader.
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
                    let value = v[offset].clone().into();
                    self[i] = value;
                    i += 1;
                    offset += 1;
                }
                i
            }
            ItemContent::Binary(v) => {
                self[0] = Value::Any(Any::Buffer(v.clone()));
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
                    self[i] = Value::Any(Any::String(v[offset].clone()));
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
                        self[i] = Value::Any(Any::String(c.to_string()));
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
                *self = Value::Any(Any::Buffer(v.clone()));
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
                    *self = Value::Any(Any::String(v.clone()));
                    1
                } else {
                    0
                }
            }
            ItemContent::String(v) => {
                let mut iter = v.chars().skip(offset);
                if let Some(c) = iter.next() {
                    *self = Value::Any(Any::String(c.to_string()));
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

/// Cursor used to traverse [Array] elements.
pub struct ArrayCursor {
    /// Internal move-aware iterator, used to abstract complexity of [Move] block traversals.
    iter: MoveIter,
    /// Parent branch of a current cursor - it should always be an [Array].
    branch: BranchPtr,
    /// In case when move iterator points at the block that has multiple elements, this field
    /// contains last visited index number within that block.
    current_block_offset: usize,
}

impl ArrayCursor {
    pub(crate) fn new(branch: BranchPtr) -> Self {
        let iter = MoveIter::new(branch.deref());
        let mut cursor = ArrayCursor {
            branch,
            iter,
            current_block_offset: 0,
        };
        cursor
    }

    /// Returns a last visited block pointer.
    #[inline]
    fn current(&self) -> Option<BlockPtr> {
        self.iter.current()
    }

    /// Reset current cursor position to point at the beginning of a corresponding [Array].
    pub fn reset(&mut self) {
        self.iter = MoveIter::new(self.branch.deref());
        self.current_block_offset = 0;
    }

    /// Tries to fill the `buf` slice with cursor content, advancing cursor position to the right,
    /// until end of cursor is reached or the buffer is fully filled.
    /// Returns a number of elements read.
    pub fn read(&mut self, buf: &mut [Value]) -> usize {
        let mut read = 0;
        if let Some(Block::Item(item)) = self.current().as_deref() {
            // check if current cursor position doesn't point at the end of the current_block
            // or that the current_block has been deleted
            if self.current_block_offset != item.len as usize && !item.is_deleted() {
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
            if let Some(Block::Item(item)) = self.iter.next().as_deref() {
                self.current_block_offset = 0;
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
                break;
            }
        }
        read
    }

    fn get_edge(&self, assoc: Assoc) -> Option<CursorRangeEdge> {
        let id = match assoc {
            Assoc::Right => self.right_id(),
            Assoc::Left => self.left_id(),
        };
        Some(CursorRangeEdge::new(id?, assoc, None))
    }

    fn right_id(&self) -> Option<ID> {
        let mut id = *self.current()?.id();
        id.clock += self.current_block_offset as u32;
        Some(id)
    }

    fn left_id(&self) -> Option<ID> {
        if self.current_block_offset != 0 {
            let mut id = *self.current()?.id();
            id.clock += self.current_block_offset as u32 - 1;
            Some(id)
        } else if let Some(Block::Item(item)) = self.current().as_deref() {
            if let Some(ptr) = item.left {
                Some(ptr.last_id())
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Returns block pointers for left and right blocks of the current cursor position.
    /// If cursor position is within a block, it'll be split.
    fn neighbours(&self, txn: &mut Transaction) -> (Option<BlockPtr>, Option<BlockPtr>) {
        let current = self.current();
        if let Some(ptr) = current {
            if self.current_block_offset != 0 {
                // current cursor position is inside of a multi-element block
                if self.current_block_offset as u32 == ptr.len() {
                    if let Block::Item(item) = ptr.deref() {
                        (current, item.right)
                    } else {
                        (current, None)
                    }
                } else {
                    let right = txn.store.blocks.split_block(
                        ptr,
                        self.current_block_offset as u32,
                        OffsetKind::Utf16,
                    );
                    (current, right)
                }
            } else if self.iter.finished() {
                // current cursor position points at the end of an Y-array
                (current, None)
            } else {
                // current cursor position points at the beginning of a block
                if let Block::Item(item) = ptr.deref() {
                    (item.left, current)
                } else {
                    (None, current)
                }
            }
        } else {
            // cursor existing within bounds of an empty array
            (None, None)
        }
    }
}

impl Cursor for ArrayCursor {
    fn seek(&mut self, index: usize) -> bool {
        self.reset();
        self.forward(index) == index
    }

    fn insert<P: Prelim>(&mut self, txn: &mut Transaction, value: P) {
        let (left, right) = self.neighbours(txn);
        let pos = ItemPosition {
            parent: TypePtr::Branch(self.branch),
            left,
            right,
            index: 0,
            current_attrs: None,
        };
        let ptr = txn.create_item(&pos, value, None);
        self.current_block_offset = 0;
        self.iter.next();
    }

    fn remove(&mut self, txn: &mut Transaction, len: usize) {
        let mut remaining = len as u32;

        if let Some(Block::Item(item)) = self.current().as_deref() {
            if !item.is_deleted() {
                let item_len = item.len;
                let offset = self.current_block_offset as u32;
                if offset < item_len {
                    let store = txn.store_mut();
                    let mut remove = self.current().unwrap();
                    if offset > 0 {
                        //split and keep the right part around
                        remove = store.blocks.split_block_inner(remove, offset).unwrap();
                    }

                    if offset + remaining < item_len {
                        // split and keep the left part around
                        store.blocks.split_block_inner(remove, remaining);
                    }

                    //self.current_block = Some(remove);
                    self.current_block_offset = item_len.min(remaining) as usize;
                    txn.delete(remove);

                    return;
                }
            }
        }

        while remaining > 0 {
            if let Some(mut ptr) = self.iter.next() {
                if !ptr.is_deleted() {
                    let len = ptr.len();
                    if remaining >= len {
                        txn.delete(ptr);
                        remaining -= len;
                    } else {
                        txn.store.blocks.split_block_inner(ptr, remaining);
                        txn.delete(ptr);
                        remaining = 0;
                    }
                }
            } else {
                break;
            }
        }

        if remaining != 0 {
            panic!("Requested removal slice exceeded length of an array.")
        }
    }

    fn forward(&mut self, offset: usize) -> usize {
        let mut remaining = offset;

        if let Some(Block::Item(item)) = self.current().as_deref() {
            self.current_block_offset += offset;
            if self.current_block_offset < item.len() as usize {
                // we moved forward but we're still within current block
                return offset;
            }
        }

        // pass as many blocks as necessary in order to reach the offset
        loop {
            if let Some(Block::Item(item)) = self.iter.next().as_deref() {
                self.current_block_offset = 0;
                if !item.is_deleted() && item.is_countable() {
                    let item_len = item.len() as usize;
                    if remaining < item_len {
                        // we're still inside of a block
                        self.current_block_offset = remaining;
                        remaining = 0;
                        break;
                    } else {
                        // we need to move to the next block
                        remaining -= item_len;
                    }
                }
            } else {
                break;
            }
        }
        offset - remaining
    }

    fn backward(&mut self, offset: usize) -> usize {
        let mut remaining = offset;

        if let Some(Block::Item(item)) = self.current().as_deref() {
            if self.current_block_offset >= offset {
                // we moved back but we're still within current block
                self.current_block_offset -= offset;
                return offset;
            }
        }

        // pass as many blocks as necessary in order to reach the offset
        loop {
            self.current_block_offset = 0;
            if let Some(Block::Item(item)) = self.iter.next_back().as_deref() {
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
                break;
            }
        }
        offset - remaining
    }

    fn range(&mut self, len: usize, start_assoc: Assoc, end_assoc: Assoc) -> CursorRange {
        let start = self.get_edge(start_assoc);

        self.forward(len);

        let end = self.get_edge(end_assoc);
        CursorRange::new(start, end, TypePtr::Branch(self.branch.clone()), -1)
    }

    fn move_range(&mut self, txn: &mut Transaction, mut range: CursorRange) {
        if let Some(edge) = range.start.as_mut() {
            edge.repair(txn.store_mut());
        }
        if let Some(edge) = range.end.as_mut() {
            edge.repair(txn.store_mut());
        }
        self.insert(txn, range);
    }
}

impl Iterator for ArrayCursor {
    type Item = Value;

    fn next(&mut self) -> Option<Self::Item> {
        let mut buf: [Value; 1] = [Value::default(); 1];
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
    /// Last visited block - if block list is not empty and Self::next has been called,
    /// this should never be None.
    current: Option<BlockPtr>,
    /// next block to be visited
    next: Option<BlockPtr>,
    /// When true, this iterator points at the end of a block list.
    finished: bool,
    /// Stack reference - it remains uninitialized until a [Move] block has been detected.
    move_stack: MoveStackRef,
}

impl MoveIter {
    fn new(branch: &Branch) -> Self {
        MoveIter {
            current: None,
            next: branch.start,
            finished: false,
            move_stack: MoveStackRef::default(),
        }
    }

    #[inline]
    fn current(&self) -> Option<BlockPtr> {
        self.current
    }

    #[inline]
    fn peek(&self) -> Option<BlockPtr> {
        self.next
    }

    #[inline]
    fn finished(&self) -> bool {
        self.finished
    }
}

impl Iterator for MoveIter {
    type Item = BlockPtr;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(ptr) = self.next {
            self.current = self.next;
            let mut retry = false;
            self.next = if let Block::Item(item) = ptr.deref() {
                if let Some(move_destination) = item.moved {
                    // this block has been moved somewhere else
                    if Some(move_destination) != self.move_stack.current_move() {
                        // skip over this block, it's not within current move frame
                        retry = true;
                        item.right
                    } else if self.next == self.move_stack.current_end() {
                        // we reached the end of current move scope, pop move stack and continue
                        let stack = self.move_stack.as_mut();
                        let block = stack.pop();
                        if let Some(Block::Item(item)) = block.as_deref() {
                            item.right
                        } else {
                            None
                        }
                    } else {
                        item.right
                    }
                } else if let ItemContent::Move(moved) = &item.content {
                    // this item represents a new moved range frame, put it onto move stack
                    let stack = self.move_stack.as_mut();
                    retry = true;
                    stack.descend(ptr, &moved.range);
                    moved
                        .range
                        .start_block()
                        .or_else(|| moved.range.parent.as_branch().and_then(|b| b.start))
                } else if item.right.is_some() {
                    item.right
                } else {
                    None
                }
            } else {
                None
            };

            if retry {
                self.next()
            } else {
                self.current
            }
        } else if let Some(stack) = self.move_stack.0.as_deref_mut() {
            if let Some(Block::Item(item)) = stack.pop().as_deref() {
                // self.next was empty but we still had items on move stack - retry
                self.next = item.right;
                self.next()
            } else {
                // move stack is empty and self.next is empty
                self.finished = true;
                None
            }
        } else {
            self.finished = true;
            None
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

    /// Returns a current [Move] block that points a destination where items within current
    /// move frame should be moved.
    fn current_move(&self) -> Option<BlockPtr> {
        let stack = self.try_get()?;
        stack.destination
    }

    /// Returns the beginning of a moved range of blocks. It's inclusive.
    fn current_start(&self) -> Option<BlockPtr> {
        let stack = self.try_get()?;
        stack.start
    }

    /// Returns the end of a moved range of blocks. It's inclusive.
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
    /// A destination location of blocks within currently processed stack frame range.
    destination: Option<BlockPtr>,
    /// An inclusive beginning of a block range of currently processed stack frame.
    start: Option<BlockPtr>,
    /// An inclusive end of a block range of currently processed stack frame.
    end: Option<BlockPtr>,

    /// In case of multiple recursive moves, subsequent moves are pushed onto this stack.
    stack: Vec<MoveFrame>,
}

impl MoveStack {
    /// Removes currently processed move stack frame and returns its destination block.
    fn pop(&mut self) -> Option<BlockPtr> {
        let mut start = None;
        let mut end = None;
        let mut moved = None;
        let result = self.destination;
        if let Some(stack_item) = self.stack.pop() {
            moved = Some(stack_item.destination);
            start = stack_item.start;
            end = stack_item.end;
        }
        self.destination = moved;
        self.start = start;
        self.end = end;
        result
    }

    /// Promotes current `destination` block and move `range` as currently processed ones.
    /// If prior this call another range was being processed, it's being stacked.
    fn descend(&mut self, destination: BlockPtr, range: &CursorRange) {
        if let Some(destination) = self.destination {
            let item = MoveFrame::new(self.start, self.end, destination);
            self.stack.push(item);
        }

        self.destination = Some(destination);
        self.start = range.start_block();
        self.end = range.end_block();
    }
}

#[derive(Debug, Clone)]
struct MoveFrame {
    start: Option<BlockPtr>,
    end: Option<BlockPtr>,
    destination: BlockPtr,
}

impl MoveFrame {
    fn new(start: Option<BlockPtr>, end: Option<BlockPtr>, destination: BlockPtr) -> Self {
        MoveFrame {
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

        let mut i = MoveIter::new(txt.as_ref());

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

        let mut i = MoveIter::new(array.as_ref());

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
}
