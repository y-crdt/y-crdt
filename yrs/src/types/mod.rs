pub mod array;
pub mod map;
pub mod text;
pub mod xml;

use crate::*;
pub use map::Map;
pub use text::Text;

use crate::block::{Block, BlockPtr, Item, ItemContent, ItemPosition, Prelim};
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
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Hash)]
pub struct BranchPtr(NonNull<Branch>);

impl BranchPtr {
    pub(crate) fn trigger(
        &self,
        txn: &Transaction,
        subs: HashSet<Option<Rc<str>>>,
    ) -> Option<Event> {
        if let Some(observers) = self.observers.as_ref() {
            Some(observers.publish(*self, txn, subs))
        } else {
            match self.type_ref() {
                TYPE_REFS_TEXT => Some(Event::Text(TextEvent::new(*self))),
                TYPE_REFS_MAP => Some(Event::Map(MapEvent::new(*self, subs))),
                TYPE_REFS_ARRAY => Some(Event::Array(ArrayEvent::new(*self))),
                TYPE_REFS_XML_TEXT => Some(Event::XmlText(XmlTextEvent::new(*self, subs))),
                TYPE_REFS_XML_ELEMENT => Some(Event::XmlElement(XmlEvent::new(*self, subs))),
                _ => None,
            }
        }
    }

    pub(crate) fn trigger_deep(&self, txn: &Transaction, e: &Events) {
        if let Some(observers) = self.deep_observers.as_ref() {
            observers.publish(txn, e);
        }
    }
}

impl Into<TypePtr> for BranchPtr {
    fn into(self) -> TypePtr {
        TypePtr::Branch(self)
    }
}

impl Deref for BranchPtr {
    type Target = Branch;

    fn deref(&self) -> &Self::Target {
        unsafe { self.0.as_ref() }
    }
}

impl DerefMut for BranchPtr {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.0.as_mut() }
    }
}

impl<'a> From<&'a mut Box<Branch>> for BranchPtr {
    fn from(branch: &'a mut Box<Branch>) -> Self {
        let ptr = NonNull::from(branch.as_mut());
        BranchPtr(ptr)
    }
}

impl<'a> From<&'a Box<Branch>> for BranchPtr {
    fn from(branch: &'a Box<Branch>) -> Self {
        let b: &Branch = &*branch;
        unsafe {
            let ptr = NonNull::new_unchecked(b as *const Branch as *mut Branch);
            BranchPtr(ptr)
        }
    }
}

impl<'a> From<&'a Branch> for BranchPtr {
    fn from(branch: &'a Branch) -> Self {
        unsafe {
            let ptr = NonNull::new_unchecked(branch as *const Branch as *mut Branch);
            BranchPtr(ptr)
        }
    }
}

impl Into<Value> for BranchPtr {
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

impl Eq for BranchPtr {}

#[cfg(not(test))]
impl PartialEq for BranchPtr {
    fn eq(&self, other: &Self) -> bool {
        NonNull::eq(&self.0, &other.0)
    }
}

#[cfg(test)]
impl PartialEq for BranchPtr {
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
    pub(crate) start: Option<BlockPtr>,

    /// A map component of this branch node, used by some of the specialized complex types
    /// including:
    ///
    /// - [Map]: all of the map elements are based on this field. The value of each entry points
    ///   to the last modified value.
    /// - [XmlElement]: this field stores attributes assigned to a given XML node.
    pub(crate) map: HashMap<Rc<str>, BlockPtr>,

    /// Unique identifier of a current branch node. It can be contain either a named string - which
    /// means, this branch is a root-level complex data structure - or a block identifier. In latter
    /// case it means, that this branch is a complex type (eg. Map or Array) nested inside of
    /// another complex type.
    pub(crate) item: Option<BlockPtr>,

    /// A tag name identifier, used only by [XmlElement].
    pub name: Option<String>,

    /// A length of an indexed sequence component of a current branch node. Map component elements
    /// are computed on demand.
    pub block_len: u32,

    pub content_len: u32,

    /// An identifier of an underlying complex data type (eg. is it an Array or a Map).
    type_ref: TypeRefs,

    pub(crate) observers: Option<Observers>,

    pub(crate) deep_observers: Option<EventHandler<Events>>,
}

impl std::fmt::Debug for Branch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Branch")
            .field("start", &self.start)
            .field("map", &self.map)
            .field("item", &self.item)
            .field("name", &self.name)
            .field("len", &self.block_len)
            .field("type_ref", &self.type_ref)
            .finish()
    }
}

impl Eq for Branch {}

