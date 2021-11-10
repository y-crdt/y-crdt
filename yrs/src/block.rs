use crate::types::{
    Branch, BranchRef, TypePtr, Value, TYPE_REFS_ARRAY, TYPE_REFS_MAP, TYPE_REFS_TEXT,
    TYPE_REFS_XML_ELEMENT, TYPE_REFS_XML_FRAGMENT, TYPE_REFS_XML_HOOK, TYPE_REFS_XML_TEXT,
};
use crate::updates::decoder::Decoder;
use crate::updates::encoder::Encoder;
use crate::*;
use lib0::any::Any;
use std::collections::HashSet;
use std::hash::Hash;
use std::panic;

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

/// Bit flag used to tell if encoded item has right origin defined.
pub const HAS_RIGHT_ORIGIN: u8 = 0b01000000;

/// Bit flag used to tell if encoded item has left origin defined.
pub const HAS_ORIGIN: u8 = 0b10000000;

/// Bit flag used to tell if encoded item has a parent subtitle defined. Subtitles are used only
/// for blocks which act as map-like types entries.
pub const HAS_PARENT_SUB: u8 = 0b00100000;

/// Block identifier, which allows to uniquely identify any element insertion in a global scope
/// (across different replicas of the same document). It consists of client ID (which is unique
/// document replica identifier) and monotonically incrementing clock value.
///
/// [ID] corresponds to a [Lamport timestamp](https://en.wikipedia.org/wiki/Lamport_timestamp) in
/// terms of its properties and guarantees.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct ID {
    /// Unique identifier of a client, which inserted corresponding item.
    pub client: u64,

    /// Monotonically incrementing sequence number, which informs about order of inserted item
    /// operation in a scope of a given `client`. This value doesn't have to increase by 1, but
    /// instead is increased by number of countable elements which make a content of an inserted
    /// block.
    pub clock: u32,
}

impl ID {
    pub fn new(client: u64, clock: u32) -> Self {
        ID { client, clock }
    }
}

/// A logical block pointer. It contains a unique block [ID], but also contains a helper metadata
/// which allows to faster locate block it points to within a block store.
#[derive(Debug, Clone, Copy, Hash)]
pub struct BlockPtr {
    /// Unique identifier of a corresponding block.
    pub id: ID,

    /// An information about offset at which current block can be found within block store.
    ///
    /// Atm. this value is not always precise, so a conservative check is performed every time
    /// it's used. If such check fails, search algorithm falls back to binary search and upon
    /// completion re-adjusts the pivot information.
    pivot: u32,
}

impl BlockPtr {
    pub fn new(id: ID, pivot: u32) -> Self {
        BlockPtr { id, pivot }
    }

    pub fn from(id: ID) -> Self {
        BlockPtr {
            id,
            pivot: id.clock,
        }
    }

    #[inline]
    pub fn pivot(&self) -> usize {
        self.pivot as usize
    }

    pub fn fix_pivot(&self, pivot: u32) {
        unsafe { std::ptr::write(&self.pivot as *const u32 as *mut u32, pivot) };
    }
}

impl Eq for BlockPtr {}

impl PartialEq for BlockPtr {
    fn eq(&self, other: &Self) -> bool {
        // BlockPtr.pivot may differ, but logically it doesn't affect block equality
        self.id == other.id
    }
}

/// An enum containing all supported block variants.
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum Block {
    /// An active block containing user data.
    Item(Item),

    Skip(Skip),

    /// Block, which is a subject of garbage collection after an [Item] has been deleted and its
    /// safe for the transaction to remove it.
    GC(GC),
}

impl Block {
    /// Since [Block] can span over multiple elements, this method returns an unique ID of the last
    /// element contained within current block.
    pub fn last_id(&self) -> ID {
        match self {
            Block::Item(item) => item.last_id(),
            Block::Skip(skip) => ID::new(skip.id.client, skip.id.clock + skip.len),
            Block::GC(gc) => ID::new(gc.id.client, gc.id.clock + gc.len),
        }
    }

    pub fn slice(&self, offset: u32) -> Self {
        match self {
            Block::Item(item) => Block::Item(item.slice(offset)),
            Block::Skip(skip) => Block::Skip(Skip {
                id: ID {
                    client: skip.id.client,
                    clock: skip.id.clock + offset,
                },
                len: skip.len - offset,
            }),
            Block::GC(gc) => Block::GC(GC {
                id: ID {
                    client: gc.id.client,
                    clock: gc.id.clock + offset,
                },
                len: gc.len,
            }),
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
            Block::Skip(_) => false,
            Block::GC(_) => true,
        }
    }

