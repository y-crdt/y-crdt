pub mod array;
pub mod map;
pub mod text;
pub mod xml;

use crate::*;
pub use map::Map;
pub use map::MapRef;
use std::borrow::Borrow;
use std::cell::RefCell;
pub use text::Text;
pub use text::TextRef;

use crate::block::{Block, BlockPtr, Item, ItemContent, ItemPosition, Prelim};
use crate::transaction::TransactionMut;
use crate::types::array::{ArrayEvent, ArrayRef};
use crate::types::map::MapEvent;
use crate::types::text::TextEvent;
use crate::types::xml::{XmlElementRef, XmlEvent, XmlTextEvent, XmlTextRef};
use atomic_refcell::AtomicRefCell;
use lib0::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::TryInto;
use std::fmt::Formatter;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::{Arc, Weak};

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

pub trait Observable: AsMut<Branch> {
    type Event;

    fn try_observer(&self) -> Option<&EventHandler<Self::Event>>;
    fn try_observer_mut(&mut self) -> Option<&mut EventHandler<Self::Event>>;

    /// Subscribes a given callback to be triggered whenever current y-type is changed.
    /// A callback is triggered whenever a transaction gets committed. This function does not
    /// trigger if changes have been observed by nested shared collections.
    ///
    /// All array-like event changes can be tracked by using [Event::delta] method.
    /// All map-like event changes can be tracked by using [Event::keys] method.
    /// All text-like event changes can be tracked by using [TextEvent::delta] method.
    ///
    /// Returns a [Subscription] which, when dropped, will unsubscribe current callback.
    fn observe<F>(&mut self, f: F) -> Subscription<Arc<dyn Fn(&TransactionMut, &Self::Event) -> ()>>
    where
        F: Fn(&TransactionMut, &Self::Event) -> () + 'static,
    {
        if let Some(eh) = self.try_observer_mut() {
            eh.subscribe(Arc::new(f))
        } else {
            panic!("Observed collection is of different type") //TODO: this should be Result::Err
        }
    }

    /// Unsubscribes a previously subscribed event callback identified by given `subscription_id`.
    fn unobserve(&self, subscription_id: SubscriptionId) {
        if let Some(eh) = self.try_observer() {
            eh.unsubscribe(subscription_id);
        }
    }
}

/// Trait implemented by shared types to display their contents in string format.
pub trait GetString {
    /// Displays the content of a current collection in string format.
    fn get_string<T: ReadTxn>(&self, txn: &T) -> String;
}

/// A wrapper around [Branch] cell, supplied with a bunch of convenience methods to operate on both
/// map-like and array-like contents of a [Branch].
#[repr(transparent)]
#[derive(Clone, Copy, Hash)]
pub struct BranchPtr(NonNull<Branch>);

impl BranchPtr {
    pub(crate) fn trigger(
        &self,
        txn: &TransactionMut,
        subs: HashSet<Option<Rc<str>>>,
    ) -> Option<Event> {
        if let Some(observers) = self.observers.as_ref() {
            Some(observers.publish(*self, txn, subs))
        } else {
            let type_ref = self.type_ref();
            match type_ref {
                TYPE_REFS_TEXT => Some(Event::Text(TextEvent::new(*self))),
                TYPE_REFS_MAP => Some(Event::Map(MapEvent::new(*self, subs))),
                TYPE_REFS_ARRAY => Some(Event::Array(ArrayEvent::new(*self))),
                TYPE_REFS_XML_TEXT => Some(Event::XmlText(XmlTextEvent::new(*self, subs))),
                TYPE_REFS_XML_ELEMENT | TYPE_REFS_XML_FRAGMENT => {
                    Some(Event::XmlFragment(XmlEvent::new(*self, subs)))
                }
                _ => None,
            }
        }
    }

