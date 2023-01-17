use crate::block::{Block, BlockPtr, ItemContent, Prelim, Unused};
use crate::block_iter::BlockIter;
use crate::transaction::TransactionMut;
use crate::types::{Branch, BranchPtr};
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::{ReadTxn, WriteTxn, ID};
use lib0::error::Error;
use std::collections::HashSet;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

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
        match (&self.start.context, &self.end.context) {
            (RelativePositionContext::Relative(id1), RelativePositionContext::Relative(id2)) => {
                id1.eq(id2)
            }
            _ => false,
        }
    }

    pub(crate) fn get_moved_coords_mut<T: WriteTxn>(
        &self,
        txn: &mut T,
    ) -> (Option<BlockPtr>, Option<BlockPtr>) {
        let start = if let Some(start) = self.start.item() {
            Self::get_item_ptr_mut(txn, start, self.start.assoc)
        } else {
            None
        };
        let end = if let Some(end) = self.end.item() {
            Self::get_item_ptr_mut(txn, end, self.end.assoc)
        } else {
            None
        };
        (start, end)
    }

    fn get_item_ptr_mut<T: WriteTxn>(txn: &mut T, id: &ID, assoc: Assoc) -> Option<BlockPtr> {
        let store = txn.store_mut();
        if assoc == Assoc::After {
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
        let start = if let Some(start) = self.start.item() {
            Self::get_item_ptr(txn, start, self.start.assoc)
        } else {
            None
        };
        let end = if let Some(end) = self.end.item() {
            Self::get_item_ptr(txn, end, self.end.assoc)
        } else {
            None
        };
        (start, end)
    }

    fn get_item_ptr<T: ReadTxn>(txn: &T, id: &ID, assoc: Assoc) -> Option<BlockPtr> {
        if assoc == Assoc::After {
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
            if self.start.assoc == Assoc::After {
                b |= 0b0000_0010
            }
            if self.end.assoc == Assoc::After {
                b |= 0b0000_0100
            }
            b |= self.priority << 6;
            b
        };
        encoder.write_var(flags);
        let id = self.start.item().unwrap();
        encoder.write_var(id.client);
        encoder.write_var(id.clock);
        if !is_collapsed {
            let id = self.end.item().unwrap();
            encoder.write_var(id.client);
            encoder.write_var(id.clock);
        }
    }
}

impl Decode for Move {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let flags: i32 = decoder.read_var()?;
        let is_collapsed = flags & 0b0000_0001 != 0;
        let start_assoc = if flags & 0b0000_0010 != 0 {
            Assoc::After
        } else {
            Assoc::Before
        };
        let end_assoc = if flags & 0b0000_0100 != 0 {
            Assoc::After
        } else {
            Assoc::Before
        };
        //TODO use BIT3 & BIT4 to indicate the case `null` is the start/end
        // BIT5 is reserved for future extensions
        let priority = flags >> 6;
        let start_id = ID::new(decoder.read_var()?, decoder.read_var()?);
        let end_id = if is_collapsed {
            start_id
        } else {
            ID::new(decoder.read_var()?, decoder.read_var()?)
        };
        let start = RelativePosition::new(RelativePositionContext::Relative(start_id), start_assoc);
        let end = RelativePosition::new(RelativePositionContext::Relative(end_id), end_assoc);
        Ok(Move::new(start, end, priority))
    }
}

impl Prelim for Move {
    type Return = Unused;

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

/// A relative position is based on the Yjs model and is not affected by document changes.
/// E.g. If you place a relative position before a certain character, it will always point to this character.
/// If you place a relative position at the end of a type, it will always point to the end of the type.
///
/// A numeric position is often unsuited for user selections, because it does not change when content is inserted
/// before or after.
///
/// ```Insert(0, 'x')('a|bc') = 'xa|bc'``` Where | is the relative position.
///
/// One of the properties must be defined.
///
/// Example:
///
/// ```rust
/// use yrs::{Assoc, Doc, RelativeIndex, Text, Transact};
///
/// let doc = Doc::new();
/// let txt = doc.get_or_insert_text("text");
/// let mut txn = doc.transact_mut();
/// txt.insert(&mut txn, 0, "abc"); // => 'abc'
///
/// // create relative position tracker (marked as . in the comments)
/// let pos = txt.position_at(&mut txn, 2, Assoc::After).unwrap(); // => 'ab.c'
///
/// // modify text
/// txt.insert(&mut txn, 1, "def"); // => 'adefb.c'
/// txt.remove_range(&mut txn, 4, 1); // => 'adef.c'
///
/// // get absolute position
/// let a = pos.absolute(&txn).unwrap();
/// assert_eq!(a.index, 4);
/// ```
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RelativePosition {
    context: RelativePositionContext,
    /// If true - associate to the right block. Otherwise associate to the left one.
    pub assoc: Assoc,
}

impl RelativePosition {
    #[inline]
    pub fn new(context: RelativePositionContext, assoc: Assoc) -> Self {
        RelativePosition { context, assoc }
    }

