use crate::store::Store;
use crate::types::TypePtr;
use crate::updates::decoder::Decoder;
use crate::updates::encoder::Encoder;
use crate::*;
use lib0::any::Any;
use std::panic;

pub const BLOCK_GC_REF_NUMBER: u8 = 0;
pub const BLOCK_ITEM_DELETED_REF_NUMBER: u8 = 1;
pub const BLOCK_ITEM_JSON_REF_NUMBER: u8 = 2;
pub const BLOCK_ITEM_BINARY_REF_NUMBER: u8 = 3;
pub const BLOCK_ITEM_STRING_REF_NUMBER: u8 = 4;
pub const BLOCK_ITEM_EMBED_REF_NUMBER: u8 = 5;
pub const BLOCK_ITEM_FORMAT_REF_NUMBER: u8 = 6;
pub const BLOCK_ITEM_TYPE_REF_NUMBER: u8 = 7;
pub const BLOCK_ITEM_ANY_REF_NUMBER: u8 = 8;
pub const BLOCK_ITEM_DOC_REF_NUMBER: u8 = 9;
pub const BLOCK_SKIP_REF_NUMBER: u8 = 10;

pub const HAS_RIGHT_ORIGIN: u8 = 0b01000000;
pub const HAS_ORIGIN: u8 = 0b10000000;
pub const HAS_PARENT_SUB: u8 = 0b00100000;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct ID {
    pub client: u64,
    pub clock: u32,
}

