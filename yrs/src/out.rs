use crate::block::{ItemContent, ItemPtr};
use crate::branch::{Branch, BranchPtr};
use crate::doc::SubDocHook;
use crate::types::{AsPrelim, ToJson};
use crate::{
    Any, ArrayRef, Doc, GetString, In, MapPrelim, MapRef, TextRef, Transaction, XmlElementRef,
    XmlFragmentRef, XmlTextRef,
};
use std::convert::TryFrom;
use std::fmt::Formatter;
use std::sync::Arc;

/// Value that can be returned by Yrs data types. This includes [Any] which is an extension
/// representation of JSON, but also nested complex collaborative structures specific to Yrs.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// Any value that it treated as a single element in its entirety.
    Any(Any),
    /// Instance of a [TextRef].
    Text(TextRef),
    /// Instance of an [ArrayRef].
    Array(ArrayRef),
    /// Instance of a [MapRef].
    Map(MapRef),
    /// Instance of a [XmlElementRef].
    XmlElement(XmlElementRef),
    /// Instance of a [XmlFragmentRef].
    XmlFragment(XmlFragmentRef),
    /// Instance of a [XmlTextRef].
    XmlText(XmlTextRef),
    /// Subdocument.
    SubDoc(SubDocHook),
    /// Instance of a [WeakRef] or unspecified type (requires manual casting).
    #[cfg(feature = "weak")]
    WeakLink(crate::WeakRef<BranchPtr>),
    /// Instance of a shared collection of undefined type. Usually happens when it refers to a root
    /// type that has not been defined locally. Can also refer to a [WeakRef] if "weak" feature flag
    /// was not set.
    UndefinedRef(BranchPtr),
}

impl Default for Out {
    fn default() -> Self {
        Out::Any(Any::Undefined)
    }
}

impl FromOut for Out {
    #[inline]
    fn from_out(value: Out, txn: &Transaction) -> Result<Self, Out>
    where
        Self: Sized,
    {
        Ok(value)
    }

    fn from_item(item: ItemPtr, txn: &Transaction) -> Option<Self>
    where
        Self: Sized,
    {
        match &item.content {
            ItemContent::Any(value) => value.last().cloned().map(Out::Any),
            ItemContent::Binary(value) => Some(Out::Any(Any::Buffer(value.clone().into()))),
            ItemContent::Deleted(_) => None,
            ItemContent::Doc(value) => Some(Out::SubDoc(SubDocHook::new(value.clone()))),
            ItemContent::JSON(value) => value
                .last()
                .and_then(|json| serde_json::from_str(&json).ok())
                .map(Out::Any),
            ItemContent::Embed(value) => Some(Out::Any(value.clone())),
            ItemContent::Format(key, value) => None,
            ItemContent::String(value) => Some(Out::Any(Any::String(value.to_string().into()))),
            ItemContent::Type(branch) => Some(Out::from(BranchPtr::from(branch))),
            ItemContent::Move(_) => None,
        }
    }
}

impl Out {
    pub fn cast<O: FromOut>(self: Out, txn: &Transaction) -> Result<O, Self> {
        O::from_out(self, txn)
    }

    /// Converts current value into stringified representation.
    pub fn to_string(self, txn: &Transaction) -> String {
        match self {
            Out::Any(a) => a.to_string(),
            Out::Text(v) => v.get_string(txn),
            Out::Array(v) => v.to_json(txn).to_string(),
            Out::Map(v) => v.to_json(txn).to_string(),
            Out::XmlElement(v) => v.get_string(txn),
            Out::XmlFragment(v) => v.get_string(txn),
            Out::XmlText(v) => v.get_string(txn),
            Out::SubDoc(v) => {
                let borrowed = v.borrow();
                borrowed.to_string()
            }
            #[cfg(feature = "weak")]
            Out::WeakLink(v) => {
                let text_ref: crate::WeakRef<TextRef> = crate::WeakRef::from(v);
                text_ref.get_string(txn)
            }
            Out::UndefinedRef(_) => "".to_string(),
        }
    }

    pub fn try_branch(&self) -> Option<&Branch> {
        match self {
            Out::Text(b) => Some(b.as_ref()),
            Out::Array(b) => Some(b.as_ref()),
            Out::Map(b) => Some(b.as_ref()),
            Out::XmlElement(b) => Some(b.as_ref()),
            Out::XmlFragment(b) => Some(b.as_ref()),
            Out::XmlText(b) => Some(b.as_ref()),
            #[cfg(feature = "weak")]
            Out::WeakLink(b) => Some(b.as_ref()),
            Out::UndefinedRef(b) => Some(b.as_ref()),
            Out::SubDoc(_) => None,
            Out::Any(_) => None,
        }
    }
}

impl TryFrom<ItemPtr> for Out {
    type Error = ItemPtr;

    fn try_from(value: ItemPtr) -> Result<Self, Self::Error> {
        match value.content.get_last() {
            None => Err(value),
            Some(v) => Ok(v),
        }
    }
}

impl AsPrelim for Out {
    type Prelim = In;

    fn as_prelim(&self, txn: &Transaction) -> Self::Prelim {
        match self {
            Out::Any(any) => In::Any(any.clone()),
            Out::Text(v) => In::Text(v.as_prelim(txn)),
            Out::Array(v) => In::Array(v.as_prelim(txn)),
            Out::Map(v) => In::Map(v.as_prelim(txn)),
            Out::XmlElement(v) => In::XmlElement(v.as_prelim(txn)),
            Out::XmlFragment(v) => In::XmlFragment(v.as_prelim(txn)),
            Out::XmlText(v) => In::XmlText(v.as_prelim(txn)),
            Out::SubDoc(v) => {
                let borrowed = v.borrow();
                In::Doc(Doc::with_options(borrowed.options.clone()))
            }
            #[cfg(feature = "weak")]
            Out::WeakLink(v) => In::WeakLink(v.as_prelim(txn)),
            Out::UndefinedRef(v) => infer_type_from_content(*v, txn),
        }
    }
}

