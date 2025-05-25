use serde::{Serialize, Serializer};
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Formatter;
use std::marker::PhantomData;
use std::sync::Arc;

pub use map::Map;
pub use map::MapRef;
pub use text::Text;
pub use text::TextRef;

use crate::block::{Item, ItemContent, ItemPtr, Prelim};
use crate::branch::{Branch, BranchPtr};
use crate::encoding::read::Error;
use crate::transaction::TransactionMut;
use crate::types::array::{ArrayEvent, ArrayRef};
use crate::types::map::MapEvent;
use crate::types::text::TextEvent;
#[cfg(feature = "weak")]
use crate::types::weak::{LinkSource, WeakEvent, WeakRef};
use crate::types::xml::{XmlElementRef, XmlEvent, XmlTextEvent, XmlTextRef};
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::*;

pub mod array;
pub mod map;
pub mod text;
#[cfg(feature = "weak")]
pub mod weak;
pub mod xml;

/// Type ref identifier for an [ArrayRef] type.
pub const TYPE_REFS_ARRAY: u8 = 0;

/// Type ref identifier for a [MapRef] type.
pub const TYPE_REFS_MAP: u8 = 1;

/// Type ref identifier for a [TextRef] type.
pub const TYPE_REFS_TEXT: u8 = 2;

/// Type ref identifier for a [XmlElementRef] type.
pub const TYPE_REFS_XML_ELEMENT: u8 = 3;

/// Type ref identifier for a [XmlFragmentRef] type. Used for compatibility.
pub const TYPE_REFS_XML_FRAGMENT: u8 = 4;

/// Type ref identifier for a [XmlHookRef] type. Used for compatibility.
pub const TYPE_REFS_XML_HOOK: u8 = 5;

/// Type ref identifier for a [XmlTextRef] type.
pub const TYPE_REFS_XML_TEXT: u8 = 6;

/// Type ref identifier for a [WeakRef] type.
pub const TYPE_REFS_WEAK: u8 = 7;

/// Type ref identifier for a [DocRef] type.
pub const TYPE_REFS_DOC: u8 = 9;

/// Placeholder type ref identifier for non-specialized AbstractType. Used only for root-level types
/// which have been integrated from remote peers before they were defined locally.
pub const TYPE_REFS_UNDEFINED: u8 = 15;

#[repr(u8)]
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TypeRef {
    Array = TYPE_REFS_ARRAY,
    Map = TYPE_REFS_MAP,
    Text = TYPE_REFS_TEXT,
    XmlElement(Arc<str>) = TYPE_REFS_XML_ELEMENT,
    XmlFragment = TYPE_REFS_XML_FRAGMENT,
    XmlHook = TYPE_REFS_XML_HOOK,
    XmlText = TYPE_REFS_XML_TEXT,
    SubDoc = TYPE_REFS_DOC,
    #[cfg(feature = "weak")]
    WeakLink(Arc<LinkSource>) = TYPE_REFS_WEAK,
    Undefined = TYPE_REFS_UNDEFINED,
}

impl TypeRef {
    pub fn kind(&self) -> u8 {
        match self {
            TypeRef::Array => TYPE_REFS_ARRAY,
            TypeRef::Map => TYPE_REFS_MAP,
            TypeRef::Text => TYPE_REFS_TEXT,
            TypeRef::XmlElement(_) => TYPE_REFS_XML_ELEMENT,
            TypeRef::XmlFragment => TYPE_REFS_XML_FRAGMENT,
            TypeRef::XmlHook => TYPE_REFS_XML_HOOK,
            TypeRef::XmlText => TYPE_REFS_XML_TEXT,
            TypeRef::SubDoc => TYPE_REFS_DOC,
            #[cfg(feature = "weak")]
            TypeRef::WeakLink(_) => TYPE_REFS_WEAK,
            TypeRef::Undefined => TYPE_REFS_UNDEFINED,
        }
    }

