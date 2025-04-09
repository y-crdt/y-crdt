use crate::block::{BlockCell, Item, ItemContent, ItemPosition, ItemPtr, Prelim};
use crate::types::array::ArrayEvent;
use crate::types::map::MapEvent;
use crate::types::text::TextEvent;
use crate::types::xml::{XmlEvent, XmlTextEvent};
use crate::types::{
    Entries, Event, Events, Path, PathSegment, RootRef, SharedRef, TypePtr, TypeRef,
};
use crate::{
    ArrayRef, Doc, MapRef, Observer, Origin, Out, ReadTxn, Subscription, TextRef, TransactionMut,
    WriteTxn, XmlElementRef, XmlFragmentRef, XmlTextRef, ID,
};
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::TryFrom;
use std::fmt::Formatter;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::sync::Arc;

/// A wrapper around [Branch] cell, supplied with a bunch of convenience methods to operate on both
/// map-like and array-like contents of a [Branch].
#[repr(transparent)]
#[derive(Clone, Copy, Hash)]
pub struct BranchPtr(NonNull<Branch>);

unsafe impl Send for BranchPtr {}
unsafe impl Sync for BranchPtr {}

impl BranchPtr {
    pub(crate) fn trigger(
        &self,
        txn: &TransactionMut,
        subs: HashSet<Option<Arc<str>>>,
    ) -> Option<Event> {
        let e = self.make_event(subs)?;
        self.observers.trigger(|fun| fun(txn, &e));
        Some(e)
    }

    pub(crate) fn trigger_deep(&self, txn: &TransactionMut, e: &Events) {
        self.deep_observers.trigger(|fun| fun(txn, e));
    }
}

impl TryFrom<ItemPtr> for BranchPtr {
    type Error = ItemPtr;

    fn try_from(value: ItemPtr) -> Result<Self, Self::Error> {
        if let ItemContent::Type(branch) = &value.content {
            Ok(BranchPtr::from(branch))
        } else {
            Err(value)
        }
    }
}

impl Into<TypePtr> for BranchPtr {
    fn into(self) -> TypePtr {
        TypePtr::Branch(self)
    }
}

impl Into<Origin> for BranchPtr {
    fn into(self) -> Origin {
        let addr = self.0.as_ptr() as usize;
        let bytes = addr.to_be_bytes();
        Origin::from(bytes.as_ref())
    }
}

impl AsRef<Branch> for BranchPtr {
    fn as_ref(&self) -> &Branch {
        self.deref()
    }
}

