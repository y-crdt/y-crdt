pub mod array;
pub mod map;
pub mod text;
#[cfg(feature = "weak")]
pub mod weak;
pub mod xml;

use crate::*;
pub use map::Map;
pub use map::MapRef;
use std::borrow::Borrow;
pub use text::Text;
pub use text::TextRef;

use crate::block::{Item, ItemContent, ItemPtr};
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
use serde::{Serialize, Serializer};
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::{TryFrom, TryInto};
use std::fmt::Formatter;
use std::marker::PhantomData;
use std::sync::Arc;

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
            TypeRef::WeakLink(data) => {
                let is_single = data.is_single();
                let start = data.quote_start.id().unwrap();
                let end = data.quote_end.id().unwrap();
                encoder.write_type_ref(TYPE_REFS_WEAK);
                let mut info = if is_single { 0u8 } else { 1u8 };
                info |= match data.quote_start.assoc {
                    Assoc::After => 2,
                    Assoc::Before => 0,
                };
                info |= match data.quote_end.assoc {
                    Assoc::After => 4,
                    Assoc::Before => 0,
                };
                encoder.write_u8(info);
                encoder.write_var(start.client);
                encoder.write_var(start.clock);
                if !is_single {
                    encoder.write_var(end.client);
                    encoder.write_var(end.clock);
                }
            }
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
                let flags = decoder.read_u8()?;
                let is_single = flags & 1u8 == 0;
                let start_assoc = if flags & 2 == 2 {
                    Assoc::After
                } else {
                    Assoc::Before
                };
                let end_assoc = if flags & 4 == 4 {
                    Assoc::After
                } else {
                    Assoc::Before
                };
                let start_id = ID::new(decoder.read_var()?, decoder.read_var()?);
                let end_id = if is_single {
                    start_id.clone()
                } else {
                    ID::new(decoder.read_var()?, decoder.read_var()?)
                };
                let start = StickyIndex::from_id(start_id, start_assoc);
                let end = StickyIndex::from_id(end_id, end_assoc);
                Ok(TypeRef::WeakLink(Arc::new(LinkSource::new(start, end))))
            }
            TYPE_REFS_UNDEFINED => Ok(TypeRef::Undefined),
            _ => Err(Error::UnexpectedValue),
        }
    }
}

#[cfg(not(target_family = "wasm"))]
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
    fn unobserve<K: Into<Origin>>(&self, key: K) {
        let mut branch = BranchPtr::from(self.as_ref());
        branch.unobserve(&key.into())
    }
}

#[cfg(target_family = "wasm")]
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
    fn unobserve<K: Into<Origin>>(&self, key: K) {
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

/// Trait implemented by all Y-types, allowing for observing events which are emitted by
/// nested types.
#[cfg(not(target_family = "wasm"))]
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
    fn unobserve_deep<K: Into<Origin>>(&self, key: K) {
        let branch = self.as_ref();
        branch.deep_observers.unsubscribe(&key.into())
    }
}

/// Trait implemented by all Y-types, allowing for observing events which are emitted by
/// nested types.
#[cfg(target_family = "wasm")]
pub trait DeepObservable: AsRef<Branch> {
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
    fn unobserve_deep<K: Into<Origin>>(&self, key: K) {
        let branch = self.as_ref();
        branch.deep_observers.unsubscribe(&key.into())
    }
}

/// Value that can be returned by Yrs data types. This includes [Any] which is an extension
/// representation of JSON, but also nested complex collaborative structures specific to Yrs.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Any value that it treated as a single element in it's entirety.
    Any(Any),
    /// Instance of a [TextRef].
    YText(TextRef),
    /// Instance of an [ArrayRef].
    YArray(ArrayRef),
    /// Instance of a [MapRef].
    YMap(MapRef),
    /// Instance of a [XmlElementRef].
    YXmlElement(XmlElementRef),
    /// Instance of a [XmlFragmentRef].
    YXmlFragment(XmlFragmentRef),
    /// Instance of a [XmlTextRef].
    YXmlText(XmlTextRef),
    /// Subdocument.
    YDoc(Doc),
    /// Instance of a [WeakRef] or unspecified type (requires manual casting).
    #[cfg(feature = "weak")]
    YWeakLink(WeakRef<BranchPtr>),
    /// Instance of a shared collection of undefined type. Usually happens when it refers to a root
    /// type that has not been defined locally. Can also refer to a [WeakRef] if "weak" feature flag
    /// was not set.
    UndefinedRef(BranchPtr),
}

