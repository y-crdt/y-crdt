use crate::doc::OffsetKind;
use crate::moving::Move;
use crate::store::Store;
use crate::transaction::TransactionMut;
use crate::types::text::update_current_attributes;
use crate::types::{
    Attrs, Branch, BranchPtr, TypePtr, Value, TYPE_REFS_ARRAY, TYPE_REFS_MAP, TYPE_REFS_TEXT,
    TYPE_REFS_UNDEFINED, TYPE_REFS_XML_ELEMENT, TYPE_REFS_XML_FRAGMENT, TYPE_REFS_XML_HOOK,
    TYPE_REFS_XML_TEXT,
};
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::*;
use lib0::any::Any;
use lib0::error::Error;
use smallstr::SmallString;
use std::collections::HashSet;
use std::hash::Hash;
use std::ops::{Deref, DerefMut};
use std::panic;
use std::ptr::NonNull;
use std::rc::Rc;

/// Bit flag used to identify [Block::GC].
pub const BLOCK_GC_REF_NUMBER: u8 = 0;

/// Bit flag used to identify items with content of type [ItemContent::Deleted].
pub const BLOCK_ITEM_DELETED_REF_NUMBER: u8 = 1;

/// Bit flag used to identify items with content of type [ItemContent::JSON].
pub const BLOCK_ITEM_JSON_REF_NUMBER: u8 = 2;

/// Bit flag used to identify items with content of type [ItemContent::Binary].
pub const BLOCK_ITEM_BINARY_REF_NUMBER: u8 = 3;

/// Bit flag used to identify items with content of type [ItemContent::String].
pub const BLOCK_ITEM_STRING_REF_NUMBER: u8 = 4;

/// Bit flag used to identify items with content of type [ItemContent::Embed].
pub const BLOCK_ITEM_EMBED_REF_NUMBER: u8 = 5;

/// Bit flag used to identify items with content of type [ItemContent::Format].
pub const BLOCK_ITEM_FORMAT_REF_NUMBER: u8 = 6;

/// Bit flag used to identify items with content of type [ItemContent::Number].
pub const BLOCK_ITEM_TYPE_REF_NUMBER: u8 = 7;

/// Bit flag used to identify items with content of type [ItemContent::Any].
pub const BLOCK_ITEM_ANY_REF_NUMBER: u8 = 8;

/// Bit flag used to identify items with content of type [ItemContent::Doc].
pub const BLOCK_ITEM_DOC_REF_NUMBER: u8 = 9;

/// Bit flag used to identify [Block::Skip].
pub const BLOCK_SKIP_REF_NUMBER: u8 = 10;

/// Bit flag used to identify items with content of type [ItemContent::Move].
pub const BLOCK_ITEM_MOVE_REF_NUMBER: u8 = 11;

/// Bit flag used to tell if encoded item has right origin defined.
pub const HAS_RIGHT_ORIGIN: u8 = 0b01000000;

/// Bit flag used to tell if encoded item has left origin defined.
pub const HAS_ORIGIN: u8 = 0b10000000;

/// Bit flag used to tell if encoded item has a parent subtitle defined. Subtitles are used only
/// for blocks which act as map-like types entries.
pub const HAS_PARENT_SUB: u8 = 0b00100000;

/// Globally unique client identifier.
pub type ClientID = u64;

/// Block identifier, which allows to uniquely identify any element insertion in a global scope
/// (across different replicas of the same document). It consists of client ID (which is unique
/// document replica identifier) and monotonically incrementing clock value.
///
/// [ID] corresponds to a [Lamport timestamp](https://en.wikipedia.org/wiki/Lamport_timestamp) in
/// terms of its properties and guarantees.
#[derive(Debug, Copy, Clone, PartialOrd, Ord, PartialEq, Eq, Hash)]
pub struct ID {
    /// Unique identifier of a client, which inserted corresponding item.
    pub client: ClientID,

    /// Monotonically incrementing sequence number, which informs about order of inserted item
    /// operation in a scope of a given `client`. This value doesn't have to increase by 1, but
    /// instead is increased by number of countable elements which make a content of an inserted
    /// block.
    pub clock: u32,
}

impl ID {
    pub fn new(client: ClientID, clock: u32) -> Self {
        ID { client, clock }
    }
}

/// A logical block pointer. It contains a unique block [ID], but also contains a helper metadata
/// which allows to faster locate block it points to within a block store.
#[repr(transparent)]
#[derive(Clone, Copy, Hash)]
pub struct BlockPtr(NonNull<Block>);

impl BlockPtr {
    pub(crate) fn delete_as_cleanup(&self, txn: &mut TransactionMut, is_local: bool) {
        txn.delete(*self);
        if is_local {
            txn.delete_set.insert(*self.id(), self.len());
        }
    }

    pub(crate) fn is_countable(&self) -> bool {
        match self.deref() {
            Block::Item(item) => item.is_countable(),
            Block::GC(_) => false,
        }
    }

    pub(crate) fn splice(&mut self, offset: u32, encoding: OffsetKind) -> Option<Box<Block>> {
        let self_ptr = self.clone();
        if offset == 0 {
            None
        } else {
            match self.deref_mut() {
                Block::Item(item) => {
                    let client = item.id.client;
                    let clock = item.id.clock;
                    let content = item.content.splice(offset as usize, encoding).unwrap();
                    item.len = offset;
                    let mut new = Box::new(Block::Item(Item {
                        id: ID::new(client, clock + offset),
                        len: content.len(OffsetKind::Utf16),
                        left: Some(self_ptr),
                        right: item.right.clone(),
                        origin: Some(ID::new(client, clock + offset - 1)),
                        right_origin: item.right_origin.clone(),
                        content,
                        parent: item.parent.clone(),
                        moved: item.moved.clone(),
                        parent_sub: item.parent_sub.clone(),
                        info: item.info.clone(),
                    }));
                    let new_ptr = BlockPtr::from(&mut new);

                    if let Some(Block::Item(right)) = item.right.as_deref_mut() {
                        right.left = Some(new_ptr);
                    }

                    if let Some(parent_sub) = item.parent_sub.as_ref() {
                        if item.right.is_none() {
                            // update parent.map
                            if let TypePtr::Branch(mut branch) = item.parent {
                                branch.map.insert(parent_sub.clone(), new_ptr);
                            }
                        }
                    }

                    item.right = Some(new_ptr);

                    Some(new)
                }
                Block::GC(gc) => Some(Box::new(Block::GC(gc.slice(offset)))),
            }
        }
    }

