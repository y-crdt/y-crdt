use crate::block::{ItemContent, Prelim};
use crate::branch::{Branch, BranchPtr};
use crate::types::text::DeltaPrelim;
use crate::types::xml::XmlDeltaPrelim;
use crate::types::TypeRef;
use crate::{
    Any, ArrayPrelim, Doc, MapPrelim, Out, TransactionMut, XmlElementPrelim, XmlFragmentPrelim,
};

/// A wrapper around [Out] type that enables it to be used as a type to be inserted into
/// shared collections. If [In] contains a shared type, it will be inserted as a deep
/// copy of the original type: therefore none of the changes applied to the original type will
/// affect the deep copy.
#[derive(Debug, Clone, PartialEq)]
pub enum In {
    Any(Any),
    Text(DeltaPrelim),
    Array(ArrayPrelim),
    Map(MapPrelim),
    XmlElement(XmlElementPrelim),
    XmlFragment(XmlFragmentPrelim),
    XmlText(XmlDeltaPrelim),
    Doc(Doc),
    #[cfg(feature = "weak")]
    WeakLink(crate::types::weak::WeakPrelim<BranchPtr>),
}

impl Prelim for In {
    type Return = Out;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        match self {
            In::Any(any) => (ItemContent::Any(vec![any]), None),
            other => {
                let type_ref = match &other {
                    In::Text(_) => TypeRef::Text,
                    In::Array(_) => TypeRef::Array,
                    In::Map(_) => TypeRef::Map,
                    In::XmlElement(v) => TypeRef::XmlElement(v.tag.clone()),
                    In::XmlFragment(_) => TypeRef::XmlFragment,
                    In::XmlText(_) => TypeRef::XmlText,
                    In::Doc(_) => TypeRef::SubDoc,
                    #[cfg(feature = "weak")]
                    In::WeakLink(v) => TypeRef::WeakLink(v.source().clone()),
                    _ => unreachable!(),
                };
                (ItemContent::Type(Branch::new(type_ref)), Some(other))
            }
        }
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        match self {
            In::Text(prelim) => prelim.integrate(txn, inner_ref),
            In::Array(prelim) => prelim.integrate(txn, inner_ref),
            In::Map(prelim) => prelim.integrate(txn, inner_ref),
            In::XmlElement(prelim) => prelim.integrate(txn, inner_ref),
            In::XmlFragment(prelim) => prelim.integrate(txn, inner_ref),
            In::XmlText(prelim) => prelim.integrate(txn, inner_ref),
            In::Doc(prelim) => prelim.integrate(txn, inner_ref),
            #[cfg(feature = "weak")]
            In::WeakLink(prelim) => prelim.integrate(txn, inner_ref),
            _ => { /* do nothing */ }
        }
    }
}

impl From<Any> for In {
    #[inline]
    fn from(value: Any) -> Self {
        In::Any(value)
    }
}

macro_rules! impl_from_any {
    ($t:ty) => {
        impl From<$t> for In {
            #[inline]
            fn from(value: $t) -> Self {
                In::Any(Any::from(value))
            }
        }
    };
}

impl_from_any!(bool);
impl_from_any!(i16);
impl_from_any!(i32);
impl_from_any!(i64);
impl_from_any!(u16);
impl_from_any!(u32);
impl_from_any!(f32);
impl_from_any!(f64);
impl_from_any!(String);
impl_from_any!(std::sync::Arc<str>);
impl_from_any!(&str);
impl_from_any!(Vec<u8>);
impl_from_any!(&[u8]);