    #[cfg(feature = "weak")]
    fn encode_weak_link<E: Encoder>(data: &LinkSource, encoder: &mut E) {
        encoder.write_type_ref(TYPE_REFS_WEAK);
        let mut info = 0u8;
        let is_single = data.is_single();
        if !is_single {
            info |= WEAK_REF_FLAGS_QUOTE;
        };
        if data.quote_start.is_root() || data.quote_end.is_root() {
            info |= WEAK_REF_FLAGS_PARENT_ROOT;
        }
        if !data.quote_start.is_relative() {
            info |= WEAK_REF_FLAGS_START_UNBOUNDED;
        }
        if !data.quote_end.is_relative() {
            info |= WEAK_REF_FLAGS_END_UNBOUNDED;
        }
        if data.quote_start.assoc == Assoc::After {
            info |= WEAK_REF_FLAGS_START_ASSOC;
        }
        if data.quote_end.assoc == Assoc::After {
            info |= WEAK_REF_FLAGS_END_ASSOC;
        }
        encoder.write_u8(info);
        match data.quote_start.scope() {
            IndexScope::Relative(id) | IndexScope::Nested(id) => {
                encoder.write_var(id.client);
                encoder.write_var(id.clock);
            }
            IndexScope::Root(name) => {
                encoder.write_string(name);
            }
        }

        match data.quote_end.scope() {
            IndexScope::Relative(id) if !is_single => {
                encoder.write_var(id.client);
                encoder.write_var(id.clock);
            }
            IndexScope::Relative(id) => {
                // for single element id is the same as start so we can infer it
            }
            IndexScope::Nested(id) => {
                encoder.write_var(id.client);
                encoder.write_var(id.clock);
            }
            IndexScope::Root(name) => {
                encoder.write_string(name);
            }
        }
    }

    #[cfg(feature = "weak")]
    fn decode_weak_link<D: Decoder>(decoder: &mut D) -> Result<Arc<LinkSource>, Error> {
        let flags = decoder.read_u8()?;
        let is_single = flags & WEAK_REF_FLAGS_QUOTE == 0;
        let start_assoc = if flags & WEAK_REF_FLAGS_START_ASSOC == WEAK_REF_FLAGS_START_ASSOC {
            Assoc::After
        } else {
            Assoc::Before
        };
        let end_assoc = if flags & WEAK_REF_FLAGS_END_ASSOC == WEAK_REF_FLAGS_END_ASSOC {
            Assoc::After
        } else {
            Assoc::Before
        };
        let is_start_unbounded =
            flags & WEAK_REF_FLAGS_START_UNBOUNDED == WEAK_REF_FLAGS_START_UNBOUNDED;
        let is_end_unbounded = flags & WEAK_REF_FLAGS_END_UNBOUNDED == WEAK_REF_FLAGS_END_UNBOUNDED;
        let is_parent_root = flags & WEAK_REF_FLAGS_PARENT_ROOT == WEAK_REF_FLAGS_PARENT_ROOT;
        let start_scope = if is_start_unbounded {
            if is_parent_root {
                let name = decoder.read_string()?;
                IndexScope::Root(name.into())
            } else {
                IndexScope::Nested(ID::new(decoder.read_var()?, decoder.read_var()?))
            }
        } else {
            IndexScope::Relative(ID::new(decoder.read_var()?, decoder.read_var()?))
        };

        let end_scope = if is_end_unbounded {
            if is_parent_root {
                let name = decoder.read_string()?;
                IndexScope::Root(name.into())
            } else {
                IndexScope::Nested(ID::new(decoder.read_var()?, decoder.read_var()?))
            }
        } else if is_single {
            start_scope.clone()
        } else {
            IndexScope::Relative(ID::new(decoder.read_var()?, decoder.read_var()?))
        };
        let start = StickyIndex::new(start_scope, start_assoc);
        let end = StickyIndex::new(end_scope, end_assoc);
        Ok(Arc::new(LinkSource::new(start, end)))
    }
}

impl std::fmt::Display for TypeRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeRef::Array => write!(f, "Array"),
            TypeRef::Map => write!(f, "Map"),
            TypeRef::Text => write!(f, "Text"),
            TypeRef::XmlElement(name) => write!(f, "XmlElement({})", name),
            TypeRef::XmlFragment => write!(f, "XmlFragment"),
            TypeRef::XmlHook => write!(f, "XmlHook"),
            TypeRef::XmlText => write!(f, "XmlText"),
            TypeRef::SubDoc => write!(f, "Doc"),
            #[cfg(feature = "weak")]
            TypeRef::WeakLink(_) => write!(f, "WeakRef"),
            TypeRef::Undefined => write!(f, "(undefined)"),
        }
    }
}