    /// Integrates current block into block store.
    /// If it returns true, it means that the block should be deleted after being added to a block store.
    pub fn integrate(&mut self, txn: &mut TransactionMut, offset: u32) -> bool {
        let self_ptr = self.clone();
        match self.deref_mut() {
            Block::GC(this) => this.integrate(offset),
            Block::Item(this) => {
                let store = txn.store_mut();
                let encoding = store.options.offset_kind;
                if offset > 0 {
                    // offset could be > 0 only in context of Update::integrate,
                    // is such case offset kind in use always means Yjs-compatible offset (utf-16)
                    this.id.clock += offset;
                    this.left = store
                        .blocks
                        .get_item_clean_end(&ID::new(this.id.client, this.id.clock - 1))
                        .map(|slice| store.materialize(slice));
                    this.origin = this.left.as_deref().map(|b: &Block| b.last_id());
                    this.content = this
                        .content
                        .splice(offset as usize, OffsetKind::Utf16)
                        .unwrap();
                    this.len -= offset;
                }

                let parent = match &this.parent {
                    TypePtr::Branch(branch) => Some(*branch),
                    TypePtr::Named(name) => {
                        let branch =
                            store.get_or_create_type(name.clone(), None, TYPE_REFS_UNDEFINED);
                        this.parent = TypePtr::Branch(branch);
                        Some(branch)
                    }
                    TypePtr::ID(id) => {
                        if let Some(block) = store.blocks.get_block(id) {
                            if let Some(branch) = block.as_branch() {
                                this.parent = TypePtr::Branch(branch);
                                Some(branch)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                let left: Option<&Block> = this.left.as_deref();
                let right: Option<&Block> = this.right.as_deref();

                let right_is_null_or_has_left = match right {
                    None => true,
                    Some(Block::Item(i)) => i.left.is_some(),
                    _ => false,
                };
                let left_has_other_right_than_self = match left {
                    Some(Block::Item(i)) => i.right != this.right,
                    _ => false,
                };

                if let Some(mut parent_ref) = parent {
                    if (left.is_none() && right_is_null_or_has_left)
                        || left_has_other_right_than_self
                    {
                        // set the first conflicting item
                        let mut o = if let Some(Block::Item(left)) = left {
                            left.right
                        } else if let Some(sub) = &this.parent_sub {
                            let mut o = parent_ref.map.get(sub).cloned();
                            while let Some(Block::Item(item)) = o.as_deref() {
                                if item.left.is_some() {
                                    o = item.left.clone();
                                    continue;
                                }
                                break;
                            }
                            o.clone()
                        } else {
                            parent_ref.start
                        };

                        let mut left = this.left.clone();
                        let mut conflicting_items = HashSet::new();
                        let mut items_before_origin = HashSet::new();

                        // Let c in conflicting_items, b in items_before_origin
                        // ***{origin}bbbb{this}{c,b}{c,b}{o}***
                        // Note that conflicting_items is a subset of items_before_origin
                        while let Some(ptr) = o {
                            if Some(ptr) == this.right {
                                break;
                            }

                            items_before_origin.insert(ptr);
                            conflicting_items.insert(ptr);
                            if let Block::Item(item) = ptr.deref() {
                                if this.origin == item.origin {
                                    // case 1
                                    if item.id.client < this.id.client {
                                        left = Some(ptr.clone());
                                        conflicting_items.clear();
                                    } else if this.right_origin == item.right_origin {
                                        // `self` and `item` are conflicting and point to the same integration
                                        // points. The id decides which item comes first. Since `self` is to
                                        // the left of `item`, we can break here.
                                        break;
                                    }
                                } else {
                                    if let Some(origin_ptr) = item
                                        .origin
                                        .as_ref()
                                        .and_then(|id| store.blocks.get_block(id))
                                    {
                                        if items_before_origin.contains(&origin_ptr) {
                                            if !conflicting_items.contains(&origin_ptr) {
                                                left = Some(ptr.clone());
                                                conflicting_items.clear();
                                            }
                                        } else {
                                            break;
                                        }
                                    } else {
                                        break;
                                    }
                                }
                                o = item.right.clone();
                            } else {
                                break;
                            }
                        }
                        this.left = left;
                    }

                    if this.parent_sub.is_none() {
                        if let Some(Block::Item(item)) = this.left.as_deref() {
                            if item.parent_sub.is_some() {
                                this.parent_sub = item.parent_sub.clone();
                            } else if let Some(Block::Item(item)) = this.right.as_deref() {
                                this.parent_sub = item.parent_sub.clone();
                            }
                        }
                    }

                    // reconnect left/right
                    if let Some(Block::Item(left)) = this.left.as_deref_mut() {
                        this.right = left.right.replace(self_ptr);
                    } else {
                        let r = if let Some(parent_sub) = &this.parent_sub {
                            // update parent map/start if necessary
                            let mut r = parent_ref.map.get(parent_sub).cloned();
                            while let Some(ptr) = r {
                                if let Block::Item(item) = ptr.deref() {
                                    if item.left.is_some() {
                                        r = item.left;
                                        continue;
                                    }
                                }
                                break;
                            }
                            r
                        } else {
                            let start = parent_ref.start.replace(self_ptr);
                            start
                        };
                        this.right = r;
                    }

                    if let Some(Block::Item(right)) = this.right.as_deref_mut() {
                        right.left = Some(self_ptr);
                    } else if let Some(parent_sub) = &this.parent_sub {
                        // set as current parent value if right === null and this is parentSub
                        parent_ref.map.insert(parent_sub.clone(), self_ptr);
                        if let Some(left) = this.left {
                            // this is the current attribute value of parent. delete right
                            txn.delete(left);
                        }
                    }

                    // adjust length of parent
                    if this.parent_sub.is_none() && this.is_countable() && !this.is_deleted() {
                        parent_ref.block_len += this.len;
                        parent_ref.content_len += this.content_len(encoding);
                    }

                    // check if this item is in a moved range
                    let left_moved = if let Some(Block::Item(i)) = this.left.as_deref() {
                        i.moved
                    } else {
                        None
                    };
                    let right_moved = if let Some(Block::Item(i)) = this.right.as_deref() {
                        i.moved
                    } else {
                        None
                    };
                    if left_moved.is_some() || right_moved.is_some() {
                        if left_moved == right_moved {
                            this.moved = left_moved;
                        } else {
                            #[inline]
                            fn try_integrate(mut ptr: BlockPtr, txn: &mut TransactionMut) {
                                let ptr_clone = ptr.clone();
                                if let Block::Item(i) = ptr.deref_mut() {
                                    if let ItemContent::Move(m) = &mut i.content {
                                        if !m.is_collapsed() {
                                            m.integrate_block(txn, ptr_clone);
                                        }
                                    }
                                }
                            }

                            if let Some(ptr) = left_moved {
                                try_integrate(ptr, txn);
                            }

                            if let Some(ptr) = right_moved {
                                try_integrate(ptr, txn);
                            }
                        }
                    }

                    match &mut this.content {
                        ItemContent::Deleted(len) => {
                            txn.delete_set.insert(this.id, *len);
                            this.mark_as_deleted();
                        }
                        ItemContent::Move(m) => m.integrate_block(txn, self_ptr),
                        ItemContent::Doc(_, _) => {
                            //// this needs to be reflected in doc.destroy as well
                            //this.doc._item = item
                            //transaction.subdocsAdded.add(this.doc)
                            //if (this.doc.shouldLoad) {
                            //    transaction.subdocsLoaded.add(this.doc)
                            //}
                            todo!()
                        }
                        ItemContent::Format(_, _) => {
                            // @todo searchmarker are currently unsupported for rich text documents
                            // /** @type {AbstractType<any>} */ (item.parent)._searchMarker = null
                        }
                        ItemContent::Type(branch) => {
                            branch.store = this.parent.as_branch().and_then(|b| b.store.clone())
                        }
                        _ => {
                            // other types don't define integration-specific actions
                        }
                    }
                    txn.add_changed_type(parent_ref, this.parent_sub.clone());
                    let parent_deleted = if let TypePtr::Branch(ptr) = &this.parent {
                        if let Some(block) = ptr.item {
                            block.is_deleted()
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if parent_deleted || (this.parent_sub.is_some() && this.right.is_some()) {
                        // delete if parent is deleted or if this is not the current attribute value of parent
                        true
                    } else {
                        false
                    }
                } else {
                    true
                    //panic!("Defect: item has no parent")
                }
            }
        }
    }

    pub(crate) fn gc(&mut self, parent_gced: bool) {
        if let Block::Item(item) = self.deref_mut() {
            if item.is_deleted() {
                item.content.gc();
                let len = item.len();
                if parent_gced {
                    let gc = Block::GC(BlockRange::new(item.id, len));
                    let self_mut = unsafe { self.0.as_mut() };
                    *self_mut = gc;
                } else {
                    item.content = ItemContent::Deleted(len);
                    item.info.clear_countable();
                }
            }
        }
    }

    /// Squashes two blocks together. Returns true if it succeeded. Squashing is possible only if
    /// blocks are of the same type, their contents are of the same type, they belong to the same
    /// parent data structure, their IDs are sequenced directly one after another and they point to
    /// each other as their left/right neighbors respectively.
    pub fn try_squash(&mut self, mut other: BlockPtr) -> bool {
        let self_ptr = self.clone();
        let other_ptr = other.clone();
        match (self.deref_mut(), other.deref_mut()) {
            (Block::Item(v1), Block::Item(v2)) => {
                if v1.id.client == v2.id.client
                    && v1.id.clock + v1.len() == v2.id.clock
                    && v2.origin == Some(v1.last_id())
                    && v1.right_origin == v2.right_origin
                    && v1.right == Some(other_ptr)
                    && v1.is_deleted() == v2.is_deleted()
                    && v1.moved == v2.moved
                    && v1.content.try_squash(&v2.content)
                {
                    v1.len = v1.content.len(OffsetKind::Utf16);
                    if let Some(Block::Item(right_right)) = v2.right.as_deref_mut() {
                        right_right.left = Some(self_ptr);
                    }
                    v1.right = v2.right;
                    true
                } else {
                    false
                }
            }
            (Block::GC(v1), Block::GC(v2)) => {
                v1.merge(v2);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn as_branch(self) -> Option<BranchPtr> {
        let item = self.as_item()?;
        if let ItemContent::Type(branch) = &item.content {
            Some(BranchPtr::from(branch))
        } else {
            None
        }
    }
}

impl Deref for BlockPtr {
    type Target = Block;

    fn deref(&self) -> &Self::Target {
        unsafe { self.0.as_ref() }
    }
}

impl DerefMut for BlockPtr {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.0.as_mut() }
    }
}

impl<'a> From<&'a mut Box<Block>> for BlockPtr {
    fn from(block: &'a mut Box<Block>) -> Self {
        BlockPtr(NonNull::from(block.as_mut()))
    }
}

impl<'a> From<&'a Box<Block>> for BlockPtr {
    fn from(block: &'a Box<Block>) -> Self {
        BlockPtr(unsafe { NonNull::new_unchecked(block.as_ref() as *const Block as *mut Block) })
    }
}

impl Eq for BlockPtr {}

impl PartialEq for BlockPtr {
    fn eq(&self, other: &Self) -> bool {
        // BlockPtr.pivot may differ, but logically it doesn't affect block equality
        self.id() == other.id()
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct BlockSlice {
    ptr: BlockPtr,
    start: u32,
    end: u32,
}

impl BlockSlice {
    pub fn new(ptr: BlockPtr, start: u32, end: u32) -> Self {
        debug_assert!(start <= end);
        BlockSlice { ptr, start, end }
    }

    /// Returns the first [ID] covered by this slice (inclusive).
    pub fn id(&self) -> ID {
        let mut id = *self.ptr.id();
        id.clock += self.start;
        id
    }

    /// Returns the last [ID] covered by this slice (inclusive).
    pub fn last_id(&self) -> ID {
        let mut id = *self.ptr.id();
        id.clock += self.end;
        id
    }

    /// Returns true when current [BlockSlice] left boundary is equal to the boundary of the
    /// [BlockPtr] this slice wraps.
    pub fn adjacent_left(&self) -> bool {
        self.start == 0
    }

    /// Returns true when current [BlockSlice] right boundary is equal to the boundary of the
    /// [BlockPtr] this slice wraps.
    pub fn adjacent_right(&self) -> bool {
        self.end == self.ptr.len() - 1
    }

    /// Returns true when boundaries marked by the current [BlockSlice] match the boundaries
    /// of the underlying [BlockPtr].
    pub fn adjacent(&self) -> bool {
        self.adjacent_left() && self.adjacent_right()
    }

    /// Returns the number of elements (counted as Yjs ID clock length) of the block range described
    /// by current [BlockSlice].
    pub fn len(&self) -> u32 {
        self.end - self.start + 1
    }

    /// Returns an underlying [BlockPtr].
    pub fn as_ptr(&self) -> BlockPtr {
        self.ptr
    }

    pub fn start(&self) -> u32 {
        self.start
    }

    pub fn end(&self) -> u32 {
        self.end
    }

    pub fn is_deleted(&self) -> bool {
        self.ptr.is_deleted()
    }

    pub fn is_countable(&self) -> bool {
        self.ptr.is_countable()
    }

    pub fn contains_id(&self, id: &ID) -> bool {
        let myself = self.ptr.id();
        myself.client == id.client
            && id.clock >= myself.clock + self.start
            && id.clock <= myself.clock + self.end
    }

    pub fn encode<E: Encoder>(&self, encoder: &mut E, store: Option<&Store>) {
        match self.ptr.deref() {
            Block::Item(item) => {
                let mut info = item.info();
                let origin = if self.adjacent_left() {
                    item.origin
                } else {
                    Some(ID::new(item.id.client, item.id.clock + self.start - 1))
                };
                if origin.is_some() {
                    info |= HAS_ORIGIN;
                }
                let cant_copy_parent_info = info & (HAS_ORIGIN | HAS_RIGHT_ORIGIN) == 0;
                encoder.write_info(info);
                if let Some(origin_id) = origin {
                    encoder.write_left_id(&origin_id);
                }
                if self.adjacent_right() {
                    if let Some(right_origin_id) = item.right_origin.as_ref() {
                        encoder.write_right_id(right_origin_id);
                    }
                }
                if cant_copy_parent_info {
                    match &item.parent {
                        TypePtr::Branch(branch) => {
                            if let Some(block) = branch.item {
                                encoder.write_parent_info(false);
                                encoder.write_left_id(block.id());
                            } else if let Some(store) = store {
                                let name = store.get_type_key(*branch).unwrap();
                                encoder.write_parent_info(true);
                                encoder.write_string(name);
                            }
                        }
                        TypePtr::Named(name) => {
                            encoder.write_parent_info(true);
                            encoder.write_string(name);
                        }
                        TypePtr::ID(id) => {
                            encoder.write_parent_info(false);
                            encoder.write_left_id(id);
                        }
                        TypePtr::Unknown => {
                            panic!("Couldn't get item's parent")
                        }
                    }

                    if let Some(parent_sub) = item.parent_sub.as_ref() {
                        encoder.write_string(parent_sub.as_ref());
                    }
                }
                item.content.encode_slice(encoder, self.start, self.end);
            }
            Block::GC(_) => {
                encoder.write_info(BLOCK_GC_REF_NUMBER);
                encoder.write_len(self.len());
            }
        }
    }

    pub fn right(&self) -> Option<BlockSlice> {
        let last_clock = self.ptr.len() - 1;
        if self.end == last_clock {
            if let Block::Item(item) = self.ptr.deref() {
                let right_ptr = item.right?;
                Some(BlockSlice::from(right_ptr))
            } else {
                None
            }
        } else {
            Some(BlockSlice::new(self.ptr, self.end + 1, last_clock))
        }
    }

    pub fn left(&self) -> Option<BlockSlice> {
        if self.start == 0 {
            if let Block::Item(item) = self.ptr.deref() {
                let left_ptr = item.left?;
                Some(BlockSlice::from(left_ptr))
            } else {
                None
            }
        } else {
            Some(BlockSlice::new(self.ptr, 0, self.start - 1))
        }
    }
}

impl From<BlockPtr> for BlockSlice {
    fn from(ptr: BlockPtr) -> Self {
        BlockSlice::new(ptr, 0, ptr.len() - 1)
    }
}

/// An enum containing all supported block variants.
#[derive(PartialEq)]
pub enum Block {
    /// An active block containing user data.
    Item(Item),

    /// Block, which is a subject of garbage collection after an [Item] has been deleted and its
    /// safe for the transaction to remove it.
    GC(BlockRange),
}

impl Block {
    /// Since [Block] can span over multiple elements, this method returns an unique ID of the last
    /// element contained within current block.
    pub fn last_id(&self) -> ID {
        match self {
            Block::Item(item) => item.last_id(),
            Block::GC(gc) => gc.last_id(),
        }
    }

    /// Tries to cast this [Block] into an immutable [Item] reference, returning `None` if block was
    /// in fact not an item.
    pub fn as_item(&self) -> Option<&Item> {
        match self {
            Block::Item(item) => Some(item),
            _ => None,
        }
    }

    /// Tries to cast this [Block] into a mutable [Item] reference, returning `None` if block was in
    /// fact not an item.
    pub fn as_item_mut(&mut self) -> Option<&mut Item> {
        match self {
            Block::Item(item) => Some(item),
            _ => None,
        }
    }

    /// Checks if current block has been deleted. Yrs uses soft deletion (a.k.a. tombstoning) for
    /// marking deleted blocks.
    pub fn is_deleted(&self) -> bool {
        match self {
            Block::Item(item) => item.is_deleted(),
            Block::GC(_) => true,
        }
    }

    pub fn encode<E: Encoder>(&self, store: Option<&Store>, encoder: &mut E) {
        match self {
            Block::Item(item) => {
                let info = item.info();
                let cant_copy_parent_info = info & (HAS_ORIGIN | HAS_RIGHT_ORIGIN) == 0;
                encoder.write_info(info);
                if let Some(origin_id) = item.origin.as_ref() {
                    encoder.write_left_id(origin_id);
                }
                if let Some(right_origin_id) = item.right_origin.as_ref() {
                    encoder.write_right_id(right_origin_id);
                }
                if cant_copy_parent_info {
                    match &item.parent {
                        TypePtr::Branch(branch) => {
                            if let Some(block) = branch.item {
                                encoder.write_parent_info(false);
                                encoder.write_left_id(block.id());
                            } else if let Some(store) = store {
                                let name = store.get_type_key(*branch).unwrap();
                                encoder.write_parent_info(true);
                                encoder.write_string(name);
                            }
                        }
                        TypePtr::Named(name) => {
                            encoder.write_parent_info(true);
                            encoder.write_string(name);
                        }
                        TypePtr::ID(id) => {
                            encoder.write_parent_info(false);
                            encoder.write_left_id(id);
                        }
                        TypePtr::Unknown => {
                            panic!("Couldn't get item's parent")
                        }
                    }
                    if let Some(parent_sub) = item.parent_sub.as_ref() {
                        encoder.write_string(parent_sub.as_ref());
                    }
                }
                item.content.encode(encoder);
            }
            Block::GC(gc) => {
                encoder.write_info(BLOCK_GC_REF_NUMBER);
                encoder.write_len(gc.len);
            }
        }
    }

    /// Returns a unique identifier of a current block.
    pub fn id(&self) -> &ID {
        match self {
            Block::Item(item) => &item.id,
            Block::GC(gc) => &gc.id,
        }
    }

    /// Returns a length of a block. For most situation it works like [Block::content_len] with a
    /// difference to a [Text]/[XmlText] contents - in order to achieve compatibility with
    /// Yjs we need to calculate string length in terms of UTF-16 character encoding.
    /// However depending on used [Encoding] scheme we may calculate string length/offsets
    /// differently.
    pub fn len(&self) -> u32 {
        match self {
            Block::Item(item) => item.len(),
            Block::GC(gc) => gc.len,
        }
    }

    /// Checks if two blocks are of the same type.
    pub fn same_type(&self, other: &Self) -> bool {
        match (self, other) {
            (Block::Item(_), Block::Item(_)) | (Block::GC(_), Block::GC(_)) => true,
            _ => false,
        }
    }

    pub fn is_gc(&self) -> bool {
        if let Block::GC(_) = self {
            true
        } else {
            false
        }
    }

    pub fn is_item(&self) -> bool {
        if let Block::Item(_) = self {
            true
        } else {
            false
        }
    }

    pub fn contains(&self, id: &ID) -> bool {
        match self {
            Block::Item(v) => v.contains(id),
            Block::GC(v) => v.contains(id),
        }
    }
}

/// A helper structure that's used to precisely describe a location of an [Item] to be inserted in
/// relation to its neighbors and parent.
#[derive(Debug)]
pub(crate) struct ItemPosition {
    pub parent: types::TypePtr,
    pub left: Option<BlockPtr>,
    pub right: Option<BlockPtr>,
    pub index: u32,
    pub current_attrs: Option<Box<Attrs>>,
}

impl ItemPosition {
    pub fn forward(&mut self) -> bool {
        if let Some(Block::Item(right)) = self.right.as_deref() {
            if !right.is_deleted() {
                match &right.content {
                    ItemContent::String(_) | ItemContent::Embed(_) => {
                        self.index += right.len();
                    }
                    ItemContent::Format(key, value) => {
                        let attrs = self
                            .current_attrs
                            .get_or_insert_with(|| Box::new(Attrs::new()));
                        update_current_attributes(attrs.as_mut(), key, value.as_ref());
                    }
                    _ => {}
                }
            }

            let new = right.right.clone();
            self.left = self.right.take();
            self.right = new;

            return true;
        }
        false
    }

    /// If current `attributes` don't confirm the same keys as the formatting wrapping
    /// current insert position, they should be unset.
    pub fn unset_missing(&self, attributes: &mut Attrs) {
        if let Some(attrs) = self.current_attrs.as_ref() {
            // if current `attributes` don't confirm the same keys as the formatting wrapping
            // current insert position, they should be unset
            for (k, _) in attrs.iter() {
                if !attributes.contains_key(k) {
                    attributes.insert(k.clone(), Any::Null);
                }
            }
        }
    }
}

/// Bit flag (4th bit) for a marked item - not used atm.
const ITEM_FLAG_MARKED: u8 = 0b0000_1000;

/// Bit flag (3rd bit) for a tombstoned (deleted) item.
const ITEM_FLAG_DELETED: u8 = 0b0000_0100;

/// Bit flag (2nd bit) for an item, which contents are considered countable.
const ITEM_FLAG_COUNTABLE: u8 = 0b0000_0010;

/// Bit flag (1st bit) used for an item which should be kept - not used atm.
const ITEM_FLAG_KEEP: u8 = 0b0000_0001;

#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ItemFlags(u8);

impl ItemFlags {
    pub fn new(source: u8) -> Self {
        ItemFlags(source)
    }

    #[inline]
    fn set(&mut self, value: u8) {
        self.0 |= value
    }

    #[inline]
    fn clear(&mut self, value: u8) {
        self.0 &= !value
    }

    #[inline]
    fn check(&self, value: u8) -> bool {
        self.0 & value == value
    }

    #[inline]
    pub fn is_keep(&self) -> bool {
        self.check(ITEM_FLAG_KEEP)
    }

    #[inline]
    pub fn set_countable(&mut self) {
        self.set(ITEM_FLAG_COUNTABLE)
    }

    #[inline]
    pub fn clear_countable(&mut self) {
        self.clear(ITEM_FLAG_COUNTABLE)
    }

    #[inline]
    pub fn is_countable(&self) -> bool {
        self.check(ITEM_FLAG_COUNTABLE)
    }

    #[inline]
    pub fn set_deleted(&mut self) {
        self.set(ITEM_FLAG_DELETED)
    }

    #[inline]
    pub fn is_deleted(&self) -> bool {
        self.check(ITEM_FLAG_DELETED)
    }

    #[inline]
    pub fn is_marked(&self) -> bool {
        self.check(ITEM_FLAG_MARKED)
    }
}

impl Into<u8> for ItemFlags {
    #[inline]
    fn into(self) -> u8 {
        self.0
    }
}

/// An item is basic unit of work in Yrs. It contains user data reinforced with all metadata
/// required for a potential conflict resolution as well as extra fields used for joining blocks
/// together as a part of indexed sequences or maps.
#[derive(PartialEq)]
pub struct Item {
    /// Unique identifier of current item.
    pub id: ID,

    pub len: u32,

    /// Pointer to left neighbor of this item. Used in sequenced collections.
    /// If `None` current item is a first one on it's `parent` collection.
    pub left: Option<BlockPtr>,

    /// Pointer to right neighbor of this item. Used in sequenced collections.
    /// If `None` current item is the last one on it's `parent` collection.
    pub right: Option<BlockPtr>,

    /// Used for concurrent insert conflict resolution. An ID of a left-side neighbor at the moment
    /// of insertion of current block.
    pub origin: Option<ID>,

    /// Used for concurrent insert conflict resolution. An ID of a right-side neighbor at the moment
    /// of insertion of current block.
    pub right_origin: Option<ID>,

    /// A user data stored inside of a current item.
    pub content: ItemContent,

    /// Pointer to a parent collection containing current item.
    pub parent: types::TypePtr,

    /// Used only when current item is used by map-like types. In such case this item works as a
    /// key-value entry of a map, and this field contains a key used by map.
    pub parent_sub: Option<Rc<str>>, //TODO: Rc since it's already used in Branch.map component

    /// This property is reused by the moved prop. In this case this property refers to an Item.
    pub moved: Option<BlockPtr>,

    /// Bit flag field which contains information about specifics of this item.
    pub info: ItemFlags,
}

#[derive(PartialEq, Eq, Clone)]
pub struct BlockRange {
    pub id: ID,
    pub len: u32,
}

impl BlockRange {
    pub fn new(id: ID, len: u32) -> Self {
        BlockRange { id, len }
    }

    pub fn last_id(&self) -> ID {
        ID::new(self.id.client, self.id.clock + self.len)
    }

    pub fn slice(&mut self, offset: u32) -> Self {
        let mut next = self.clone();
        next.id.clock += offset;
        next.len -= offset;
        next
    }

    pub fn integrate(&mut self, pivot: u32) -> bool {
        if pivot > 0 {
            self.id.clock += pivot;
            self.len -= pivot;
        }

        false
    }

    #[inline]
    pub fn merge(&mut self, other: &Self) {
        self.len += other.len;
    }

    pub fn contains(&self, id: &ID) -> bool {
        self.id.client == id.client
            && id.clock >= self.id.clock
            && id.clock < self.id.clock + self.len
    }
}

impl std::fmt::Debug for BlockRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for BlockRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}-{})", self.id, self.len)
    }
}

impl Item {
    pub(crate) fn new(
        id: ID,
        left: Option<BlockPtr>,
        origin: Option<ID>,
        right: Option<BlockPtr>,
        right_origin: Option<ID>,
        parent: TypePtr,
        parent_sub: Option<Rc<str>>,
        content: ItemContent,
    ) -> Box<Block> {
        let info = ItemFlags::new(if content.is_countable() {
            ITEM_FLAG_COUNTABLE
        } else {
            0
        });
        let len = content.len(OffsetKind::Utf16);
        let mut item = Box::new(Block::Item(Item {
            id,
            len,
            left,
            right,
            origin,
            right_origin,
            content,
            parent,
            parent_sub,
            info,
            moved: None,
        }));
        let item_ptr = BlockPtr::from(&mut item);
        if let ItemContent::Type(branch) = &mut item.as_item_mut().unwrap().content {
            branch.item = Some(item_ptr);
        }
        item
    }

    pub fn contains(&self, id: &ID) -> bool {
        self.id.client == id.client
            && id.clock >= self.id.clock
            && id.clock < self.id.clock + self.len()
    }

    /// Checks if current item is marked as deleted (tombstoned). Yrs uses soft item deletion
    /// mechanism.
    pub fn is_deleted(&self) -> bool {
        self.info.is_deleted()
    }

    /// Checks if item content can be considered countable. Countable elements can be split
    /// and joined together.
    pub fn is_countable(&self) -> bool {
        self.info.is_countable()
    }

    pub(crate) fn mark_as_deleted(&mut self) {
        self.info.set_deleted()
    }

    /// Assign left/right neighbors of the block. This may require for origin/right_origin
    /// blocks to be already present in block store - which may not be the case during block
    /// decoding. We decode entire update first, and apply individual blocks second, hence
    /// repair function is called before applying the block rather than on decode.
    pub(crate) fn repair(&mut self, store: &mut Store) {
        if let Some(origin) = self.origin.as_ref() {
            self.left = store
                .blocks
                .get_item_clean_end(origin)
                .map(|slice| store.materialize(slice));
        }

        if let Some(origin) = self.right_origin.as_ref() {
            self.right = store
                .blocks
                .get_item_clean_start(origin)
                .map(|slice| store.materialize(slice));
        }

        // We have all missing ids, now find the items

        // In the original Y.js algorithm we decoded items as we go and attached them to client
        // block list. During that process if we had right origin but no left, we made a lookup for
        // right origin's parent and attach it as a parent of current block.
        //
        // Here since we decode all blocks first, then apply them, we might not find them in
        // the block store during decoding. Therefore we retroactively reattach it here.

        match &self.parent {
            TypePtr::Unknown => {
                if let Some(Block::Item(item)) = self.left.as_deref() {
                    if let TypePtr::Unknown = item.parent {
                        if let Some(Block::Item(item)) = self.right.as_deref() {
                            self.parent = item.parent.clone();
                            self.parent_sub = item.parent_sub.clone();
                        }
                    } else {
                        self.parent = item.parent.clone();
                        self.parent_sub = item.parent_sub.clone();
                    }
                } else if let Some(Block::Item(item)) = self.right.as_deref() {
                    self.parent = item.parent.clone();
                    self.parent_sub = item.parent_sub.clone();
                }
            }
            TypePtr::Named(name) => {
                let branch = store.get_or_create_type(name.clone(), None, TYPE_REFS_UNDEFINED);
                self.parent = branch.into();
            }
            TypePtr::ID(id) => {
                let ptr = store.blocks.get_block(id).unwrap();
                self.parent = if let Block::Item(item) = ptr.deref() {
                    match &item.content {
                        ItemContent::Type(branch) => TypePtr::Branch(BranchPtr::from(branch)),
                        ItemContent::Deleted(_) => TypePtr::Unknown,
                        _ => panic!("Defect: parent points to a block which is not a shared type"),
                    }
                } else {
                    TypePtr::Unknown
                };
            }
            _ => {}
        }
    }

    /// Returns a number of elements stored within this item. These elements don't have to exists
    /// in reality ie. when item has been deleted, corresponding content no longer exists but `len`
    /// still refers to a number of elements current block used to represent.
    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn content_len(&self, kind: OffsetKind) -> u32 {
        self.content.len(kind)
    }

    /// Returns an ID of the last element that can be considered a part of this item.
    pub fn last_id(&self) -> ID {
        ID::new(self.id.client, self.id.clock + self.len() - 1)
    }

    fn info(&self) -> u8 {
        let info = if self.origin.is_some() { HAS_ORIGIN } else { 0 } // is left null
            | if self.right_origin.is_some() { HAS_RIGHT_ORIGIN } else { 0 } // is right null
            | if self.parent_sub.is_some() { HAS_PARENT_SUB } else { 0 }
            | (self.content.get_ref_number() & 0b1111);
        info
    }
}

#[derive(PartialEq, PartialOrd, Eq, Ord, Debug, Clone)]
pub struct SplittableString {
    content: SmallString<[u8; 8]>,
}

impl SplittableString {
    pub fn len(&self, kind: OffsetKind) -> usize {
        let len = self.content.len();
        if len == 1 {
            len // quite often strings are single-letter, so we don't care about OffsetKind
        } else {
            match kind {
                OffsetKind::Bytes => len,
                OffsetKind::Utf16 => self.utf16_len(),
                OffsetKind::Utf32 => self.unicode_len(),
            }
        }
    }

    #[inline(always)]
    pub fn as_str(&self) -> &str {
        self.content.as_str()
    }

    #[inline(always)]
    pub fn utf16_len(&self) -> usize {
        self.encode_utf16().count()
    }

    pub fn unicode_len(&self) -> usize {
        self.content.chars().count()
    }

    /// Maps given offset onto block offset. This means, that given an `offset` provided
    /// in given `encoding` we want the output as a UTF-16 compatible offset (required
    /// by Yjs for compatibility reasons).
    pub(crate) fn block_offset(&self, offset: u32, kind: OffsetKind) -> u32 {
        match kind {
            OffsetKind::Utf16 => offset,
            OffsetKind::Bytes => {
                let mut remaining = offset;
                let mut i = 0;
                // since this offset is used to splitting later on - and we can only split entire
                // characters - we're computing by characters
                for c in self.content.chars() {
                    if remaining == 0 {
                        break;
                    }
                    remaining -= c.len_utf8() as u32;
                    i += c.len_utf16() as u32;
                }
                i
            }
            OffsetKind::Utf32 => self
                .content
                .chars()
                .take(offset as usize)
                .fold(0, |sum, c| sum + c.len_utf16() as u32),
        }
    }

    pub fn push_str(&mut self, str: &str) {
        self.content.push_str(str);
    }
}

impl std::fmt::Display for SplittableString {
    #[inline(always)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.content.fmt(f)
    }
}

impl Into<SmallString<[u8; 8]>> for SplittableString {
    #[inline(always)]
    fn into(self) -> SmallString<[u8; 8]> {
        self.content
    }
}

impl Into<Box<str>> for SplittableString {
    #[inline(always)]
    fn into(self) -> Box<str> {
        self.content.into_string().into_boxed_str()
    }
}

impl From<SmallString<[u8; 8]>> for SplittableString {
    fn from(content: SmallString<[u8; 8]>) -> Self {
        SplittableString { content }
    }
}

impl<'a> From<&'a str> for SplittableString {
    fn from(str: &'a str) -> Self {
        Self::from(SmallString::from_str(str))
    }
}

impl Deref for SplittableString {
    type Target = str;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.content
    }
}

pub(crate) fn split_str(str: &str, offset: usize, kind: OffsetKind) -> (&str, &str) {
    fn map_utf16_offset(str: &str, offset: u32) -> u32 {
        let mut off = 0;
        let mut i = 0;
        for c in str.chars() {
            if i >= offset {
                break;
            }
            off += c.len_utf8() as u32;
            i += c.len_utf16() as u32;
        }
        off
    }

    fn map_unicode_offset(str: &str, offset: u32) -> u32 {
        let mut off = 0;
        let mut i = 0;
        for c in str.chars() {
            if i >= offset {
                break;
            }
            off += c.len_utf8();
            i += 1;
        }
        off as u32
    }

    let off = match kind {
        OffsetKind::Bytes => offset,
        OffsetKind::Utf16 => map_utf16_offset(str, offset as u32) as usize,
        OffsetKind::Utf32 => map_unicode_offset(str, offset as u32) as usize,
    };
    str.split_at(off)
}

/// An enum describing the type of a user content stored as part of one or more
/// (if items were squashed) insert operations.
#[derive(Debug, PartialEq)]
pub enum ItemContent {
    /// Any JSON-like primitive type range.
    Any(Vec<Any>),

    /// A binary data eg. images.
    Binary(Vec<u8>),

    /// A marker for delete item data, which describes a number of deleted elements.
    /// Deleted elements also don't contribute to an overall length of containing collection type.
    Deleted(u32),

    Doc(Box<str>, Box<Any>),
    JSON(Vec<String>), // String is JSON
    Embed(Box<Any>),

    /// Formatting attribute entry. Format attributes are not considered countable and don't
    /// contribute to an overall length of a collection they are applied to.
    Format(Rc<str>, Box<Any>),

    /// A chunk of text, usually applied by collaborative text insertion.
    String(SplittableString),

    /// A reference of a branch node. Branch nodes define a complex collection types, such as
    /// arrays, maps or XML elements.
    Type(Box<Branch>),
    Move(Box<Move>),
}

impl ItemContent {
    /// Returns a reference number used to determine a content type.
    /// It's used during encoding/decoding of a containing block.
    pub fn get_ref_number(&self) -> u8 {
        match self {
            ItemContent::Any(_) => BLOCK_ITEM_ANY_REF_NUMBER,
            ItemContent::Binary(_) => BLOCK_ITEM_BINARY_REF_NUMBER,
            ItemContent::Deleted(_) => BLOCK_ITEM_DELETED_REF_NUMBER,
            ItemContent::Doc(_, _) => BLOCK_ITEM_DOC_REF_NUMBER,
            ItemContent::JSON(_) => BLOCK_ITEM_JSON_REF_NUMBER,
            ItemContent::Embed(_) => BLOCK_ITEM_EMBED_REF_NUMBER,
            ItemContent::Format(_, _) => BLOCK_ITEM_FORMAT_REF_NUMBER,
            ItemContent::String(_) => BLOCK_ITEM_STRING_REF_NUMBER,
            ItemContent::Type(_) => BLOCK_ITEM_TYPE_REF_NUMBER,
            ItemContent::Move(_) => BLOCK_ITEM_MOVE_REF_NUMBER,
        }
    }

    /// Checks if item content can be considered countable. Countable elements contribute to
    /// a length of the block they are contained by. Most of the item content variants are countable
    /// with exception for [ItemContent::Deleted] (which length describes number of removed
    /// elements) and [ItemContent::Format] (which is used for storing text formatting tags).
    pub fn is_countable(&self) -> bool {
        match self {
            ItemContent::Any(_) => true,
            ItemContent::Binary(_) => true,
            ItemContent::Doc(_, _) => true,
            ItemContent::JSON(_) => true,
            ItemContent::Embed(_) => true,
            ItemContent::String(_) => true,
            ItemContent::Type(_) => true,
            ItemContent::Deleted(_) => false,
            ItemContent::Format(_, _) => false,
            ItemContent::Move(_) => false,
        }
    }

    /// Returns a number of separate elements contained within current item content struct.
    ///
    /// Separate elements can be split in order to put another block in between them. Definition of
    /// separation depends on a item content kin, eg. [ItemContent::String], [ItemContent::Any],
    /// [ItemContent::JSON] and [ItemContent::Deleted] can have variable length as they may be split
    /// by other insert operations. Other variants (eg. [ItemContent::Binary]) are considered as
    /// a single element and therefore their length is always 1 and are not considered as subject of
    /// splitting.
    ///
    /// In cases of counting number of visible elements, `len` method should be used together with
    /// [ItemContent::is_countable].
    pub fn len(&self, kind: OffsetKind) -> u32 {
        match self {
            ItemContent::Deleted(deleted) => *deleted,
            ItemContent::String(str) => str.len(kind) as u32,
            ItemContent::Any(v) => v.len() as u32,
            ItemContent::JSON(v) => v.len() as u32,
            _ => 1,
        }
    }

    /// Reads a contents of current [ItemContent] into a given `buf`, starting from provided
    /// `offset`. Returns a number of elements read this way (it cannot be longer than `buf`'s len.
    pub fn read(&self, offset: usize, buf: &mut [Value]) -> usize {
        if buf.is_empty() {
            0
        } else {
            match self {
                ItemContent::Any(values) => {
                    let mut i = offset;
                    let mut j = 0;
                    while i < values.len() && j < buf.len() {
                        let any = &values[i];
                        buf[j] = Value::Any(any.clone());
                        i += 1;
                        j += 1;
                    }
                    j
                }
                ItemContent::String(v) => {
                    let chars = v.chars().skip(offset).take(buf.len());
                    let mut j = 0;
                    for c in chars {
                        buf[j] = Value::Any(Any::String(c.to_string().into_boxed_str()));
                        j += 1;
                    }
                    j
                }
                ItemContent::JSON(elements) => {
                    let mut i = offset;
                    let mut j = 0;
                    while i < elements.len() && j < buf.len() {
                        let elem = &elements[i];
                        buf[j] = Value::Any(Any::String(elem.clone().into_boxed_str()));
                        i += 1;
                        j += 1;
                    }
                    j
                }
                ItemContent::Binary(v) => {
                    buf[0] = Value::Any(Any::Buffer(v.clone().into_boxed_slice()));
                    1
                }
                ItemContent::Doc(_, v) => {
                    buf[0] = Value::Any(*v.clone());
                    1
                }
                ItemContent::Type(c) => {
                    let branch_ref = BranchPtr::from(c);
                    buf[0] = branch_ref.into();
                    1
                }
                ItemContent::Embed(any) => {
                    buf[0] = Value::Any(any.as_ref().clone());
                    1
                }
                ItemContent::Move(_) => 0,
                ItemContent::Deleted(_) => 0,
                ItemContent::Format(_, _) => 0,
            }
        }
    }

    pub fn get_content(&self) -> Vec<Value> {
        let len = self.len(OffsetKind::Utf32) as usize;
        let mut values = vec![Value::default(); len];
        let read = self.read(0, &mut values);
        if read == len {
            values
        } else {
            Vec::default()
        }
    }

    pub fn get_first(&self) -> Option<Value> {
        match self {
            ItemContent::Any(v) => v.first().map(|a| Value::Any(a.clone())),
            ItemContent::Binary(v) => Some(Value::Any(Any::Buffer(v.clone().into_boxed_slice()))),
            ItemContent::Deleted(_) => None,
            ItemContent::Move(_) => None,
            ItemContent::Doc(_, v) => Some(Value::Any(*v.clone())),
            ItemContent::JSON(v) => v
                .first()
                .map(|v| Value::Any(Any::String(v.clone().into_boxed_str()))),
            ItemContent::Embed(v) => Some(Value::Any(v.as_ref().clone())),
            ItemContent::Format(_, _) => None,
            ItemContent::String(v) => Some(Value::Any(Any::String(v.clone().into()))),
            ItemContent::Type(c) => Some(BranchPtr::from(c).into()),
        }
    }

    pub fn get_last(&self) -> Option<Value> {
        match self {
            ItemContent::Any(v) => v.last().map(|a| Value::Any(a.clone())),
            ItemContent::Binary(v) => Some(Value::Any(Any::Buffer(v.clone().into_boxed_slice()))),
            ItemContent::Deleted(_) => None,
            ItemContent::Move(_) => None,
            ItemContent::Doc(_, v) => Some(Value::Any(*v.clone())),
            ItemContent::JSON(v) => v
                .last()
                .map(|v| Value::Any(Any::String(v.clone().into_boxed_str()))),
            ItemContent::Embed(v) => Some(Value::Any(v.as_ref().clone())),
            ItemContent::Format(_, _) => None,
            ItemContent::String(v) => Some(Value::Any(Any::String(v.clone().into()))),
            ItemContent::Type(c) => Some(BranchPtr::from(c).into()),
        }
    }

    /// Encodes a slice of a current [ItemContent] within an index bounds of (start..=end) - both
    /// sides inclusive.
    pub fn encode_slice<E: Encoder>(&self, encoder: &mut E, start: u32, end: u32) {
        match self {
            ItemContent::Deleted(_) => encoder.write_len(end - start + 1),
            ItemContent::Binary(buf) => encoder.write_buf(buf),
            ItemContent::String(s) => {
                let slice = if start != 0 {
                    let (_, right) = split_str(&s, start as usize, OffsetKind::Utf16);
                    right
                } else {
                    &s
                };
                let slice = if end != 0 {
                    let (left, _) =
                        split_str(&slice, (end - start + 1) as usize, OffsetKind::Utf16);
                    left
                } else {
                    slice
                };
                encoder.write_string(slice)
            }
            ItemContent::Embed(s) => encoder.write_json(s.as_ref()),
            ItemContent::JSON(s) => {
                encoder.write_len(end - start + 1);
                for i in start..=end {
                    encoder.write_string(s[i as usize].as_str())
                }
            }
            ItemContent::Format(k, v) => {
                encoder.write_key(k.as_ref());
                encoder.write_json(v.as_ref());
            }
            ItemContent::Type(inner) => {
                encoder.write_type_ref(inner.type_ref());
                let type_ref = inner.type_ref();
                if type_ref == types::TYPE_REFS_XML_ELEMENT || type_ref == types::TYPE_REFS_XML_HOOK
                {
                    encoder.write_key(inner.name.as_ref().unwrap().as_ref())
                }
            }
            ItemContent::Any(any) => {
                encoder.write_len(end - start + 1);
                for i in start..=end {
                    encoder.write_any(&any[i as usize]);
                }
            }
            ItemContent::Doc(key, any) => {
                encoder.write_string(key.as_ref());
                encoder.write_any(any);
            }
            ItemContent::Move(m) => m.encode(encoder),
        }
    }

    pub fn encode<E: Encoder>(&self, encoder: &mut E) {
        match self {
            ItemContent::Deleted(len) => encoder.write_len(*len),
            ItemContent::Binary(buf) => encoder.write_buf(buf),
            ItemContent::String(s) => encoder.write_string(s.as_str()),
            ItemContent::Embed(s) => encoder.write_json(s.as_ref()),
            ItemContent::JSON(s) => {
                encoder.write_len(s.len() as u32);
                for json in s.iter() {
                    encoder.write_string(json.as_str())
                }
            }
            ItemContent::Format(k, v) => {
                encoder.write_key(k.as_ref());
                encoder.write_json(v.as_ref());
            }
            ItemContent::Type(inner) => {
                let type_ref = inner.type_ref();
                encoder.write_type_ref(type_ref);
                if type_ref == types::TYPE_REFS_XML_ELEMENT || type_ref == types::TYPE_REFS_XML_HOOK
                {
                    encoder.write_key(inner.name.as_ref().unwrap().as_ref())
                }
            }
            ItemContent::Any(any) => {
                encoder.write_len(any.len() as u32);
                for a in any.iter() {
                    encoder.write_any(a);
                }
            }
            ItemContent::Doc(key, any) => {
                encoder.write_string(key.as_ref());
                encoder.write_any(any);
            }
            ItemContent::Move(m) => m.encode(encoder),
        }
    }

    pub fn decode<D: Decoder>(decoder: &mut D, ref_num: u8) -> Result<Self, Error> {
        match ref_num & 0b1111 {
            BLOCK_ITEM_DELETED_REF_NUMBER => Ok(ItemContent::Deleted(decoder.read_len()?)),
            BLOCK_ITEM_JSON_REF_NUMBER => {
                let mut remaining = decoder.read_len()? as i32;
                let mut buf = Vec::with_capacity(remaining as usize);
                while remaining >= 0 {
                    buf.push(decoder.read_string()?.to_owned());
                    remaining -= 1;
                }
                Ok(ItemContent::JSON(buf))
            }
            BLOCK_ITEM_BINARY_REF_NUMBER => Ok(ItemContent::Binary(decoder.read_buf()?.to_owned())),
            BLOCK_ITEM_STRING_REF_NUMBER => Ok(ItemContent::String(decoder.read_string()?.into())),
            BLOCK_ITEM_EMBED_REF_NUMBER => Ok(ItemContent::Embed(decoder.read_json()?.into())),
            BLOCK_ITEM_FORMAT_REF_NUMBER => Ok(ItemContent::Format(
                decoder.read_key()?,
                decoder.read_json()?.into(),
            )),
            BLOCK_ITEM_TYPE_REF_NUMBER => {
                let type_ref = decoder.read_type_ref()?;
                let name = if type_ref == TYPE_REFS_XML_ELEMENT || type_ref == TYPE_REFS_XML_HOOK {
                    Some(decoder.read_key()?.to_owned())
                } else {
                    None
                };
                let inner = Branch::new(type_ref, name);
                Ok(ItemContent::Type(inner))
            }
            BLOCK_ITEM_ANY_REF_NUMBER => {
                let len = decoder.read_len()? as usize;
                let mut values = Vec::with_capacity(len);
                let mut i = 0;
                while i < len {
                    values.push(decoder.read_any()?);
                    i += 1;
                }
                Ok(ItemContent::Any(values))
            }
            BLOCK_ITEM_MOVE_REF_NUMBER => {
                let m = Move::decode(decoder)?;
                Ok(ItemContent::Move(Box::new(m)))
            }
            BLOCK_ITEM_DOC_REF_NUMBER => Ok(ItemContent::Doc(
                decoder.read_string()?.into(),
                Box::new(decoder.read_any()?),
            )),
            _ => Err(Error::UnexpectedValue),
        }
    }

    pub(crate) fn splice(&mut self, offset: usize, encoding: OffsetKind) -> Option<ItemContent> {
        match self {
            ItemContent::Any(value) => {
                let (left, right) = value.split_at(offset);
                let left = left.to_vec();
                let right = right.to_vec();
                *self = ItemContent::Any(left);
                Some(ItemContent::Any(right))
            }
            ItemContent::String(string) => {
                // compute offset given in unicode code points into byte position
                let (left, right) = split_str(&string, offset, encoding);
                let left: SplittableString = left.into();
                let right: SplittableString = right.into();

                //TODO: do we need that in Rust?
                //let split_point = left.chars().last().unwrap();
                //if split_point >= 0xD800 as char && split_point <= 0xDBFF as char {
                //    // Last character of the left split is the start of a surrogate utf16/ucs2 pair.
                //    // We don't support splitting of surrogate pairs because this may lead to invalid documents.
                //    // Replace the invalid character with a unicode replacement character (� / U+FFFD)
                //    left.replace_range((offset-1)..offset, "�");
                //    right.replace_range(0..1, "�");
                //}
                *self = ItemContent::String(left);

                Some(ItemContent::String(right))
            }
            ItemContent::Deleted(len) => {
                let right = ItemContent::Deleted(*len - offset as u32);
                *len = offset as u32;
                Some(right)
            }
            ItemContent::JSON(value) => {
                let (left, right) = value.split_at(offset);
                let left = left.to_vec();
                let right = right.to_vec();
                *self = ItemContent::JSON(left);
                Some(ItemContent::JSON(right))
            }
            _ => None,
        }
    }

    /// Tries to squash two item content structures together.
    pub fn try_squash(&mut self, other: &Self) -> bool {
        //TODO: change `other` to Self (not ref) and return type to Option<Self> (none if merge suceeded)
        match (self, other) {
            (ItemContent::Any(v1), ItemContent::Any(v2)) => {
                v1.append(&mut v2.clone());
                true
            }
            (ItemContent::Deleted(v1), ItemContent::Deleted(v2)) => {
                *v1 = *v1 + *v2;
                true
            }
            (ItemContent::JSON(v1), ItemContent::JSON(v2)) => {
                v1.append(&mut v2.clone());
                true
            }
            (ItemContent::String(v1), ItemContent::String(v2)) => {
                v1.push_str(v2.as_str());
                true
            }
            _ => false,
        }
    }

    pub(crate) fn gc(&mut self) {
        match self {
            ItemContent::Type(branch) => {
                let mut curr = branch.start.take();
                while let Some(mut ptr) = curr {
                    if let Block::Item(item) = ptr.deref_mut() {
                        curr = item.right.clone();
                        ptr.gc(true);
                        continue;
                    }
                    break;
                }

                for (_, ptr) in branch.map.drain() {
                    curr = Some(ptr);
                    while let Some(mut ptr) = curr {
                        if let Block::Item(item) = ptr.deref_mut() {
                            curr = item.left.clone();
                            ptr.gc(true);
                            continue;
                        }
                        break;
                    }
                }
            }
            ItemContent::Doc(_, _) => {
                /*
                if (doc._item) {
                  console.error('This document was already integrated as a sub-document. You should create a second instance instead with the same guid.')
                }
                /**
                 * @type {Doc}
                 */
                this.doc = doc
                /**
                 * @type {any}
                 */
                const opts = {}
                this.opts = opts
                if (!doc.gc) {
                  opts.gc = false
                }
                if (doc.autoLoad) {
                  opts.autoLoad = true
                }
                if (doc.meta !== null) {
                  opts.meta = doc.meta
                }
                 */
                todo!()
            }
            _ => {}
        }
    }
}

impl std::fmt::Debug for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, len: {}", self.id, self.len)?;
        match &self.parent {
            TypePtr::Unknown => {}
            TypePtr::Branch(b) => {
                if let Some(ptr) = b.item.as_ref() {
                    write!(f, ", parent: {}", ptr.id())?;
                } else {
                    write!(f, ", parent: <root>")?;
                }
            }
            other => {
                write!(f, ", parent: {}", other)?;
            }
        }
        if let Some(m) = self.moved {
            write!(f, ", moved-to: {}", m)?;
        }
        if let Some(origin) = self.origin.as_ref() {
            write!(f, ", origin-l: {}", origin)?;
        }
        if let Some(origin) = self.right_origin.as_ref() {
            write!(f, ", origin-r: {}", origin)?;
        }
        if let Some(left) = self.left.as_ref() {
            write!(f, ", left: {}", left.id())?;
        }
        if let Some(right) = self.right.as_ref() {
            write!(f, ", right: {}", right.id())?;
        }
        if let Some(key) = self.parent_sub.as_ref() {
            write!(f, ", '{}' =>", key)?;
        } else {
            write!(f, ":")?;
        }
        if self.is_deleted() {
            write!(f, " ~{}~)", &self.content)
        } else {
            write!(f, " {})", &self.content)
        }
    }
}

impl std::fmt::Display for ItemContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ItemContent::String(s) => write!(f, "'{}'", s),
            ItemContent::Any(s) => {
                write!(f, "[")?;
                let mut iter = s.iter();
                if let Some(a) = iter.next() {
                    write!(f, "{}", a.to_string())?;
                }
                while let Some(a) = iter.next() {
                    write!(f, ", {}", a.to_string())?;
                }
                write!(f, "]")
            }
            ItemContent::JSON(s) => {
                write!(f, "{{")?;
                let mut iter = s.iter();
                if let Some(a) = iter.next() {
                    write!(f, "{}", a)?;
                }
                while let Some(a) = iter.next() {
                    write!(f, ", {}", a)?;
                }
                write!(f, "}}")
            }
            ItemContent::Format(k, v) => write!(f, "<{}={}>", k, v),
            ItemContent::Deleted(s) => write!(f, "deleted({})", s),
            ItemContent::Binary(s) => write!(f, "{:?}", s),
            ItemContent::Type(inner) => match inner.type_ref() {
                TYPE_REFS_ARRAY => {
                    if let Some(ptr) = inner.start {
                        write!(f, "<array(head: {})>", ptr)
                    } else {
                        write!(f, "<array>")
                    }
                }
                TYPE_REFS_MAP => {
                    write!(f, "<map({{")?;
                    let mut iter = inner.map.iter();
                    if let Some((k, ptr)) = iter.next() {
                        write!(f, "'{}': {}", k, ptr)?;
                    }
                    while let Some((k, ptr)) = iter.next() {
                        write!(f, ", '{}': {}", k, ptr)?;
                    }
                    write!(f, "}})>")
                }
                TYPE_REFS_TEXT => {
                    if let Some(start) = inner.start {
                        write!(f, "<text(head: {})>", start)
                    } else {
                        write!(f, "<text>")
                    }
                }
                TYPE_REFS_XML_ELEMENT => {
                    write!(f, "<xml element: {}>", inner.name.as_ref().unwrap())
                }
                TYPE_REFS_XML_FRAGMENT => write!(f, "<xml fragment>"),
                TYPE_REFS_XML_HOOK => write!(f, "<xml hook>"),
                TYPE_REFS_XML_TEXT => write!(f, "<xml text>"),
                _ => write!(f, "<undefined type ref>"),
            },
            ItemContent::Move(m) => std::fmt::Display::fmt(m.as_ref(), f),
            _ => Ok(()),
        }
    }
}

