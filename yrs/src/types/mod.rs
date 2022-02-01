pub mod array;
pub mod map;
pub mod text;
pub mod xml;

use crate::*;
pub use map::Map;
pub use text::Text;

use crate::block::{BlockPtr, Item, ItemContent, ItemPosition, Prelim};
use crate::block_store::BlockStore;
use crate::event::EventHandler;
use crate::types::array::{Array, ArrayEvent};
use crate::types::map::MapEvent;
use crate::types::text::TextEvent;
use crate::types::xml::{XmlElement, XmlEvent, XmlText, XmlTextEvent};
use lib0::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Formatter;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::rc::Rc;

pub type TypeRefs = u8;

/// Type ref identifier for an [Array] type.
pub const TYPE_REFS_ARRAY: TypeRefs = 0;

/// Type ref identifier for a [Map] type.
pub const TYPE_REFS_MAP: TypeRefs = 1;

/// Type ref identifier for a [Text] type.
pub const TYPE_REFS_TEXT: TypeRefs = 2;

/// Type ref identifier for a [XmlElement] type.
pub const TYPE_REFS_XML_ELEMENT: TypeRefs = 3;

/// Type ref identifier for a [XmlFragment] type. Used for compatibility.
pub const TYPE_REFS_XML_FRAGMENT: TypeRefs = 4;

/// Type ref identifier for a [XmlHook] type. Used for compatibility.
pub const TYPE_REFS_XML_HOOK: TypeRefs = 5;

/// Type ref identifier for a [XmlText] type.
pub const TYPE_REFS_XML_TEXT: TypeRefs = 6;

/// Placeholder type ref identifier for non-specialized AbstractType. Used only for root-level types
/// which have been integrated from remote peers before they were defined locally.
pub const TYPE_REFS_UNDEFINED: TypeRefs = 15;

/// A wrapper around [Branch] cell, supplied with a bunch of convenience methods to operate on both
/// map-like and array-like contents of a [Branch].
#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct BranchRef(NonNull<Branch>);

impl Deref for BranchRef {
    type Target = Branch;

    fn deref(&self) -> &Self::Target {
        unsafe { self.0.as_ref() }
    }
}

impl DerefMut for BranchRef {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.0.as_mut() }
    }
}

impl<'a> From<&'a mut Box<Branch>> for BranchRef {
    fn from(branch: &'a mut Box<Branch>) -> Self {
        let ptr = NonNull::from(branch.as_mut());
        BranchRef(ptr)
    }
}

impl<'a> From<&'a Box<Branch>> for BranchRef {
    fn from(branch: &'a Box<Branch>) -> Self {
        let b: &Branch = &*branch;
        unsafe {
            let ptr = NonNull::new_unchecked(b as *const Branch as *mut Branch);
            BranchRef(ptr)
        }
    }
}

impl Into<Value> for BranchRef {
    /// Converts current branch data into a [Value]. It uses a type ref information to resolve,
    /// which value variant is a correct one for this branch. Since branch represent only complex
    /// types [Value::Any] will never be returned from this method.
    fn into(self) -> Value {
        match self.type_ref() {
            TYPE_REFS_ARRAY => Value::YArray(Array::from(self)),
            TYPE_REFS_MAP => Value::YMap(Map::from(self)),
            TYPE_REFS_TEXT => Value::YText(Text::from(self)),
            TYPE_REFS_XML_ELEMENT => Value::YXmlElement(XmlElement::from(self)),
            TYPE_REFS_XML_FRAGMENT => Value::YXmlElement(XmlElement::from(self)),
            TYPE_REFS_XML_TEXT => Value::YXmlText(XmlText::from(self)),
            //TYPE_REFS_XML_HOOK => Value::YXmlElement(XmlElement::from(self)),
            other => panic!("Cannot convert to value - unsupported type ref: {}", other),
        }
    }
}

impl Eq for BranchRef {}

#[cfg(not(test))]
impl PartialEq for BranchRef {
    fn eq(&self, other: &Self) -> bool {
        NonNull::eq(&self.0, &other.0)
    }
}