impl AsMut<Branch> for BranchPtr {
    fn as_mut(&mut self) -> &mut Branch {
        self.deref_mut()
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
        let ptr = NonNull::from(branch.as_ref());
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

impl Into<Out> for BranchPtr {
    /// Converts current branch data into a [Out]. It uses a type ref information to resolve,
    /// which value variant is a correct one for this branch. Since branch represent only complex
    /// types [Out::Any] will never be returned from this method.
    fn into(self) -> Out {
        match self.type_ref() {
            TypeRef::Array => Out::YArray(ArrayRef::from(self)),
            TypeRef::Map => Out::YMap(MapRef::from(self)),
            TypeRef::Text => Out::YText(TextRef::from(self)),
            TypeRef::XmlElement(_) => Out::YXmlElement(XmlElementRef::from(self)),
            TypeRef::XmlFragment => Out::YXmlFragment(XmlFragmentRef::from(self)),
            TypeRef::XmlText => Out::YXmlText(XmlTextRef::from(self)),
            //TYPE_REFS_XML_HOOK => Value::YXmlHook(XmlHookRef::from(self)),
            #[cfg(feature = "weak")]
            TypeRef::WeakLink(_) => Out::YWeakLink(crate::WeakRef::from(self)),
            _ => Out::UndefinedRef(self),
        }
    }
}

impl Eq for BranchPtr {}

#[cfg(not(test))]
impl PartialEq for BranchPtr {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0.as_ptr(), other.0.as_ptr())
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
        write!(f, "{:?}", self.id())
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
    pub(crate) start: Option<ItemPtr>,

    /// A map component of this branch node, used by some of the specialized complex types
    /// including:
    ///
    /// - [Map]: all of the map elements are based on this field. The value of each entry points
    ///   to the last modified value.
    /// - [XmlElement]: this field stores attributes assigned to a given XML node.
    pub(crate) map: HashMap<Arc<str>, ItemPtr>,

    /// Unique identifier of a current branch node. It can be contain either a named string - which
    /// means, this branch is a root-level complex data structure - or a block identifier. In latter
    /// case it means, that this branch is a complex type (eg. Map or Array) nested inside of
    /// another complex type.
    pub(crate) item: Option<ItemPtr>,

    /// For root-level types, this is a name of a branch.
    pub(crate) name: Option<Arc<str>>,

    /// A length of an indexed sequence component of a current branch node. Map component elements
    /// are computed on demand.
    pub block_len: u32,

    pub content_len: u32,

    /// An identifier of an underlying complex data type (eg. is it an Array or a Map).
    pub(crate) type_ref: TypeRef,

    pub(crate) observers: Observer<ObserveFn>,

    pub(crate) deep_observers: Observer<DeepObserveFn>,
}

#[cfg(feature = "sync")]
type ObserveFn = Box<dyn Fn(&TransactionMut, &Event) + Send + Sync + 'static>;
#[cfg(feature = "sync")]
type DeepObserveFn = Box<dyn Fn(&TransactionMut, &Events) + Send + Sync + 'static>;

#[cfg(not(feature = "sync"))]
type ObserveFn = Box<dyn Fn(&TransactionMut, &Event) + 'static>;
#[cfg(not(feature = "sync"))]
type DeepObserveFn = Box<dyn Fn(&TransactionMut, &Events) + 'static>;

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
            && self.block_len == other.block_len
            && self.type_ref == other.type_ref
    }
}

impl Branch {
    pub fn new(type_ref: TypeRef) -> Box<Self> {
        Box::new(Self {
            start: None,
            map: HashMap::default(),
            block_len: 0,
            content_len: 0,
            item: None,
            name: None,
            type_ref,
            observers: Observer::default(),
            deep_observers: Observer::default(),
        })
    }

    pub fn is_deleted(&self) -> bool {
        match self.item {
            Some(ptr) => ptr.is_deleted(),
            None => false,
        }
    }

    pub fn id(&self) -> BranchID {
        if let Some(ptr) = self.item {
            BranchID::Nested(ptr.id)
        } else if let Some(name) = &self.name {
            BranchID::Root(name.clone())
        } else {
            unreachable!("Could not get ID for branch")
        }
    }

    pub fn as_subdoc(&self) -> Option<Doc> {
        let item = self.item?;
        if let ItemContent::Doc(_, doc) = &item.content {
            Some(doc.clone())
        } else {
            None
        }
    }

    /// Returns an identifier of an underlying complex data type (eg. is it an Array or a Map).
    pub fn type_ref(&self) -> &TypeRef {
        &self.type_ref
    }