impl std::fmt::Display for ItemPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(index: {}", self.index)?;
        if let Some(l) = self.left.as_ref() {
            write!(f, ", left: {}", l)?;
        }
        if let Some(r) = self.right.as_ref() {
            write!(f, ", right: {}", r)?;
        }
        write!(f, ")")
    }
}

/// A trait used for preliminary types, that can be inserted into nested YArray/YMap structures.
pub trait Prelim: Sized {
    /// This method is used to create initial content required in order to create a block item.
    /// A supplied `ptr` can be used to identify block that is about to be created to store
    /// the returned content.
    ///
    /// Since this method may decide to consume `self` or not, a second optional return parameter
    /// is used when `self` was not consumed - which is the case for complex types creation such as
    /// YMap or YArray. In such case it will be passed later on to [Self::integrate] method.
    fn into_content(self, txn: &mut TransactionMut) -> (ItemContent, Option<Self>);

    /// Method called once an original item filled with content from [Self::into_content] has been
    /// added to block store. This method is used by complex types such as maps or arrays to append
    /// the original contents of prelim struct into YMap, YArray etc.
    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr);
}

impl<T> Prelim for T
where
    T: Into<Any>,
{
    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let value: Any = self.into();
        (ItemContent::Any(vec![value]), None)
    }

    fn integrate(self, _txn: &mut TransactionMut, _inner_ref: BranchPtr) {}
}