#[cfg(test)]
impl PartialEq for BranchRef {
    fn eq(&self, other: &Self) -> bool {
        if NonNull::eq(&self.0, &other.0) {
            true
        } else {
            let a: &Branch = self.deref();
            let b: &Branch = other.deref();
            a.eq(b)
        }
    }
}

/// Branch describes a content of a complex Yrs data structures, such as arrays or maps.
pub struct Branch {
    /// A pointer to a first block of a indexed sequence component of this branch node. If `None`,
    /// it means that sequence is empty or a branch doesn't act as an indexed sequence. Indexed
    /// sequences include:
    ///
    /// - [Array]: all elements are stored as a double linked list, while the head of the list is
    ///   kept in this field.
    /// - [XmlElement]: this field acts as a head to a first child element stored within current XML
    ///   node.
    /// - [Text] and [XmlText]: this field point to a first chunk of text appended to collaborative
    ///   text data structure.
    pub start: Option<BlockPtr>,

    /// A map component of this branch node, used by some of the specialized complex types
    /// including:
    ///
    /// - [Map]: all of the map elements are based on this field. The value of each entry points
    ///   to the last modified value.
    /// - [XmlElement]: this field stores attributes assigned to a given XML node.
    pub map: HashMap<Rc<str>, BlockPtr>,

    /// Unique identifier of a current branch node. It can be contain either a named string - which
    /// means, this branch is a root-level complex data structure - or a block identifier. In latter
    /// case it means, that this branch is a complex type (eg. Map or Array) nested inside of
    /// another complex type.
    pub ptr: TypePtr,

    /// A tag name identifier, used only by [XmlElement].
    pub name: Option<String>,

    /// A length of an indexed sequence component of a current branch node. Map component elements
    /// are computed on demand.
    pub block_len: u32,

    pub content_len: u32,

    /// An identifier of an underlying complex data type (eg. is it an Array or a Map).
    type_ref: TypeRefs,

    pub(crate) observers: Option<Observers>,
}

impl std::fmt::Debug for Branch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Branch")
            .field("start", &self.start)
            .field("map", &self.map)
            .field("ptr", &self.ptr)
            .field("name", &self.name)
            .field("len", &self.block_len)
            .field("type_ref", &self.type_ref)
            .finish()
    }
}

impl Eq for Branch {}

impl PartialEq for Branch {
    fn eq(&self, other: &Self) -> bool {
        self.ptr == other.ptr
            && self.start == other.start
            && self.map == other.map
            && self.name == other.name
            && self.block_len == other.block_len
            && self.type_ref == other.type_ref
    }
}

impl Branch {
    pub fn new(ptr: TypePtr, type_ref: TypeRefs, name: Option<String>) -> Self {
        Self {
            start: None,
            map: HashMap::default(),
            block_len: 0,
            content_len: 0,
            ptr,
            name,
            type_ref,
            observers: None,
        }
    }

    /// Returns an identifier of an underlying complex data type (eg. is it an Array or a Map).
    pub fn type_ref(&self) -> TypeRefs {
        self.type_ref & 0b1111
    }

    /// Returns a length of an indexed sequence component of a current branch node.
    /// Map component elements are computed on demand.
    pub fn len(&self) -> u32 {
        self.block_len
    }

    pub fn content_len(&self, _: &Transaction) -> u32 {
        self.content_len
    }

