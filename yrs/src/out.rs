use crate::branch::{Branch, BranchPtr};
use crate::types::ToJson;
use crate::{
    any, Any, ArrayRef, Doc, GetString, MapRef, ReadTxn, TextRef, WeakRef, XmlElementRef,
    XmlFragmentRef, XmlTextRef,
};
use std::convert::TryFrom;
use std::fmt::Formatter;
use std::sync::Arc;

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
                use std::convert::TryInto;
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