    #[inline]
    pub fn context(&self) -> &RelativePositionContext {
        &self.context
    }

    pub fn item(&self) -> Option<&ID> {
        if let RelativePositionContext::Relative(id) = &self.context {
            Some(id)
        } else {
            None
        }
    }

    pub fn absolute<T: ReadTxn>(&self, txn: &T) -> Option<AbsolutePosition> {
        let mut branch = None;
        let mut index = 0;

        match &self.context {
            RelativePositionContext::Relative(right_id) => {
                let store = txn.store();
                if store.blocks.get_state(&right_id.client) <= right_id.clock {
                    // type does not exist yet
                    return None;
                }
                let (right, diff) = store.follow_redone(right_id);
                if let Block::Item(item) = right.deref() {
                    if let Some(b) = item.parent.as_branch() {
                        branch = Some(b.clone());
                        match b.item {
                            Some(i) if i.is_deleted() => { /* do nothing */ }
                            _ => {
                                // adjust position based on left association if necessary
                                index = if item.is_deleted() || !item.is_countable() {
                                    0
                                } else if self.assoc == Assoc::After {
                                    diff
                                } else {
                                    diff + 1
                                };
                                let encoding = store.options.offset_kind;
                                let mut n = item.left;
                                while let Some(Block::Item(item)) = n.as_deref() {
                                    if !item.is_deleted() && item.is_countable() {
                                        index += item.content_len(encoding);
                                    }
                                    n = item.left;
                                }
                            }
                        }
                    }
                }
            }
            RelativePositionContext::Nested(id) => {
                let store = txn.store();
                if store.blocks.get_state(&id.client) <= id.clock {
                    // type does not exist yet
                    return None;
                }
                let (ptr, _) = store.follow_redone(id);
                let item = ptr.as_item()?; // early return if ptr is GC
                if let ItemContent::Type(b) = &item.content {
                    branch = Some(BranchPtr::from(b));
                } // else - branch remains null
            }
            RelativePositionContext::Root(name) => {
                branch = txn.store().get_type(name.clone());
                if let Some(ptr) = branch.as_ref() {
                    index = if self.assoc == Assoc::After {
                        ptr.content_len
                    } else {
                        0
                    };
                }
            }
        }

        if let Some(ptr) = branch {
            Some(AbsolutePosition::new(ptr, index, self.assoc))
        } else {
            None
        }
    }

    fn get_context<T: ReadTxn>(branch: BranchPtr, txn: &T) -> RelativePositionContext {
        if let Some(ptr) = branch.item {
            RelativePositionContext::Nested(*ptr.id())
        } else {
            let root = txn.store().get_type_key(branch).unwrap().clone();
            RelativePositionContext::Root(root)
        }
    }

    pub(crate) fn from_type_index(
        txn: &mut TransactionMut,
        branch: BranchPtr,
        mut index: u32,
        assoc: Assoc,
    ) -> Option<Self> {
        if assoc == Assoc::Before {
            if index == 0 {
                let context = Self::get_context(branch, txn);
                return Some(Self::new(context, assoc));
            }
            index -= 1;
        }

        let mut walker = BlockIter::new(branch);
        if !walker.try_forward(txn, index) {
            panic!("Block iter couldn't move forward");
        }
        if walker.finished() {
            if assoc == Assoc::Before {
                let context = if let Some(ptr) = walker.next_item() {
                    RelativePositionContext::Relative(ptr.last_id())
                } else {
                    Self::get_context(branch, txn)
                };
                Some(Self::new(context, assoc))
            } else {
                None
            }
        } else {
            let context = if let Some(ptr) = walker.next_item() {
                let mut id = ptr.id().clone();
                id.clock += walker.rel();
                RelativePositionContext::Relative(id)
            } else {
                Self::get_context(branch, txn)
            };
            Some(Self::new(context, assoc))
        }
    }

