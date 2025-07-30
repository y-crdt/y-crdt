use crate::block::{ItemContent, ItemPtr, Prelim, Unused};
use crate::block_iter::BlockIter;
use crate::branch::{Branch, BranchPtr};
use crate::encoding::read::Error;
use crate::transaction::TransactionMut;
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::{BranchID, ReadTxn, WriteTxn, ID};
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashSet;
use std::fmt::Formatter;
use std::sync::Arc;

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
    pub(crate) overrides: Option<HashSet<ItemPtr>>,
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
    ) -> (Option<ItemPtr>, Option<ItemPtr>) {
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

    fn get_item_ptr_mut<T: WriteTxn>(txn: &mut T, id: &ID, assoc: Assoc) -> Option<ItemPtr> {
        let store = txn.store_mut();
        if assoc == Assoc::After {
            let slice = store.blocks.get_item_clean_start(id)?;
            if slice.adjacent() {
                Some(slice.ptr)
            } else {
                Some(store.materialize(slice))
            }
        } else {
            let slice = store.blocks.get_item_clean_end(id)?;
            let ptr = if slice.adjacent() {
                slice.ptr
            } else {
                store.materialize(slice)
            };
            ptr.right
        }
    }

    pub(crate) fn get_moved_coords<T: ReadTxn>(
        &self,
        txn: &T,
    ) -> (Option<ItemPtr>, Option<ItemPtr>) {
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

    fn get_item_ptr<T: ReadTxn>(txn: &T, id: &ID, assoc: Assoc) -> Option<ItemPtr> {
        if assoc == Assoc::After {
            let slice = txn.store().blocks.get_item_clean_start(id)?;
            debug_assert!(slice.adjacent()); //TODO: remove once confirmed that slice always fits block range
            Some(slice.ptr)
        } else {
            let slice = txn.store().blocks.get_item_clean_end(id)?;
            debug_assert!(slice.adjacent()); //TODO: remove once confirmed that slice always fits block range
            slice.ptr.right
        }
    }

    pub(crate) fn find_move_loop<T: ReadTxn>(
        &self,
        txn: &mut T,
        moved: ItemPtr,
        tracked_moved_items: &mut HashSet<ItemPtr>,
    ) -> bool {
        if tracked_moved_items.contains(&moved) {
            true
        } else {
            tracked_moved_items.insert(moved.clone());
            let (mut start, end) = self.get_moved_coords(txn);
            while let Some(item) = start.as_deref() {
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

    fn push_override(&mut self, ptr: ItemPtr) {
        let e = self.overrides.get_or_insert_with(HashSet::default);
        e.insert(ptr);
    }

    pub(crate) fn integrate_block(&mut self, txn: &mut TransactionMut, item: ItemPtr) {
        let (init, end) = self.get_moved_coords_mut(txn);
        let mut max_priority = 0i32;
        let adapt_priority = self.priority < 0;
        let mut start = init;
        while start != end && start.is_some() {
            let start_ptr = start.unwrap().clone();
            if let Some(start_item) = start.as_deref_mut() {
                let mut prev_move = start_item.moved;
                let next_prio = if let Some(m) = prev_move.as_deref() {
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
                        if let ItemContent::Move(m) = &moved_ptr.content {
                            if m.is_collapsed() {
                                moved_ptr.delete_as_cleanup(txn, adapt_priority);
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
                } else if let Some(moved_item) = prev_move.as_deref_mut() {
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

    pub(crate) fn delete(&self, txn: &mut TransactionMut, item: ItemPtr) {
        let (mut start, end) = self.get_moved_coords(txn);
        while start != end && start.is_some() {
            if let Some(mut start_ptr) = start {
                if start_ptr.moved == Some(item) {
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
                    start_ptr.moved = None;
                }
                start = start_ptr.right;
                continue;
            }
            break;
        }

        fn reintegrate(mut item: ItemPtr, txn: &mut TransactionMut) {
            let deleted = item.is_deleted();
            let ptr = item.clone();
            if let ItemContent::Move(content) = &mut item.content {
                if deleted {
                    // potentially we can integrate the items that reIntegrateItem overrides
                    if let Some(overrides) = &content.overrides {
                        for &inner in overrides.iter() {
                            reintegrate(inner, txn);
                        }
                    }
                } else {
                    content.integrate_block(txn, ptr)
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
        let id = self.start.id().unwrap();
        encoder.write_var(id.client);
        encoder.write_var(id.clock);
        if !is_collapsed {
            let id = self.end.id().unwrap();
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
    /// If true - associate to the right block. Otherwise, associate to the left one.
    pub assoc: Assoc,
}

impl StickyIndex {
    pub fn new(scope: IndexScope, assoc: Assoc) -> Self {
        StickyIndex { scope, assoc }
    }

    pub fn from_id(id: ID, assoc: Assoc) -> Self {
        Self::new(IndexScope::Relative(id), assoc)
    }

    pub fn from_type<T, B>(_txn: &T, branch: &B, assoc: Assoc) -> Self
    where
        T: ReadTxn,
        B: AsRef<Branch>,
    {
        let branch = branch.as_ref();
        if let Some(ptr) = branch.item {
            let id = ptr.id().clone();
            Self::new(IndexScope::Nested(id), assoc)
        } else if let Some(name) = &branch.name {
            Self::new(IndexScope::Root(name.clone()), assoc)
        } else {
            unreachable!()
        }
    }

    #[inline]
    pub fn scope(&self) -> &IndexScope {
        &self.scope
    }

    /// Scope refers to root collection.
    #[inline]
    pub fn is_root(&self) -> bool {
        if let IndexScope::Root(_) = &self.scope {
            true
        } else {
            false
        }
    }

    /// Scope refers to nested shared collection.
    #[inline]
    pub fn is_nested(&self) -> bool {
        if let IndexScope::Nested(_) = &self.scope {
            true
        } else {
            false
        }
    }

    /// Scope refers to a position relative to another block.
    #[inline]
    pub fn is_relative(&self) -> bool {
        if let IndexScope::Relative(_) = &self.scope {
            true
        } else {
            false
        }
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
                if store.blocks.get_clock(&right_id.client) <= right_id.clock {
                    // type does not exist yet
                    return None;
                }
                let right = store.follow_redone(right_id);
                if let Some(right) = right {
                    if let Some(b) = right.ptr.parent.as_branch() {
                        branch = Some(b.clone());
                        match b.item {
                            Some(i) if i.is_deleted() => { /* do nothing */ }
                            _ => {
                                // adjust position based on left association if necessary
                                index = if right.is_deleted() || !right.is_countable() {
                                    0
                                } else if self.assoc == Assoc::After {
                                    right.start
                                } else {
                                    right.start + 1
                                };
                                let encoding = store.offset_kind;
                                let mut n = right.ptr.left;
                                while let Some(item) = n.as_deref() {
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
                if store.blocks.get_clock(&id.client) <= id.clock {
                    // type does not exist yet
                    return None;
                }
                let item = store.follow_redone(id)?; // early return if item is GC'ed
                if let ItemContent::Type(b) = &item.ptr.content {
                    // we don't need to materilized ItemContent::Type - they are always 1-length
                    branch = Some(BranchPtr::from(b.as_ref()));
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

    pub fn at<T: ReadTxn>(
        txn: &T,
        branch: BranchPtr,
        mut index: u32,
        assoc: Assoc,
    ) -> Option<Self> {
        if assoc == Assoc::Before {
            if index == 0 {
                let context = IndexScope::from_branch(branch);
                return Some(StickyIndex::new(context, assoc));
            }
            index -= 1;
        }

        let mut walker = BlockIter::new(branch);
        if !walker.try_forward(txn, index) {
            return None;
        }
        if walker.finished() {
            if assoc == Assoc::Before {
                let context = if let Some(ptr) = walker.next_item() {
                    IndexScope::Relative(ptr.last_id())
                } else {
                    IndexScope::from_branch(branch)
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
                IndexScope::from_branch(branch)
            };
            Some(Self::new(context, assoc))
        }
    }

    pub(crate) fn within_range(&self, ptr: Option<ItemPtr>) -> bool {
        if self.assoc == Assoc::Before {
            return false;
        } else if let Some(item) = ptr.as_deref() {
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

    pub(crate) fn get_item<T: ReadTxn>(&self, txn: &T) -> Option<ItemPtr> {
        let branch = match &self.scope {
            IndexScope::Relative(id) => {
                // position relative to existing block
                let item = txn.store().blocks.get_item(id)?;
                return if self.assoc == Assoc::After && &item.last_id() == id {
                    item.right
                } else {
                    Some(item)
                };
            }
            IndexScope::Nested(id) => {
                // position at the beginning/end of a nested type
                let item = txn.store().blocks.get_item(id)?;
                item.as_branch()?
            }
            IndexScope::Root(name) => {
                // position at the beginning/end of a root type
                let branch = txn.store().types.get(name.as_ref())?;
                BranchPtr::from(branch)
            }
        };
        match &self.assoc {
            // get first item of a branch
            Assoc::Before => branch.start,
            // get last item of a branch
            Assoc::After => {
                let mut item = branch.item?;
                while let Some(right) = item.right {
                    item = right;
                }
                Some(item)
            }
        }
    }
}

impl Encode for StickyIndex {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        self.scope.encode(encoder);
        self.assoc.encode(encoder);
    }
}

impl Decode for StickyIndex {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let context = IndexScope::decode(decoder)?;
        let assoc = Assoc::decode(decoder)?;
        Ok(Self::new(context, assoc))
    }
}

impl Serialize for StickyIndex {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("StickyIndex", 2)?;
        match &self.scope {
            IndexScope::Relative(id) => s.serialize_field("item", id)?,
            IndexScope::Nested(id) => s.serialize_field("type", id)?,
            IndexScope::Root(name) => s.serialize_field("tname", name)?,
        }
        let assoc = self.assoc as i8;
        s.serialize_field("assoc", &assoc)?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for StickyIndex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StickyIndexVisitor;
        impl<'de> Visitor<'de> for StickyIndexVisitor {
            type Value = StickyIndex;

            fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
                write!(formatter, "StickyIndex")
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut item = None;
                let mut ttype = None;
                let mut tname = None;
                let mut assoc = None;
                while let Some(key) = access.next_key::<String>()? {
                    match &*key {
                        "item" => {
                            item = access.next_value()?;
                        }
                        "type" => {
                            ttype = access.next_value()?;
                        }
                        "tname" => {
                            tname = access.next_value()?;
                        }
                        "assoc" => {
                            assoc = access.next_value()?;
                        }
                        _ => {
                            return Err(serde::de::Error::unknown_field(
                                &*key,
                                &["item", "type", "tname", "assoc"],
                            ));
                        }
                    }
                }
                let scope = if let Some(id) = item {
                    IndexScope::Relative(id)
                } else if let Some(name) = tname {
                    IndexScope::Root(name)
                } else if let Some(id) = ttype {
                    IndexScope::Nested(id)
                } else {
                    return Err(serde::de::Error::missing_field("item"));
                };
                let assoc = if let Some(assoc) = assoc {
                    match assoc {
                        0 => Assoc::After,
                        -1 => Assoc::Before,
                        _ => return Err(serde::de::Error::custom("invalid assoc value")),
                    }
                } else {
                    Assoc::default()
                };
                Ok(StickyIndex::new(scope, assoc))
            }
        }
        deserializer.deserialize_map(StickyIndexVisitor)
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
#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum IndexScope {
    /// [StickyIndex] is relative to a given block [ID]. This happens whenever we set [StickyIndex]
    /// somewhere inside the non-empty shared collection.
    Relative(ID),
    /// If a containing collection is a nested y-type, which is empty, this case allows us to
    /// identify that nested type.
    Nested(ID),
    /// If a containing collection is a root-level y-type, which is empty, this case allows us to
    /// identify that nested type.
    Root(Arc<str>),
}

impl IndexScope {
    pub fn from_branch(branch: BranchPtr) -> Self {
        match branch.id() {
            BranchID::Nested(id) => IndexScope::Nested(id),
            BranchID::Root(name) => IndexScope::Root(name),
        }
    }
}

impl Encode for IndexScope {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
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

impl Serialize for Assoc {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Assoc::After => serializer.serialize_i8(0),
            Assoc::Before => serializer.serialize_i8(-1),
        }
    }
}

impl<'de> Deserialize<'de> for Assoc {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct AssocVisitor;
        impl Visitor<'_> for AssocVisitor {
            type Value = Assoc;

            fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
                write!(formatter, "Assoc")
            }

            #[inline]
            fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Assoc::After)
            }

            #[inline]
            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if v < 0 {
                    Ok(Assoc::Before)
                } else {
                    Ok(Assoc::After)
                }
            }
        }
        deserializer.deserialize_i64(AssocVisitor)
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

/// Trait used to retrieve a [StickyIndex] corresponding to a given human-readable index.
/// Unlike standard indexes [StickyIndex] enables to track the location inside of a shared
/// y-types, even in the face of concurrent updates.
pub trait IndexedSequence: AsRef<Branch> {
    /// Returns a [StickyIndex] equivalent to a human-readable `index`.
    /// Returns `None` if `index` is beyond the length of current sequence.
    fn sticky_index<T: ReadTxn>(
        &self,
        txn: &T,
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
    use crate::{Doc, IndexScope, IndexedSequence, StickyIndex, Text, TextRef, Transact, ID};
    use serde::{Deserialize, Serialize};

    fn check_sticky_indexes(doc: &Doc, text: &TextRef) {
        // test if all positions are encoded and restored correctly
        let mut txn = doc.transact_mut();
        let len = text.len(&txn);
        for i in 0..len {
            // for all types of associations..
            for assoc in [Assoc::After, Assoc::Before] {
                let rel_pos = text.sticky_index(&mut txn, i, assoc).unwrap();
                let encoded = rel_pos.encode_v1();
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

        check_sticky_indexes(&doc, &txt);
    }

    #[test]
    fn sticky_index_case_2() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");

        txt.insert(&mut doc.transact_mut(), 0, "abc");
        check_sticky_indexes(&doc, &txt);
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

        check_sticky_indexes(&doc, &txt);
    }

    #[test]
    fn sticky_index_case_4() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");

        txt.insert(&mut doc.transact_mut(), 0, "1");
        check_sticky_indexes(&doc, &txt);
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

        check_sticky_indexes(&doc, &txt);
    }

    #[test]
    fn sticky_index_case_6() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        check_sticky_indexes(&doc, &txt);
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

    #[test]
    fn sticky_index_is_yjs_compatible() {
        // below is the example data passed from Yjs tiptap plugin
        #[derive(Serialize, Deserialize)]
        struct AwarenessData {
            user: User,
            cursor: Cursor,
        }
        #[derive(Serialize, Deserialize)]
        struct Cursor {
            anchor: StickyIndex,
            head: StickyIndex,
        }
        #[derive(Serialize, Deserialize)]
        struct User {
            name: String,
            color: String,
        }
        let json = serde_json::json!({
            "user":{"name":"Grace","color":"#FAF594"},
            "cursor":{
                "anchor":{"type":{"client":3731284436u32,"clock":1},"tname":null,"item":{"client":3731284436u32,"clock":20},"assoc":-1},
                "head":{"type":{"client":3731284436u32,"clock":1},"tname":null,"item":{"client":3731284436u32,"clock":20},"assoc":-1}
            }
        });
        let data: AwarenessData = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(
            data.cursor.anchor,
            StickyIndex::new(IndexScope::Relative(ID::new(3731284436, 20)), Assoc::Before)
        );
        assert_eq!(
            data.cursor.head,
            StickyIndex::new(IndexScope::Relative(ID::new(3731284436, 20)), Assoc::Before)
        );
        let json2 = serde_json::to_value(&data).unwrap();
        assert_eq!(
            json2["cursor"]["anchor"]["item"],
            serde_json::json!({"client":3731284436u32,"clock":20})
        );
        assert_eq!(json2["cursor"]["anchor"]["assoc"], serde_json::json!(-1));
        assert_eq!(
            json2["cursor"]["head"]["item"],
            serde_json::json!({"client":3731284436u32,"clock":20})
        );
        assert_eq!(json2["cursor"]["head"]["assoc"], serde_json::json!(-1));
    }
}