    /// Integrates a new incoming block into a block store.
    pub fn integrate(&mut self, txn: &mut Transaction<'_>, pivot: u32, offset: u32) -> bool {
        match self {
            Block::Item(item) => item.integrate(txn, pivot, offset),
            Block::GC(gc) => gc.integrate(offset),
            Block::Skip(_) => {
                panic!("Block::Skip cannot be integrated")
            }
        }
    }

    /// Squashes two blocks together. Returns true if it succeeded. Squashing is possible only if
    /// blocks are of the same type, their contents are of the same type, they belong to the same
    /// parent data structure, their IDs are sequenced directly one after another and they point to
    /// each other as their left/right neighbors respectively.
    pub fn try_squash(&mut self, other: &Self) -> bool {
        match (self, other) {
            (Block::Item(v1), Block::Item(v2)) => v1.try_squash(v2),
            (Block::GC(v1), Block::GC(v2)) => {
                v1.merge(v2);
                true
            }
            (Block::Skip(v1), Block::Skip(v2)) => {
                v1.merge(v2);
                true
            }
            _ => false,
        }
    }

    pub fn encode_with_offset<E: Encoder>(&self, encoder: &mut E, offset: u32) {
        if let Block::Item(item) = self {
            let origin = if offset > 0 {
                Some(ID::new(item.id.client, item.id.clock + offset - 1))
            } else {
                item.origin
            };
            let info = item.info();
            let cant_copy_parent_info = info & (HAS_ORIGIN | HAS_RIGHT_ORIGIN) == 0;
            encoder.write_info(info);
            if let Some(origin_id) = origin {
                encoder.write_left_id(&origin_id);
            }
            if let Some(right_origin_id) = item.right_origin.as_ref() {
                encoder.write_right_id(right_origin_id);
            }
            if cant_copy_parent_info {
                if let TypePtr::Id(id) = &item.parent {
                    encoder.write_parent_info(false);
                    encoder.write_left_id(&id.id);
                } else if let TypePtr::Named(name) = &item.parent {
                    encoder.write_parent_info(true);
                    encoder.write_string(name)
                } else {
                    panic!("Couldn't get item's parent")
                }

                if let Some(parent_sub) = item.parent_sub.as_ref() {
                    encoder.write_string(parent_sub.as_str());
                }
            }
            item.content.encode_with_offset(encoder, offset);
        }
    }

