pub mod array;
pub mod map;
pub mod text;
pub mod xml;

use crate::*;
pub use map::Map;
pub use text::Text;

use crate::block::{BlockPtr, Item, ItemContent, ItemPosition};
use lib0::any::Any;
use std::cell::Cell;
use std::collections::HashMap;
use std::hash::Hasher;
use std::rc::Rc;

pub type TypeRefs = u8;

pub const TYPE_REFS_ARRAY: TypeRefs = 0;
pub const TYPE_REFS_MAP: TypeRefs = 1;
pub const TYPE_REFS_TEXT: TypeRefs = 2;
pub const TYPE_REFS_XML_ELEMENT: TypeRefs = 3;
pub const TYPE_REFS_XML_FRAGMENT: TypeRefs = 4;
pub const TYPE_REFS_XML_HOOK: TypeRefs = 5;
pub const TYPE_REFS_XML_TEXT: TypeRefs = 6;

/// Placeholder for non-specialized AbstractType.
pub const TYPE_REFS_UNDEFINED: TypeRefs = 7;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Inner {
    pub start: Cell<Option<BlockPtr>>,
    pub map: HashMap<String, BlockPtr>,
    pub ptr: TypePtr,
    pub name: Option<String>,
    pub len: u32,
    type_ref: TypeRefs,
}

impl Inner {
    pub fn new(ptr: TypePtr, name: Option<String>, type_ref: TypeRefs) -> Self {
        Self {
            start: Cell::from(None),
            map: HashMap::default(),
            len: 0,
            ptr,
            name,
            type_ref,
        }
    }

    pub fn type_ref(&self) -> TypeRefs {
        self.type_ref & 0b1111
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    /// Converts current root type into [Any] object equivalent that resembles enhanced JSON payload.
    pub fn to_json(&self, txn: &Transaction) -> Any {
        match self.type_ref() {
            TYPE_REFS_ARRAY => todo!(),
            TYPE_REFS_MAP => Map::to_json_inner(self, txn),
            TYPE_REFS_TEXT => Any::String(Text::to_string_inner(self, txn)),
            TYPE_REFS_XML_ELEMENT => todo!(),
            TYPE_REFS_XML_FRAGMENT => todo!(),
            TYPE_REFS_XML_HOOK => todo!(),
            TYPE_REFS_XML_TEXT => todo!(),
            other => todo!(),
        }
    }

    /// Get iterator over (String, Block) entries of a map component of a current root type.
    /// Deleted blocks are skipped by this iterator.
    pub(crate) fn entries<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> Entries<'b, 'txn> {
        Entries::new(&self.ptr, txn)
    }

    /// Get iterator over Block entries of an array component of a current root type.
    /// Deleted blocks are skipped by this iterator.
    pub(crate) fn iter<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> Iter<'b, 'txn> {
        Iter::new(self.start.get(), txn)
    }

    /// Returns a materialized value of non-deleted entry under a given `key` of a map component
    /// of a current root type.
    pub(crate) fn get(&self, txn: &Transaction<'_>, key: &str) -> Option<Any> {
        let ptr = self.map.get(key)?;
        let item = txn.store.blocks.get_item(ptr)?;
        if item.is_deleted() {
            None
        } else {
            item.content.get_content_last(txn)
        }
    }

    /// Inserts a value given `key` of a map component of a current root type.
    pub(crate) fn insert<V: Into<ItemContent>>(
        &self,
        txn: &mut Transaction,
        key: String,
        value: V,
    ) {
        let pos = {
            let left = self.map.get(&key);
            ItemPosition {
                parent: self.ptr.clone(),
                left: left.cloned(),
                right: None,
                index: 0,
            }
        };

        txn.create_item(&pos, value.into(), Some(key));
    }

    /// Removes an entry under given `key` of a map component of a current root type, returning
    /// a materialized representation of value stored underneath if entry existed prior deletion.
    pub(crate) fn remove(&self, txn: &mut Transaction, key: &str) -> Option<Any> {
        let ptr = self.map.get(key)?;
        let prev = {
            let item = txn.store.blocks.get_item(ptr)?;
            if item.is_deleted() {
                None
            } else {
                item.content.get_content_last(txn)
            }
        };
        txn.delete(ptr);
        prev
    }

    pub(crate) fn insert_at<V: Into<ItemContent>>(
        &self,
        txn: &mut Transaction,
        index: u32,
        value: V,
    ) {
        let (start, parent) = {
            if index <= self.len() {
                (self.start.get(), self.ptr.clone())
            } else {
                panic!("Cannot insert item at index over the length of an array")
            }
        };
        let (left, right) = if index == 0 {
            (None, None)
        } else {
            Self::index_to_ptr(txn, start, index)
        };
        let content = value.into();
        let pos = ItemPosition {
            parent,
            left,
            right,
            index: 0,
        };
        txn.create_item(&pos, content, None);
    }

    /// Returns a first non-deleted item from an array component of a current root type.
    pub(crate) fn first<'a, 'b>(&'a self, txn: &'b Transaction) -> Option<&'b Item> {
        let mut ptr = self.start.get();
        while let Some(p) = ptr {
            let mut item = txn.store.blocks.get_item(&p)?;
            if item.is_deleted() {
                ptr = item.right.clone();
            } else {
                return Some(item);
            }
        }

        None
    }