/// Marks is weak ref is quotation spanning over multiple elements.
const WEAK_REF_FLAGS_QUOTE: u8 = 0b0000_0001;
/// Marks is start boundary of weak ref is [Assoc::After].
const WEAK_REF_FLAGS_START_ASSOC: u8 = 0b0000_0010;
/// Marks is end boundary of weak ref is [Assoc::After].
const WEAK_REF_FLAGS_END_ASSOC: u8 = 0b0000_0100;
/// Marks if start boundary of weak ref is unbounded.
const WEAK_REF_FLAGS_START_UNBOUNDED: u8 = 0b0000_1000;
/// Marks if end boundary of weak ref is unbounded.
const WEAK_REF_FLAGS_END_UNBOUNDED: u8 = 0b0001_0000;
/// Marks if weak ref references a root type. Only needed for both sides unbounded elements.
const WEAK_REF_FLAGS_PARENT_ROOT: u8 = 0b0010_0000;

impl Encode for TypeRef {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        match self {
            TypeRef::Array => encoder.write_type_ref(TYPE_REFS_ARRAY),
            TypeRef::Map => encoder.write_type_ref(TYPE_REFS_MAP),
            TypeRef::Text => encoder.write_type_ref(TYPE_REFS_TEXT),
            TypeRef::XmlElement(name) => {
                encoder.write_type_ref(TYPE_REFS_XML_ELEMENT);
                encoder.write_key(&name);
            }
            TypeRef::XmlFragment => encoder.write_type_ref(TYPE_REFS_XML_FRAGMENT),
            TypeRef::XmlHook => encoder.write_type_ref(TYPE_REFS_XML_HOOK),
            TypeRef::XmlText => encoder.write_type_ref(TYPE_REFS_XML_TEXT),
            TypeRef::SubDoc => encoder.write_type_ref(TYPE_REFS_DOC),
            #[cfg(feature = "weak")]
            TypeRef::WeakLink(data) => Self::encode_weak_link(data, encoder),
            TypeRef::Undefined => encoder.write_type_ref(TYPE_REFS_UNDEFINED),
        }
    }
}

impl Decode for TypeRef {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let type_ref = decoder.read_type_ref()?;
        match type_ref {
            TYPE_REFS_ARRAY => Ok(TypeRef::Array),
            TYPE_REFS_MAP => Ok(TypeRef::Map),
            TYPE_REFS_TEXT => Ok(TypeRef::Text),
            TYPE_REFS_XML_ELEMENT => Ok(TypeRef::XmlElement(decoder.read_key()?)),
            TYPE_REFS_XML_FRAGMENT => Ok(TypeRef::XmlFragment),
            TYPE_REFS_XML_HOOK => Ok(TypeRef::XmlHook),
            TYPE_REFS_XML_TEXT => Ok(TypeRef::XmlText),
            TYPE_REFS_DOC => Ok(TypeRef::SubDoc),
            #[cfg(feature = "weak")]
            TYPE_REFS_WEAK => {
                let source = Self::decode_weak_link(decoder)?;
                Ok(TypeRef::WeakLink(source))
            }
            TYPE_REFS_UNDEFINED => Ok(TypeRef::Undefined),
            _ => Err(Error::UnexpectedValue),
        }
    }
}

#[cfg(feature = "sync")]
pub trait Observable: AsRef<Branch> {
    type Event;

    /// Subscribes a given callback to be triggered whenever current y-type is changed.
    /// A callback is triggered whenever a transaction gets committed. This function does not
    /// trigger if changes have been observed by nested shared collections.
    ///
    /// All array-like event changes can be tracked by using [Event::delta] method.
    /// All map-like event changes can be tracked by using [Event::keys] method.
    /// All text-like event changes can be tracked by using [TextEvent::delta] method.
    ///
    /// Returns a [Subscription] which, when dropped, will unsubscribe current callback.
    fn observe<F>(&self, f: F) -> Subscription
    where
        F: Fn(&TransactionMut, &Self::Event) + Send + Sync + 'static,
        Event: AsRef<Self::Event>,
    {
        let mut branch = BranchPtr::from(self.as_ref());
        branch.observe(move |txn, e| {
            let mapped_event = e.as_ref();
            f(txn, mapped_event)
        })
    }

