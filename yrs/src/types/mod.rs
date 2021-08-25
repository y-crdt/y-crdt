pub mod array;
pub mod map;
pub mod text;
pub mod xml;

use crate::*;
pub use map::Map;
pub use text::Text;

use crate::block::{BlockPtr, Item, ItemContent};
use crate::types::xml::XmlElement;
use lib0::any::Any;
use std::cell::{BorrowMutError, Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::fmt::Formatter;
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

#[derive(Debug, Clone)]
pub struct InnerRef(Rc<RefCell<Inner>>);

impl InnerRef {
    pub fn new(inner: Inner) -> Self {
        InnerRef(Rc::new(RefCell::new(inner)))
    }

    pub fn borrow(&self) -> Ref<Inner> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<Inner> {
        self.0.borrow_mut()
    }

    pub fn try_borrow_mut(&self) -> Result<RefMut<Inner>, BorrowMutError> {
        self.0.try_borrow_mut()
    }

    pub(crate) fn remove_at(&self, txn: &mut Transaction, index: u32, mut len: u32) {
        let start = {
            let parent = self.borrow();
            parent.start
        };
        let (_, mut ptr) = if index == 0 {
            (None, start)
        } else {
            Inner::index_to_ptr(txn, start, index)
        };
        while len > 0 {
            if let Some(mut p) = ptr {
                if let Some(item) = txn.store.blocks.get_item(&p) {
                    if !item.is_deleted() {
                        let item_len = item.len();
                        let (l, r) = if len < item_len {
                            p.id.clock += len;
                            len = 0;
                            txn.store.blocks.split_block(&p)
                        } else {
                            len -= item_len;
                            (ptr, item.right.clone())
                        };
                        txn.delete(&l.unwrap());
                        ptr = r;
                    } else {
                        ptr = item.right.clone();
                    }
                }
            } else {
                break;
            }
        }

        if len > 0 {
            panic!("Array length exceeded");
        }
    }
}

impl AsRef<Inner> for InnerRef {
    fn as_ref<'a>(&'a self) -> &'a Inner {
        unsafe { &*self.0.as_ptr() as &'a Inner }
    }
}

impl Eq for InnerRef {}

#[cfg(not(test))]
impl PartialEq for InnerRef {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

#[cfg(test)]
impl PartialEq for InnerRef {
    fn eq(&self, other: &Self) -> bool {
        if Rc::ptr_eq(&self.0, &other.0) {
            true
        } else {
            self.0.borrow().eq(&other.0.borrow())
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Inner {
    pub start: Option<BlockPtr>,
    pub map: HashMap<String, BlockPtr>,
    pub ptr: TypePtr,
    pub name: Option<String>,
    pub item: Option<BlockPtr>,
    pub len: u32,
    type_ref: TypeRefs,
}

impl Inner {
    pub fn new(ptr: TypePtr, type_ref: TypeRefs, name: Option<String>) -> Self {
        Self {
            start: None,
            map: HashMap::default(),
            len: 0,
            item: None,
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
            TYPE_REFS_ARRAY => {
                let values = self
                    .iter(txn)
                    .flat_map(|i| i.content.get_content(txn))
                    .collect();
                Any::Array(values)
            }
            TYPE_REFS_MAP => Map::to_json_inner(self, txn),
            TYPE_REFS_TEXT => Any::String(Text::to_string_inner(self, txn)),
            TYPE_REFS_XML_ELEMENT => Any::String(XmlElement::to_string_inner(self, txn)),
            TYPE_REFS_XML_FRAGMENT => Any::String(XmlElement::to_string_inner(self, txn)),
            TYPE_REFS_XML_HOOK => Map::to_json_inner(self, txn),
            TYPE_REFS_XML_TEXT => Any::String(Text::to_string_inner(self, txn)),
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
        Iter::new(self.start, txn)
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

    pub(crate) fn get_at<'a, 'b>(
        &'a self,
        txn: &'b Transaction,
        mut index: u32,
    ) -> Option<(&'b ItemContent, usize)> {
        let mut ptr = self.start;
        while let Some(p) = ptr {
            let item = txn.store.blocks.get_item(&p)?;
            let len = item.len();
            if !item.is_deleted() && item.is_countable() {
                if index < len {
                    return Some((&item.content, index as usize));
                }
            }
            index -= len;
            ptr = item.right.clone();
        }

        None
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

    /// Returns a first non-deleted item from an array component of a current root type.
    pub(crate) fn first<'a, 'b>(&'a self, txn: &'b Transaction) -> Option<&'b Item> {
        let mut ptr = self.start;
        while let Some(p) = ptr {
            let item = txn.store.blocks.get_item(&p)?;
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
            TYPE_REFS_ARRAY => write!(f, "YArray(start: {})", self.start.unwrap()),
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
            TYPE_REFS_TEXT => write!(f, "YText(start: {})", self.start.unwrap()),
            TYPE_REFS_XML_ELEMENT => {
                write!(f, "YXmlElement")?;
                if let Some(start) = self.start.as_ref() {
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
            TYPE_REFS_XML_HOOK => {
                write!(f, "YXmlHook(")?;
                let mut iter = self.map.iter();
                if let Some((k, v)) = iter.next() {
                    write!(f, "'{}': {}", k, v)?;
                }
                while let Some((k, v)) = iter.next() {
                    write!(f, ", '{}': {}", k, v)?;
                }
                write!(f, ")")
            }
            TYPE_REFS_XML_TEXT => write!(f, "YXmlText(start: {})", self.start.unwrap()),
            other => {
                write!(f, "UnknownRef")?;
                if let Some(start) = self.start.as_ref() {
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
        let inner = txn.store.get_type(ptr).unwrap();
        let iter = inner.as_ref().map.iter();
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

impl std::fmt::Display for TypePtr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            TypePtr::Id(ptr) => write!(f, "{}", ptr),
            TypePtr::Named(name) => write!(f, "'{}'", name),
        }
    }
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