    pub(crate) fn repair_type_ref(&mut self, type_ref: TypeRef) {
        if self.type_ref == TypeRef::Undefined {
            self.type_ref = type_ref;
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
    pub(crate) fn get<T: ReadTxn>(&self, _txn: &T, key: &str) -> Option<Out> {
        let item = self.map.get(key)?;
        if !item.is_deleted() {
            item.content.get_last()
        } else {
            None
        }
    }

    /// Given an `index` parameter, returns an item content reference which contains that index
    /// together with an offset inside of this content, which points precisely to an `index`
    /// location within wrapping item content.
    /// If `index` was outside of the array component boundary of current branch node, `None` will
    /// be returned.
    pub(crate) fn get_at(&self, mut index: u32) -> Option<(&ItemContent, usize)> {
        let mut ptr = self.start.as_ref();
        while let Some(item) = ptr.map(ItemPtr::deref) {
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
    pub(crate) fn remove(&self, txn: &mut TransactionMut, key: &str) -> Option<Out> {
        let item = *self.map.get(key)?;
        let prev = if !item.is_deleted() {
            item.content.get_last()
        } else {
            None
        };
        txn.delete(item);
        prev
    }

    /// Returns a first non-deleted item from an array component of a current root type.
    pub(crate) fn first(&self) -> Option<&Item> {
        let mut ptr = self.start.as_ref();
        while let Some(item) = ptr.map(ItemPtr::deref) {
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
    /// If `index` is outside the range of an array component of current branch node, both tuple
    /// values will be `None`.
    fn index_to_ptr(
        txn: &mut TransactionMut,
        mut ptr: Option<ItemPtr>,
        mut index: u32,
    ) -> (Option<ItemPtr>, Option<ItemPtr>) {
        let encoding = txn.store.offset_kind;
        while let Some(item) = ptr {
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
                    let right = txn.store.blocks.split_block(item, index, encoding);
                    if let Some(_) = item.moved {
                        if let Some(src) = right {
                            if let Some(&prev_dst) = txn.prev_moved.get(&item) {
                                txn.prev_moved.insert(src, prev_dst);
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
            if let Some(item) = ptr {
                let encoding = txn.store().offset_kind;
                if !item.is_deleted() {
                    let content_len = item.content_len(encoding);
                    let (l, r) = if remaining < content_len {
                        let offset = if let ItemContent::String(s) = &item.content {
                            s.block_offset(remaining, encoding)
                        } else {
                            remaining
                        };
                        remaining = 0;
                        let new_right = txn.store.blocks.split_block(item, offset, encoding);
                        if let Some(_) = item.moved {
                            if let Some(src) = new_right {
                                if let Some(&prev_dst) = txn.prev_moved.get(&item) {
                                    txn.prev_moved.insert(src, prev_dst);
                                }
                            }
                        }
                        (item, new_right)
                    } else {
                        remaining -= content_len;
                        (item, item.right.clone())
                    };
                    txn.delete(l);
                    ptr = r;
                } else {
                    ptr = item.right.clone();
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
    ) -> Option<ItemPtr> {
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
        while let Some(item) = &child.item {
            if parent.item == child.item {
                break;
            }
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
                    if !ptr.is_deleted() && ptr.is_countable() {
                        i += ptr.len();
                    }
                    c = ptr.right;
                }
                path.push_front(PathSegment::Index(i));
            }
        }
        path
    }

    #[cfg(feature = "sync")]
    pub fn observe<F>(&mut self, f: F) -> Subscription
    where
        F: Fn(&TransactionMut, &Event) + Send + Sync + 'static,
    {
        self.observers.subscribe(Box::new(f))
    }

    #[cfg(not(feature = "sync"))]
    pub fn observe<F>(&mut self, f: F) -> Subscription
    where
        F: Fn(&TransactionMut, &Event) + 'static,
    {
        self.observers.subscribe(Box::new(f))
    }

    #[cfg(feature = "sync")]

    pub fn observe_with<F>(&mut self, key: Origin, f: F)
    where
        F: Fn(&TransactionMut, &Event) + Send + Sync + 'static,
    {
        self.observers.subscribe_with(key, Box::new(f))
    }

    #[cfg(not(feature = "sync"))]
    pub fn observe_with<F>(&mut self, key: Origin, f: F)
    where
        F: Fn(&TransactionMut, &Event) + 'static,
    {
        self.observers.subscribe_with(key, Box::new(f))
    }

    pub fn unobserve(&mut self, key: &Origin) -> bool {
        self.observers.unsubscribe(&key)
    }

    #[cfg(feature = "sync")]
    pub fn observe_deep<F>(&self, f: F) -> Subscription
    where
        F: Fn(&TransactionMut, &Events) + Send + Sync + 'static,
    {
        self.deep_observers.subscribe(Box::new(f))
    }

    #[cfg(not(feature = "sync"))]
    pub fn observe_deep<F>(&self, f: F) -> Subscription
    where
        F: Fn(&TransactionMut, &Events) + 'static,
    {
        self.deep_observers.subscribe(Box::new(f))
    }

    #[cfg(feature = "sync")]
    pub fn observe_deep_with<F>(&self, key: Origin, f: F)
    where
        F: Fn(&TransactionMut, &Events) + Send + Sync + 'static,
    {
        self.deep_observers.subscribe_with(key, Box::new(f))
    }

    #[cfg(not(feature = "sync"))]
    pub fn observe_deep_with<F>(&self, key: Origin, f: F)
    where
        F: Fn(&TransactionMut, &Events) + 'static,
    {
        self.deep_observers.subscribe_with(key, Box::new(f))
    }

    pub(crate) fn is_parent_of(&self, mut ptr: Option<ItemPtr>) -> bool {
        while let Some(i) = ptr.as_deref() {
            if let Some(parent) = i.parent.as_branch() {
                if parent.deref() == self {
                    return true;
                }
                ptr = parent.item;
            } else {
                break;
            }
        }
        false
    }

    pub(crate) fn make_event(&self, keys: HashSet<Option<Arc<str>>>) -> Option<Event> {
        let self_ptr = BranchPtr::from(self);
        let event = match self.type_ref() {
            TypeRef::Array => Event::Array(ArrayEvent::new(self_ptr)),
            TypeRef::Map => Event::Map(MapEvent::new(self_ptr, keys)),
            TypeRef::Text => Event::Text(TextEvent::new(self_ptr)),
            TypeRef::XmlElement(_) | TypeRef::XmlFragment => {
                Event::XmlFragment(XmlEvent::new(self_ptr, keys))
            }
            TypeRef::XmlText => Event::XmlText(XmlTextEvent::new(self_ptr, keys)),
            #[cfg(feature = "weak")]
            TypeRef::WeakLink(_) => Event::Weak(crate::types::weak::WeakEvent::new(self_ptr)),
            _ => return None,
        };

        Some(event)
    }
}

pub(crate) struct Iter<'a, T> {
    ptr: Option<&'a ItemPtr>,
    _txn: &'a T,
}

impl<'a, T: ReadTxn> Iter<'a, T> {
    fn new(ptr: Option<&'a ItemPtr>, txn: &'a T) -> Self {
        Iter { ptr, _txn: txn }
    }
}

impl<'a, T: ReadTxn> Iterator for Iter<'a, T> {
    type Item = &'a Item;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.ptr.take()?;
        self.ptr = item.right.as_ref();
        Some(item)
    }
}

/// A logical reference to a root-level shared collection. It can be shared across different
/// documents to reference the same logical type.
///
/// # Example
///
/// ```rust
/// use yrs::{Doc, RootRef, SharedRef, TextRef, Transact};
///
/// let root = TextRef::root("hello");
///
/// let doc1 = Doc::new();
/// let txt1 = root.get_or_create(&mut doc1.transact_mut());
///
/// let doc2 = Doc::new();
/// let txt2 = root.get_or_create(&mut doc2.transact_mut());
///
/// // instances of TextRef point to different heap objects
/// assert_ne!(&txt1 as *const _, &txt2 as *const _);
///
/// // logical descriptors of both TextRef are the same as they refer to the
/// // same logical entity
/// assert_eq!(txt1.hook(), txt2.hook());
/// ```
#[repr(transparent)]
#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct Root<S> {
    /// Unique identifier of root-level shared collection.
    pub name: Arc<str>,
    _tag: PhantomData<S>,
}

impl<S: RootRef> Root<S> {
    /// Creates a new logical reference for a root-level shared collection of a given name and type.
    /// Returned value can be used to resolve instances of root-level types by calling [Root::get]
    /// or [Root::get_or_create].
    pub fn new<N: Into<Arc<str>>>(name: N) -> Self {
        Root {
            name: name.into(),
            _tag: PhantomData::default(),
        }
    }

    /// Returns a reference to a shared root-level collection current [Root] represents, or creates
    /// it if it wasn't instantiated before.
    pub fn get_or_create<T: WriteTxn>(&self, txn: &mut T) -> S {
        let store = txn.store_mut();
        let branch = store.get_or_create_type(self.name.clone(), S::type_ref());
        S::from(branch)
    }
}

impl<S: SharedRef> Root<S> {
    /// Returns a reference to a shared collection current [Root] represents, or returns `None` if
    /// that collection hasn't been instantiated yet.
    pub fn get<T: ReadTxn>(&self, txn: &T) -> Option<S> {
        txn.store().get_type(self.name.clone()).map(S::from)
    }
}

impl<S> Into<BranchID> for Root<S> {
    fn into(self) -> BranchID {
        BranchID::Root(self.name)
    }
}

/// A logical reference used to represent a shared collection nested within another one. Unlike
/// [Root]-level types which cannot be deleted and exist eternally, [Nested] collections can be
/// added (therefore don't exist prior their instantiation) and deleted (so that any [SharedRef]
/// values referencing them become unsafe and can point to objects that no longer exists!).
///
/// Use [Nested::get] in order to materialize current nested logical reference into shared ref type.
///
/// # Example
///
/// ```rust
/// use yrs::{Doc, Map, Nested, SharedRef, TextPrelim, TextRef, Transact, WriteTxn};
///
/// let doc = Doc::new();
/// let mut txn = doc.transact_mut();
/// let root = txn.get_or_insert_map("root"); // root-level collection
/// let text = root.insert(&mut txn, "nested", TextPrelim::new("")); // nested collection
///
/// // convert nested TextRef into logical pointer
/// let nested: Nested<TextRef> = text.hook().into_nested().unwrap();
///
/// // logical reference can be used to retrieve accessible TextRef when its alive
/// assert_eq!(nested.get(&txn), Some(text));
///
/// // delete nested collection
/// root.remove(&mut txn, "nested");
///
/// // logical reference cannot resolve shared collections that have been deleted already
/// assert_eq!(nested.get(&txn), None);
/// ```
#[repr(transparent)]
#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct Nested<S> {
    pub id: ID,
    _tag: PhantomData<S>,
}

impl<S> Nested<S> {
    pub fn new(id: ID) -> Self {
        Nested {
            id,
            _tag: PhantomData::default(),
        }
    }
}

impl<S: SharedRef> Nested<S> {
    /// If current [Nested] logical reference points to an instantiated and not-deleted shared
    /// collection, a reference to that collection will be returned.
    /// If the referenced collection has been deleted or was not yet present in current transaction
    /// scope i.e. due to missing update, a `None` will be returned.  
    pub fn get<T: ReadTxn>(&self, txn: &T) -> Option<S> {
        let store = txn.store();
        let block = store.blocks.get_block(&self.id)?;
        if let BlockCell::Block(block) = block {
            if let ItemContent::Type(branch) = &block.content {
                if let Some(ptr) = branch.item {
                    if !ptr.is_deleted() {
                        return Some(S::from(BranchPtr::from(&*branch)));
                    }
                }
            }
        }
        None
    }
}

impl<S> Into<BranchID> for Nested<S> {
    fn into(self) -> BranchID {
        BranchID::Nested(self.id)
    }
}

/// A descriptor used to reference to shared collections by their unique logical identifiers,
/// which can be either [Root]-level collections or shared collections [Nested] into each other.
/// It can be resolved from any shared reference using [SharedRef::hook].
#[derive(Clone, Serialize, Deserialize)]
pub struct Hook<S> {
    id: BranchID,
    _tag: PhantomData<S>,
}

impl<S> Hook<S> {
    /// Unique logical identifier of a shared collection.
    #[inline]
    pub fn id(&self) -> &BranchID {
        &self.id
    }
}

impl<S> std::fmt::Debug for Hook<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:?}", self.id)
    }
}

impl<S> Eq for Hook<S> {}

impl<S> PartialEq for Hook<S> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<S> Hash for Hook<S> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state)
    }
}