    /// Get iterator over (String, Block) entries of a map component of a current root type.
    /// Deleted blocks are skipped by this iterator.
    pub(crate) fn entries<'a, 'b>(&'a self, txn: &'b Transaction) -> Entries<'b> {
        Entries::new(&self.ptr, txn)
    }

    /// Get iterator over Block entries of an array component of a current root type.
    /// Deleted blocks are skipped by this iterator.
    pub(crate) fn iter<'a, 'b>(&'a self, txn: &'b Transaction) -> Iter<'b> {
        Iter::new(self.start, txn)
    }

    /// Returns a materialized value of non-deleted entry under a given `key` of a map component
    /// of a current root type.
    pub(crate) fn get(&self, txn: &Transaction, key: &str) -> Option<Value> {
        let ptr = self.map.get(key)?;
        let item = txn.store().blocks.get_item(ptr)?;
        if item.is_deleted() {
            None
        } else {
            item.content.get_content_last(txn)
        }
    }

    /// Given an `index` parameter, returns an item content reference which contains that index
    /// together with an offset inside of this content, which points precisely to an `index`
    /// location within wrapping item content.
    /// If `index` was outside of the array component boundary of current branch node, `None` will
    /// be returned.
    pub(crate) fn get_at<'a, 'b>(
        &'a self,
        blocks: &'b BlockStore,
        mut index: u32,
    ) -> Option<(&'b ItemContent, usize)> {
        let mut ptr = self.start;
        while let Some(p) = ptr {
            let item = blocks.get_item(&p)?;
            let len = item.len();
            if !item.is_deleted() && item.is_countable() {
                if index < len {
                    return Some((&item.content, index as usize));
                }

                index -= len;
            }
            ptr = item.right.clone();
        }

        None
    }

    /// Removes an entry under given `key` of a map component of a current root type, returning
    /// a materialized representation of value stored underneath if entry existed prior deletion.
    pub(crate) fn remove(&self, txn: &mut Transaction, key: &str) -> Option<Value> {
        let ptr = self.map.get(key)?;
        let prev = {
            let item = txn.store().blocks.get_item(ptr)?;
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
            let item = txn.store().blocks.get_item(&p)?;
            if item.is_deleted() {
                ptr = item.right.clone();
            } else {
                return Some(item);
            }
        }

        None
    }

    /// Given an `index` and start block `ptr`, returns a pair of block pointers.
    ///
    /// If `index` happens to point inside of an existing block content, such block will be split at
    /// position of an `index`. In such case left tuple value contains start of a block pointer on
    /// a left side of an `index` and a pointer to a block directly on the right side of an `index`.
    ///
    /// If `index` point to the end of a block and no splitting is necessary, tuple will return only
    /// left side (beginning of a block), while right side will be `None`.
    ///
    /// If `index` is outside of the range of an array component of current branch node, both tuple
    /// values will be `None`.
    fn index_to_ptr(
        txn: &mut Transaction,
        mut ptr: Option<BlockPtr>,
        mut index: u32,
    ) -> (Option<BlockPtr>, Option<BlockPtr>) {
        let store = txn.store_mut();
        let encoding = store.options.offset_kind;
        while let Some(p) = ptr {
            let item = store
                .blocks
                .get_item(&p)
                .expect("No item for a given pointer was found.");
            let content_len = item.content_len(encoding);
            if !item.is_deleted() && item.is_countable() {
                if index == content_len {
                    let left = Some(p.clone());
                    let right = item.right.clone();
                    return (left, right);
                } else if index < content_len {
                    let index = if let ItemContent::String(s) = &item.content {
                        s.block_offset(index, encoding)
                    } else {
                        index
                    };
                    let split_point = ID::new(item.id.client, item.id.clock + index);
                    let ptr = BlockPtr::new(split_point, p.pivot() as u32);
                    let (left, mut right) = store.blocks.split_block(&ptr);
                    if right.is_none() {
                        if let Some(left_ptr) = left.as_ref() {
                            if let Some(left) = store.blocks.get_item(left_ptr) {
                                right = left.right.clone();
                            }
                        }
                    }
                    return (left, right);
                }
                index -= content_len;
            }
            ptr = item.right.clone();
        }
        (None, None)
    }
    /// Removes up to a `len` of countable elements from current branch sequence, starting at the
    /// given `index`. Returns number of removed elements.
    pub(crate) fn remove_at(&self, txn: &mut Transaction, index: u32, len: u32) -> u32 {
        let mut remaining = len;
        let start = { self.start };
        let (_, mut ptr) = if index == 0 {
            (None, start)
        } else {
            Branch::index_to_ptr(txn, start, index)
        };
        while remaining > 0 {
            if let Some(mut p) = ptr {
                let encoding = txn.store().options.offset_kind;
                if let Some(item) = txn.store().blocks.get_item(&p) {
                    if !item.is_deleted() {
                        let content_len = item.content_len(encoding);
                        let (l, r) = if remaining < content_len {
                            let offset = if let ItemContent::String(s) = &item.content {
                                s.block_offset(remaining, encoding)
                            } else {
                                remaining
                            };
                            p.id.clock += offset;
                            remaining = 0;
                            txn.store_mut().blocks.split_block(&p)
                        } else {
                            remaining -= content_len;
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

        len - remaining
    }

    /// Inserts a preliminary `value` into a current branch indexed sequence component at the given
    /// `index`. Returns an item reference created as a result of this operation.
    pub(crate) fn insert_at<'t, V: Prelim>(
        &self,
        txn: &'t mut Transaction,
        index: u32,
        value: V,
    ) -> &'t Item {
        let (start, parent) = {
            if index <= self.len() {
                (self.start, self.ptr.clone())
            } else {
                panic!("Cannot insert item at index over the length of an array")
            }
        };
        let (left, right) = if index == 0 {
            (None, None)
        } else {
            Branch::index_to_ptr(txn, start, index)
        };
        let pos = ItemPosition {
            parent,
            left,
            right,
            index: 0,
            current_attrs: None,
        };

        txn.create_item(&pos, value, None)
    }

    pub(crate) fn path(from: BranchRef, to: BranchRef, txn: &Transaction) -> Path {
        let parent = from;
        let mut child = to;
        let mut path = VecDeque::default();
        while let TypePtr::Id(ptr) = &child.ptr {
            if parent.ptr == child.ptr {
                break;
            }
            let item = txn.store().blocks.get_item(ptr).unwrap();
            if let Some(parent_sub) = item.parent_sub.clone() {
                // parent is map-ish
                path.push_front(PathSegment::Key(parent_sub));
                child = txn.store().get_type(&item.parent).unwrap().clone();
            } else {
                // parent is array-ish
                let mut i = 0;
                child = txn.store().get_type(&item.parent).unwrap();
                let mut c = child.start.clone();
                while let Some(ptr) = c {
                    if ptr.id == item.id {
                        break;
                    }
                    let cc = txn.store().blocks.get_block(&ptr).unwrap();
                    if !cc.is_deleted() {
                        i += 1;
                    }
                    if let Some(cci) = cc.as_item() {
                        c = cci.right.clone();
                    } else {
                        break;
                    }
                }
                path.push_front(PathSegment::Index(i));
            }
        }
        path
    }
}

/// Value that can be returned by Yrs data types. This includes [Any] which is an extension
/// representation of JSON, but also nested complex collaborative structures specific to Yrs.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Primitive value.
    Any(Any),
    YText(Text),
    YArray(Array),
    YMap(Map),
    YXmlElement(XmlElement),
    YXmlText(XmlText),
}

impl Default for Value {
    fn default() -> Self {
        Value::Any(Any::Null)
    }
}

impl Value {
    /// Converts current value into [Any] object equivalent that resembles enhanced JSON payload.
    /// Rules are:
    ///
    /// - Primitive types ([Value::Any]) are passed right away, as no transformation is needed.
    /// - [Value::YArray] is converted into JSON-like array.
    /// - [Value::YMap] is converted into JSON-like object map.
    /// - [Value::YText], [Value::YXmlText] and [Value::YXmlElement] are converted into strings
    ///   (XML types are stringified XML representation).
    pub fn to_json(self, txn: &Transaction) -> Any {
        match self {
            Value::Any(a) => a,
            Value::YText(v) => Any::String(v.to_string(txn).into_boxed_str()),
            Value::YArray(v) => v.to_json(txn),
            Value::YMap(v) => v.to_json(txn),
            Value::YXmlElement(v) => Any::String(v.to_string(txn).into_boxed_str()),
            Value::YXmlText(v) => Any::String(v.to_string(txn).into_boxed_str()),
        }
    }

    /// Converts current value into stringified representation.
    pub fn to_string(self, txn: &Transaction) -> String {
        match self {
            Value::Any(a) => a.to_string(),
            Value::YText(v) => v.to_string(txn),
            Value::YArray(v) => v.to_json(txn).to_string(),
            Value::YMap(v) => v.to_json(txn).to_string(),
            Value::YXmlElement(v) => v.to_string(txn),
            Value::YXmlText(v) => v.to_string(txn),
        }
    }
}

impl<T> From<T> for Value
where
    T: Into<Any>,
{
    fn from(v: T) -> Self {
        let any: Any = v.into();
        Value::Any(any)
    }
}

impl std::fmt::Display for Branch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.type_ref() {
            TYPE_REFS_ARRAY => {
                if let Some(ptr) = self.start {
                    write!(f, "YArray(start: {})", ptr)
                } else {
                    write!(f, "YArray")
                }
            }
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
            TYPE_REFS_TEXT => {
                if let Some(ptr) = self.start.as_ref() {
                    write!(f, "YText(start: {})", ptr)
                } else {
                    write!(f, "YText")
                }
            }
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
            TYPE_REFS_XML_TEXT => {
                if let Some(ptr) = self.start {
                    write!(f, "YXmlText(start: {})", ptr)
                } else {
                    write!(f, "YXmlText")
                }
            }
            _ => {
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

pub(crate) struct Entries<'a> {
    pub txn: &'a Transaction,
    iter: std::collections::hash_map::Iter<'a, Rc<str>, BlockPtr>,
}

impl<'a> Entries<'a> {
    pub(crate) fn new<'b>(ptr: &'b TypePtr, txn: &'a Transaction) -> Self {
        let inner = txn.store().get_type_raw(ptr).unwrap();
        let iter = inner.map.iter();
        Entries { txn, iter }
    }
}

impl<'a> Iterator for Entries<'a> {
    type Item = (&'a str, &'a Item);

    fn next(&mut self) -> Option<Self::Item> {
        let (mut key, ptr) = self.iter.next()?;
        let mut block = self.txn.store().blocks.get_item(ptr);
        loop {
            match block {
                Some(item) if !item.is_deleted() => {
                    break;
                }
                _ => {
                    let (k, ptr) = self.iter.next()?;
                    key = k;
                    block = self.txn.store().blocks.get_item(ptr);
                }
            }
        }
        let item = block.unwrap();
        Some((key, item))
    }
}

pub(crate) struct Iter<'a> {
    ptr: Option<BlockPtr>,
    txn: &'a Transaction,
}