impl PartialEq for Branch {
    fn eq(&self, other: &Self) -> bool {
        self.item == other.item
            && self.start == other.start
            && self.map == other.map
            && self.name == other.name
            && self.block_len == other.block_len
            && self.type_ref == other.type_ref
    }
}

impl Branch {
    pub fn new(type_ref: TypeRefs, name: Option<String>) -> Box<Self> {
        Box::new(Self {
            start: None,
            map: HashMap::default(),
            block_len: 0,
            content_len: 0,
            item: None,
            name,
            type_ref,
            observers: None,
            deep_observers: None,
        })
    }

    /// Returns an identifier of an underlying complex data type (eg. is it an Array or a Map).
    pub fn type_ref(&self) -> TypeRefs {
        self.type_ref & 0b1111
    }

    pub(crate) fn repair_type_ref(&mut self, type_ref: TypeRefs) {
        if self.type_ref() == TYPE_REFS_UNDEFINED {
            // cleanup the TYPE_REFS_UNDEFINED bytes and set a new type ref
            self.type_ref = (type_ref & (!TYPE_REFS_UNDEFINED)) | type_ref;
        }
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
    pub(crate) fn entries(&self) -> Entries {
        Entries::new(&self.map)
    }

    /// Get iterator over Block entries of an array component of a current root type.
    /// Deleted blocks are skipped by this iterator.
    pub(crate) fn iter(&self) -> Iter {
        Iter::new(self.start.as_ref())
    }

    /// Returns a materialized value of non-deleted entry under a given `key` of a map component
    /// of a current root type.
    pub(crate) fn get(&self, key: &str) -> Option<Value> {
        let block = self.map.get(key)?;
        match block.deref() {
            Block::Item(item) if !item.is_deleted() => item.content.get_content_last(),
            _ => None,
        }
    }

    /// Given an `index` parameter, returns an item content reference which contains that index
    /// together with an offset inside of this content, which points precisely to an `index`
    /// location within wrapping item content.
    /// If `index` was outside of the array component boundary of current branch node, `None` will
    /// be returned.
    pub(crate) fn get_at(&self, mut index: u32) -> Option<(&ItemContent, usize)> {
        let mut ptr = self.start.as_ref();
        while let Some(Block::Item(item)) = ptr.map(BlockPtr::deref) {
            let len = item.len();
            if !item.is_deleted() && item.is_countable() {
                if index < len {
                    return Some((&item.content, index as usize));
                }

                index -= len;
            }
            ptr = item.right.as_ref();
        }

        None
    }

    /// Removes an entry under given `key` of a map component of a current root type, returning
    /// a materialized representation of value stored underneath if entry existed prior deletion.
    pub(crate) fn remove(&self, txn: &mut Transaction, key: &str) -> Option<Value> {
        let ptr = *self.map.get(key)?;
        let prev = match ptr.deref() {
            Block::Item(item) if !item.is_deleted() => item.content.get_content_last(),
            _ => None,
        };
        txn.delete(ptr);
        prev
    }

    /// Returns a first non-deleted item from an array component of a current root type.
    pub(crate) fn first(&self) -> Option<&Item> {
        let mut ptr = self.start.as_ref();
        while let Some(Block::Item(item)) = ptr.map(BlockPtr::deref) {
            if item.is_deleted() {
                ptr = item.right.as_ref();
            } else {
                return Some(item);
            }
        }

        None
    }

    /// Given an `index` and start block `ptr`, returns a pair of block pointers.
    ///
    /// If `index` happens to point inside of an existing block content, such block will be split at
    /// position of an `index`. In such case left tuple value contains end of a block pointer on
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
        while let Some(Block::Item(item)) = ptr.as_deref() {
            let content_len = item.content_len(encoding);
            if !item.is_deleted() && item.is_countable() {
                if index == content_len {
                    let left = ptr;
                    let right = item.right.clone();
                    return (left, right);
                } else if index < content_len {
                    let index = if let ItemContent::String(s) = &item.content {
                        s.block_offset(index, encoding)
                    } else {
                        index
                    };
                    let right = store.blocks.split_block(ptr.unwrap(), index);
                    return (ptr, right);
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
            if let Some(p) = ptr {
                let encoding = txn.store().options.offset_kind;
                if let Block::Item(item) = p.deref() {
                    if !item.is_deleted() {
                        let content_len = item.content_len(encoding);
                        let (l, r) = if remaining < content_len {
                            let offset = if let ItemContent::String(s) = &item.content {
                                s.block_offset(remaining, encoding)
                            } else {
                                remaining
                            };
                            remaining = 0;
                            let new_right = txn.store_mut().blocks.split_block(p, offset);
                            (p, new_right)
                        } else {
                            remaining -= content_len;
                            (p, item.right.clone())
                        };
                        txn.delete(l);
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
    pub(crate) fn insert_at<V: Prelim>(
        &self,
        txn: &mut Transaction,
        index: u32,
        value: V,
    ) -> BlockPtr {
        let (start, parent) = {
            if index <= self.len() {
                (self.start, BranchPtr::from(self))
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
            parent: parent.into(),
            left,
            right,
            index: 0,
            current_attrs: None,
        };

        txn.create_item(&pos, value, None)
    }

    pub(crate) fn path(from: BranchPtr, to: BranchPtr) -> Path {
        let parent = from;
        let mut child = to;
        let mut path = VecDeque::default();
        while let Some(ptr) = &child.item {
            if parent.item == child.item {
                break;
            }
            let item = ptr.as_item().unwrap();
            let item_id = item.id.clone();
            let parent_sub = item.parent_sub.clone();
            child = *item.parent.as_branch().unwrap();
            if let Some(parent_sub) = parent_sub {
                // parent is map-ish
                path.push_front(PathSegment::Key(parent_sub));
            } else {
                // parent is array-ish
                let mut i = 0;
                let mut c = child.start;
                while let Some(ptr) = c {
                    if ptr.id() == &item_id {
                        break;
                    }
                    if !ptr.is_deleted() {
                        i += 1;
                    }
                    if let Block::Item(cci) = ptr.deref() {
                        c = cci.right;
                    } else {
                        break;
                    }
                }
                path.push_front(PathSegment::Index(i));
            }
        }
        path
    }

    pub fn observe_deep<F>(&mut self, f: F) -> Subscription<Events>
    where
        F: Fn(&Transaction, &Events) -> () + 'static,
    {
        let eh = self
            .deep_observers
            .get_or_insert_with(EventHandler::default);
        eh.subscribe(f)
    }

    pub fn unobserve_deep(&mut self, subscription_id: SubscriptionId) {
        if let Some(eh) = self.deep_observers.as_mut() {
            eh.unsubscribe(subscription_id);
        }
    }
}

/// Trait implemented by all Y-types, allowing for observing events which are emitted by
/// nested types.
pub trait DeepObservable {
    /// Subscribe a callback `f` for all events emitted by this and nested collaborative types.
    /// Callback is accepting transaction which triggered that event and event itself, wrapped
    /// within an [Event] structure.
    ///
    /// This method returns a subscription, which will automatically unsubscribe current callback
    /// when dropped.
    fn observe_deep<F>(&mut self, f: F) -> Subscription<Events>
    where
        F: Fn(&Transaction, &Events) -> () + 'static;

    /// Unobserves callback identified by `subscription_id` (which can be obtained by consuming
    /// [Subscription] using `into` cast).
    fn unobserve_deep(&mut self, subscription_id: SubscriptionId);
}

impl<T> DeepObservable for T
where
    T: AsMut<Branch>,
{
    fn observe_deep<F>(&mut self, f: F) -> Subscription<Events>
    where
        F: Fn(&Transaction, &Events) -> () + 'static,
    {
        self.as_mut().observe_deep(f)
    }

    fn unobserve_deep(&mut self, subscription_id: SubscriptionId) {
        self.as_mut().unobserve_deep(subscription_id)
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
    pub fn to_json(self) -> Any {
        match self {
            Value::Any(a) => a,
            Value::YText(v) => Any::String(v.to_string().into_boxed_str()),
            Value::YArray(v) => v.to_json(),
            Value::YMap(v) => v.to_json(),
            Value::YXmlElement(v) => Any::String(v.to_string().into_boxed_str()),
            Value::YXmlText(v) => Any::String(v.to_string().into_boxed_str()),
        }
    }

    /// Converts current value into stringified representation.
    pub fn to_string(self) -> String {
        match self {
            Value::Any(a) => a.to_string(),
            Value::YText(v) => v.to_string(),
            Value::YArray(v) => v.to_json().to_string(),
            Value::YMap(v) => v.to_json().to_string(),
            Value::YXmlElement(v) => v.to_string(),
            Value::YXmlText(v) => v.to_string(),
        }
    }

    pub fn to_ytext(self) -> Option<Text> {
        if let Value::YText(text) = self {
            Some(text)
        } else {
            None
        }
    }

    pub fn to_yarray(self) -> Option<Array> {
        if let Value::YArray(array) = self {
            Some(array)
        } else {
            None
        }
    }

    pub fn to_ymap(self) -> Option<Map> {
        if let Value::YMap(map) = self {
            Some(map)
        } else {
            None
        }
    }

    pub fn to_yxml_elem(self) -> Option<XmlElement> {
        if let Value::YXmlElement(xml) = self {
            Some(xml)
        } else {
            None
        }
    }

    pub fn to_yxml_text(self) -> Option<XmlText> {
        if let Value::YXmlText(xml) = self {
            Some(xml)
        } else {
            None
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
    iter: std::collections::hash_map::Iter<'a, Rc<str>, BlockPtr>,
}

impl<'a> Entries<'a> {
    pub(crate) fn new(source: &'a HashMap<Rc<str>, BlockPtr>) -> Self {
        Entries {
            iter: source.iter(),
        }
    }
}

impl<'a> Iterator for Entries<'a> {
    type Item = (&'a str, &'a Item);

    fn next(&mut self) -> Option<Self::Item> {
        let (mut key, ptr) = self.iter.next()?;
        let mut block = ptr;
        loop {
            match block.deref() {
                Block::Item(item) if !item.is_deleted() => {
                    break;
                }
                _ => {
                    let (k, ptr) = self.iter.next()?;
                    key = k;
                    block = ptr;
                }
            }
        }
        let item = block.as_item().unwrap();
        Some((key, item))
    }
}

pub(crate) struct Iter<'a> {
    ptr: Option<&'a BlockPtr>,
}

impl<'a> Iter<'a> {
    fn new(ptr: Option<&'a BlockPtr>) -> Self {
        Iter { ptr }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a Item;

    fn next(&mut self) -> Option<Self::Item> {
        let ptr = self.ptr.take()?;
        let item = ptr.as_item()?;
        self.ptr = item.right.as_ref();
        Some(item)
    }
}

/// Type pointer - used to localize a complex [Branch] node within a scope of a document store.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum TypePtr {
    /// Temporary value - used only when block is deserialized right away, but had not been
    /// integrated into block store yet. As part of block integration process, items are
    /// repaired and their fields (including parent) are being rewired.
    Unknown,

    /// Pointer to another block. Used in nested data types ie. YMap containing another YMap.
    Branch(BranchPtr),

    /// Temporary state representing top-level type.
    Named(Rc<str>),

    /// Temporary state representing nested-level type.
    ID(ID),
}

impl TypePtr {
    pub(crate) fn as_branch(&self) -> Option<&BranchPtr> {
        if let TypePtr::Branch(ptr) = self {
            Some(ptr)
        } else {
            None
        }
    }
}

impl std::fmt::Display for TypePtr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            TypePtr::Unknown => write!(f, "unknown"),
            TypePtr::Branch(ptr) => {
                if let Some(i) = ptr.item {
                    write!(f, "{}", i.id())
                } else {
                    write!(f, "null")
                }
            }
            TypePtr::ID(id) => write!(f, "{}", id),
            TypePtr::Named(name) => write!(f, "{}", name),
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
        branch_ref: BranchPtr,
        txn: &Transaction,
        keys: HashSet<Option<Rc<str>>>,
    ) -> Event {
        match self {
            Observers::Text(eh) => {
                let e = TextEvent::new(branch_ref);
                eh.publish(txn, &e);
                Event::Text(e)
            }
            Observers::Array(eh) => {
                let e = ArrayEvent::new(branch_ref);
                eh.publish(txn, &e);
                Event::Array(e)
            }
            Observers::Map(eh) => {
                let e = MapEvent::new(branch_ref, keys);
                eh.publish(txn, &e);
                Event::Map(e)
            }
            Observers::Xml(eh) => {
                let e = XmlEvent::new(branch_ref, keys);
                eh.publish(txn, &e);
                Event::XmlElement(e)
            }
            Observers::XmlText(eh) => {
                let e = XmlTextEvent::new(branch_ref, keys);
                eh.publish(txn, &e);
                Event::XmlText(e)
            }
        }
    }
}

/// A path describing nesting structure between shared collections containing each other. It's a
/// collection of segments which refer to either index (in case of [Array] or [XmlElement]) or
/// string key (in case of [Map]) where successor shared collection can be found within subsequent
/// parent types.
pub type Path = VecDeque<PathSegment>;

/// A single segment of a [Path].
#[derive(Debug, Clone, PartialEq)]
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
    target: BranchPtr,
    keys_changed: &HashSet<Option<Rc<str>>>,
) -> HashMap<Rc<str>, EntryChange> {
    let mut keys = HashMap::new();
    for opt in keys_changed.iter() {
        if let Some(key) = opt {
            let block = target.map.get(key.as_ref()).cloned();
            if let Some(Block::Item(item)) = block.as_deref() {
                if item.id.clock >= txn.before_state.get(&item.id.client) {
                    let mut prev = item.left;
                    while let Some(Block::Item(p)) = prev.as_deref() {
                        if !txn.has_added(&p.id) {
                            break;
                        }
                        prev = p.left;
                    }

                    if txn.has_deleted(&item.id) {
                        if let Some(Block::Item(prev)) = prev.as_deref() {
                            if txn.has_deleted(&prev.id) {
                                let old_value = prev.content.get_content_last().unwrap_or_default();
                                keys.insert(key.clone(), EntryChange::Removed(old_value));
                            }
                        }
                    } else {
                        let new_value = item.content.get_content_last().unwrap();
                        if let Some(Block::Item(prev)) = prev.as_deref() {
                            if txn.has_deleted(&prev.id) {
                                let old_value = prev.content.get_content_last().unwrap_or_default();
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
                    let old_value = item.content.get_content_last().unwrap_or_default();
                    keys.insert(key.clone(), EntryChange::Removed(old_value));
                }
            }
        }
    }

    keys
}

pub(crate) fn event_change_set(txn: &Transaction, start: Option<BlockPtr>) -> ChangeSet<Change> {
    let mut added = HashSet::new();
    let mut deleted = HashSet::new();
    let mut delta = Vec::new();

    let mut last_op = None;

    let mut current = start;

    while let Some(Block::Item(item)) = current.as_deref() {
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
                inserts.append(&mut item.content.get_content());
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

        current = item.right;
    }

    match last_op.take() {
        Some(Change::Retain(_)) => { /* do nothing */ }
        Some(change) => delta.push(change),
        None => { /* do nothing */ }
    }

    ChangeSet::new(added, deleted, delta)
}

pub struct Events(Vec<NonNull<Event>>);

impl Events {
    pub(crate) fn new(events: &mut Vec<&Event>) -> Self {
        events.sort_by(|&a, &b| {
            let path1 = a.path();
            let path2 = b.path();
            path1.len().cmp(&path2.len())
        });
        let mut inner = Vec::with_capacity(events.len());
        for &e in events.iter() {
            inner.push(unsafe { NonNull::new_unchecked(e as *const Event as *mut Event) });
        }
        Events(inner)
    }

    pub fn iter(&self) -> EventsIter {
        EventsIter(self.0.iter())
    }
}

pub struct EventsIter<'a>(std::slice::Iter<'a, NonNull<Event>>);

impl<'a> Iterator for EventsIter<'a> {
    type Item = &'a Event;

    fn next(&mut self) -> Option<Self::Item> {
        let e = self.0.next()?;
        Some(unsafe { e.as_ref() })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<'a> ExactSizeIterator for EventsIter<'a> {
    fn len(&self) -> usize {
        self.0.len()
    }
}

/// Generalized wrapper around events fired by specialized shared data types.
pub enum Event {
    Text(TextEvent),
    Array(ArrayEvent),
    Map(MapEvent),
    XmlElement(XmlEvent),
    XmlText(XmlTextEvent),
}

impl Event {
    pub(crate) fn set_current_target(&mut self, target: BranchPtr) {
        match self {
            Event::Text(e) => e.current_target = target,
            Event::Array(e) => e.current_target = target,
            Event::Map(e) => e.current_target = target,
            Event::XmlElement(e) => e.current_target = target,
            Event::XmlText(e) => e.current_target = target,
        }
    }

    /// Returns a path from root type to a shared type which triggered current [Event]. This path
    /// consists of string names or indexes, which can be used to access nested type.
    pub fn path(&self) -> Path {
        match self {
            Event::Text(e) => e.path(),
            Event::Array(e) => e.path(),
            Event::Map(e) => e.path(),
            Event::XmlElement(e) => e.path(),
            Event::XmlText(e) => e.path(),
        }
    }

    /// Returns a shared data types which triggered current [Event].
    pub fn target(&self) -> Value {
        match self {
            Event::Text(e) => Value::YText(e.target().clone()),
            Event::Array(e) => Value::YArray(e.target().clone()),
            Event::Map(e) => Value::YMap(e.target().clone()),
            Event::XmlElement(e) => Value::YXmlElement(e.target().clone()),
            Event::XmlText(e) => Value::YXmlText(e.target().clone()),
        }
    }
}