impl ID {
    pub fn new(client: u64, clock: u32) -> Self {
        ID { client, clock }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct BlockPtr {
    pub id: ID,
    pub pivot: u32,
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
}

#[derive(Debug, PartialEq)]
pub enum Block {
    Item(Item),
    Skip(Skip),
    GC(GC),
}

impl Block {
    pub fn as_item(&self) -> Option<&Item> {
        match self {
            Block::Item(item) => Some(item),
            _ => None,
        }
    }

    pub fn as_item_mut(&mut self) -> Option<&mut Item> {
        match self {
            Block::Item(item) => Some(item),
            _ => None,
        }
    }

    pub fn is_deleted(&self) -> bool {
        match self {
            Block::Item(item) => item.deleted,
            Block::Skip(_) => false,
            Block::GC(_) => true,
        }
    }

    pub fn integrate(&mut self, store: &mut Store, pivot: u32) {
        match self {
            Block::Item(item) => item.integrate(store, pivot),
            Block::GC(gc) => gc.integrate(store, pivot),
            Block::Skip(_) => {
                panic!("Block::Skip cannot be integrated")
            }
        }
    }

    pub fn try_merge(&mut self, other: &Self) -> bool {
        match (self, other) {
            (Block::Item(v1), Block::Item(v2)) => v1.try_merge(v2),
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

    pub fn encode<E: Encoder>(&self, store: &Store, encoder: &mut E) {
        match self {
            Block::Item(item) => {
                let info = if item.origin.is_some() { HAS_ORIGIN } else { 0 } // is left null
                    | if item.right_origin.is_some() { HAS_RIGHT_ORIGIN } else { 0 }; // is right null
                encoder.write_info(info);
                if let Some(origin_id) = item.origin.as_ref() {
                    encoder.write_left_id(origin_id);
                }
                if let Some(right_origin_id) = item.right_origin.as_ref() {
                    encoder.write_right_id(right_origin_id);
                }
                if item.origin.is_none() && item.right_origin.is_none() {
                    match &item.parent {
                        types::TypePtr::NamedRef(type_name_ref) => {
                            let type_name = store.get_type_name(*type_name_ref);
                            encoder.write_parent_info(true);
                            encoder.write_string(type_name);
                        }
                        types::TypePtr::Id(id) => {
                            encoder.write_parent_info(false);
                            encoder.write_left_id(&id.id);
                        }
                        types::TypePtr::Named(name) => {
                            encoder.write_parent_info(true);
                            encoder.write_string(name)
                        }
                    }
                }
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

    pub fn id(&self) -> &ID {
        match self {
            Block::Item(item) => &item.id,
            Block::Skip(skip) => &skip.id,
            Block::GC(gc) => &gc.id,
        }
    }

    pub fn len(&self) -> u32 {
        match self {
            Block::Item(item) => item.content.len(),
            Block::Skip(skip) => skip.len,
            Block::GC(gc) => gc.len,
        }
    }

    pub fn clock_end(&self) -> u32 {
        match self {
            Block::Item(item) => item.id.clock + item.content.len(),
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
}

#[derive(Debug, Clone)]
pub struct ItemPosition {
    pub parent: types::TypePtr,
    pub after: Option<BlockPtr>,
}

#[derive(Debug, PartialEq)]
pub struct Item {
    pub id: ID,
    pub left: Option<BlockPtr>,
    pub right: Option<BlockPtr>,
    pub origin: Option<ID>,
    pub right_origin: Option<ID>,
    pub content: ItemContent,
    pub parent: types::TypePtr,
    pub parent_sub: Option<String>,
    pub deleted: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Skip {
    pub id: ID,
    pub len: u32,
}

impl Skip {
    #[inline]
    pub fn merge(&mut self, other: &Self) {
        self.len += other.len;
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct GC {
    pub id: ID,
    pub len: u32,
}

impl GC {
    pub fn integrate(&mut self, store: &mut Store, pivot: u32) {
        if pivot > 0 {
            self.id.clock += pivot;
            self.len -= pivot;
        }
    }

    #[inline]
    pub fn merge(&mut self, other: &Self) {
        self.len += other.len;
    }
}

impl Item {
    pub fn integrate(&mut self, store: &mut Store, pivot: u32) {
        if pivot > 0 {
            self.id.clock += pivot;
            todo!()
        }

        let blocks = &mut store.blocks;
        // No conflict resolution yet..
        // We only implement the reconnection part:
        if let Some(right_id) = self.right {
            let right = blocks.get_item_mut(&right_id);
            right.left = Some(BlockPtr { pivot, id: self.id });
        }
        match self.left {
            Some(left_id) => {
                let left = blocks.get_item_mut(&left_id);
                left.right = Some(BlockPtr { pivot, id: self.id });
            }
            None => {
                let parent_type = store.init_type_from_ptr(&self.parent).unwrap();
                parent_type.start.set(Some(BlockPtr { pivot, id: self.id }));
            }
        }
    }

    pub fn len(&self) -> u32 {
        self.content.len()
    }

    pub fn split(&mut self, diff: u32) -> Item {
        let client = self.id.client;
        let clock = self.id.clock;
        let other = Item {
            id: ID::new(client, clock + diff),
            left: Some(BlockPtr::from(ID::new(client, clock + diff - 1))),
            right: self.right.clone(),
            origin: Some(ID::new(client, clock + diff - 1)),
            right_origin: self.right_origin.clone(),
            content: self.content.splice(diff as usize).unwrap(),
            parent: self.parent.clone(),
            parent_sub: self.parent_sub.clone(),
            deleted: self.deleted,
        };

        self.right = Some(BlockPtr::from(other.id));
        other
    }

    pub fn last_id(&self) -> ID {
        ID::new(self.id.client, self.id.clock + self.len() - 1)
    }

    /// Tries to merge current [Item] with another, returning true if merge was performed successfully.
    pub fn try_merge(&mut self, other: &Self) -> bool {
        if self.id.client == other.id.client
            && self.id.clock + self.len() == other.id.clock
            && other.origin == Some(self.last_id())
            && self.right == Some(BlockPtr::from(other.id.clone()))
            && self.right_origin == other.right_origin
            && self.deleted == other.deleted
            && self.content.try_merge(&other.content)
        {
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
}

#[derive(Debug, PartialEq)]
pub enum ItemContent {
    Any(Vec<Any>),
    Binary(Vec<u8>),
    Deleted(u32),
    Doc(String, Any),
    JSON(String),           // String is JSON
    Embed(String),          // String is JSON
    Format(String, String), // key, value: JSON
    String(String),
    Type(types::Inner),
}

impl ItemContent {
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

    pub fn len(&self) -> u32 {
        match self {
            ItemContent::Deleted(deleted) => *deleted,
            ItemContent::String(str) => {
                // @todo this should return the length in utf16!
                str.len() as u32
            }
            _ => 1,
        }
    }

    pub fn decode<D: Decoder>(decoder: &mut D, ref_num: u8, ptr: block::BlockPtr) -> Self {
        match ref_num & 0b1111 {
            BLOCK_ITEM_DELETED_REF_NUMBER => ItemContent::Deleted(decoder.read_len()),
            BLOCK_ITEM_JSON_REF_NUMBER => ItemContent::JSON(decoder.read_string().to_owned()),
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
                let inner = types::Inner::new(inner_ptr, name, type_ref);
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
                todo!()
            }
            _ => None,
        }
    }

    pub fn try_merge(&mut self, other: &Self) -> bool {
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
                todo!()
            }
            (ItemContent::String(v1), ItemContent::String(v2)) => {
                v1.push_str(v2.as_str());
                true
            }
            _ => false,
        }
    }
}