impl<S: SharedRef> Hook<S> {
    /// Returns a reference to a shared collection current hook points to, if it exists and
    /// (in case of nested collections) has not been deleted.
    ///
    /// # Example
    ///
    /// ```rust
    /// use yrs::{Hook, Doc, Map, MapRef, Nested, SharedRef, TextPrelim, TextRef, Transact, WriteTxn};
    ///
    /// let doc = Doc::new();
    /// let mut txn = doc.transact_mut();
    /// let root = txn.get_or_insert_map("root"); // root-level collection
    /// let nested = root.insert(&mut txn, "nested", TextPrelim::new("")); // nested collection
    ///
    /// let root_hook: Hook<MapRef> = root.hook();
    /// let nested_hook: Hook<TextRef> = nested.hook();
    ///
    /// // hook can be used to retrieve collection reference as long as its alive
    /// assert_eq!(nested_hook.get(&txn), Some(nested));
    ///
    /// // after nested collection is deleted it can no longer be referenced
    /// root.remove(&mut txn, "nested");
    /// assert_eq!(nested_hook.get(&txn), None, "wtf");
    ///
    /// // descriptors work also for root types
    /// assert_eq!(root_hook.get(&txn), Some(root));
    /// ```
    pub fn get<T: ReadTxn>(&self, txn: &T) -> Option<S> {
        let branch = self.id.get_branch(txn)?;
        match branch.item {
            Some(ptr) if ptr.is_deleted() => None,
            _ => Some(S::from(branch)),
        }
    }