#[derive(Debug)]
pub(crate) struct PrelimString(pub SmallString<[u8; 8]>);

impl Prelim for PrelimString {
    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        (ItemContent::String(self.0.into()), None)
    }

    fn integrate(self, _txn: &mut TransactionMut, _inner_ref: BranchPtr) {}
}

#[derive(Debug)]
pub(crate) struct PrelimEmbed(pub Any);

impl Prelim for PrelimEmbed {
    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        (ItemContent::Embed(Box::new(self.0)), None)
    }

    fn integrate(self, _txn: &mut TransactionMut, _inner_ref: BranchPtr) {}
}

impl std::fmt::Display for ID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{}#{}>", self.client, self.clock)
    }
}

impl std::fmt::Debug for BlockPtr {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for BlockPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({})", self.id())
    }
}

impl std::fmt::Display for Block {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Block::Item(item) => write!(f, "Item{}", item),
            Block::GC(gc) => write!(f, "GC{}", gc),
        }
    }
}

impl std::fmt::Debug for Block {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod test {
    use crate::block::{split_str, SplittableString};
    use crate::doc::OffsetKind;
    use std::ops::Deref;

    #[test]
    fn splittable_string_len() {
        let s: SplittableString = "Zażółć gęślą jaźń😀 女".into();

        assert_eq!(s.len(OffsetKind::Bytes), 34, "wrong byte length");
        assert_eq!(s.len(OffsetKind::Utf16), 21, "wrong UTF-16 length");
        assert_eq!(s.len(OffsetKind::Utf32), 20, "wrong Unicode chars count");
    }

