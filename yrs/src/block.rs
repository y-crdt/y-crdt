use crate::doc::OffsetKind;
use crate::store::Store;
use crate::types::{
    Attrs, Branch, BranchRef, TypePtr, Value, TYPE_REFS_ARRAY, TYPE_REFS_MAP, TYPE_REFS_TEXT,
    TYPE_REFS_XML_ELEMENT, TYPE_REFS_XML_FRAGMENT, TYPE_REFS_XML_HOOK, TYPE_REFS_XML_TEXT,
};
use crate::updates::decoder::Decoder;
use crate::updates::encoder::Encoder;
use crate::*;
use lib0::any::Any;
use smallstr::SmallString;
use std::collections::HashSet;
use std::hash::Hash;
use std::ops::Deref;
use std::panic;
use std::pin::Pin;
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
#[derive(Debug, PartialEq)]
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

    pub fn slice(&mut self, offset: u32) -> Option<Self> {
        if offset == 0 {
            None
        } else {
            match self {
                Block::Item(item) => Some(Block::Item(item.slice(offset)?)),
                Block::Skip(skip) => {
                    let mut next = skip.clone();
                    next.id.clock += offset;
                    next.len -= offset;
                    Some(Block::Skip(next))
                }
                Block::GC(gc) => {
                    let mut next = gc.clone();
                    next.id.clock += offset;
                    next.len -= offset;
                    Some(Block::GC(next))
                }
            }
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
    pub fn integrate(&mut self, txn: &mut Transaction, pivot: u32, offset: u32) -> bool {
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
                    encoder.write_string(parent_sub.as_ref());
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
                        encoder.write_string(parent_sub.as_ref());
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

    /// Returns a length of a block. For most situation it works like [Block::content_len] with a
    /// difference to a [Text]/[XmlText] contents - in order to achieve compatibility with
    /// Yjs we need to calculate string length in terms of UTF-16 character encoding.
    /// However depending on used [Encoding] scheme we may calculate string length/offsets
    /// differently.
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
    pub current_attrs: Option<Box<Attrs>>,
}

impl ItemPosition {
    pub fn forward(&mut self, txn: &Transaction) -> bool {
        if let Some(right) = self.right.as_ref() {
            if let Some(item) = txn.store().blocks.get_item(right) {
                if !item.is_deleted() {
                    match &item.content {
                        ItemContent::String(_) | ItemContent::Embed(_) => {
                            self.index += item.len();
                        }
                        ItemContent::Format(key, value) => {
                            let attrs = self
                                .current_attrs
                                .get_or_insert_with(|| Box::new(Attrs::new()));
                            Text::update_current_attributes(attrs.as_mut(), key, value.as_ref());
                        }
                        _ => {}
                    }
                }

                self.left = self.right.take();
                self.right = item.right.clone();

                return true;
            }
        }
        false
    }

    /// If current `attributes` don't confirm the same keys as the formatting wrapping
    /// current insert position, they should be unset.
    pub fn unset_missing(&self, attributes: &mut Attrs) {
        if let Some(attrs) = self.current_attrs.as_ref() {
            // if current `attributes` don't confirm the same keys as the formatting wrapping
            // current insert position, they should be unset
            for (k, v) in attrs.iter() {
                if !attributes.contains_key(k) {
                    attributes.insert(k.clone(), Any::Null);
                }
            }
        }
    }
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
#[derive(Debug, PartialEq)]
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
    pub parent_sub: Option<Rc<str>>, //TODO: Rc since it's already used in Branch.map component

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
        parent_sub: Option<Rc<str>>,
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
    pub(crate) fn repair(&mut self, store: &mut Store) {
        if let Some(origin_id) = self.origin {
            let ptr = BlockPtr::from(origin_id);
            if let Some(item) = store.blocks.get_item(&ptr) {
                let len = item.len();
                if origin_id.clock == item.id.clock + len - 1 {
                    self.left = Some(BlockPtr::new(item.id, ptr.pivot));
                } else {
                    let ptr =
                        BlockPtr::new(ID::new(origin_id.client, origin_id.clock + 1), ptr.pivot);
                    let (l, _) = store.blocks.split_block(&ptr);
                    self.left = l;
                }
            }
        }

        if let Some(id) = self.right_origin {
            let (l, r) = store.blocks.split_block(&BlockPtr::from(id));
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
                    store.blocks.get_item(l)
                } else if let Some(r) = self.right.as_ref() {
                    store.blocks.get_item(r)
                } else {
                    None
                };

                if let Some(item) = src {
                    self.parent = item.parent.clone();
                    self.parent_sub = item.parent_sub.clone();
                }
            }
            TypePtr::Id(parent_ptr) => {
                if let Some(i) = store.blocks.get_item(parent_ptr) {
                    if let ItemContent::Type(t) = &i.content {
                        self.parent = t.ptr.clone();
                    }
                }
            }
            TypePtr::Named(_) => {}
        }
    }

    /// Integrates current block into block store.
    /// If it returns true, it means that the block should be deleted after being added to a block store.
    pub fn integrate(&mut self, txn: &mut Transaction, pivot: u32, offset: u32) -> bool {
        let mut store = txn.store_mut();
        let encoding = store.options.offset_kind;
        if offset > 0 {
            self.id.clock += offset;
            let (left, _) = store
                .blocks
                .split_block(&BlockPtr::from(ID::new(self.id.client, self.id.clock - 1)));
            if let Some(mut left) = left {
                if let Some(origin) = store.blocks.get_item(&left) {
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

        let parent = match store.get_type(&self.parent) {
            None => store.init_type_from_ptr(&self.parent),
            parent => parent,
        };

        let left = self
            .left
            .as_ref()
            .and_then(|ptr| store.blocks.get_block(ptr));

        let right = self
            .right
            .as_ref()
            .and_then(|ptr| store.blocks.get_block(ptr));

        let right_is_null_or_has_left = match right {
            None => true,
            Some(Block::Item(i)) => i.left.is_some(),
            _ => false,
        };
        let left_has_other_right_than_self = match left {
            Some(Block::Item(i)) => i.right != self.right,
            _ => false,
        };

        if let Some(mut parent_ref) = parent {
            if (left.is_none() && right_is_null_or_has_left) || left_has_other_right_than_self {
                // set the first conflicting item
                let mut o = if let Some(Block::Item(left)) = left {
                    left.right
                } else if let Some(sub) = &self.parent_sub {
                    let mut o = parent_ref.map.get(sub);
                    while let Some(ptr) = o {
                        if let Some(item) = store.blocks.get_item(ptr) {
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
                    if let Some(Block::Item(item)) = store.blocks.get_block(&ptr) {
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
                if let Some(left) = store.blocks.get_item_mut(left_id) {
                    self.right = left.right.replace(BlockPtr::new(self.id, pivot));
                }
            } else {
                let r = if let Some(parent_sub) = &self.parent_sub {
                    let start = parent_ref.map.get(parent_sub).cloned();
                    let mut r = start.as_ref();

                    while let Some(ptr) = r {
                        if let Some(item) = store.blocks.get_item(ptr) {
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
                if let Some(right) = store.blocks.get_item_mut(right_id) {
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
                parent_ref.block_len += self.len();
                parent_ref.content_len += self.content_len(encoding);
            }

            self.integrate_content(txn, pivot, &mut *parent_ref);
            txn.add_changed_type(&*parent_ref, self.parent_sub.clone());
            store = txn.store_mut();
            let parent_deleted = if let TypePtr::Id(ptr) = &self.parent {
                if let Some(item) = store.blocks.get_item(ptr) {
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
        self.content.len(OffsetKind::Utf16)
    }

    pub fn content_len(&self, kind: OffsetKind) -> u32 {
        self.content.len(kind)
    }

    pub fn slice(&mut self, diff: u32) -> Option<Item> {
        if diff == 0 {
            None
        } else {
            let client = self.id.client;
            let clock = self.id.clock;
            Some(Item {
                id: ID::new(client, clock + diff),
                left: Some(BlockPtr::from(ID::new(client, clock + diff - 1))),
                right: self.right.clone(),
                origin: Some(ID::new(client, clock + diff - 1)),
                right_origin: self.right_origin.clone(),
                content: self.content.splice(diff as usize).unwrap(),
                parent: self.parent.clone(),
                parent_sub: self.parent_sub.clone(),
                info: self.info.clone(),
            })
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

    fn info(&self) -> u8 {
        let info = if self.origin.is_some() { HAS_ORIGIN } else { 0 } // is left null
            | if self.right_origin.is_some() { HAS_RIGHT_ORIGIN } else { 0 } // is right null
            | if self.parent_sub.is_some() { HAS_PARENT_SUB } else { 0 }
            | (self.content.get_ref_number() & 0b1111);
        info
    }

    fn integrate_content(&mut self, txn: &mut Transaction, _pivot: u32, _parent: &mut Branch) {
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
            ItemContent::Type(_inner) => {
                // this.type._integrate(transaction.doc, item)
            }
            _ => {
                // other types don't define integration-specific actions
            }
        }
    }
}

#[derive(PartialEq, PartialOrd, Eq, Ord, Debug, Clone)]
pub struct SplittableString {
    content: SmallString<[u8; 8]>,
    utf16_len: usize,
}

impl SplittableString {
    pub fn len(&self, kind: OffsetKind) -> usize {
        match kind {
            OffsetKind::Bytes => self.content.len(),
            OffsetKind::Utf16 => self.utf16_len(),
            OffsetKind::Utf32 => self.unicode_len(),
        }
    }

    #[inline(always)]
    pub fn as_str(&self) -> &str {
        self.content.as_str()
    }

    #[inline(always)]
    pub fn utf16_len(&self) -> usize {
        self.utf16_len
    }

    pub fn unicode_len(&self) -> usize {
        self.content.chars().count()
    }

    pub fn split_at(&self, offset: usize, kind: OffsetKind) -> (&str, &str) {
        let off = match kind {
            OffsetKind::Bytes => offset,
            OffsetKind::Utf16 => self.map_utf16_offset(offset as u32) as usize,
            OffsetKind::Utf32 => self.map_unicode_offset(offset as u32) as usize,
        };
        self.content.split_at(off)
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
                for c in self.content.encode_utf16() {
                    if remaining == 0 {
                        break;
                    }
                    let utf8_len = if c < 0x80 { 1 } else { 2 };
                    remaining -= utf8_len;
                    i += 1;
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
        let count = str.encode_utf16().count();
        self.content.push_str(str);
        self.utf16_len += count;
    }

    fn map_utf16_offset(&self, offset: u32) -> u32 {
        let mut off = 0;
        let mut i = 0;
        for c in self.content.encode_utf16() {
            if i >= offset {
                break;
            }
            off += if c < 0x80 { 1 } else { 2 };
            i += 1;
        }
        off
    }

    fn map_unicode_offset(&self, offset: u32) -> u32 {
        let mut off = 0;
        let mut i = 0;
        for c in self.content.chars() {
            if i >= offset {
                break;
            }
            off += c.len_utf8();
            i += 1;
        }
        off as u32
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
        let utf16_len = content.encode_utf16().count();
        SplittableString { content, utf16_len }
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
    Format(Box<str>, Box<Any>),

    /// A chunk of text, usually applied by collaborative text insertion.
    String(SplittableString),

    /// A reference of a branch node. Branch nodes define a complex collection types, such as
    /// arrays, maps or XML elements.
    Type(Pin<Box<Branch>>),
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
    pub fn len(&self, kind: OffsetKind) -> u32 {
        match self {
            ItemContent::Deleted(deleted) => *deleted,
            ItemContent::String(str) => str.len(kind) as u32,
            ItemContent::Any(v) => v.len() as u32,
            ItemContent::JSON(v) => v.len() as u32,
            _ => 1,
        }
    }

    /// Returns a formatted content of an item. For complex types (represented by [BranchRef] nodes)
    /// it will return a target type of a branch node (eg. Yrs [Array], [Map] or [XmlElement]). For
    /// other types it will returns a vector of elements stored within current block. Since block
    /// may describe a chunk of values within it, it always returns a vector of values.
    pub fn get_content(&self, txn: &Transaction) -> Vec<Value> {
        match self {
            ItemContent::Any(v) => v.iter().map(|a| Value::Any(a.clone())).collect(),
            ItemContent::Binary(v) => vec![Value::Any(Any::Buffer(v.clone().into_boxed_slice()))],
            ItemContent::Deleted(_) => Vec::default(),
            ItemContent::Doc(_, v) => vec![Value::Any(*v.clone())],
            ItemContent::JSON(v) => v
                .iter()
                .map(|v| Value::Any(Any::String(v.clone().into_boxed_str())))
                .collect(),
            ItemContent::Embed(v) => vec![Value::Any(v.as_ref().clone())],
            ItemContent::Format(_, _) => Vec::default(),
            ItemContent::String(v) => v
                .chars()
                .map(|c| Value::Any(Any::String(c.to_string().into_boxed_str())))
                .collect(),
            ItemContent::Type(c) => {
                let branch_ref = BranchRef::from(c);
                vec![branch_ref.into()]
            }
        }
    }

    /// Similar to [get_content], but it only returns the latest result and doesn't materialize
    /// others for performance reasons.
    pub fn get_content_last(&self, txn: &Transaction) -> Option<Value> {
        match self {
            ItemContent::Any(v) => v.last().map(|a| Value::Any(a.clone())),
            ItemContent::Binary(v) => Some(Value::Any(Any::Buffer(v.clone().into_boxed_slice()))),
            ItemContent::Deleted(_) => None,
            ItemContent::Doc(_, v) => Some(Value::Any(*v.clone())),
            ItemContent::JSON(v) => v
                .last()
                .map(|v| Value::Any(Any::String(v.clone().into_boxed_str()))),
            ItemContent::Embed(v) => Some(Value::Any(v.as_ref().clone())),
            ItemContent::Format(_, _) => None,
            ItemContent::String(v) => Some(Value::Any(Any::String(v.clone().into()))),
            ItemContent::Type(c) => Some(BranchRef::from(c).into()),
        }
    }

    pub fn encode_with_offset<E: Encoder>(&self, encoder: &mut E, offset: u32) {
        match self {
            ItemContent::Deleted(len) => encoder.write_len(*len - offset),
            ItemContent::Binary(buf) => encoder.write_buf(buf),
            ItemContent::String(s) => encoder.write_string(&s.as_str()[(offset as usize)..]),
            ItemContent::Embed(s) => encoder.write_json(s.as_ref()),
            ItemContent::JSON(s) => {
                encoder.write_len(s.len() as u32 - offset);
                for i in (offset as usize)..s.len() {
                    encoder.write_string(s[i].as_str())
                }
            }
            ItemContent::Format(k, v) => {
                encoder.write_string(k.as_ref());
                encoder.write_json(v.as_ref());
            }
            ItemContent::Type(inner) => {
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
                encoder.write_string(key.as_ref());
                encoder.write_any(any);
            }
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
                encoder.write_string(k.as_ref());
                encoder.write_json(v.as_ref());
            }
            ItemContent::Type(inner) => {
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
                encoder.write_string(key.as_ref());
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
            BLOCK_ITEM_STRING_REF_NUMBER => ItemContent::String(decoder.read_string().into()),
            BLOCK_ITEM_EMBED_REF_NUMBER => ItemContent::Embed(decoder.read_json().into()),
            BLOCK_ITEM_FORMAT_REF_NUMBER => {
                ItemContent::Format(decoder.read_string().into(), decoder.read_json().into())
            }
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
                ItemContent::Type(inner)
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
                ItemContent::Doc(decoder.read_string().into(), Box::new(decoder.read_any()))
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
                // compute offset given in unicode code points into byte position
                let (left, right) = string.split_at(offset, OffsetKind::Utf16);
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

    pub(crate) fn gc(&mut self, txn: &Transaction) {
        match self {
            ItemContent::Type(branch) => {
                let store = txn.store();
                let mut curr = branch.start.take();
                while let Some(ptr) = curr {
                    if let Some(block) = store.blocks.get_block_mut(&ptr) {
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
                        if let Some(block) = store.blocks.get_block_mut(&ptr) {
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
                TYPE_REFS_TEXT => write!(f, "<text(head: {})>", inner.start.unwrap()),
                TYPE_REFS_XML_ELEMENT => {
                    write!(f, "<xml element: {}>", inner.name.as_ref().unwrap())
                }
                TYPE_REFS_XML_FRAGMENT => write!(f, "<xml fragment>"),
                TYPE_REFS_XML_HOOK => write!(f, "<xml hook>"),
                TYPE_REFS_XML_TEXT => write!(f, "<xml text>"),
                _ => write!(f, "<undefined type ref>"),
            },
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
pub(crate) struct PrelimText(pub SmallString<[u8; 8]>);

impl Prelim for PrelimText {
    fn into_content(self, _txn: &mut Transaction, _ptr: TypePtr) -> (ItemContent, Option<Self>) {
        (ItemContent::String(self.0.into()), None)
    }

    fn integrate(self, _txn: &mut Transaction, _inner_ref: BranchRef) {}
}

#[derive(Debug)]
pub(crate) struct PrelimEmbed(pub Any);

impl Prelim for PrelimEmbed {
    fn into_content(self, _txn: &mut Transaction, _ptr: TypePtr) -> (ItemContent, Option<Self>) {
        (ItemContent::Embed(Box::new(self.0)), None)
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
    use crate::block::{BlockPtr, SplittableString};
    use crate::doc::OffsetKind;
    use crate::ID;
    use std::ops::Deref;

    #[test]
    fn block_ptr_pivot() {
        let ptr = BlockPtr::new(ID::new(1, 2), 3);
        assert_eq!(ptr.pivot(), 3);
        ptr.fix_pivot(4);
        assert_eq!(ptr.pivot(), 4);
    }

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

        let (a, b) = s.split_at(18, OffsetKind::Utf32);
        assert_eq!(a, "Zażółć gęślą jaźń😀");
        assert_eq!(b, "ありがとうございます");

        let (a, b) = s.split_at(19, OffsetKind::Utf16);
        assert_eq!(a, "Zażółć gęślą jaźń😀");
        assert_eq!(b, "ありがとうございます");

        let (a, b) = s.split_at(30, OffsetKind::Bytes);
        assert_eq!(a, "Zażółć gęślą jaźń😀");
        assert_eq!(b, "ありがとうございます");
    }
}