impl<'a> Iter<'a> {
    fn new(start: Option<BlockPtr>, txn: &'a Transaction) -> Self {
        Iter { ptr: start, txn }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a Item;

    fn next(&mut self) -> Option<Self::Item> {
        let ptr = self.ptr.take()?;
        let item = self.txn.store().blocks.get_item(&ptr)?;
        self.ptr = item.right;
        Some(item)
    }
}

/// Type pointer - used to localize a complex [Branch] node within a scope of a document store.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypePtr {
    /// Temporary value - used only when block is deserialized right away, but had not been
    /// integrated into block store yet. As part of block integration process, items are
    /// repaired and their fields (including parent) are being rewired.
    Unknown,

    /// Pointer to another block. Used in nested data types ie. YMap containing another YMap.
    Id(block::BlockPtr),

    /// Pointer to a root-level type.
    Named(Rc<str>),
}

impl std::fmt::Display for TypePtr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            TypePtr::Unknown => write!(f, "unknown"),
            TypePtr::Id(ptr) => write!(f, "{}", ptr),
            TypePtr::Named(name) => write!(f, "'{}'", name.as_ref()),
        }
    }
}

pub(crate) enum Observers {
    Text(EventHandler<crate::types::text::TextEvent>),
    Array(EventHandler<crate::types::array::ArrayEvent>),
    Map(EventHandler<crate::types::map::MapEvent>),
    Xml(EventHandler<crate::types::xml::XmlEvent>),
    XmlText(EventHandler<crate::types::xml::XmlTextEvent>),
}