    /// Subscribes a given callback to be triggered whenever current y-type is changed.
    /// A callback is triggered whenever a transaction gets committed. This function does not
    /// trigger if changes have been observed by nested shared collections.
    ///
    /// All array-like event changes can be tracked by using [Event::delta] method.
    /// All map-like event changes can be tracked by using [Event::keys] method.
    /// All text-like event changes can be tracked by using [TextEvent::delta] method.
    ///
    /// Provided key may be used later to unsubscribe from the event.
    fn observe_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &Self::Event) + Send + Sync + 'static,
        Event: AsRef<Self::Event>,
    {
        let mut branch = BranchPtr::from(self.as_ref());
        branch.observe_with(key.into(), move |txn, e| {
            let mapped_event = e.as_ref();
            f(txn, mapped_event)
        })
    }

    /// Unsubscribes a given callback identified by key, that was previously subscribed using [Self::observe_with].
    fn unobserve<K: Into<Origin>>(&self, key: K) -> bool {
        let mut branch = BranchPtr::from(self.as_ref());
        branch.unobserve(&key.into())
    }
}

#[cfg(not(feature = "sync"))]
pub trait Observable: AsRef<Branch> {
    type Event;

    /// Subscribes a given callback to be triggered whenever current y-type is changed.
    /// A callback is triggered whenever a transaction gets committed. This function does not
    /// trigger if changes have been observed by nested shared collections.
    ///
    /// All array-like event changes can be tracked by using [Event::delta] method.
    /// All map-like event changes can be tracked by using [Event::keys] method.
    /// All text-like event changes can be tracked by using [TextEvent::delta] method.
    ///
    /// Returns a [Subscription] which, when dropped, will unsubscribe current callback.
    fn observe<F>(&self, f: F) -> Subscription
    where
        F: Fn(&TransactionMut, &Self::Event) + 'static,
        Event: AsRef<Self::Event>,
    {
        let mut branch = BranchPtr::from(self.as_ref());
        branch.observe(move |txn, e| {
            let mapped_event = e.as_ref();
            f(txn, mapped_event)
        })
    }

    /// Subscribes a given callback to be triggered whenever current y-type is changed.
    /// A callback is triggered whenever a transaction gets committed. This function does not
    /// trigger if changes have been observed by nested shared collections.
    ///
    /// All array-like event changes can be tracked by using [Event::delta] method.
    /// All map-like event changes can be tracked by using [Event::keys] method.
    /// All text-like event changes can be tracked by using [TextEvent::delta] method.
    ///
    /// Provided key may be used later to unsubscribe from the event.
    fn observe_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &Self::Event) + 'static,
        Event: AsRef<Self::Event>,
    {
        let mut branch = BranchPtr::from(self.as_ref());
        branch.observe_with(key.into(), move |txn, e| {
            let mapped_event = e.as_ref();
            f(txn, mapped_event)
        })
    }

    /// Unsubscribes a given callback identified by key, that was previously subscribed using [Self::observe_with].
    fn unobserve<K: Into<Origin>>(&self, key: K) -> bool {
        let mut branch = BranchPtr::from(self.as_ref());
        branch.unobserve(&key.into())
    }
}

/// Trait implemented by shared types to display their contents in string format.
pub trait GetString {
    /// Displays the content of a current collection in string format.
    fn get_string<T: ReadTxn>(&self, txn: &T) -> String;
}

/// A subset of [SharedRef] used to mark collaborative collections that can be used as a
/// root level collections. This includes common types like [ArrayRef], [MapRef], [TextRef] and
/// [XmlFragmentRef].
///
/// Some types like [XmlTextRef] and [XmlElementRef] are not bound to be used as root-level types
/// since they have limited capabilities (i.e. cannot propagate XML node name).
///
/// Other types like [WeakRef] are not supposed to be used at root-level since they refer to
/// elements created prior to them, while root-level types are virtually immortal and technically
/// exist for the whole lifespan of their document.
pub trait RootRef: SharedRef {
    fn type_ref() -> TypeRef;

    /// Create a logical collaborative collection reference to a root-level type with a given `name`
    fn root<N: Into<Arc<str>>>(name: N) -> Root<Self> {
        Root::new(name)
    }
}

/// Common trait for shared collaborative collection types in Yrs.
pub trait SharedRef: From<BranchPtr> + AsRef<Branch> {
    /// Returns a logical descriptor of a current shared collection.
    fn hook(&self) -> Hook<Self> {
        let branch = self.as_ref();
        Hook::from(branch.id())
    }
}

/// Trait which allows conversion back to a prelim type that can be used to create a new shared
/// that's a deep copy equivalent of a current type.
pub trait AsPrelim {
    type Prelim: Prelim<Return = Self>;

    /// Converts current type contents into a [Prelim] type that can be used to create a new
    /// type that's a deep copy equivalent of a current type.
    fn as_prelim<T: ReadTxn>(&self, txn: &T) -> Self::Prelim;
}