    pub(crate) fn within_range(&self, ptr: Option<BlockPtr>) -> bool {
        if self.assoc == Assoc::Before {
            return false;
        } else if let Some(Block::Item(item)) = ptr.as_deref() {
            if let Some(ptr) = item.left {
                if let Some(pos) = self.item() {
                    return ptr.last_id() != *pos;
                }
            }
            false
        } else {
            true
        }
    }
}

impl Encode for RelativePosition {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        self.context.encode(encoder);
        self.assoc.encode(encoder);
    }
}

impl Decode for RelativePosition {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let context = RelativePositionContext::decode(decoder)?;
        let assoc = Assoc::decode(decoder)?;
        Ok(Self::new(context, assoc))
    }
}

impl std::fmt::Display for RelativePosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.assoc == Assoc::Before {
            write!(f, "<")?;
        }
        if let Some(id) = self.item() {
            write!(f, "{}", id)?;
        }
        if self.assoc == Assoc::After {
            write!(f, ">")?;
        }
        Ok(())
    }
}

/// Struct describing context in which [RelativePosition] is placed. For items pointing inside of
/// the shared typed sequence it's always [RelativePosition::Relative] which refers to a block [ID]
/// found under corresponding position.
///
/// In case when a containing collection is empty, there's a no block [ID] that can be used as point
/// of reference. In that case we store either a parent collection root type name or its branch [ID]
/// instead (if collection is nested into another).
///
/// Using [ID]s guarantees that corresponding [RelativePosition] doesn't shift under incoming
/// concurrent updates.
#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum RelativePositionContext {
    /// [RelativePosition] is relative to a given block [ID]. This is what happens most of the time.
    Relative(ID),
    /// If a containing collection is a nested y-type, which is empty, this case allows us to
    /// identify that nested type.
    Nested(ID),
    /// If a containing collection is a root-level y-type, which is empty, this case allows us to
    /// identify that nested type.
    Root(Rc<str>),
}

impl Encode for RelativePositionContext {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        match self {
            RelativePositionContext::Relative(id) => {
                encoder.write_var(0);
                encoder.write_var(id.client);
                encoder.write_var(id.clock);
            }
            RelativePositionContext::Nested(id) => {
                encoder.write_var(2);
                encoder.write_var(id.client);
                encoder.write_var(id.clock);
            }
            RelativePositionContext::Root(type_name) => {
                encoder.write_var(1);
                encoder.write_string(&type_name);
            }
        }
    }
}

impl Decode for RelativePositionContext {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let tag: u8 = decoder.read_var()?;
        match tag {
            0 => {
                let client = decoder.read_var()?;
                let clock = decoder.read_var()?;
                Ok(RelativePositionContext::Relative(ID::new(client, clock)))
            }
            1 => {
                let type_name = decoder.read_string()?;
                Ok(RelativePositionContext::Root(type_name.into()))
            }
            2 => {
                let client = decoder.read_var()?;
                let clock = decoder.read_var()?;
                Ok(RelativePositionContext::Nested(ID::new(client, clock)))
            }
            _ => Err(Error::UnexpectedValue),
        }
    }
}

/// Association type used by [RelativePosition]. In general [RelativePosition] refers to a cursor
/// space between two elements (eg. "ab.c" where "abc" is our string and `.` is the [RelativePosition]
/// placement). However in a situation when another peer is updating a collection concurrently,
/// a new set of elements may be inserted into that space, expanding it in the result. In such case
/// [Assoc] tells us if the [RelativePosition] should stick to location before or after referenced index.
#[repr(i8)]
#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum Assoc {
    /// The corresponding [RelativePosition] points to space **after** the referenced [ID].
    After = 0,
    /// The corresponding [RelativePosition] points to space **before** the referenced [ID].
    Before = -1,
}

impl Default for Assoc {
    #[inline]
    fn default() -> Self {
        Assoc::After
    }
}

impl Encode for Assoc {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        match self {
            Assoc::Before => encoder.write_var(-1),
            Assoc::After => encoder.write_var(0),
        }
    }
}