impl Default for Value {
    fn default() -> Self {
        Value::Any(Any::Undefined)
    }
}

impl Value {
    #[inline]
    pub fn cast<T>(self) -> Result<T, Self>
    where
        T: TryFrom<Self, Error = Self>,
    {
        T::try_from(self)
    }

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
            Value::YDoc(v) => v.to_string(),
            #[cfg(feature = "weak")]
            Value::YWeakLink(v) => {
                let text_ref: crate::WeakRef<TextRef> = crate::WeakRef::from(v);
                text_ref.get_string(txn)
            }
            Value::UndefinedRef(_) => "".to_string(),
        }
    }

    pub fn try_branch(&self) -> Option<&Branch> {
        match self {
            Value::YText(b) => Some(b.as_ref()),
            Value::YArray(b) => Some(b.as_ref()),
            Value::YMap(b) => Some(b.as_ref()),
            Value::YXmlElement(b) => Some(b.as_ref()),
            Value::YXmlFragment(b) => Some(b.as_ref()),
            Value::YXmlText(b) => Some(b.as_ref()),
            #[cfg(feature = "weak")]
            Value::YWeakLink(b) => Some(b.as_ref()),
            Value::UndefinedRef(b) => Some(b.as_ref()),
            Value::YDoc(_) => None,
            Value::Any(_) => None,
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

//FIXME: what we would like to have is an automatic trait implementation of TryFrom<Value> for
// any type that implements TryFrom<Any,Error=Any>, but this causes compiler error.
macro_rules! impl_try_from {
    ($t:ty) => {
        impl TryFrom<Value> for $t {
            type Error = Value;

            fn try_from(value: Value) -> Result<Self, Self::Error> {
                match value {
                    Value::Any(any) => any.try_into().map_err(Value::Any),
                    other => Err(other),
                }
            }
        }
    };
}

impl_try_from!(bool);
impl_try_from!(f32);
impl_try_from!(f64);
impl_try_from!(i16);
impl_try_from!(i32);
impl_try_from!(u16);
impl_try_from!(u32);
impl_try_from!(i64);
impl_try_from!(isize);
impl_try_from!(String);
impl_try_from!(Arc<str>);
impl_try_from!(Vec<u8>);
impl_try_from!(Arc<[u8]>);

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
            Value::YText(v) => Any::from(v.get_string(txn)),
            Value::YArray(v) => v.to_json(txn),
            Value::YMap(v) => v.to_json(txn),
            Value::YXmlElement(v) => Any::from(v.get_string(txn)),
            Value::YXmlText(v) => Any::from(v.get_string(txn)),
            Value::YXmlFragment(v) => Any::from(v.get_string(txn)),
            Value::YDoc(doc) => any!({"guid": doc.guid().as_ref()}),
            #[cfg(feature = "weak")]
            Value::YWeakLink(_) => Any::Undefined,
            Value::UndefinedRef(_) => Any::Undefined,
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Any(v) => std::fmt::Display::fmt(v, f),
            Value::YText(_) => write!(f, "TextRef"),
            Value::YArray(_) => write!(f, "ArrayRef"),
            Value::YMap(_) => write!(f, "MapRef"),
            Value::YXmlElement(_) => write!(f, "XmlElementRef"),
            Value::YXmlFragment(_) => write!(f, "XmlFragmentRef"),
            Value::YXmlText(_) => write!(f, "XmlTextRef"),
            #[cfg(feature = "weak")]
            Value::YWeakLink(_) => write!(f, "WeakRef"),
            Value::YDoc(v) => write!(f, "Doc(guid:{})", v.options().guid),
            Value::UndefinedRef(_) => write!(f, "UndefinedRef"),
        }
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

impl<'a, T: ReadTxn> Entries<'a, T, T>
where
    T: Borrow<T> + ReadTxn,
{
    pub fn from(source: &'a HashMap<Arc<str>, ItemPtr>, txn: T) -> Self {
        Entries::new(source, txn)
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
            #[cfg(feature = "weak")]
            Event::Weak(e) => Value::YWeakLink(e.as_target().clone()),
        }
    }
}

pub trait ToJson {
    /// Converts all contents of a current type into a JSON-like representation.
    fn to_json<T: ReadTxn>(&self, txn: &T) -> Any;
}
