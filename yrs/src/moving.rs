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
    pub start: StickyIndex,
    pub end: StickyIndex,
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
    pub fn new(start: StickyIndex, end: StickyIndex, priority: i32) -> Self {
        Move {
            start,
            end,
            priority,
            overrides: None,
        }
    }

    pub fn is_collapsed(&self) -> bool {
        match (&self.start.scope, &self.end.scope) {
            (IndexScope::Relative(id1), IndexScope::Relative(id2)) => id1.eq(id2),
            _ => false,
        }
    }

    pub(crate) fn get_moved_coords_mut<T: WriteTxn>(
        &self,
        txn: &mut T,
    ) -> (Option<BlockPtr>, Option<BlockPtr>) {
        let start = if let Some(start) = self.start.id() {
            Self::get_item_ptr_mut(txn, start, self.start.assoc)
        } else {
            None
        };
        let end = if let Some(end) = self.end.id() {
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
        let start = if let Some(start) = self.start.id() {
            Self::get_item_ptr(txn, start, self.start.assoc)
        } else {
            None
        };
        let end = if let Some(end) = self.end.id() {
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
    fn encode<E: Encoder>(&self, encoder: &mut E) -> Result<(), Error> {
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
        let id = self.start.id().unwrap();
        encoder.write_var(id.client);
        encoder.write_var(id.clock);
        if !is_collapsed {
            let id = self.end.id().unwrap();
            encoder.write_var(id.client);
            encoder.write_var(id.clock);
        }

        Ok(())
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
        let start = StickyIndex::new(IndexScope::Relative(start_id), start_assoc);
        let end = StickyIndex::new(IndexScope::Relative(end_id), end_assoc);
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

/// A sticky index is based on the Yjs model and is not affected by document changes.
/// E.g. If you place a sticky index before a certain character, it will always point to this character.
/// If you place a sticky index at the end of a type, it will always point to the end of the type.
///
/// A numeric position is often unsuited for user selections, because it does not change when content is inserted
/// before or after.
///
/// ```Insert(0, 'x')('a.bc') = 'xa.bc'``` Where `.` is the relative position.
///
/// Example:
///
/// ```rust
/// use yrs::{Assoc, Doc, IndexedSequence, Text, Transact};
///
/// let doc = Doc::new();
/// let txt = doc.get_or_insert_text("text");
/// let mut txn = doc.transact_mut();
/// txt.insert(&mut txn, 0, "abc"); // => 'abc'
///
/// // create position tracker (marked as . in the comments)
/// let pos = txt.sticky_index(&mut txn, 2, Assoc::After).unwrap(); // => 'ab.c'
///
/// // modify text
/// txt.insert(&mut txn, 1, "def"); // => 'adefb.c'
/// txt.remove_range(&mut txn, 4, 1); // => 'adef.c'
///
/// // get current offset index within the containing collection
/// let a = pos.get_offset(&txn).unwrap();
/// assert_eq!(a.index, 4);
/// ```
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct StickyIndex {
    scope: IndexScope,
    /// If true - associate to the right block. Otherwise associate to the left one.
    pub assoc: Assoc,
}

impl StickyIndex {
    #[inline]
    pub fn new(scope: IndexScope, assoc: Assoc) -> Self {
        StickyIndex { scope, assoc }
    }

    #[inline]
    pub fn scope(&self) -> &IndexScope {
        &self.scope
    }

    /// Returns an [ID] of the block position which is used as a reference to keep track the location
    /// of current [StickyIndex] even in face of changes performed by different peers.
    ///
    /// Returns `None` if current [StickyIndex] has been created on an empty shared collection (in
    /// that case there's no block that we can refer to).
    pub fn id(&self) -> Option<&ID> {
        if let IndexScope::Relative(id) = &self.scope {
            Some(id)
        } else {
            None
        }
    }

    /// Maps current [StickyIndex] onto [Offset] which points to shared collection and a
    /// human-readable index in that collection.
    ///
    /// That index is only valid at the current point in time - if i.e. another update from remote
    /// peer has been applied, it may have changed relative index position that [StickyIndex] points
    /// to, so that [Offset]'s index will no longer point to the same place.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use yrs::{Assoc, Doc, IndexedSequence, Text, Transact};
    ///
    /// let doc = Doc::new();
    /// let text = doc.get_or_insert_text("text");
    /// let mut txn = doc.transact_mut();
    ///
    /// text.insert(&mut txn, 0, "hello world");
    ///
    /// const INDEX: u32 = 4;
    ///
    /// // set perma index at position before letter 'o' => "hell.o world"
    /// let pos = text.sticky_index(&mut txn, INDEX, Assoc::After).unwrap();
    /// let off = pos.get_offset(&txn).unwrap();
    /// assert_eq!(off.index, INDEX);
    ///
    /// // perma index will maintain it's position before letter 'o' even if another update
    /// // shifted it's index inside of the text
    /// text.insert(&mut txn, 1, "(see)"); // => "h(see)ell.o world" where . is perma index position
    /// let off2 = pos.get_offset(&txn).unwrap();
    /// assert_ne!(off2.index, off.index); // offset index changed due to new insert above
    /// ```
    pub fn get_offset<T: ReadTxn>(&self, txn: &T) -> Option<Offset> {
        let mut branch = None;
        let mut index = 0;

        match &self.scope {
            IndexScope::Relative(right_id) => {
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
            IndexScope::Nested(id) => {
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
            IndexScope::Root(name) => {
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
            Some(Offset::new(ptr, index, self.assoc))
        } else {
            None
        }
    }

    fn get_context<T: ReadTxn>(branch: BranchPtr, txn: &T) -> IndexScope {
        if let Some(ptr) = branch.item {
            IndexScope::Nested(*ptr.id())
        } else {
            let root = txn.store().get_type_key(branch).unwrap().clone();
            IndexScope::Root(root)
        }
    }

    pub fn at(
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
                    IndexScope::Relative(ptr.last_id())
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
                IndexScope::Relative(id)
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
                if let Some(pos) = self.id() {
                    return ptr.last_id() != *pos;
                }
            }
            false
        } else {
            true
        }
    }
}

impl Encode for StickyIndex {
    fn encode<E: Encoder>(&self, encoder: &mut E) -> Result<(), Error> {
        self.scope.encode(encoder)?;
        self.assoc.encode(encoder)?;
        Ok(())
    }
}

impl Decode for StickyIndex {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let context = IndexScope::decode(decoder)?;
        let assoc = Assoc::decode(decoder)?;
        Ok(Self::new(context, assoc))
    }
}

impl std::fmt::Display for StickyIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.assoc == Assoc::Before {
            write!(f, "<")?;
        }
        if let Some(id) = self.id() {
            write!(f, "{}", id)?;
        }
        if self.assoc == Assoc::After {
            write!(f, ">")?;
        }
        Ok(())
    }
}

/// Struct describing context in which [StickyIndex] is placed. For items pointing inside of
/// the shared typed sequence it's always [StickyIndex::Relative] which refers to a block [ID]
/// found under corresponding position.
///
/// In case when a containing collection is empty, there's a no block [ID] that can be used as point
/// of reference. In that case we store either a parent collection root type name or its branch [ID]
/// instead (if collection is nested into another).
///
/// Using [ID]s guarantees that corresponding [StickyIndex] doesn't shift under incoming
/// concurrent updates.
#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum IndexScope {
    /// [StickyIndex] is relative to a given block [ID]. This happens whenever we set [StickyIndex]
    /// somewhere inside of the non-empty shared collection.
    Relative(ID),
    /// If a containing collection is a nested y-type, which is empty, this case allows us to
    /// identify that nested type.
    Nested(ID),
    /// If a containing collection is a root-level y-type, which is empty, this case allows us to
    /// identify that nested type.
    Root(Rc<str>),
}

impl Encode for IndexScope {
    fn encode<E: Encoder>(&self, encoder: &mut E) -> Result<(), Error> {
        match self {
            IndexScope::Relative(id) => {
                encoder.write_var(0);
                encoder.write_var(id.client);
                encoder.write_var(id.clock);
            }
            IndexScope::Nested(id) => {
                encoder.write_var(2);
                encoder.write_var(id.client);
                encoder.write_var(id.clock);
            }
            IndexScope::Root(type_name) => {
                encoder.write_var(1);
                encoder.write_string(&type_name);
            }
        }

        Ok(())
    }
}

impl Decode for IndexScope {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let tag: u8 = decoder.read_var()?;
        match tag {
            0 => {
                let client = decoder.read_var()?;
                let clock = decoder.read_var()?;
                Ok(IndexScope::Relative(ID::new(client, clock)))
            }
            1 => {
                let type_name = decoder.read_string()?;
                Ok(IndexScope::Root(type_name.into()))
            }
            2 => {
                let client = decoder.read_var()?;
                let clock = decoder.read_var()?;
                Ok(IndexScope::Nested(ID::new(client, clock)))
            }
            _ => Err(Error::UnexpectedValue),
        }
    }
}