/// Trait which allows to generate a [Prelim]-compatible type that - when integrated - will be
/// converted into an instance of a current type.
pub trait DefaultPrelim {
    type Prelim: Prelim<Return = Self>;

    /// Returns an instance of [Prelim]-compatible type, which will turn into reference of a current
    /// type after being integrated into the document store.
    fn default_prelim() -> Self::Prelim;
}

/// Trait implemented by all Y-types, allowing for observing events which are emitted by
/// nested types.
#[cfg(feature = "sync")]
pub trait DeepObservable: AsRef<Branch> {
    /// Subscribe a callback `f` for all events emitted by this and nested collaborative types.
    /// Callback is accepting transaction which triggered that event and event itself, wrapped
    /// within an [Event] structure.
    ///
    /// In case when a nested shared type (e.g. [MapRef],[ArrayRef],[TextRef]) is being removed,
    /// all of its contents will be removed first. So the observed value will be empty. For example,
    /// The value wrapped in the [EntryChange::Removed] of the [Event::Map] will be empty.
    ///
    /// This method returns a subscription, which will automatically unsubscribe current callback
    /// when dropped.
    fn observe_deep<F>(&self, f: F) -> Subscription
    where
        F: Fn(&TransactionMut, &Events) + Send + Sync + 'static,
    {
        let branch = self.as_ref();
        branch.deep_observers.subscribe(Box::new(f))
    }

    /// Subscribe a callback `f` for all events emitted by this and nested collaborative types.
    /// Callback is accepting transaction which triggered that event and event itself, wrapped
    /// within an [Event] structure.
    ///
    /// In case when a nested shared type (e.g. [MapRef],[ArrayRef],[TextRef]) is being removed,
    /// all of its contents will be removed first. So the observed value will be empty. For example,
    /// The value wrapped in the [EntryChange::Removed] of the [Event::Map] will be empty.
    ///
    /// This method uses a subscription key, which can be later used to cancel this callback via
    /// [Self::unobserve_deep].
    fn observe_deep_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &Events) + Send + Sync + 'static,
    {
        let branch = self.as_ref();
        branch
            .deep_observers
            .subscribe_with(key.into(), Box::new(f))
    }

    /// Unsubscribe a callback identified by a given key, that was previously subscribed using
    /// [Self::observe_deep_with].
    fn unobserve_deep<K: Into<Origin>>(&self, key: K) -> bool {
        let branch = self.as_ref();
        branch.deep_observers.unsubscribe(&key.into())
    }
}

/// Trait implemented by all Y-types, allowing for observing events which are emitted by
/// nested types.
#[cfg(not(feature = "sync"))]
pub trait DeepObservable: AsRef<Branch> {
    /// Subscribe a callback `f` for all events emitted by this and nested collaborative types.
    /// Callback is accepting transaction which triggered that event and event itself, wrapped
    /// within an [Event] structure.
    ///
    /// In case when a nested shared type (e.g. [MapRef],[ArrayRef],[TextRef]) is being removed,
    /// all of its contents will be removed first. So the observed value will be empty. For example,
    /// The value wrapped in the [EntryChange::Removed] of the [Event::Map] will be empty.
    ///
    /// This method returns a subscription, which will automatically unsubscribe current callback
    /// when dropped.
    fn observe_deep<F>(&self, f: F) -> Subscription
    where
        F: Fn(&TransactionMut, &Events) + 'static,
    {
        let branch = self.as_ref();
        branch.deep_observers.subscribe(Box::new(f))
    }

    /// Subscribe a callback `f` for all events emitted by this and nested collaborative types.
    /// Callback is accepting transaction which triggered that event and event itself, wrapped
    /// within an [Event] structure.
    ///
    /// In case when a nested shared type (e.g. [MapRef],[ArrayRef],[TextRef]) is being removed,
    /// all of its contents will be removed first. So the observed value will be empty. For example,
    /// The value wrapped in the [EntryChange::Removed] of the [Event::Map] will be empty.
    ///
    /// This method uses a subscription key, which can be later used to cancel this callback via
    /// [Self::unobserve_deep].
    fn observe_deep_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &Events) + 'static,
    {
        let branch = self.as_ref();
        branch
            .deep_observers
            .subscribe_with(key.into(), Box::new(f))
    }

    /// Unsubscribe a callback identified by a given key, that was previously subscribed using
    /// [Self::observe_deep_with].
    fn unobserve_deep<K: Into<Origin>>(&self, key: K) -> bool {
        let branch = self.as_ref();
        branch.deep_observers.unsubscribe(&key.into())
    }
}