impl Observers {
    pub fn text() -> Self {
        Observers::Text(EventHandler::default())
    }
    pub fn array() -> Self {
        Observers::Array(EventHandler::default())
    }
    pub fn map() -> Self {
        Observers::Map(EventHandler::default())
    }
    pub fn xml() -> Self {
        Observers::Xml(EventHandler::default())
    }
    pub fn xml_text() -> Self {
        Observers::XmlText(EventHandler::default())
    }

    pub fn publish(
        &self,
        branch_ref: BranchRef,
        txn: &Transaction,
        keys: HashSet<Option<Rc<str>>>,
    ) {
        match self {
            Observers::Text(eh) => eh.publish(txn, &TextEvent::new(branch_ref)),
            Observers::Array(eh) => eh.publish(txn, &ArrayEvent::new(branch_ref)),
            Observers::Map(eh) => eh.publish(txn, &MapEvent::new(branch_ref, keys)),
            Observers::Xml(eh) => eh.publish(txn, &XmlEvent::new(branch_ref, keys)),
            Observers::XmlText(eh) => eh.publish(txn, &XmlTextEvent::new(branch_ref, keys)),
        }
    }
}

/// A path describing nesting structure between shared collections containing each other. It's a
/// collection of segments which refer to either index (in case of [Array] or [XmlElement]) or
/// string key (in case of [Map]) where successor shared collection can be found within subsequent
/// parent types.
pub type Path = VecDeque<PathSegment>;