    /// Attempts to convert current [Hook] type into [Nested] one.
    /// Returns `None` if current descriptor doesn't reference a nested shared collection.  
    pub fn into_nested(self) -> Option<Nested<S>> {
        match self.id {
            BranchID::Nested(id) => Some(Nested::new(id)),
            BranchID::Root(_) => None,
        }
    }
}

impl<S: RootRef> Hook<S> {
    /// Attempts to convert current [Hook] type into [Root] one.
    /// Returns `None` if current descriptor doesn't reference a root-level shared collection.
    pub fn into_root(self) -> Option<Root<S>> {
        match self.id {
            BranchID::Root(name) => Some(Root::new(name)),
            BranchID::Nested(_) => None,
        }
    }
}

impl<S> From<Root<S>> for Hook<S> {
    fn from(root: Root<S>) -> Self {
        Hook {
            id: root.into(),
            _tag: PhantomData::default(),
        }
    }
}

impl<S> From<Nested<S>> for Hook<S> {
    fn from(nested: Nested<S>) -> Self {
        Hook {
            id: nested.into(),
            _tag: PhantomData::default(),
        }
    }
}

impl<S> From<BranchID> for Hook<S> {
    fn from(id: BranchID) -> Self {
        Hook {
            id,
            _tag: PhantomData::default(),
        }
    }
}