impl std::fmt::Display for Branch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.type_ref() {
            TypeRef::Array => {
                if let Some(ptr) = self.start {
                    write!(f, "YArray(start: {})", ptr)
                } else {
                    write!(f, "YArray")
                }
            }
            TypeRef::Map => {
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
            TypeRef::Text => {
                if let Some(ptr) = self.start.as_ref() {
                    write!(f, "YText(start: {})", ptr)
                } else {
                    write!(f, "YText")
                }
            }
            TypeRef::XmlFragment => {
                write!(f, "YXmlFragment")?;
                if let Some(start) = self.start.as_ref() {
                    write!(f, "(start: {})", start)?;
                }
                Ok(())
            }
            TypeRef::XmlElement(name) => {
                write!(f, "YXmlElement('{}',", name)?;
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
            TypeRef::XmlHook => {
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
            TypeRef::XmlText => {
                if let Some(ptr) = self.start {
                    write!(f, "YXmlText(start: {})", ptr)
                } else {
                    write!(f, "YXmlText")
                }
            }
            TypeRef::SubDoc => {
                write!(f, "Subdoc")
            }
            #[cfg(feature = "weak")]
            TypeRef::WeakLink(w) => {
                if w.is_single() {
                    write!(f, "WeakRef({})", w.quote_start)
                } else {
                    write!(f, "WeakRef({}..{})", w.quote_start, w.quote_end)
                }
            }
            TypeRef::Undefined => {
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
    iter: std::collections::hash_map::Iter<'a, Arc<str>, ItemPtr>,
    txn: B,
    _marker: PhantomData<T>,
}

impl<'a, B, T: ReadTxn> Entries<'a, B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
    pub fn new(source: &'a HashMap<Arc<str>, ItemPtr>, txn: B) -> Self {
        Entries {
            iter: source.iter(),
            txn,
            _marker: PhantomData::default(),
        }
    }
}

impl<'a, T: ReadTxn> Entries<'a, &'a T, T>
where
    T: Borrow<T> + ReadTxn,
{
    pub fn from_ref(source: &'a HashMap<Arc<str>, ItemPtr>, txn: &'a T) -> Self {
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
        let (mut key, mut ptr) = self.iter.next()?;
        while ptr.is_deleted() {
            (key, ptr) = self.iter.next()?;
        }
        Some((key, ptr))
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
    Named(Arc<str>),

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

/// A path describing nesting structure between shared collections containing each other. It's a
/// collection of segments which refer to either index (in case of [Array] or [XmlElement]) or
/// string key (in case of [Map]) where successor shared collection can be found within subsequent
/// parent types.
pub type Path = VecDeque<PathSegment>;

/// A single segment of a [Path].
#[derive(Debug, Clone, PartialEq)]
pub enum PathSegment {
    /// Key segments are used to inform how to access child shared collections within a [Map] types.
    Key(Arc<str>),

    /// Index segments are used to inform how to access child shared collections within an [Array]
    /// or [XmlElement] types.
    Index(u32),
}

impl Serialize for PathSegment {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            PathSegment::Key(key) => serializer.serialize_str(&*key),
            PathSegment::Index(i) => serializer.serialize_u32(*i),
        }
    }
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
    Added(Vec<Out>),

    /// Determines a change that resulted in removing a consecutive range of existing elements,
    /// either XML child nodes for [XmlElement] or various elements stored in an [Array].
    Removed(u32),

    /// Determines a number of consecutive unchanged elements. Used to recognize non-edited spaces
    /// between [Change::Added] and/or [Change::Removed] chunks.
    Retain(u32),
}

/// A single change done over a map-component of shared data type.
#[derive(Clone, PartialEq)]
pub enum EntryChange {
    /// Informs about a new value inserted under specified entry.
    Inserted(Out),

    /// Informs about a change of old value (1st field) to a new one (2nd field) under
    /// a corresponding entry.
    Updated(Out, Out),

    /// Informs about a removal of a corresponding entry - contains a removed value.
    Removed(Out),
}

impl std::fmt::Debug for EntryChange {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            EntryChange::Inserted(out) => write!(f, "Inserted({out:?})"),
            EntryChange::Updated(old, new) => write!(f, "Updated({old:?}, {new:?})"),
            EntryChange::Removed(out) => {
                f.write_str("Removed(")?;
                // To avoid panicking on removed references, output the type name rather than the reference.
                match out {
                    Out::Any(any) => write!(f, "{any:?}")?,
                    Out::YText(_) => write!(f, "YText")?,
                    Out::YArray(_) => write!(f, "YArray")?,
                    Out::YMap(_) => write!(f, "YMap")?,
                    Out::YXmlElement(_) => write!(f, "YXmlElement")?,
                    Out::YXmlFragment(_) => write!(f, "YXmlFragment")?,
                    Out::YXmlText(_) => write!(f, "YXmlText")?,
                    Out::YDoc(_) => write!(f, "YDoc")?,
                    #[cfg(feature = "weak")]
                    Out::YWeakLink(_) => write!(f, "YWeakLink")?,
                    Out::UndefinedRef(_) => write!(f, "UndefinedRef")?,
                }
                f.write_str(")")
            }
        }
    }
}

/// A single change done over a text-like types: [Text] or [XmlText].
#[derive(Debug, Clone, PartialEq)]
pub enum Delta<T = Out> {
    /// Determines a change that resulted in insertion of a piece of text, which optionally could
    /// have been formatted with provided set of attributes.
    Inserted(T, Option<Box<Attrs>>),