impl Decode for Assoc {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let tag: i8 = decoder.read_var()?;
        Ok(if tag >= 0 {
            Assoc::After
        } else {
            Assoc::Before
        })
    }
}

/// Trait used to retrieve a [RelativePosition] corresponding to a given human-readable index.
/// Unlike standard indexes relative position enables to track the location inside of a shared
/// y-types, even in the face of concurrent updates.
pub trait RelativeIndex: AsRef<Branch> {
    /// Returns a [RelativePosition] equivalent to a human-readable `index`.
    /// Returns `None` if `index` is beyond the length of current sequence.
    fn position_at(
        &self,
        txn: &mut TransactionMut,
        index: u32,
        assoc: Assoc,
    ) -> Option<RelativePosition> {
        RelativePosition::from_type_index(txn, BranchPtr::from(self.as_ref()), index, assoc)
    }
}

/// [AbsolutePosition] is a result of mapping of [RelativePosition] onto document store at a current
/// point in time.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct AbsolutePosition {
    /// Pointer to a collection type [AbsolutePosition] refers to.
    pub branch: BranchPtr,
    /// Human readable index corresponding to this [AbsolutePosition].
    pub index: u32,
    /// Association type used by [RelativePosition] this structure was created from.
    pub assoc: Assoc,
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

#[cfg(test)]
mod test {
    use crate::moving::Assoc;
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::Encode;
    use crate::{Doc, RelativeIndex, RelativePosition, Text, TextRef, Transact};

    fn check_relative_positions(text: &TextRef) {
        // test if all positions are encoded and restored correctly
        let mut txn = text.transact_mut();
        let len = text.len(&txn);
        for i in 0..len {
            // for all types of associations..
            for assoc in [Assoc::After, Assoc::Before] {
                let rel_pos = text.position_at(&mut txn, i, assoc).unwrap();
                let encoded = rel_pos.encode_v1();
                let decoded = RelativePosition::decode_v1(&encoded).unwrap();
                let abs_pos = decoded.absolute(&txn).expect(&format!(
                    "absolute position not found for index {} of {}",
                    i, decoded
                ));
                assert_eq!(abs_pos.index, i);
                assert_eq!(abs_pos.assoc, assoc);
            }
        }
    }

    #[test]
    fn relative_position_case_1() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");

        {
            let mut txn = doc.transact_mut();
            txt.insert(&mut txn, 0, "1");
            txt.insert(&mut txn, 0, "abc");
            txt.insert(&mut txn, 0, "z");
            txt.insert(&mut txn, 0, "y");
            txt.insert(&mut txn, 0, "x");
        }

        check_relative_positions(&txt);
    }

    #[test]
    fn relative_position_case_2() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");

        txt.insert(&mut doc.transact_mut(), 0, "abc");
        check_relative_positions(&txt);
    }

    #[test]
    fn relative_position_case_3() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");

        {
            let mut txn = doc.transact_mut();
            txt.insert(&mut txn, 0, "abc");
            txt.insert(&mut txn, 0, "1");
            txt.insert(&mut txn, 0, "xyz");
        }

        check_relative_positions(&txt);
    }

    #[test]
    fn relative_position_case_4() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");

        txt.insert(&mut doc.transact_mut(), 0, "1");
        check_relative_positions(&txt);
    }

    #[test]
    fn relative_position_case_5() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");

        {
            let mut txn = doc.transact_mut();
            txt.insert(&mut txn, 0, "2");
            txt.insert(&mut txn, 0, "1");
        }

        check_relative_positions(&txt);
    }

    #[test]
    fn relative_position_case_6() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        check_relative_positions(&txt);
    }

    #[test]
    fn relative_position_association_difference() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");

        let mut txn = doc.transact_mut();
        txt.insert(&mut txn, 0, "2");
        txt.insert(&mut txn, 0, "1");

        let rpos_right = txt.position_at(&mut txn, 1, Assoc::After).unwrap();
        let rpos_left = txt.position_at(&mut txn, 1, Assoc::Before).unwrap();

        txt.insert(&mut txn, 1, "x");

        let pos_right = rpos_right.absolute(&txn).unwrap();
        let pos_left = rpos_left.absolute(&txn).unwrap();

        assert_eq!(pos_right.index, 2);
        assert_eq!(pos_left.index, 1);
    }
}