fn infer_type_from_content(branch: BranchPtr, txn: &Transaction) -> In {
    let has_map = !branch.map.is_empty();
    let mut ptr = branch.start;
    let has_list = ptr.is_some();
    let mut possible_text = false;
    while let Some(curr) = ptr {
        if !curr.is_deleted() {
            possible_text = match &curr.content {
                ItemContent::Embed(_) | ItemContent::Format(_, _) | ItemContent::String(_) => true,
                _ => false,
            };
            break;
        }
        ptr = curr.right;
    }

    match (has_map, has_list, possible_text) {
        (true, false, false) => In::Map(MapRef::from(branch).as_prelim(txn)),
        (false, true, false) => In::Array(ArrayRef::from(branch).as_prelim(txn)),
        (false, _, true) => In::Text(TextRef::from(branch).as_prelim(txn)),
        (true, _, true) => In::XmlText(XmlTextRef::from(branch).as_prelim(txn)),
        (true, true, false) => In::XmlElement(XmlElementRef::from(branch).as_prelim(txn)),
        _ => In::Map(MapPrelim::default()), // if we have no content, default to map
    }
}

impl<T> From<T> for Out
where
    T: Into<Any>,
{
    fn from(v: T) -> Self {
        let any: Any = v.into();
        Out::Any(any)
    }
}

pub trait FromOut {
    /// Converts [Out] value into a type that implements this trait.
    fn from_out(value: Out, txn: &Transaction) -> Result<Self, Out>
    where
        Self: Sized;

    /// Converts [Out] value into a type that implements this trait.
    fn from_item(item: ItemPtr, txn: &Transaction) -> Option<Self>
    where
        Self: Sized;
}

//FIXME: what we would like to have is an automatic trait implementation of TryFrom<Value> for
// any type that implements TryFrom<Any,Error=Any>, but this causes compiler error.
macro_rules! impl_from_out {
    ($t:ty) => {
        impl FromOut for $t {
            fn from_out(value: Out, _txn: &Transaction) -> Result<Self, Out> {
                use std::convert::TryInto;
                match value {
                    Out::Any(any) => any.try_into().map_err(Out::Any),
                    other => Err(other),
                }
            }

            fn from_item(item: ItemPtr, txn: &Transaction) -> Option<Self>
            where
                Self: Sized,
            {
                use std::convert::TryInto;
                match &item.content {
                    ItemContent::Any(any) => {
                        let any = any.last()?.clone();
                        any.try_into().ok()
                    }
                    _ => None,
                }
            }
        }
    };
}

impl_from_out!(bool);
impl_from_out!(f32);
impl_from_out!(f64);
impl_from_out!(i16);
impl_from_out!(i32);
impl_from_out!(u16);
impl_from_out!(u32);
impl_from_out!(i64);
impl_from_out!(isize);
impl_from_out!(String);
impl_from_out!(Arc<str>);
impl_from_out!(Vec<u8>);
impl_from_out!(Arc<[u8]>);

impl ToJson for Out {
    /// Converts current value into [Any] object equivalent that resembles enhanced JSON payload.
    /// Rules are:
    ///
    /// - Primitive types ([Out::Any]) are passed right away, as no transformation is needed.
    /// - [Out::Array] is converted into JSON-like array.
    /// - [Out::Map] is converted into JSON-like object map.
    /// - [Out::Text], [Out::XmlText] and [Out::XmlElement] are converted into strings
    ///   (XML types are stringified XML representation).
    fn to_json(&self, txn: &Transaction) -> Any {
        match self {
            Out::Any(a) => a.clone(),
            Out::Text(v) => Any::from(v.get_string(txn)),
            Out::Array(v) => v.to_json(txn),
            Out::Map(v) => v.to_json(txn),
            Out::XmlElement(v) => Any::from(v.get_string(txn)),
            Out::XmlText(v) => Any::from(v.get_string(txn)),
            Out::XmlFragment(v) => Any::from(v.get_string(txn)),
            Out::SubDoc(doc) => {
                let borrowed = doc.borrow();
                borrowed.to_json()
            }
            #[cfg(feature = "weak")]
            Out::WeakLink(_) => Any::Undefined,
            Out::UndefinedRef(_) => Any::Undefined,
        }
    }
}

impl std::fmt::Display for Out {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Out::Any(v) => std::fmt::Display::fmt(v, f),
            Out::Text(_) => write!(f, "TextRef"),
            Out::Array(_) => write!(f, "ArrayRef"),
            Out::Map(_) => write!(f, "MapRef"),
            Out::XmlElement(_) => write!(f, "XmlElementRef"),
            Out::XmlFragment(_) => write!(f, "XmlFragmentRef"),
            Out::XmlText(_) => write!(f, "XmlTextRef"),
            #[cfg(feature = "weak")]
            Out::WeakLink(_) => write!(f, "WeakRef"),
            Out::SubDoc(subdoc) => {
                let borrowed = subdoc.borrow();
                write!(f, "Doc(guid:{})", borrowed.options.guid)
            }
            Out::UndefinedRef(_) => write!(f, "UndefinedRef"),
        }
    }
}