    /// Determines a change that resulted in removing a consecutive range of characters.
    Deleted(u32),

    /// Determines a number of consecutive unchanged characters. Used to recognize non-edited spaces
    /// between [Delta::Inserted] and/or [Delta::Deleted] chunks. Can contain an optional set of
    /// attributes, which have been used to format an existing piece of text.
    Retain(u32, Option<Box<Attrs>>),
}

impl<T> Delta<T> {
    pub fn map<U, F>(self, f: F) -> Delta<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Delta::Inserted(value, attrs) => Delta::Inserted(f(value), attrs),
            Delta::Deleted(len) => Delta::Deleted(len),
            Delta::Retain(len, attrs) => Delta::Retain(len, attrs),
        }
    }
}

impl Delta<In> {
    pub fn retain(len: u32) -> Self {
        Delta::Retain(len, None)
    }

    pub fn insert<T: Into<In>>(value: T) -> Self {
        Delta::Inserted(value.into(), None)
    }

    pub fn insert_with<T: Into<In>>(value: T, attrs: Attrs) -> Self {
        Delta::Inserted(value.into(), Some(Box::new(attrs)))
    }

    pub fn delete(len: u32) -> Self {
        Delta::Deleted(len)
    }
}

/// An alias for map of attributes used as formatting parameters by [Text] and [XmlText] types.
pub type Attrs = HashMap<Arc<str>, Any>;