    fn index_to_ptr(
        txn: &mut Transaction,
        mut ptr: Option<BlockPtr>,
        mut index: u32,
    ) -> (Option<BlockPtr>, Option<BlockPtr>) {
        while let Some(p) = ptr {
            let item = txn
                .store
                .blocks
                .get_item(&p)
                .expect("No item for a given pointer was found.");
            let len = item.len();
            if !item.is_deleted() && item.is_countable() {
                if index == len {
                    let left = Some(p.clone());
                    let right = item.right.clone();
                    return (left, right);
                } else if index < len {
                    let split_point = ID::new(item.id.client, item.id.clock + index);
                    let ptr = BlockPtr::new(split_point, p.pivot() as u32);
                    let (left, mut right) = txn.store.blocks.split_block(&ptr);
                    if right.is_none() {
                        if let Some(left_ptr) = left.as_ref() {
                            if let Some(left) = txn.store.blocks.get_item(left_ptr) {
                                right = left.right.clone();
                            }
                        }
                    }
                    return (left, right);
                }
                index -= len;
            }
            ptr = item.right.clone();
        }
        (None, None)
    }
}

impl std::fmt::Display for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.type_ref() {
            TYPE_REFS_ARRAY => write!(f, "YArray(start: {})", self.start.get().unwrap()),
            TYPE_REFS_MAP => {
                write!(f, "YMap(")?;
                let mut iter = self.map.iter();
                if let Some((k, v)) = iter.next() {
                    write!(f, "'{}': {}", k, v)?;
                }
                while let Some((k, v)) = iter.next() {
                    write!(f, ", '{}': {}", k, v)?;
                }
                write!(f, ")")
            }
            TYPE_REFS_TEXT => write!(f, "YText(start: {})", self.start.get().unwrap()),
            TYPE_REFS_XML_ELEMENT => todo!(),
            TYPE_REFS_XML_FRAGMENT => todo!(),
            TYPE_REFS_XML_HOOK => todo!(),
            TYPE_REFS_XML_TEXT => todo!(),
            other => {
                write!(f, "UnknownRef")?;
                if let Some(start) = self.start.get() {
                    write!(f, "(start: {})", start)?;
                }
                if !self.map.is_empty() {
                    write!(f, " {{")?;
                    let mut iter = self.map.iter();
                    if let Some((k, v)) = iter.next() {
                        write!(f, "'{}': {}", k, v)?;
                    }
                    while let Some((k, v)) = iter.next() {
                        write!(f, ", '{}': {}", k, v)?;
                    }
                    write!(f, "}}")?;
                }
                Ok(())
            }
        }
    }
}

pub(crate) struct Entries<'a, 'txn> {
    pub txn: &'a Transaction<'txn>,
    iter: std::collections::hash_map::Iter<'a, String, BlockPtr>,
}

impl<'a, 'txn> Entries<'a, 'txn> {
    pub(crate) fn new<'b>(ptr: &'b TypePtr, txn: &'a Transaction<'txn>) -> Self {
        let rc = txn.store.get_type(ptr).unwrap();
        let cell = rc.as_ref();
        let iter = unsafe { &*cell.as_ptr() as &'a Inner }.map.iter();
        Entries { txn, iter }
    }
}

impl<'a, 'txn> Iterator for Entries<'a, 'txn> {
    type Item = (&'a String, &'a Item);

    fn next(&mut self) -> Option<Self::Item> {
        let (mut key, ptr) = self.iter.next()?;
        let mut block = self.txn.store.blocks.get_item(ptr);
        loop {
            match block {
                Some(item) if !item.is_deleted() => {
                    break;
                }
                _ => {
                    let (k, ptr) = self.iter.next()?;
                    key = k;
                    block = self.txn.store.blocks.get_item(ptr);
                }
            }
        }
        let item = block.unwrap();
        Some((key, item))
    }
}

pub(crate) struct Iter<'a, 'txn> {
    ptr: Option<BlockPtr>,
    txn: &'a Transaction<'txn>,
}

impl<'a, 'txn> Iter<'a, 'txn> {
    fn new(start: Option<BlockPtr>, txn: &'a Transaction<'txn>) -> Self {
        Iter { ptr: start, txn }
    }
}

impl<'a, 'txn> Iterator for Iter<'a, 'txn> {
    type Item = &'a Item;

    fn next(&mut self) -> Option<Self::Item> {
        let ptr = self.ptr.take()?;
        let item = self.txn.store.blocks.get_item(&ptr)?;
        self.ptr = item.right;
        Some(item)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypePtr {
    Id(block::BlockPtr),
    Named(Rc<String>),
}

#[derive(Default)]
pub(crate) struct XorHasher(u64);

impl Hasher for XorHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut i = 0;
        let mut buf = [0u8; 8];
        while i <= bytes.len() - 8 {
            buf.copy_from_slice(&bytes[i..i + 8]);
            self.0 ^= u64::from_ne_bytes(buf);
            i += 8;
        }
        while i < bytes.len() {
            self.0 ^= bytes[i] as u64;
            i += 1;
        }
    }

    fn write_u32(&mut self, value: u32) {
        self.0 ^= value as u64;
    }

    fn write_u64(&mut self, value: u64) {
        self.0 ^= value;
    }

    fn write_usize(&mut self, value: usize) {
        self.0 ^= value as u64;
    }
}
