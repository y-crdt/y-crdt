use crate::store::Store;
use crate::updates::encoder::{EncoderV1, UpdateEncoder};
use crate::*;
use lib0::any::Any;
use std::panic;
use updates::decoder::UpdateDecoder;

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
    pub fn from(id: ID) -> BlockPtr {
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

    pub fn encode(&self, store: &Store, encoder: &mut EncoderV1) {
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

#[derive(Debug, PartialEq, Eq)]
pub struct GC {
    pub id: ID,
    pub len: u32,
}

impl Item {
    #[inline(always)]
    pub fn integrate(&self, store: &mut Store, pivot: u32) {
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

    pub fn decode(
        decoder: &mut updates::decoder::DecoderV1,
        ref_num: u8,
        ptr: block::BlockPtr,
    ) -> Self {
        match ref_num & 0b1111 {
            BLOCK_ITEM_DELETED_REF_NUMBER => ItemContent::Deleted(decoder.read_len()),
            BLOCK_ITEM_JSON_REF_NUMBER => ItemContent::JSON(decoder.read_string().to_owned()),
            BLOCK_ITEM_BINARY_REF_NUMBER => ItemContent::Binary(decoder.read_buffer().to_owned()),
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
}