pub(crate) fn event_keys(
    txn: &TransactionMut,
    target: BranchPtr,
    keys_changed: &HashSet<Option<Arc<str>>>,
) -> HashMap<Arc<str>, EntryChange> {
    let mut keys = HashMap::new();
    for opt in keys_changed.iter() {
        if let Some(key) = opt {
            let block = target.map.get(key.as_ref()).cloned();
            if let Some(item) = block.as_deref() {
                if item.id.clock >= txn.before_state.get(&item.id.client) {
                    let mut prev = item.left;
                    while let Some(p) = prev.as_deref() {
                        if !txn.has_added(&p.id) {
                            break;
                        }
                        prev = p.left;
                    }

                    if txn.has_deleted(&item.id) {
                        if let Some(prev) = prev.as_deref() {
                            if txn.has_deleted(&prev.id) {
                                let old_value = prev.content.get_last().unwrap_or_default();
                                keys.insert(key.clone(), EntryChange::Removed(old_value));
                            }
                        }
                    } else {
                        let new_value = item.content.get_last().unwrap();
                        if let Some(prev) = prev.as_deref() {
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

pub(crate) fn event_change_set(txn: &TransactionMut, start: Option<ItemPtr>) -> ChangeSet<Change> {
    let mut added = HashSet::new();
    let mut deleted = HashSet::new();
    let mut delta = Vec::new();

    let mut moved_stack = Vec::new();
    let mut curr_move: Option<ItemPtr> = None;
    let mut curr_move_is_new = false;
    let mut curr_move_is_deleted = false;
    let mut curr_move_end: Option<ItemPtr> = None;
    let mut last_op = None;

    #[derive(Default)]
    struct MoveStackItem {
        end: Option<ItemPtr>,
        moved: Option<ItemPtr>,
        is_new: bool,
        is_deleted: bool,
    }

    fn is_moved_by_new(ptr: Option<ItemPtr>, txn: &TransactionMut) -> bool {
        let mut moved = ptr;
        while let Some(item) = moved.as_deref() {
            if txn.has_added(&item.id) {
                return true;
            } else {
                moved = item.moved;
            }
        }

        false
    }

    let encoding = txn.store().offset_kind;
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
            if let Some(item) = current {
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
                        && (txn.prev_moved.get(&item).cloned() == curr_move)
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
                        && !txn.prev_moved.contains_key(&item)
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
                        || txn.prev_moved.contains_key(&item)
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
        }

        current = if let Some(i) = current.as_deref() {
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

pub struct Events<'a>(Vec<&'a Event>);

impl<'a> Events<'a> {
    pub(crate) fn new(events: &Vec<&'a Event>) -> Self {
        let mut events = events.clone();
        events.sort_by(|&a, &b| {
            let path1 = a.path();
            let path2 = b.path();
            path1.len().cmp(&path2.len())
        });
        Events(events)
    }

    pub fn iter(&self) -> EventsIter {
        EventsIter(self.0.iter())
    }
}

pub struct EventsIter<'a>(std::slice::Iter<'a, &'a Event>);

impl<'a> Iterator for EventsIter<'a> {
    type Item = &'a Event;

    fn next(&mut self) -> Option<Self::Item> {
        let e = self.0.next()?;
        Some(e)
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
    #[cfg(feature = "weak")]
    Weak(WeakEvent),
}

impl AsRef<TextEvent> for Event {
    fn as_ref(&self) -> &TextEvent {
        if let Event::Text(e) = self {
            e
        } else {
            panic!("subscribed callback expected TextRef collection");
        }
    }
}

impl AsRef<ArrayEvent> for Event {
    fn as_ref(&self) -> &ArrayEvent {
        if let Event::Array(e) = self {
            e
        } else {
            panic!("subscribed callback expected ArrayRef collection");
        }
    }
}

impl AsRef<MapEvent> for Event {
    fn as_ref(&self) -> &MapEvent {
        if let Event::Map(e) = self {
            e
        } else {
            panic!("subscribed callback expected MapRef collection");
        }
    }
}

impl AsRef<XmlTextEvent> for Event {
    fn as_ref(&self) -> &XmlTextEvent {
        if let Event::XmlText(e) = self {
            e
        } else {
            panic!("subscribed callback expected XmlTextRef collection");
        }
    }
}

impl AsRef<XmlEvent> for Event {
    fn as_ref(&self) -> &XmlEvent {
        if let Event::XmlFragment(e) = self {
            e
        } else {
            panic!("subscribed callback expected Xml node");
        }
    }
}

#[cfg(feature = "weak")]
impl AsRef<WeakEvent> for Event {
    fn as_ref(&self) -> &WeakEvent {
        if let Event::Weak(e) = self {
            e
        } else {
            panic!("subscribed callback expected WeakRef reference");
        }
    }
}

impl Event {
    pub(crate) fn set_current_target(&mut self, target: BranchPtr) {
        match self {
            Event::Text(e) => e.current_target = target,
            Event::Array(e) => e.current_target = target,
            Event::Map(e) => e.current_target = target,
            Event::XmlText(e) => e.current_target = target,
            Event::XmlFragment(e) => e.current_target = target,
            #[cfg(feature = "weak")]
            Event::Weak(e) => e.current_target = target,
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
            #[cfg(feature = "weak")]
            Event::Weak(e) => e.path(),
        }
    }

    /// Returns a shared data types which triggered current [Event].
    pub fn target(&self) -> Out {
        match self {
            Event::Text(e) => Out::YText(e.target().clone()),
            Event::Array(e) => Out::YArray(e.target().clone()),
            Event::Map(e) => Out::YMap(e.target().clone()),
            Event::XmlText(e) => Out::YXmlText(e.target().clone()),
            Event::XmlFragment(e) => match e.target() {
                XmlOut::Element(n) => Out::YXmlElement(n.clone()),
                XmlOut::Fragment(n) => Out::YXmlFragment(n.clone()),
                XmlOut::Text(n) => Out::YXmlText(n.clone()),
            },
            #[cfg(feature = "weak")]
            Event::Weak(e) => Out::YWeakLink(e.as_target().clone()),
        }
    }
}

pub trait ToJson {
    /// Converts all contents of a current type into a JSON-like representation.
    fn to_json<T: ReadTxn>(&self, txn: &T) -> Any;
}