    pub fn encode<E: Encoder>(&self, encoder: &mut E) {
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
                    if let TypePtr::Id(id) = &item.parent {
                        encoder.write_parent_info(false);
                        encoder.write_left_id(&id.id);
                    } else if let TypePtr::Named(name) = &item.parent {
                        encoder.write_parent_info(true);
                        encoder.write_string(name)
                    } else {
                        panic!("Couldn't get item's parent")
                    }

                    if let Some(parent_sub) = item.parent_sub.as_ref() {
                        encoder.write_string(parent_sub.as_str());
                    }
                }
                item.content.encode(encoder);
            }
            Block::Skip(skip) => {
                encoder.write_info(BLOCK_SKIP_REF_NUMBER);
                encoder.write_len(skip.len);
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
            Block::Skip(skip) => &skip.id,
            Block::GC(gc) => &gc.id,
        }
    }

    /// Returns a number of elements stored within this block. These elements don't have to exists
    /// in reality ie. when block was garbage collected or tombstoned, corresponding content no
    /// longer exists but `len` still refers to a number of elements current block used to
    /// represent.
    pub fn len(&self) -> u32 {
        match self {
            Block::Item(item) => item.len(),
            Block::Skip(skip) => skip.len,
            Block::GC(gc) => gc.len,
        }
    }

    /// Returns a last clock value of a current block. This is exclusive value meaning, that
    /// using it with current block's client ID will point to the beginning of a next block.
    pub fn clock_end(&self) -> u32 {
        match self {
            Block::Item(item) => item.id.clock + item.len(),
            Block::Skip(skip) => skip.id.clock + skip.len,
            Block::GC(gc) => gc.id.clock + gc.len,
        }
    }

    /// Returns an ID of a block, current item depends upon
    /// (meaning: dependency must appear in the store before current item).
    pub fn dependency(&self) -> Option<&ID> {
        match self {
            Block::Item(item) => item.dependency(),
            _ => None,
        }
    }

    /// Checks if two blocks are of the same type.
    pub fn same_type(&self, other: &Self) -> bool {
        match (self, other) {
            (Block::Item(_), Block::Item(_))
            | (Block::GC(_), Block::GC(_))
            | (Block::Skip(_), Block::Skip(_)) => true,
            _ => false,
        }
    }

    pub fn is_skip(&self) -> bool {
        if let Block::Skip(_) = self {
            true
        } else {
            false
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
            Block::Skip(v) => v.contains(id),
            Block::GC(v) => v.contains(id),
        }
    }

    pub(crate) fn gc(&mut self, txn: &Transaction, parent_gced: bool) {
        if let Block::Item(item) = self {
            if item.is_deleted() {
                item.content.gc(txn);
                let len = item.len();
                if parent_gced {
                    *self = Block::GC(GC::new(item.id, len));
                } else {
                    item.content = ItemContent::Deleted(len);
                    item.info = item.info & !ITEM_FLAG_COUNTABLE;
                }
            }
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
}

/// Bit flag (4th bit) for a marked item - not used atm.
const ITEM_FLAG_MARKED: u8 = 0b1000;

/// Bit flag (3rd bit) for a tombstoned (deleted) item.
const ITEM_FLAG_DELETED: u8 = 0b0100;

/// Bit flag (2nd bit) for an item, which contents are considered countable.
const ITEM_FLAG_COUNTABLE: u8 = 0b0010;

/// Bit flag (1st bit) used for an item which should be kept - not used atm.
const ITEM_FLAG_KEEP: u8 = 0b0001;

/// An item is basic unit of work in Yrs. It contains user data reinforced with all metadata
/// required for a potential conflict resolution as well as extra fields used for joining blocks
/// together as a part of indexed sequences or maps.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Item {
    /// Unique identifier of current item.
    pub id: ID,

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
    pub parent_sub: Option<String>, //TODO: Rc since it's already used in Branch.map component

    /// Bit flag field which contains information about specifics of this item.
    pub info: u8,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Skip {
    pub id: ID,
    pub len: u32,
}

impl Skip {
    pub fn new(id: ID, len: u32) -> Self {
        Skip { id, len }
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

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct GC {
    pub id: ID,
    pub len: u32,
}

impl GC {
    pub fn new(id: ID, len: u32) -> Self {
        GC { id, len }
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

impl Item {
    pub(crate) fn new(
        id: ID,
        left: Option<BlockPtr>,
        origin: Option<ID>,
        right: Option<BlockPtr>,
        right_origin: Option<ID>,
        parent: TypePtr,
        parent_sub: Option<String>,
        content: ItemContent,
    ) -> Self {
        let info = if content.is_countable() {
            ITEM_FLAG_COUNTABLE
        } else {
            0
        };
        Item {
            id,
            left,
            right,
            origin,
            right_origin,
            content,
            parent,
            parent_sub,
            info: info,
        }
    }

    pub fn contains(&self, id: &ID) -> bool {
        self.id.client == id.client
            && id.clock >= self.id.clock
            && id.clock < self.id.clock + self.len()
    }

    //TODO: not used yet
    pub fn marked(&self) -> bool {
        self.info & ITEM_FLAG_MARKED == ITEM_FLAG_MARKED
    }

    //TODO: not used yet
    pub fn keep(&self) -> bool {
        self.info & ITEM_FLAG_KEEP == ITEM_FLAG_KEEP
    }

    /// Checks if current item is marked as deleted (tombstoned). Yrs uses soft item deletion
    /// mechanism.
    pub fn is_deleted(&self) -> bool {
        self.info & ITEM_FLAG_DELETED == ITEM_FLAG_DELETED
    }

    /// Checks if item content can be considered countable. Countable elements can be split
    /// and joined together.
    pub fn is_countable(&self) -> bool {
        self.info & ITEM_FLAG_COUNTABLE == ITEM_FLAG_COUNTABLE
    }

    pub(crate) fn mark_as_deleted(&mut self) {
        self.info |= ITEM_FLAG_DELETED;
    }

    /// Assign left/right neighbors of the block. This may require for origin/right_origin
    /// blocks to be already present in block store - which may not be the case during block
    /// decoding. We decode entire update first, and apply individual blocks second, hence
    /// repair function is called before applying the block rather than on decode.
    pub fn repair(&mut self, txn: &mut Transaction<'_>) {
        if let Some(origin_id) = self.origin {
            let ptr = BlockPtr::from(origin_id);
            if let Some(item) = txn.store.blocks.get_item(&ptr) {
                let len = item.len();
                if origin_id.clock == item.id.clock + len - 1 {
                    self.left = Some(BlockPtr::new(item.id, ptr.pivot));
                } else {
                    let ptr =
                        BlockPtr::new(ID::new(origin_id.client, origin_id.clock + 1), ptr.pivot);
                    let (l, _) = txn.store.blocks.split_block(&ptr);
                    self.left = l;
                }
            }
        }

        if let Some(id) = self.right_origin {
            let (l, r) = txn.store.blocks.split_block(&BlockPtr::from(id));
            // if we got a split, point to right-side
            // if right side is None, then no split happened and `l` is right neighbor
            self.right = r.or(l);
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
                let src = if let Some(l) = self.left.as_ref() {
                    txn.store.blocks.get_item(l)
                } else if let Some(r) = self.right.as_ref() {
                    txn.store.blocks.get_item(r)
                } else {
                    None
                };

                if let Some(item) = src {
                    self.parent = item.parent.clone();
                    self.parent_sub = item.parent_sub.clone();
                }
            }
            TypePtr::Id(parent_ptr) => {
                if let Some(i) = txn.store.blocks.get_item(parent_ptr) {
                    if let ItemContent::Type(t) = &i.content {
                        let inner = t.borrow();
                        self.parent = inner.ptr.clone();
                    }
                }
            }
            TypePtr::Named(_) => {}
        }
    }

    /// Integrates current block into block store.
    /// If it returns true, it means that the block should be deleted after being added to a block store.
    pub fn integrate(&mut self, txn: &mut Transaction<'_>, pivot: u32, offset: u32) -> bool {
        if offset > 0 {
            self.id.clock += offset;
            let (left, _) = txn
                .store
                .blocks
                .split_block(&BlockPtr::from(ID::new(self.id.client, self.id.clock - 1)));
            if let Some(mut left) = left {
                if let Some(origin) = txn.store.blocks.get_item(&left) {
                    self.origin = Some(origin.last_id());
                    left.id = origin.id;
                    self.left = Some(left);
                }
            } else {
                self.left = None;
                self.origin = None;
            }
            self.content.splice(offset as usize);
        }

        let parent = match txn.store.get_type(&self.parent).cloned() {
            None => txn.store.init_type_from_ptr(&self.parent),
            parent => parent,
        };

        let left = self
            .left
            .as_ref()
            .and_then(|ptr| txn.store.blocks.get_block(ptr));

        let right = self
            .right
            .as_ref()
            .and_then(|ptr| txn.store.blocks.get_block(ptr));

        let right_is_null_or_has_left = match right {
            None => true,
            Some(Block::Item(i)) => i.left.is_some(),
            _ => false,
        };
        let left_has_other_right_than_self = match left {
            Some(Block::Item(i)) => i.right != self.right,
            _ => false,
        };

        if let Some(p) = parent {
            let mut parent_ref = p.borrow_mut();

            if (left.is_none() && right_is_null_or_has_left) || left_has_other_right_than_self {
                // set the first conflicting item
                let mut o = if let Some(Block::Item(left)) = left {
                    left.right
                } else if let Some(sub) = &self.parent_sub {
                    let mut o = parent_ref.map.get(sub);
                    while let Some(ptr) = o {
                        if let Some(item) = txn.store.blocks.get_item(ptr) {
                            if item.left.is_some() {
                                o = item.left.as_ref();
                                continue;
                            }
                        }
                        break;
                    }
                    o.cloned()
                } else {
                    parent_ref.start
                };

                let mut left = self.left.clone();
                let mut conflicting_items = HashSet::new();
                let mut items_before_origin = HashSet::new();

                // Let c in conflicting_items, b in items_before_origin
                // ***{origin}bbbb{this}{c,b}{c,b}{o}***
                // Note that conflicting_items is a subset of items_before_origin
                while let Some(ptr) = o {
                    if Some(ptr) == self.right {
                        break;
                    }

                    items_before_origin.insert(ptr.id.clone());
                    conflicting_items.insert(ptr.id.clone());
                    if let Some(Block::Item(item)) = txn.store.blocks.get_block(&ptr) {
                        if self.origin == item.origin {
                            // case 1
                            if ptr.id.client < self.id.client {
                                left = Some(ptr.clone());
                                conflicting_items.clear();
                            } else if self.right_origin == item.right_origin {
                                // `self` and `item` are conflicting and point to the same integration
                                // points. The id decides which item comes first. Since `self` is to
                                // the left of `item`, we can break here.
                                break;
                            }
                        } else {
                            if let Some(item_origin) = item.origin {
                                if items_before_origin.contains(&item_origin) {
                                    if !conflicting_items.contains(&item_origin) {
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
                self.left = left;
            }

            self.try_reassign_parent_sub(left);
            self.try_reassign_parent_sub(right);

            // reconnect left/right
            if let Some(left_id) = self.left.as_ref() {
                if let Some(left) = txn.store.blocks.get_item_mut(left_id) {
                    self.right = left.right.replace(BlockPtr::new(self.id, pivot));
                }
            } else {
                let r = if let Some(parent_sub) = &self.parent_sub {
                    let start = parent_ref.map.get(parent_sub).cloned();
                    let mut r = start.as_ref();

                    while let Some(ptr) = r {
                        if let Some(item) = txn.store.blocks.get_item(ptr) {
                            if item.left.is_some() {
                                r = item.left.as_ref();
                                continue;
                            }
                        }
                        break;
                    }
                    r.cloned()
                } else {
                    let start = parent_ref.start.replace(BlockPtr::new(self.id, pivot));
                    start
                };
                self.right = r;
            }

            if let Some(right_id) = self.right.as_ref() {
                if let Some(right) = txn.store.blocks.get_item_mut(right_id) {
                    right.left = Some(BlockPtr::new(self.id, pivot));
                }
            } else if let Some(parent_sub) = &self.parent_sub {
                // set as current parent value if right === null and this is parentSub
                parent_ref
                    .map
                    .insert(parent_sub.clone(), BlockPtr::new(self.id, pivot));
                if let Some(left) = self.left {
                    // this is the current attribute value of parent. delete right
                    txn.delete(&left);
                }
            }

            if self.parent_sub.is_none() && self.is_countable() && !self.is_deleted() {
                parent_ref.len += self.len();
            }

            self.integrate_content(txn, pivot, &mut *parent_ref);
            txn.add_changed_type(&*parent_ref, self.parent_sub.as_ref());
            let parent_deleted = if let TypePtr::Id(ptr) = &self.parent {
                if let Some(item) = txn.store.blocks.get_item(ptr) {
                    item.is_deleted()
                } else {
                    true
                }
            } else {
                false
            };
            if parent_deleted || (self.parent_sub.is_some() && self.right.is_some()) {
                // delete if parent is deleted or if this is not the current attribute value of parent
                true
            } else {
                false
            }
        } else {
            panic!("Defect: item has no parent")
        }
    }

    fn try_reassign_parent_sub(&mut self, block: Option<&Block>) {
        if self.parent_sub.is_none() {
            if let Some(Block::Item(item)) = block {
                //TODO: make parent_sub Rc<String> and clone from left unconditionally
                if item.parent_sub.is_some() {
                    self.parent_sub = item.parent_sub.clone();
                }
            }
        }
    }

    /// Returns a number of elements stored within this item. These elements don't have to exists
    /// in reality ie. when item has been deleted, corresponding content no longer exists but `len`
    /// still refers to a number of elements current block used to represent.
    pub fn len(&self) -> u32 {
        self.content.len()
    }

    pub fn slice(&self, diff: u32) -> Item {
        if diff == 0 {
            self.clone()
        } else {
            let client = self.id.client;
            let clock = self.id.clock;
            Item {
                id: ID::new(client, clock + diff),
                left: Some(BlockPtr::from(ID::new(client, clock + diff - 1))),
                right: self.right.clone(),
                origin: Some(ID::new(client, clock + diff - 1)),
                right_origin: self.right_origin.clone(),
                content: self.content.clone().splice(diff as usize).unwrap(),
                parent: self.parent.clone(),
                parent_sub: self.parent_sub.clone(),
                info: self.info.clone(),
            }
        }
    }

    /// Splits current item in two and a given `diff` offset. Returns a new item created as result
    /// of this split.
    pub fn split(&mut self, diff: u32) -> Item {
        let client = self.id.client;
        let clock = self.id.clock;
        let other = Item {
            id: ID::new(client, clock + diff),
            left: Some(BlockPtr::from(self.id)),
            right: self.right.clone(),
            origin: Some(ID::new(client, clock + diff - 1)),
            right_origin: self.right_origin.clone(),
            content: self.content.splice(diff as usize).unwrap(),
            parent: self.parent.clone(),
            parent_sub: self.parent_sub.clone(),
            info: self.info.clone(),
        };
        self.right = Some(BlockPtr::from(other.id));
        other
    }

    /// Returns an ID of the last element that can be considered a part of this item.
    pub fn last_id(&self) -> ID {
        ID::new(self.id.client, self.id.clock + self.len() - 1)
    }

    /// Tries to merge current [Item] with another, returning true if merge was performed successfully.
    pub fn try_squash(&mut self, other: &Self) -> bool {
        if self.id.client == other.id.client
            && self.id.clock + self.len() == other.id.clock
            && other.origin == Some(self.last_id())
            && self.right == Some(BlockPtr::from(other.id.clone()))
            && self.right_origin == other.right_origin
            && self.is_deleted() == other.is_deleted()
            && self.content.try_squash(&other.content)
        {
            self.right = other.right;
            //TODO: self.right.left = self
            true
        } else {
            false
        }
    }

    /// Returns an ID of a block, current item depends upon
    /// (meaning: dependency must appear in the store before current item).
    pub fn dependency(&self) -> Option<&ID> {
        self.origin
            .as_ref()
            .or_else(|| self.right_origin.as_ref())
            .or_else(|| match &self.parent {
                TypePtr::Id(ptr) => Some(&ptr.id),
                _ => None,
            })
    }

    fn info(&self) -> u8 {
        let info = if self.origin.is_some() { HAS_ORIGIN } else { 0 } // is left null
            | if self.right_origin.is_some() { HAS_RIGHT_ORIGIN } else { 0 } // is right null
            | if self.parent_sub.is_some() { HAS_PARENT_SUB } else { 0 }
            | (self.content.get_ref_number() & 0b1111);
        info
    }

    fn integrate_content(&mut self, txn: &mut Transaction<'_>, pivot: u32, parent: &mut Branch) {
        match &mut self.content {
            ItemContent::Deleted(len) => {
                txn.delete_set.insert(self.id, *len);
                self.mark_as_deleted();
            }
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
            ItemContent::Type(inner) => {
                // this.type._integrate(transaction.doc, item)
                let ptr = Some(BlockPtr::new(self.id.clone(), pivot));
                match inner.try_borrow_mut() {
                    Ok(mut parent) => parent.item = ptr,
                    Err(_) => parent.item = ptr,
                }
            }
            _ => {
                // other types don't define integration-specific actions
            }
        }
    }
}

/// An enum describing the type of a user content stored as part of one or more
/// (if items were squashed) insert operations.
#[derive(Debug, PartialEq, Clone)]
pub enum ItemContent {
    /// Any JSON-like primitive type range.
    Any(Vec<Any>),

    /// A binary data eg. images.
    Binary(Vec<u8>),

    /// A marker for delete item data, which describes a number of deleted elements.
    /// Deleted elements also don't contribute to an overall length of containing collection type.
    Deleted(u32),

    Doc(String, Any),
    JSON(Vec<String>), // String is JSON
    Embed(String),     // String is JSON

    /// Formatting attribute entry. Format attributes are not considered countable and don't
    /// contribute to an overall length of a collection they are applied to.
    Format(String, String), // key, value: JSON

    /// A chunk of text, usually applied by collaborative text insertion.
    String(String),

    /// A reference of a branch node. Branch nodes define a complex collection types, such as
    /// arrays, maps or XML elements.
    Type(BranchRef),
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
    pub fn len(&self) -> u32 {
        match self {
            ItemContent::Deleted(deleted) => *deleted,
            ItemContent::String(str) => {
                // @todo this should return the length in utf16!
                str.len() as u32
            }
            ItemContent::Any(v) => v.len() as u32,
            ItemContent::JSON(v) => v.len() as u32,
            _ => 1,
        }
    }

    /// Returns a formatted content of an item. For complex types (represented by [BranchRef] nodes)
    /// it will return a target type of a branch node (eg. Yrs [Array], [Map] or [XmlElement]). For
    /// other types it will returns a vector of elements stored within current block. Since block
    /// may describe a chunk of values within it, it always returns a vector of values.
    pub fn get_content(&self, txn: &Transaction<'_>) -> Vec<Value> {
        match self {
            ItemContent::Any(v) => v.iter().map(|a| Value::Any(a.clone())).collect(),
            ItemContent::Binary(v) => vec![Value::Any(Any::Buffer(v.clone().into_boxed_slice()))],
            ItemContent::Deleted(_) => Vec::default(),
            ItemContent::Doc(_, v) => vec![Value::Any(v.clone())],
            ItemContent::JSON(v) => v
                .iter()
                .map(|v| Value::Any(Any::String(v.clone())))
                .collect(),
            ItemContent::Embed(v) => vec![Value::Any(Any::String(v.clone()))],
            ItemContent::Format(_, _) => Vec::default(),
            ItemContent::String(v) => v
                .chars()
                .map(|c| Value::Any(Any::String(c.to_string())))
                .collect(),
            ItemContent::Type(c) => {
                vec![c.clone().into_value(txn)]
            }
        }
    }

    /// Similar to [get_content], but it only returns the latest result and doesn't materialize
    /// others for performance reasons.
    pub fn get_content_last(&self, txn: &Transaction<'_>) -> Option<Value> {
        match self {
            ItemContent::Any(v) => v.last().map(|a| Value::Any(a.clone())),
            ItemContent::Binary(v) => Some(Value::Any(Any::Buffer(v.clone().into_boxed_slice()))),
            ItemContent::Deleted(_) => None,
            ItemContent::Doc(_, v) => Some(Value::Any(v.clone())),
            ItemContent::JSON(v) => v.last().map(|v| Value::Any(Any::String(v.clone()))),
            ItemContent::Embed(v) => Some(Value::Any(Any::String(v.clone()))),
            ItemContent::Format(_, _) => None,
            ItemContent::String(v) => Some(Value::Any(Any::String(v.clone()))),
            ItemContent::Type(c) => Some(c.clone().into_value(txn)),
        }
    }

    pub fn encode_with_offset<E: Encoder>(&self, encoder: &mut E, offset: u32) {
        match self {
            ItemContent::Deleted(len) => encoder.write_len(*len - offset),
            ItemContent::Binary(buf) => encoder.write_buf(buf),
            ItemContent::String(s) => encoder.write_string(&s.as_str()[(offset as usize)..]),
            ItemContent::Embed(s) => encoder.write_string(s.as_str()),
            ItemContent::JSON(s) => {
                encoder.write_len(s.len() as u32 - offset);
                for i in (offset as usize)..s.len() {
                    encoder.write_string(s[i].as_str())
                }
            }
            ItemContent::Format(k, v) => {
                encoder.write_string(k.as_str());
                encoder.write_string(v.as_str());
            }
            ItemContent::Type(c) => {
                let inner = c.borrow();
                encoder.write_type_ref(inner.type_ref());
                let type_ref = inner.type_ref();
                if type_ref == types::TYPE_REFS_XML_ELEMENT || type_ref == types::TYPE_REFS_XML_HOOK
                {
                    encoder.write_key(inner.name.as_ref().unwrap().as_str())
                }
            }
            ItemContent::Any(any) => {
                encoder.write_len(any.len() as u32 - offset);
                for i in (offset as usize)..any.len() {
                    encoder.write_any(&any[i]);
                }
            }
            ItemContent::Doc(key, any) => {
                encoder.write_string(key.as_str());
                encoder.write_any(any);
            }
        }
    }

    pub fn encode<E: Encoder>(&self, encoder: &mut E) {
        match self {
            ItemContent::Deleted(len) => encoder.write_len(*len),
            ItemContent::Binary(buf) => encoder.write_buf(buf),
            ItemContent::String(s) => encoder.write_string(s.as_str()),
            ItemContent::Embed(s) => encoder.write_string(s.as_str()),
            ItemContent::JSON(s) => {
                encoder.write_len(s.len() as u32);
                for json in s.iter() {
                    encoder.write_string(json.as_str())
                }
            }
            ItemContent::Format(k, v) => {
                encoder.write_string(k.as_str());
                encoder.write_string(v.as_str());
            }
            ItemContent::Type(c) => {
                let inner = c.borrow();
                let type_ref = inner.type_ref();
                encoder.write_type_ref(type_ref);
                if type_ref == types::TYPE_REFS_XML_ELEMENT || type_ref == types::TYPE_REFS_XML_HOOK
                {
                    encoder.write_key(inner.name.as_ref().unwrap().as_str())
                }
            }
            ItemContent::Any(any) => {
                encoder.write_len(any.len() as u32);
                for a in any.iter() {
                    encoder.write_any(a);
                }
            }
            ItemContent::Doc(key, any) => {
                encoder.write_string(key.as_str());
                encoder.write_any(any);
            }
        }
    }

    pub fn decode<D: Decoder>(decoder: &mut D, ref_num: u8, ptr: block::BlockPtr) -> Self {
        match ref_num & 0b1111 {
            BLOCK_ITEM_DELETED_REF_NUMBER => ItemContent::Deleted(decoder.read_len()),
            BLOCK_ITEM_JSON_REF_NUMBER => {
                let mut remaining = decoder.read_len() as i32;
                let mut buf = Vec::with_capacity(remaining as usize);
                while remaining >= 0 {
                    buf.push(decoder.read_string().to_owned());
                    remaining -= 1;
                }
                ItemContent::JSON(buf)
            }
            BLOCK_ITEM_BINARY_REF_NUMBER => ItemContent::Binary(decoder.read_buf().to_owned()),
            BLOCK_ITEM_STRING_REF_NUMBER => ItemContent::String(decoder.read_string().to_owned()),
            BLOCK_ITEM_EMBED_REF_NUMBER => ItemContent::Embed(decoder.read_string().to_owned()),
            BLOCK_ITEM_FORMAT_REF_NUMBER => ItemContent::Format(
                decoder.read_string().to_owned(),
                decoder.read_string().to_owned(),
            ),
            BLOCK_ITEM_TYPE_REF_NUMBER => {
                let type_ref = decoder.read_type_ref();
                let name = if type_ref == types::TYPE_REFS_XML_ELEMENT
                    || type_ref == types::TYPE_REFS_XML_HOOK
                {
                    Some(decoder.read_key().to_owned())
                } else {
                    None
                };
                let inner_ptr = types::TypePtr::Id(ptr);
                let inner = types::Branch::new(inner_ptr, type_ref, name);
                ItemContent::Type(BranchRef::new(inner))
            }
            BLOCK_ITEM_ANY_REF_NUMBER => {
                let len = decoder.read_len() as usize;
                let mut values = Vec::with_capacity(len);
                let mut i = 0;
                while i < len {
                    values.push(decoder.read_any());
                    i += 1;
                }
                ItemContent::Any(values)
            }
            BLOCK_ITEM_DOC_REF_NUMBER => {
                ItemContent::Doc(decoder.read_string().to_owned(), decoder.read_any())
            }
            info => panic!("ItemContent::decode unrecognized info flag: {}", info),
        }
    }

    pub(crate) fn splice(&mut self, offset: usize) -> Option<ItemContent> {
        match self {
            ItemContent::Any(value) => {
                let (left, right) = value.split_at(offset);
                let left = left.to_vec();
                let right = right.to_vec();
                *self = ItemContent::Any(left);
                Some(ItemContent::Any(right))
            }
            ItemContent::String(string) => {
                let (left, right) = string.split_at(offset);
                let left = left.to_string();
                let right = right.to_string();

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

    pub(crate) fn gc(&self, txn: &Transaction) {
        match self {
            ItemContent::Type(branch_ref) => {
                let mut branch = branch_ref.borrow_mut();
                let mut curr = branch.start.take();
                while let Some(ptr) = curr {
                    if let Some(block) = txn.store.blocks.get_block_mut(&ptr) {
                        if let Block::Item(item) = block {
                            curr = item.right.clone();
                            block.gc(txn, true);
                            continue;
                        }
                    }
                    break;
                }

                for (_, ptr) in branch.map.drain() {
                    curr = Some(ptr);
                    while let Some(ptr) = curr {
                        if let Some(block) = txn.store.blocks.get_block_mut(&ptr) {
                            if let Block::Item(item) = block {
                                curr = item.left.clone();
                                block.gc(txn, true);
                                continue;
                            }
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

impl std::fmt::Display for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}", self.id)?;
        write!(f, ", parent: {}", self.parent)?;
        if let Some(origin) = self.origin.as_ref() {
            write!(f, ", origin-l: {}", origin)?;
        }
        if let Some(origin) = self.right_origin.as_ref() {
            write!(f, ", origin-r: {}", origin)?;
        }
        if let Some(left) = self.left.as_ref() {
            write!(f, ", left: {}", left.id)?;
        }
        if let Some(right) = self.right.as_ref() {
            write!(f, ", right: {}", right.id)?;
        }
        if let Some(key) = self.parent_sub.as_ref() {
            write!(f, ", '{}' =>", key)?;
        } else {
            write!(f, ":")?;
        }
        if self.is_deleted() {
            write!(f, " ~{}~)", &self.content)
        } else {
            write!(f, " '{}')", &self.content)
        }
    }
}

impl std::fmt::Display for ItemContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ItemContent::String(s) => write!(f, "{}", s),
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
            ItemContent::Deleted(s) => write!(f, "deleted({})", s),
            ItemContent::Binary(s) => write!(f, "{:?}", s),
            ItemContent::Type(t) => {
                let inner = t.borrow();
                match inner.type_ref() {
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
                    TYPE_REFS_TEXT => write!(f, "<text(head: {})>", inner.start.unwrap()),
                    TYPE_REFS_XML_ELEMENT => {
                        write!(f, "<xml element: {}>", inner.name.as_ref().unwrap())
                    }
                    TYPE_REFS_XML_FRAGMENT => write!(f, "<xml fragment>"),
                    TYPE_REFS_XML_HOOK => write!(f, "<xml hook>"),
                    TYPE_REFS_XML_TEXT => write!(f, "<xml text>"),
                    _ => write!(f, "<undefined type ref>"),
                }
            }
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
    fn into_content(self, txn: &mut Transaction, ptr: TypePtr) -> (ItemContent, Option<Self>);

    /// Method called once an original item filled with content from [Self::into_content] has been
    /// added to block store. This method is used by complex types such as maps or arrays to append
    /// the original contents of prelim struct into YMap, YArray etc.
    fn integrate(self, txn: &mut Transaction, inner_ref: BranchRef);
}

impl<T> Prelim for T
where
    T: Into<Any>,
{
    fn into_content(self, _txn: &mut Transaction, _ptr: TypePtr) -> (ItemContent, Option<Self>) {
        let value: Any = self.into();
        (ItemContent::Any(vec![value]), None)
    }

    fn integrate(self, _txn: &mut Transaction, _inner_ref: BranchRef) {}
}

#[derive(Debug)]
pub struct PrelimText(pub String);

impl Prelim for PrelimText {
    fn into_content(self, _txn: &mut Transaction, _ptr: TypePtr) -> (ItemContent, Option<Self>) {
        (ItemContent::String(self.0), None)
    }

    fn integrate(self, _txn: &mut Transaction, _inner_ref: BranchRef) {}
}

impl std::fmt::Display for ID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{}#{}>", self.client, self.clock)
    }
}

impl std::fmt::Display for BlockPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}->{})", self.id, self.pivot())
    }
}

impl std::fmt::Display for Block {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Block::Item(item) = self {
            item.fmt(f)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod test {
    use crate::block::BlockPtr;
    use crate::ID;

    #[test]
    fn block_ptr_pivot() {
        let ptr = BlockPtr::new(ID::new(1, 2), 3);
        assert_eq!(ptr.pivot(), 3);
        ptr.fix_pivot(4);
        assert_eq!(ptr.pivot(), 4);
    }
}