    pub(crate) fn trigger_deep(&self, txn: &TransactionMut, e: &Events) {
        if let Some(o) = self.deep_observers.as_ref() {
            for fun in o.callbacks() {
                fun(txn, e);
            }
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

        let ptr = unsafe { NonNull::new_unchecked(b as *const Branch as *mut Branch) };
        BranchPtr(ptr)
    }
}

impl<'a> From<&'a Branch> for BranchPtr {
    fn from(branch: &'a Branch) -> Self {
        let ptr = unsafe { NonNull::new_unchecked(branch as *const Branch as *mut Branch) };
        BranchPtr(ptr)
    }
}

impl Into<Value> for BranchPtr {
    /// Converts current branch data into a [Value]. It uses a type ref information to resolve,
    /// which value variant is a correct one for this branch. Since branch represent only complex
    /// types [Value::Any] will never be returned from this method.
    fn into(self) -> Value {
        match self.type_ref() {
            TYPE_REFS_ARRAY => Value::YArray(ArrayRef::from(self)),
            TYPE_REFS_MAP => Value::YMap(MapRef::from(self)),
            TYPE_REFS_TEXT => Value::YText(TextRef::from(self)),
            TYPE_REFS_XML_ELEMENT => Value::YXmlElement(XmlElementRef::from(self)),
            TYPE_REFS_XML_FRAGMENT => Value::YXmlElement(XmlElementRef::from(self)),
            TYPE_REFS_XML_TEXT => Value::YXmlText(XmlTextRef::from(self)),
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

impl std::fmt::Debug for BranchPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let branch: &Branch = &self;
        write!(f, "{}", branch)
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

    pub(crate) store: Option<Weak<AtomicRefCell<Store>>>,

    /// A tag name identifier, used only by [XmlElement].
    pub name: Option<Rc<str>>,

    /// A length of an indexed sequence component of a current branch node. Map component elements
    /// are computed on demand.
    pub block_len: u32,

    pub content_len: u32,

    /// An identifier of an underlying complex data type (eg. is it an Array or a Map).
    type_ref: TypeRefs,

    pub(crate) observers: Option<Observers>,

    pub(crate) deep_observers: Option<Observer<Arc<dyn Fn(&TransactionMut, &Events)>>>,
}

impl std::fmt::Debug for Branch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
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
    pub fn new(type_ref: TypeRefs, name: Option<Rc<str>>) -> Box<Self> {
        Box::new(Self {
            start: None,
            map: HashMap::default(),
            block_len: 0,
            content_len: 0,
            item: None,
            store: None,
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

    pub fn content_len(&self) -> u32 {
        self.content_len
    }

    /// Get iterator over (String, Block) entries of a map component of a current root type.
    /// Deleted blocks are skipped by this iterator.
    pub(crate) fn entries<'a, T: ReadTxn + 'a>(&'a self, txn: &'a T) -> Entries<'a, &'a T, T> {
        Entries::from_ref(&self.map, txn)
    }

    /// Get iterator over Block entries of an array component of a current root type.
    /// Deleted blocks are skipped by this iterator.
    pub(crate) fn iter<'a, T: ReadTxn + 'a>(&'a self, txn: &'a T) -> Iter<'a, T> {
        Iter::new(self.start.as_ref(), txn)
    }

    /// Returns a materialized value of non-deleted entry under a given `key` of a map component
    /// of a current root type.
    pub(crate) fn get<T: ReadTxn>(&self, txn: &T, key: &str) -> Option<Value> {
        let block = self.map.get(key)?;
        match block.deref() {
            Block::Item(item) if !item.is_deleted() => item.content.get_last(),
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
    pub(crate) fn remove(&self, txn: &mut TransactionMut, key: &str) -> Option<Value> {
        let ptr = *self.map.get(key)?;
        let prev = match ptr.deref() {
            Block::Item(item) if !item.is_deleted() => item.content.get_last(),
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
        txn: &mut TransactionMut,
        mut ptr: Option<BlockPtr>,
        mut index: u32,
    ) -> (Option<BlockPtr>, Option<BlockPtr>) {
        let encoding = txn.store.options.offset_kind;
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
                    let p = ptr.unwrap();
                    let right = txn.store.blocks.split_block(p, index, encoding);
                    if let Block::Item(item) = p.deref() {
                        if let Some(_) = item.moved {
                            if let Some(src) = right {
                                if let Some(&prev_dst) = txn.prev_moved.get(&p) {
                                    txn.prev_moved.insert(src, prev_dst);
                                }
                            }
                        }
                    }
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
    pub(crate) fn remove_at(&self, txn: &mut TransactionMut, index: u32, len: u32) -> u32 {
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
                            let new_right = txn.store.blocks.split_block(p, offset, encoding);
                            if let Block::Item(item) = p.deref() {
                                if let Some(_) = item.moved {
                                    if let Some(src) = new_right {
                                        if let Some(&prev_dst) = txn.prev_moved.get(&p) {
                                            txn.prev_moved.insert(src, prev_dst);
                                        }
                                    }
                                }
                            }
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
        txn: &mut TransactionMut,
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

    pub fn observe_deep<F>(&mut self, f: F) -> DeepEventsSubscription
    where
        F: Fn(&TransactionMut, &Events) -> () + 'static,
    {
        let eh = self.deep_observers.get_or_insert_with(Observer::default);
        eh.subscribe(Arc::new(f))
    }

    pub fn unobserve_deep(&mut self, subscription_id: SubscriptionId) {
        if let Some(eh) = self.deep_observers.as_mut() {
            eh.unsubscribe(subscription_id);
        }
    }
}

pub type DeepEventsSubscription = crate::Subscription<Arc<dyn Fn(&TransactionMut, &Events) -> ()>>;

/// Trait implemented by all Y-types, allowing for observing events which are emitted by
/// nested types.
pub trait DeepObservable {
    /// Subscribe a callback `f` for all events emitted by this and nested collaborative types.
    /// Callback is accepting transaction which triggered that event and event itself, wrapped
    /// within an [Event] structure.
    ///
    /// This method returns a subscription, which will automatically unsubscribe current callback
    /// when dropped.
    fn observe_deep<F>(&mut self, f: F) -> DeepEventsSubscription
    where
        F: Fn(&TransactionMut, &Events) -> () + 'static;

    /// Unobserves callback identified by `subscription_id` (which can be obtained by consuming
    /// [Subscription] using `into` cast).
    fn unobserve_deep(&mut self, subscription_id: SubscriptionId);
}

impl<T> DeepObservable for T
where
    T: AsMut<Branch>,
{
    fn observe_deep<F>(&mut self, f: F) -> DeepEventsSubscription
    where
        F: Fn(&TransactionMut, &Events) -> () + 'static,
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
    YText(TextRef),
    YArray(ArrayRef),
    YMap(MapRef),
    YXmlElement(XmlElementRef),
    YXmlFragment(XmlFragmentRef),
    YXmlText(XmlTextRef),
}

impl Default for Value {
    fn default() -> Self {
        Value::Any(Any::Null)
    }
}

impl Value {
    /// Converts current value into stringified representation.
    pub fn to_string<T: ReadTxn>(self, txn: &T) -> String {
        match self {
            Value::Any(a) => a.to_string(),
            Value::YText(v) => v.get_string(txn),
            Value::YArray(v) => v.to_json(txn).to_string(),
            Value::YMap(v) => v.to_json(txn).to_string(),
            Value::YXmlElement(v) => v.get_string(txn),
            Value::YXmlFragment(v) => v.get_string(txn),
            Value::YXmlText(v) => v.get_string(txn),
        }
    }

    pub fn to_ytext(self) -> Option<TextRef> {
        if let Value::YText(text) = self {
            Some(text)
        } else {
            None
        }
    }

    pub fn to_yarray(self) -> Option<ArrayRef> {
        if let Value::YArray(array) = self {
            Some(array)
        } else {
            None
        }
    }

    pub fn to_ymap(self) -> Option<MapRef> {
        if let Value::YMap(map) = self {
            Some(map)
        } else {
            None
        }
    }

    pub fn to_yxml_elem(self) -> Option<XmlElementRef> {
        if let Value::YXmlElement(xml) = self {
            Some(xml)
        } else {
            None
        }
    }

    pub fn to_yxml_fragment(self) -> Option<XmlFragmentRef> {
        if let Value::YXmlFragment(xml) = self {
            Some(xml)
        } else {
            None
        }
    }

    pub fn to_yxml_text(self) -> Option<XmlTextRef> {
        if let Value::YXmlText(xml) = self {
            Some(xml)
        } else {
            None
        }
    }
}

impl TryInto<TextRef> for Value {
    type Error = Self;

    fn try_into(self) -> Result<TextRef, Self::Error> {
        if let Value::YText(value) = self {
            Ok(value)
        } else {
            Err(self)
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

impl ToJson for Value {
    /// Converts current value into [Any] object equivalent that resembles enhanced JSON payload.
    /// Rules are:
    ///
    /// - Primitive types ([Value::Any]) are passed right away, as no transformation is needed.
    /// - [Value::YArray] is converted into JSON-like array.
    /// - [Value::YMap] is converted into JSON-like object map.
    /// - [Value::YText], [Value::YXmlText] and [Value::YXmlElement] are converted into strings
    ///   (XML types are stringified XML representation).
    fn to_json<T: ReadTxn>(&self, txn: &T) -> Any {
        match self {
            Value::Any(a) => a.clone(),
            Value::YText(v) => Any::String(v.get_string(txn).into_boxed_str()),
            Value::YArray(v) => v.to_json(txn),
            Value::YMap(v) => v.to_json(txn),
            Value::YXmlElement(v) => Any::String(v.get_string(txn).into_boxed_str()),
            Value::YXmlText(v) => Any::String(v.get_string(txn).into_boxed_str()),
            Value::YXmlFragment(v) => Any::String(v.get_string(txn).into_boxed_str()),
        }
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
            TYPE_REFS_XML_FRAGMENT => {
                write!(f, "YXmlFragment")?;
                if let Some(start) = self.start.as_ref() {
                    write!(f, "(start: {})", start)?;
                }
                Ok(())
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

#[derive(Debug)]
pub(crate) struct Entries<'a, B, T> {
    iter: std::collections::hash_map::Iter<'a, Rc<str>, BlockPtr>,
    txn: B,
    _marker: PhantomData<T>,
}

impl<'a, B, T: ReadTxn> Entries<'a, B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
    pub fn new(source: &'a HashMap<Rc<str>, BlockPtr>, txn: B) -> Self {
        Entries {
            iter: source.iter(),
            txn,
            _marker: PhantomData::default(),
        }
    }
}

impl<'a, T: ReadTxn> Entries<'a, T, T>
where
    T: Borrow<T> + ReadTxn,
{
    pub fn from(source: &'a HashMap<Rc<str>, BlockPtr>, txn: T) -> Self {
        Entries::new(source, txn)
    }
}

impl<'a, T: ReadTxn> Entries<'a, &'a T, T>
where
    T: Borrow<T> + ReadTxn,
{
    pub fn from_ref(source: &'a HashMap<Rc<str>, BlockPtr>, txn: &'a T) -> Self {
        Entries::new(source, txn)
    }
}

impl<'a, B, T> Iterator for Entries<'a, B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
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

pub(crate) struct Iter<'a, T> {
    ptr: Option<&'a BlockPtr>,
    _txn: &'a T,
}

impl<'a, T: ReadTxn> Iter<'a, T> {
    fn new(ptr: Option<&'a BlockPtr>, txn: &'a T) -> Self {
        Iter { ptr, _txn: txn }
    }
}

impl<'a, T: ReadTxn> Iterator for Iter<'a, T> {
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
pub enum TypePtr {
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

type EventHandler<T> = Observer<Arc<dyn Fn(&TransactionMut, &T) -> ()>>;

pub(crate) enum Observers {
    Text(EventHandler<crate::types::text::TextEvent>),
    Array(EventHandler<crate::types::array::ArrayEvent>),
    Map(EventHandler<crate::types::map::MapEvent>),
    XmlFragment(EventHandler<crate::types::xml::XmlEvent>),
    XmlText(EventHandler<crate::types::xml::XmlTextEvent>),
}

impl Observers {
    pub fn text() -> Self {
        Observers::Text(Observer::default())
    }
    pub fn array() -> Self {
        Observers::Array(Observer::default())
    }
    pub fn map() -> Self {
        Observers::Map(Observer::default())
    }
    pub fn xml_fragment() -> Self {
        Observers::XmlFragment(Observer::default())
    }
    pub fn xml_text() -> Self {
        Observers::XmlText(Observer::default())
    }

    pub fn publish(
        &self,
        branch_ref: BranchPtr,
        txn: &TransactionMut,
        keys: HashSet<Option<Rc<str>>>,
    ) -> Event {
        match self {
            Observers::Text(eh) => {
                let e = TextEvent::new(branch_ref);
                for fun in eh.callbacks() {
                    fun(txn, &e);
                }
                Event::Text(e)
            }
            Observers::Array(eh) => {
                let e = ArrayEvent::new(branch_ref);
                for fun in eh.callbacks() {
                    fun(txn, &e);
                }
                Event::Array(e)
            }
            Observers::Map(eh) => {
                let e = MapEvent::new(branch_ref, keys);
                for fun in eh.callbacks() {
                    fun(txn, &e);
                }
                Event::Map(e)
            }
            Observers::XmlFragment(eh) => {
                let e = XmlEvent::new(branch_ref, keys);
                for fun in eh.callbacks() {
                    fun(txn, &e);
                }
                Event::XmlFragment(e)
            }
            Observers::XmlText(eh) => {
                let e = XmlTextEvent::new(branch_ref, keys);
                for fun in eh.callbacks() {
                    fun(txn, &e);
                }
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
pub type Attrs = HashMap<Rc<str>, Any>;

pub(crate) fn event_keys(
    txn: &TransactionMut,
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
                                let old_value = prev.content.get_last().unwrap_or_default();
                                keys.insert(key.clone(), EntryChange::Removed(old_value));
                            }
                        }
                    } else {
                        let new_value = item.content.get_last().unwrap();
                        if let Some(Block::Item(prev)) = prev.as_deref() {
                            if txn.has_deleted(&prev.id) {
                                let old_value = prev.content.get_last().unwrap_or_default();
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
                    let old_value = item.content.get_last().unwrap_or_default();
                    keys.insert(key.clone(), EntryChange::Removed(old_value));
                }
            }
        }
    }

    keys
}

pub(crate) fn event_change_set(txn: &TransactionMut, start: Option<BlockPtr>) -> ChangeSet<Change> {
    let mut added = HashSet::new();
    let mut deleted = HashSet::new();
    let mut delta = Vec::new();

    let mut moved_stack = Vec::new();
    let mut curr_move: Option<BlockPtr> = None;
    let mut curr_move_is_new = false;
    let mut curr_move_is_deleted = false;
    let mut curr_move_end: Option<BlockPtr> = None;
    let mut last_op = None;

    #[derive(Default)]
    struct MoveStackItem {
        end: Option<BlockPtr>,
        moved: Option<BlockPtr>,
        is_new: bool,
        is_deleted: bool,
    }

    fn is_moved_by_new(ptr: Option<BlockPtr>, txn: &TransactionMut) -> bool {
        let mut moved = ptr;
        while let Some(Block::Item(item)) = moved.as_deref() {
            if txn.has_added(&item.id) {
                return true;
            } else {
                moved = item.moved;
            }
        }

        false
    }

    let encoding = txn.store().options.offset_kind;
    let mut current = start;
    loop {
        if current == curr_move_end && curr_move.is_some() {
            current = curr_move;
            let item: MoveStackItem = moved_stack.pop().unwrap_or_default();
            curr_move_is_new = item.is_new;
            curr_move_is_deleted = item.is_deleted;
            curr_move = item.moved;
            curr_move_end = item.end;
        } else {
            if let Some(ptr) = current {
                if let Block::Item(item) = ptr.deref() {
                    if let ItemContent::Move(m) = &item.content {
                        if item.moved == curr_move {
                            moved_stack.push(MoveStackItem {
                                end: curr_move_end,
                                moved: curr_move,
                                is_new: curr_move_is_new,
                                is_deleted: curr_move_is_deleted,
                            });
                            let txn = unsafe {
                                //TODO: remove this - find a way to work with get_moved_coords
                                // without need for &mut Transaction
                                (txn as *const TransactionMut as *mut TransactionMut)
                                    .as_mut()
                                    .unwrap()
                            };
                            let (start, end) = m.get_moved_coords(txn);
                            curr_move = current;
                            curr_move_end = end;
                            curr_move_is_new = curr_move_is_new || txn.has_added(&item.id);
                            curr_move_is_deleted = curr_move_is_deleted || item.is_deleted();
                            current = start;
                            continue; // do not move to item.right
                        }
                    } else if item.moved != curr_move {
                        if !curr_move_is_new
                            && item.is_countable()
                            && (!item.is_deleted() || txn.has_deleted(&item.id))
                            && !txn.has_added(&item.id)
                            && (item.moved.is_none()
                                || curr_move_is_deleted
                                || is_moved_by_new(item.moved, txn))
                            && (txn.prev_moved.get(&ptr).cloned() == curr_move)
                        {
                            match item.moved {
                                Some(ptr) if txn.has_added(ptr.id()) => {
                                    let len = item.content_len(encoding);
                                    last_op = match last_op.take() {
                                        Some(Change::Removed(i)) => Some(Change::Removed(i + len)),
                                        Some(op) => {
                                            delta.push(op);
                                            Some(Change::Removed(len))
                                        }
                                        None => Some(Change::Removed(len)),
                                    };
                                }
                                _ => {}
                            }
                        }
                    } else if item.is_deleted() {
                        if !curr_move_is_new
                            && txn.has_deleted(&item.id)
                            && !txn.has_added(&item.id)
                            && !txn.prev_moved.contains_key(&ptr)
                        {
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
                        if curr_move_is_new
                            || txn.has_added(&item.id)
                            || txn.prev_moved.contains_key(&ptr)
                        {
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
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        current = if let Some(Block::Item(i)) = current.as_deref() {
            i.right
        } else {
            None
        };
    }

    match last_op.take() {
        None | Some(Change::Retain(_)) => { /* do nothing */ }
        Some(change) => delta.push(change),
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
    XmlFragment(XmlEvent),
    XmlText(XmlTextEvent),
}

impl Event {
    pub(crate) fn set_current_target(&mut self, target: BranchPtr) {
        match self {
            Event::Text(e) => e.current_target = target,
            Event::Array(e) => e.current_target = target,
            Event::Map(e) => e.current_target = target,
            Event::XmlText(e) => e.current_target = target,
            Event::XmlFragment(e) => e.current_target = target,
        }
    }

    /// Returns a path from root type to a shared type which triggered current [Event]. This path
    /// consists of string names or indexes, which can be used to access nested type.
    pub fn path(&self) -> Path {
        match self {
            Event::Text(e) => e.path(),
            Event::Array(e) => e.path(),
            Event::Map(e) => e.path(),
            Event::XmlText(e) => e.path(),
            Event::XmlFragment(e) => e.path(),
        }
    }

    /// Returns a shared data types which triggered current [Event].
    pub fn target(&self) -> Value {
        match self {
            Event::Text(e) => Value::YText(e.target().clone()),
            Event::Array(e) => Value::YArray(e.target().clone()),
            Event::Map(e) => Value::YMap(e.target().clone()),
            Event::XmlText(e) => Value::YXmlText(e.target().clone()),
            Event::XmlFragment(e) => match e.target() {
                XmlNode::Element(n) => Value::YXmlElement(n.clone()),
                XmlNode::Fragment(n) => Value::YXmlFragment(n.clone()),
                XmlNode::Text(n) => Value::YXmlText(n.clone()),
            },
        }
    }
}

pub trait ToJson {
    /// Converts all contents of a current type into a JSON-like representation.
    fn to_json<T: ReadTxn>(&self, txn: &T) -> Any;
}
