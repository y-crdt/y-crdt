use crate::*;
use std::panic;
use lib0::any::Any;
use updates::decoder::UpdateDecoder;
use crate::updates::encoder::{EncoderV1, UpdateEncoder};
use lib0::binary::{BIT8, BIT7};

const BLOCK_GC_REF_NUMBER: u8 = 0;
const BLOCK_ITEM_DELETED_REF_NUMBER: u8 = 1;
const BLOCK_ITEM_JSON_REF_NUMBER: u8 = 2;
const BLOCK_ITEM_BINARY_REF_NUMBER: u8 = 3;
const BLOCK_ITEM_STRING_REF_NUMBER: u8 = 4;
const BLOCK_ITEM_EMBED_REF_NUMBER: u8 = 5;
const BLOCK_ITEM_FORMAT_REF_NUMBER: u8 = 6;
const BLOCK_ITEM_TYPE_REF_NUMBER: u8 = 7;
const BLOCK_ITEM_ANY_REF_NUMBER: u8 = 8;
const BLOCK_ITEM_DOC_REF_NUMBER: u8 = 9;
const BLOCK_SKIP_REF_NUMBER: u8 = 10;

#[derive(Copy, Clone)]
pub struct ID {
    pub client: u64,
    pub clock: u32,
}

#[derive(Copy, Clone)]
pub struct BlockPtr {
    pub id: ID,
    pub pivot: u32,
}

impl BlockPtr {
    pub fn from (id: ID) -> BlockPtr {
        BlockPtr { id, pivot: id.clock }
    }
}

pub enum Block {
    Item(Item),
    Skip(Skip),
    GC(GC),
}

impl Block {
    pub fn as_item(&self) -> Option<&Item> {
        match self {
            Block::Item(item) => Some(item),
            _ => None
        }
    }

    pub fn as_item_mut(&mut self) -> Option<&mut Item> {
        match self {
            Block::Item(item) => Some(item),
            _ => None
        }
    }

    pub fn is_deleted(&self) -> bool {
        match self {
            Block::Item(item) => item.deleted,
            Block::Skip(_) => false,
            Block::GC(_) => false,
        }
    }

    pub fn encode(&self, store: &Store, encoder: &mut EncoderV1) {
        match self {
            Block::Item(item) => {
                let info = if item.origin.is_some() { BIT8 } else { 0 } // is left null
                    | if item.right_origin.is_some() { BIT7 } else { 0 }; // is right null
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
            },
            Block::Skip(skip) => {
                encoder.write_info(10);
                encoder.write_len(skip.len);
            },
            Block::GC(gc) => {
                encoder.write_info(0);
                encoder.write_len(gc.len);
            }
        }
    }
}

pub enum ItemContent {
    Any(Any),
    Binary(Vec<u8>),
    Deleted(u32),
    Doc(String, Any),
    JSON(String), // String is JSON
    Embed(String), // String is JSON
    Format(String, String), // key, value: JSON
    String(String),
    Type(types::Inner),
}

#[derive(Clone)]
pub struct ItemPosition {
    pub parent: types::TypePtr,
    pub after: Option<BlockPtr>,
}


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

pub struct Skip {
    pub id: ID,
    pub len: u32

}
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
    pub fn len (&self) -> u32 {
        self.content.len()
    }
}

impl ItemContent {
    pub fn get_ref_number (&self) -> u8 {
        match self {
            ItemContent::Any(_) => {
                BLOCK_ITEM_ANY_REF_NUMBER
            }
            ItemContent::Binary(_) => {
                BLOCK_ITEM_BINARY_REF_NUMBER
            }
            ItemContent::Deleted(_) => {
                BLOCK_ITEM_DELETED_REF_NUMBER
            }
            ItemContent::Doc(_, _) => {
                BLOCK_ITEM_DOC_REF_NUMBER
            }
            ItemContent::JSON(_) => {
                BLOCK_ITEM_JSON_REF_NUMBER
            }
            ItemContent::Embed(_) => {
                BLOCK_ITEM_EMBED_REF_NUMBER
            }
            ItemContent::Format(_, _) => {
                BLOCK_ITEM_FORMAT_REF_NUMBER
            }
            ItemContent::String(_) => {
                BLOCK_ITEM_STRING_REF_NUMBER
            }
            ItemContent::Type(_) => {
                BLOCK_ITEM_TYPE_REF_NUMBER
            }
        }
    }