/// Association type used by [StickyIndex]. In general [StickyIndex] refers to a cursor
/// space between two elements (eg. "ab.c" where "abc" is our string and `.` is the [StickyIndex]
/// placement). However in a situation when another peer is updating a collection concurrently,
/// a new set of elements may be inserted into that space, expanding it in the result. In such case
/// [Assoc] tells us if the [StickyIndex] should stick to location before or after referenced index.
#[repr(i8)]
#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum Assoc {
    /// The corresponding [StickyIndex] points to space **after** the referenced [ID].
    After = 0,
    /// The corresponding [StickyIndex] points to space **before** the referenced [ID].
    Before = -1,
}

impl Default for Assoc {
    #[inline]
    fn default() -> Self {
        Assoc::After
    }
}

impl Encode for Assoc {
    fn encode<E: Encoder>(&self, encoder: &mut E) -> Result<(), Error> {
        match self {
            Assoc::Before => encoder.write_var(-1),
            Assoc::After => encoder.write_var(0),
        }
        Ok(())
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

/// Trait used to retrieve a [StickyIndex] corresponding to a given human-readable index.
/// Unlike standard indexes [StickyIndex] enables to track the location inside of a shared
/// y-types, even in the face of concurrent updates.
pub trait IndexedSequence: AsRef<Branch> {
    /// Returns a [StickyIndex] equivalent to a human-readable `index`.
    /// Returns `None` if `index` is beyond the length of current sequence.
    fn sticky_index(
        &self,
        txn: &mut TransactionMut,
        index: u32,
        assoc: Assoc,
    ) -> Option<StickyIndex> {
        StickyIndex::at(txn, BranchPtr::from(self.as_ref()), index, assoc)
    }
}

/// [Offset] is a result of mapping of [StickyIndex] onto document store at a current
/// point in time.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Offset {
    /// Pointer to a collection type [Offset] refers to.
    pub branch: BranchPtr,
    /// Human readable index corresponding to this [Offset].
    pub index: u32,
    /// Association type used by [StickyIndex] this structure was created from.
    pub assoc: Assoc,
}

impl Offset {
    fn new(branch: BranchPtr, index: u32, assoc: Assoc) -> Self {
        Offset {
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
    use crate::{Doc, IndexedSequence, StickyIndex, Text, TextRef, Transact};

    fn check_sticky_indexes(text: &TextRef) {
        // test if all positions are encoded and restored correctly
        let mut txn = text.transact_mut();
        let len = text.len(&txn);
        for i in 0..len {
            // for all types of associations..
            for assoc in [Assoc::After, Assoc::Before] {
                let rel_pos = text.sticky_index(&mut txn, i, assoc).unwrap();
                let encoded = rel_pos.encode_v1().unwrap();
                let decoded = StickyIndex::decode_v1(&encoded).unwrap();
                let abs_pos = decoded
                    .get_offset(&txn)
                    .expect(&format!("offset not found for index {} of {}", i, decoded));
                assert_eq!(abs_pos.index, i);
                assert_eq!(abs_pos.assoc, assoc);
            }
        }
    }

    #[test]
    fn sticky_index_case_1() {
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

        check_sticky_indexes(&txt);
    }

    #[test]
    fn sticky_index_case_2() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");

        txt.insert(&mut doc.transact_mut(), 0, "abc");
        check_sticky_indexes(&txt);
    }

    #[test]
    fn sticky_index_case_3() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");

        {
            let mut txn = doc.transact_mut();
            txt.insert(&mut txn, 0, "abc");
            txt.insert(&mut txn, 0, "1");
            txt.insert(&mut txn, 0, "xyz");
        }

        check_sticky_indexes(&txt);
    }

    #[test]
    fn sticky_index_case_4() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");

        txt.insert(&mut doc.transact_mut(), 0, "1");
        check_sticky_indexes(&txt);
    }

    #[test]
    fn sticky_index_case_5() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");

        {
            let mut txn = doc.transact_mut();
            txt.insert(&mut txn, 0, "2");
            txt.insert(&mut txn, 0, "1");
        }

        check_sticky_indexes(&txt);
    }

    #[test]
    fn sticky_index_case_6() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        check_sticky_indexes(&txt);
    }

    #[test]
    fn sticky_index_association_difference() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");

        let mut txn = doc.transact_mut();
        txt.insert(&mut txn, 0, "2");
        txt.insert(&mut txn, 0, "1");

        let rpos_right = txt.sticky_index(&mut txn, 1, Assoc::After).unwrap();
        let rpos_left = txt.sticky_index(&mut txn, 1, Assoc::Before).unwrap();

        txt.insert(&mut txn, 1, "x");

        let pos_right = rpos_right.get_offset(&txn).unwrap();
        let pos_left = rpos_left.get_offset(&txn).unwrap();

        assert_eq!(pos_right.index, 2);
        assert_eq!(pos_left.index, 1);
    }
}
