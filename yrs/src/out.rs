use crate::block::{ItemContent, ItemPtr};
use crate::branch::{Branch, BranchPtr};
use crate::types::{AsPrelim, ToJson};
use crate::{
    any, Any, ArrayRef, Doc, GetString, In, MapPrelim, MapRef, ReadTxn, TextRef, XmlElementRef,
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
    YWeakLink(crate::WeakRef<BranchPtr>),
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

impl Out {
    /// Attempts to convert current [Out] value directly onto a different type, as along as it
    /// implements [TryFrom] trait. If conversion is not possible, the original value is returned.
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
            Out::Any(a) => a.to_string(),
            Out::YText(v) => v.get_string(txn),
            Out::YArray(v) => v.to_json(txn).to_string(),
            Out::YMap(v) => v.to_json(txn).to_string(),
            Out::YXmlElement(v) => v.get_string(txn),
            Out::YXmlFragment(v) => v.get_string(txn),
            Out::YXmlText(v) => v.get_string(txn),
            Out::YDoc(v) => v.to_string(),
            #[cfg(feature = "weak")]
            Out::YWeakLink(v) => {
                let text_ref: crate::WeakRef<TextRef> = crate::WeakRef::from(v);
                text_ref.get_string(txn)
            }
            Out::UndefinedRef(_) => "".to_string(),
        }
    }

    pub fn try_branch(&self) -> Option<&Branch> {
        match self {
            Out::YText(b) => Some(b.as_ref()),
            Out::YArray(b) => Some(b.as_ref()),
            Out::YMap(b) => Some(b.as_ref()),
            Out::YXmlElement(b) => Some(b.as_ref()),
            Out::YXmlFragment(b) => Some(b.as_ref()),
            Out::YXmlText(b) => Some(b.as_ref()),
            #[cfg(feature = "weak")]
            Out::YWeakLink(b) => Some(b.as_ref()),
            Out::UndefinedRef(b) => Some(b.as_ref()),
            Out::YDoc(_) => None,
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

    fn as_prelim<T: ReadTxn>(&self, txn: &T) -> Self::Prelim {
        match self {
            Out::Any(any) => In::Any(any.clone()),
            Out::YText(v) => In::Text(v.as_prelim(txn)),
            Out::YArray(v) => In::Array(v.as_prelim(txn)),
            Out::YMap(v) => In::Map(v.as_prelim(txn)),
            Out::YXmlElement(v) => In::XmlElement(v.as_prelim(txn)),
            Out::YXmlFragment(v) => In::XmlFragment(v.as_prelim(txn)),
            Out::YXmlText(v) => In::XmlText(v.as_prelim(txn)),
            Out::YDoc(v) => In::Doc(v.clone()),
            #[cfg(feature = "weak")]
            Out::YWeakLink(v) => In::WeakLink(v.as_prelim(txn)),
            Out::UndefinedRef(v) => infer_type_from_content(*v, txn),
        }
    }
}

fn infer_type_from_content<T: ReadTxn>(branch: BranchPtr, txn: &T) -> In {
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

//FIXME: what we would like to have is an automatic trait implementation of TryFrom<Value> for
// any type that implements TryFrom<Any,Error=Any>, but this causes compiler error.
macro_rules! impl_try_from {
    ($t:ty) => {
        impl TryFrom<Out> for $t {
            type Error = Out;

            fn try_from(value: Out) -> Result<Self, Self::Error> {
                use std::convert::TryInto;
                match value {
                    Out::Any(any) => any.try_into().map_err(Out::Any),
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

impl ToJson for Out {
    /// Converts current value into [Any] object equivalent that resembles enhanced JSON payload.
    /// Rules are:
    ///
    /// - Primitive types ([Out::Any]) are passed right away, as no transformation is needed.
    /// - [Out::YArray] is converted into JSON-like array.
    /// - [Out::YMap] is converted into JSON-like object map.
    /// - [Out::YText], [Out::YXmlText] and [Out::YXmlElement] are converted into strings
    ///   (XML types are stringified XML representation).
    fn to_json<T: ReadTxn>(&self, txn: &T) -> Any {
        match self {
            Out::Any(a) => a.clone(),
            Out::YText(v) => Any::from(v.get_string(txn)),
            Out::YArray(v) => v.to_json(txn),
            Out::YMap(v) => v.to_json(txn),
            Out::YXmlElement(v) => Any::from(v.get_string(txn)),
            Out::YXmlText(v) => Any::from(v.get_string(txn)),
            Out::YXmlFragment(v) => Any::from(v.get_string(txn)),
            Out::YDoc(doc) => any!({"guid": doc.guid().as_ref()}),
            #[cfg(feature = "weak")]
            Out::YWeakLink(_) => Any::Undefined,
            Out::UndefinedRef(_) => Any::Undefined,
        }
    }
}

impl std::fmt::Display for Out {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Out::Any(v) => std::fmt::Display::fmt(v, f),
            Out::YText(_) => write!(f, "TextRef"),
            Out::YArray(_) => write!(f, "ArrayRef"),
            Out::YMap(_) => write!(f, "MapRef"),
            Out::YXmlElement(_) => write!(f, "XmlElementRef"),
            Out::YXmlFragment(_) => write!(f, "XmlFragmentRef"),
            Out::YXmlText(_) => write!(f, "XmlTextRef"),
            #[cfg(feature = "weak")]
            Out::YWeakLink(_) => write!(f, "WeakRef"),
            Out::YDoc(v) => write!(f, "Doc(guid:{})", v.guid()),
            Out::UndefinedRef(_) => write!(f, "UndefinedRef"),
        }
    }
}