    pub fn len(&self) -> u32 {
        match self {
            ItemContent::Deleted(deletedContent) => {
                *deletedContent
            }
            ItemContent::String(str) => {
                // @todo this should return the length in utf16!
                str.len() as u32
            }
            _ => {
                1
            }
        }
    }

    pub fn write (&self) {
        match self {
            ItemContent::Any(content) => {}
            ItemContent::Binary(content) => {}
            ItemContent::Deleted(content) => {}
            ItemContent::Doc(content, _) => {}
            ItemContent::JSON(content) => {}
            ItemContent::Embed(content) => {}
            ItemContent::Format(_, _) => {}
            ItemContent::String(content) => {}
            ItemContent::Type(content) => {

            }
        }
    }
    pub fn read (update_decoder: &mut updates::decoder::DecoderV1, ref_num: u16, ptr: block::BlockPtr) -> Self {
        match ref_num {
            1 => { // Content Deleted
               ItemContent::Deleted(update_decoder.read_len())
            }
            2 => { // Content JSON
               ItemContent::JSON(update_decoder.read_string().to_owned())
            }
            3 => { // Content Binary
               ItemContent::Binary(update_decoder.read_buffer().to_owned())
            }
            4 => { // Content String
               ItemContent::String(update_decoder.read_string().to_owned())
            }
            5 => { // Content Embed
               ItemContent::Embed(update_decoder.read_string().to_owned())
            }
            6 => { // Content Format
               ItemContent::Format(update_decoder.read_string().to_owned(), update_decoder.read_string().to_owned())
            }
            7 => { // Content Type
                let type_ref = update_decoder.read_type_ref();
                let name = if type_ref == types::TypeRefs::YXmlElement || type_ref == types::TypeRefs::YXmlHook {
                    Some(update_decoder.read_key().to_owned())
                } else {
                    None
                };
                let innerPtr = types::TypePtr::Id(ptr);
                let inner = types::Inner::new(innerPtr, name, type_ref);
                ItemContent::Type(inner)
            }
            8 => { // Content Any
               ItemContent::Any(update_decoder.read_any())
            }
            9 => { // Content Doc
               ItemContent::Doc(update_decoder.read_string().to_owned(), update_decoder.read_any())
            }
            _ => { // Unknown
                panic!("Unknown content type");
            }
        }
    }
}

impl Block {
    pub fn write (&self, mut encoder: impl updates::encoder::UpdateEncoder, offset: u32) {
        match self {
            Block::Item(item) => {
                let origin = if offset > 0 { Some(ID { client: item.id.client, clock: item.id.clock }) } else { item.origin };
                encoder.write_info(
                    item.content.get_ref_number() |
                    if origin.is_none() { 0 } else { 0b10000000 } | // origin is defined
                    if item.right_origin.is_none() { 0 } else { 0b01000000 } | // right_origin is defined
                    if item.parent_sub.is_none() { 0 } else { 0b00100000 } // parent_sub is defined
                );
                if let Some(lo) = origin {
                    encoder.write_left_id(&lo);
                }
                if let Some(ro) = item.right_origin {
                    encoder.write_right_id(&ro);
                }
                if origin.is_none() && item.right_origin.is_none() {
                    match &item.parent {
                        types::TypePtr::Named(name) => {
                            // @todo write control variables here
                            encoder.write_string(name);
                        }
                        types::TypePtr::NamedRef(_) => {}
                        types::TypePtr::Id(_) => {}
                    }
                }
            }
            Block::Skip(skip) => {
                encoder.write_info(BLOCK_SKIP_REF_NUMBER);
                // write as var_uint because Skips can't make use of predilcatble length-encoding
                encoder.rest_encoder().write_var_uint(skip.len - offset)
            }
            Block::GC(gc) => {
                encoder.write_info(BLOCK_GC_REF_NUMBER);
                encoder.write_len(gc.len - offset)
            }
        }
    }
}



impl Block {
    pub fn id (&self) -> &ID {
        match self {
            Block::Item(item) => {
                &item.id
            }
            Block::Skip(skip) => {
                &skip.id
            }
            Block::GC(gc) => {
                &gc.id
            }
        }
    }
    pub fn len (&self) -> u32 {
        match self {
            Block::Item(item) => {
                item.content.len()
            }
            Block::Skip(skip) => {
                skip.len
            }
            Block::GC(gc) => {
                gc.len
            }
        }
    }
}