/// A single segment of a [Path].
pub enum PathSegment {
    /// Key segments are used to inform how to access child shared collections within a [Map] types.
    Key(Rc<str>),

    /// Index segments are used to inform how to access child shared collections within an [Array]
    /// or [XmlElement] types.
    Index(u32),
}

pub(crate) struct ChangeSet<D> {
    added: HashSet<ID>,
    deleted: HashSet<ID>,
    delta: Vec<D>,
}

impl<D> ChangeSet<D> {
    pub fn new(added: HashSet<ID>, deleted: HashSet<ID>, delta: Vec<D>) -> Self {
        ChangeSet {
            added,
            deleted,
            delta,
        }
    }
}

/// A single change done over an array-component of shared data type.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// Determines a change that resulted in adding a consecutive number of new elements:
    /// - For [Array] it's a range of inserted elements.
    /// - For [XmlElement] it's a range of inserted child XML nodes.
    Added(Vec<Value>),

    /// Determines a change that resulted in removing a consecutive range of existing elements,
    /// either XML child nodes for [XmlElement] or various elements stored in an [Array].
    Removed(u32),

    /// Determines a number of consecutive unchanged elements. Used to recognize non-edited spaces
    /// between [Change::Added] and/or [Change::Removed] chunks.
    Retain(u32),
}

/// A single change done over a map-component of shared data type.
#[derive(Debug, Clone, PartialEq)]
pub enum EntryChange {
    /// Informs about a new value inserted under specified entry.
    Inserted(Value),

    /// Informs about a change of old value (1st field) to a new one (2nd field) under
    /// a corresponding entry.
    Updated(Value, Value),

    /// Informs about a removal of a corresponding entry - contains a removed value.
    Removed(Value),
}

/// A single change done over a text-like types: [Text] or [XmlText].
#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    /// Determines a change that resulted in insertion of a piece of text, which optionally could
    /// have been formatted with provided set of attributes.
    Inserted(Value, Option<Box<Attrs>>),

    /// Determines a change that resulted in removing a consecutive range of characters.
    Deleted(u32),

    /// Determines a number of consecutive unchanged characters. Used to recognize non-edited spaces
    /// between [Delta::Inserted] and/or [Delta::Deleted] chunks. Can contain an optional set of
    /// attributes, which have been used to format an existing piece of text.
    Retain(u32, Option<Box<Attrs>>),
}