    #[test]
    fn splittable_string_push_str() {
        let mut s: SplittableString = "Zażółć gęślą jaźń😀".into();
        s.push_str("ありがとうございます");

        assert_eq!(
            s.deref(),
            &"Zażółć gęślą jaźń😀ありがとうございます".to_string()
        );

        assert_eq!(s.len(OffsetKind::Bytes), 60, "wrong byte length");
        assert_eq!(s.len(OffsetKind::Utf16), 29, "wrong UTF-16 length");
        assert_eq!(s.len(OffsetKind::Utf32), 28, "wrong Unicode chars count");
    }

    #[test]
    fn splittable_string_split_str() {
        let s: SplittableString = "Zażółć gęślą jaźń😀ありがとうございます".into();

        let (a, b) = split_str(&s, 18, OffsetKind::Utf32);
        assert_eq!(a, "Zażółć gęślą jaźń😀");
        assert_eq!(b, "ありがとうございます");

        let (a, b) = split_str(&s, 19, OffsetKind::Utf16);
        assert_eq!(a, "Zażółć gęślą jaźń😀");
        assert_eq!(b, "ありがとうございます");

        let (a, b) = split_str(&s, 30, OffsetKind::Bytes);
        assert_eq!(a, "Zażółć gęślą jaźń😀");
        assert_eq!(b, "ありがとうございます");
    }
}