impl<S> Into<BranchID> for Hook<S> {
    fn into(self) -> BranchID {
        self.id
    }
}

/// An unique logical identifier of a shared collection. Can be shared across document boundaries
/// to reference to the same logical entity across different replicas of a document.
#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum BranchID {
    Nested(ID),
    Root(Arc<str>),
}

impl BranchID {
    #[inline]
    pub fn get_root<T: ReadTxn, K: Borrow<str>>(txn: &T, name: K) -> Option<BranchPtr> {
        txn.store().get_type(name)
    }

    pub fn get_nested<T: ReadTxn>(txn: &T, id: &ID) -> Option<BranchPtr> {
        let block = txn.store().blocks.get_block(id)?;
        if let BlockCell::Block(block) = block {
            if let ItemContent::Type(branch) = &block.content {
                return Some(BranchPtr::from(&*branch));
            }
        }
        None
    }

    pub fn get_branch<T: ReadTxn>(&self, txn: &T) -> Option<BranchPtr> {
        match self {
            BranchID::Root(name) => Self::get_root(txn, name.as_ref()),
            BranchID::Nested(id) => Self::get_nested(txn, id),
        }
    }
}

impl std::fmt::Debug for BranchID {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            BranchID::Nested(id) => write!(f, "{}", id),
            BranchID::Root(name) => write!(f, "'{}'", name),
        }
    }
}