/// An alias for map of attributes used as formatting parameters by [Text] and [XmlText] types.
pub type Attrs = HashMap<Box<str>, Any>;

pub(crate) fn event_keys(
    txn: &Transaction,
    target: BranchRef,
    keys_changed: &HashSet<Option<Rc<str>>>,
) -> HashMap<Rc<str>, EntryChange> {
    let mut keys = HashMap::new();
    for opt in keys_changed.iter() {
        if let Some(key) = opt {
            let item = target
                .map
                .get(key.as_ref())
                .and_then(|ptr| txn.store().blocks.get_item(ptr));
            if let Some(item) = item {
                if item.id.clock >= txn.before_state.get(&item.id.client) {
                    let mut prev = item
                        .left
                        .as_ref()
                        .and_then(|ptr| txn.store().blocks.get_item(ptr));
                    while let Some(p) = prev {
                        if !txn.has_added(&p.id) {
                            break;
                        }
                        prev = p
                            .left
                            .as_ref()
                            .and_then(|ptr| txn.store().blocks.get_item(ptr));
                    }

                    if txn.has_deleted(&item.id) {
                        if let Some(prev) = prev {
                            if txn.has_deleted(&prev.id) {
                                let old_value =
                                    prev.content.get_content_last(txn).unwrap_or_default();
                                keys.insert(key.clone(), EntryChange::Removed(old_value));
                            }
                        }
                    } else {
                        let new_value = item.content.get_content_last(txn).unwrap();
                        if let Some(prev) = prev {
                            if txn.has_deleted(&prev.id) {
                                let old_value =
                                    prev.content.get_content_last(txn).unwrap_or_default();
                                keys.insert(
                                    key.clone(),
                                    EntryChange::Updated(old_value, new_value),
                                );

                                continue;
                            }
                        }

                        keys.insert(key.clone(), EntryChange::Inserted(new_value));
                    }
                } else if txn.has_deleted(&item.id) {
                    let old_value = item.content.get_content_last(txn).unwrap_or_default();
                    keys.insert(key.clone(), EntryChange::Removed(old_value));
                }
            }
        }
    }

    keys
}

pub(crate) fn event_change_set(txn: &Transaction, start: Option<&BlockPtr>) -> ChangeSet<Change> {
    let mut added = HashSet::new();
    let mut deleted = HashSet::new();
    let mut delta = Vec::new();

    let mut last_op = None;

    let mut current = start.and_then(|ptr| txn.store().blocks.get_item(&ptr));

    while let Some(item) = current {
        if item.is_deleted() {
            if txn.has_deleted(&item.id) && !txn.has_added(&item.id) {
                let removed = match last_op.take() {
                    None => 0,
                    Some(Change::Removed(c)) => c,
                    Some(other) => {
                        delta.push(other);
                        0
                    }
                };
                last_op = Some(Change::Removed(removed + item.len()));
                deleted.insert(item.id);
            } // else nop
        } else {
            if txn.has_added(&item.id) {
                let mut inserts = match last_op.take() {
                    None => Vec::with_capacity(item.len() as usize),
                    Some(Change::Added(values)) => values,
                    Some(other) => {
                        delta.push(other);
                        Vec::with_capacity(item.len() as usize)
                    }
                };
                inserts.append(&mut item.content.get_content(txn));
                last_op = Some(Change::Added(inserts));
                added.insert(item.id);
            } else {
                let retain = match last_op.take() {
                    None => 0,
                    Some(Change::Retain(c)) => c,
                    Some(other) => {
                        delta.push(other);
                        0
                    }
                };
                last_op = Some(Change::Retain(retain + item.len()));
            }
        }

        current = item
            .right
            .as_ref()
            .and_then(|ptr| txn.store().blocks.get_item(ptr));
    }

    match last_op.take() {
        Some(Change::Retain(_)) => { /* do nothing */ }
        Some(change) => delta.push(change),
        None => { /* do nothing */ }
    }

    ChangeSet::new(added, deleted, delta)
}
