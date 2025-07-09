use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::collections::{HashMap, HashSet};
use std::convert::{TryFrom, TryInto};
use std::fmt::Write;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use crate::block::{EmbedPrelim, Item, ItemContent, ItemPosition, ItemPtr, Prelim};
use crate::block_iter::BlockIter;
use crate::transaction::TransactionMut;
use crate::types::text::{diff_between, TextEvent, YChange};
use crate::types::{
    event_change_set, event_keys, AsPrelim, Branch, BranchPtr, Change, ChangeSet, DefaultPrelim,
    Delta, Entries, EntryChange, MapRef, Out, Path, RootRef, SharedRef, ToJson, TypePtr, TypeRef,
};
use crate::{
    Any, ArrayRef, BranchID, DeepObservable, GetString, In, IndexedSequence, Map, Observable,
    ReadTxn, StickyIndex, Text, TextRef, ID,
};

pub trait XmlPrelim: Prelim {}

/// Trait shared by preliminary types that can be used as XML nodes: [XmlElementPrelim],
/// [XmlFragmentPrelim] and [XmlTextPrelim].
#[derive(Debug, Clone, PartialEq)]
pub enum XmlIn {
    Text(XmlDeltaPrelim),
    Element(XmlElementPrelim),
    Fragment(XmlFragmentPrelim),
}

impl From<XmlDeltaPrelim> for XmlIn {
    #[inline]
    fn from(value: XmlDeltaPrelim) -> Self {
        Self::Text(value)
    }
}

impl From<XmlTextPrelim> for XmlIn {
    #[inline]
    fn from(value: XmlTextPrelim) -> Self {
        Self::Text(value.into())
    }
}

impl From<XmlElementPrelim> for XmlIn {
    #[inline]
    fn from(value: XmlElementPrelim) -> Self {
        Self::Element(value)
    }
}

impl From<XmlFragmentPrelim> for XmlIn {
    #[inline]
    fn from(value: XmlFragmentPrelim) -> Self {
        Self::Fragment(value)
    }
}

impl XmlPrelim for XmlIn {}

impl Prelim for XmlIn {
    type Return = XmlOut;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let type_ref = match &self {
            XmlIn::Text(_) => TypeRef::XmlText,
            XmlIn::Element(prelim) => TypeRef::XmlElement(prelim.tag.clone()),
            XmlIn::Fragment(_) => TypeRef::XmlFragment,
        };
        (ItemContent::Type(Branch::new(type_ref)), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        match self {
            XmlIn::Text(prelim) => prelim.integrate(txn, inner_ref),
            XmlIn::Element(prelim) => prelim.integrate(txn, inner_ref),
            XmlIn::Fragment(prelim) => prelim.integrate(txn, inner_ref),
        }
    }
}

/// A return type from XML elements retrieval methods. It's an enum of all supported values, that
/// can be nested inside [XmlElementRef]. These are other [XmlElementRef]s, [XmlFragmentRef]s
/// or [XmlTextRef] values.
#[derive(Debug, Clone)]
pub enum XmlOut {
    Element(XmlElementRef),
    Fragment(XmlFragmentRef),
    Text(XmlTextRef),
}

impl XmlOut {
    pub fn as_ptr(&self) -> BranchPtr {
        match self {
            XmlOut::Element(n) => n.0,
            XmlOut::Fragment(n) => n.0,
            XmlOut::Text(n) => n.0,
        }
    }

    pub fn id(&self) -> BranchID {
        self.as_ptr().id()
    }

    /// If current underlying [XmlOut] is wrapping a [XmlElementRef], it will be returned.
    /// Otherwise, a `None` will be returned.
    pub fn into_xml_element(self) -> Option<XmlElementRef> {
        match self {
            XmlOut::Element(n) => Some(n),
            _ => None,
        }
    }

    /// If current underlying [XmlOut] is wrapping a [XmlFragmentRef], it will be returned.
    /// Otherwise, a `None` will be returned.
    pub fn into_xml_fragment(self) -> Option<XmlFragmentRef> {
        match self {
            XmlOut::Fragment(n) => Some(n),
            _ => None,
        }
    }

    /// If current underlying [XmlOut] is wrapping a [XmlTextRef], it will be returned.
    /// Otherwise, a `None` will be returned.
    pub fn into_xml_text(self) -> Option<XmlTextRef> {
        match self {
            XmlOut::Text(n) => Some(n),
            _ => None,
        }
    }
}

impl AsRef<Branch> for XmlOut {
    fn as_ref(&self) -> &Branch {
        match self {
            XmlOut::Element(n) => n.as_ref(),
            XmlOut::Fragment(n) => n.as_ref(),
            XmlOut::Text(n) => n.as_ref(),
        }
    }
}

impl TryInto<XmlElementRef> for XmlOut {
    type Error = XmlOut;

    fn try_into(self) -> Result<XmlElementRef, Self::Error> {
        match self {
            XmlOut::Element(xml) => Ok(xml),
            other => Err(other),
        }
    }
}

impl TryInto<XmlTextRef> for XmlOut {
    type Error = XmlOut;

    fn try_into(self) -> Result<XmlTextRef, Self::Error> {
        match self {
            XmlOut::Text(xml) => Ok(xml),
            other => Err(other),
        }
    }
}

impl TryInto<XmlFragmentRef> for XmlOut {
    type Error = XmlOut;

    fn try_into(self) -> Result<XmlFragmentRef, Self::Error> {
        match self {
            XmlOut::Fragment(xml) => Ok(xml),
            other => Err(other),
        }
    }
}

impl TryFrom<BranchPtr> for XmlOut {
    type Error = BranchPtr;

    fn try_from(value: BranchPtr) -> Result<Self, Self::Error> {
        match value.type_ref {
            TypeRef::XmlElement(_) => Ok(XmlOut::Element(XmlElementRef::from(value))),
            TypeRef::XmlFragment => Ok(XmlOut::Fragment(XmlFragmentRef::from(value))),
            TypeRef::XmlText => Ok(XmlOut::Text(XmlTextRef::from(value))),
            _ => Err(value),
        }
    }
}

impl TryFrom<Out> for XmlOut {
    type Error = Out;

    fn try_from(value: Out) -> Result<Self, Self::Error> {
        match value {
            Out::YXmlElement(n) => Ok(XmlOut::Element(n)),
            Out::YXmlFragment(n) => Ok(XmlOut::Fragment(n)),
            Out::YXmlText(n) => Ok(XmlOut::Text(n)),
            other => Err(other),
        }
    }
}

impl From<XmlOut> for Out {
    fn from(value: XmlOut) -> Self {
        match value {
            XmlOut::Element(xml) => Out::YXmlElement(xml),
            XmlOut::Fragment(xml) => Out::YXmlFragment(xml),
            XmlOut::Text(xml) => Out::YXmlText(xml),
        }
    }
}

impl TryFrom<ItemPtr> for XmlOut {
    type Error = ItemPtr;

    fn try_from(value: ItemPtr) -> Result<Self, Self::Error> {
        if let Some(branch) = value.clone().as_branch() {
            match branch.type_ref {
                TypeRef::XmlElement(_) => Ok(XmlOut::Element(XmlElementRef::from(branch))),
                TypeRef::XmlFragment => Ok(XmlOut::Fragment(XmlFragmentRef::from(branch))),
                TypeRef::XmlText => Ok(XmlOut::Text(XmlTextRef::from(branch))),
                _ => return Err(value),
            }
        } else {
            Err(value)
        }
    }
}

/// XML element data type. It represents an XML node, which can contain key-value attributes
/// (interpreted as strings) as well as other nested XML elements or rich text (represented by
/// [XmlTextRef] type).
///
/// In terms of conflict resolution, [XmlElementRef] uses following rules:
///
/// - Attribute updates use logical last-write-wins principle, meaning the past updates are
///   automatically overridden and discarded by newer ones, while concurrent updates made by
///   different peers are resolved into a single value using document id seniority to establish
///   an order.
/// - Child node insertion uses sequencing rules from other Yrs collections - elements are inserted
///   using interleave-resistant algorithm, where order of concurrent inserts at the same index
///   is established using peer's document id seniority.
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct XmlElementRef(BranchPtr);

impl SharedRef for XmlElementRef {}
impl Xml for XmlElementRef {}
impl XmlFragment for XmlElementRef {}
impl IndexedSequence for XmlElementRef {}

impl AsRef<XmlFragmentRef> for XmlElementRef {
    #[inline]
    fn as_ref(&self) -> &XmlFragmentRef {
        unsafe { std::mem::transmute(self) }
    }
}

impl AsRef<ArrayRef> for XmlElementRef {
    #[inline]
    fn as_ref(&self) -> &ArrayRef {
        unsafe { std::mem::transmute(self) }
    }
}

impl XmlElementRef {
    /// A tag name of a current top-level XML node, eg. node `<p></p>` has "p" as it's tag name.
    pub fn try_tag(&self) -> Option<&Arc<str>> {
        if let TypeRef::XmlElement(tag) = &self.0.type_ref {
            Some(tag)
        } else {
            // this could happen only if we reinterpret top level type as XmlElementRef
            None
        }
    }

    /// A tag name of a current top-level XML node, eg. node `<p></p>` has "p" as it's tag name.
    pub fn tag(&self) -> &Arc<str> {
        self.try_tag().expect("XmlElement tag was not defined")
    }
}

impl GetString for XmlElementRef {
    /// Converts current XML node into a textual representation. This representation if flat, it
    /// doesn't include any indentation.
    fn get_string<T: ReadTxn>(&self, txn: &T) -> String {
        let tag: &str = self.tag();
        let inner = self.0;
        let mut s = String::new();
        write!(&mut s, "<{}", tag).unwrap();
        let attributes = Attributes(inner.entries(txn));
        for (k, v) in attributes {
            write!(&mut s, " {}=\"{}\"", k, v).unwrap();
        }
        write!(&mut s, ">").unwrap();
        for i in inner.iter(txn) {
            if !i.is_deleted() {
                for content in i.content.get_content() {
                    write!(&mut s, "{}", content.to_string(txn)).unwrap();
                }
            }
        }
        write!(&mut s, "</{}>", tag).unwrap();
        s
    }
}

impl DeepObservable for XmlElementRef {}
impl Observable for XmlElementRef {
    type Event = XmlEvent;
}

impl AsRef<Branch> for XmlElementRef {
    fn as_ref(&self) -> &Branch {
        &self.0
    }
}

impl Eq for XmlElementRef {}
impl PartialEq for XmlElementRef {
    fn eq(&self, other: &Self) -> bool {
        self.0.id() == other.0.id()
    }
}

impl From<BranchPtr> for XmlElementRef {
    fn from(inner: BranchPtr) -> Self {
        XmlElementRef(inner)
    }
}

impl TryFrom<ItemPtr> for XmlElementRef {
    type Error = ItemPtr;

    fn try_from(value: ItemPtr) -> Result<Self, Self::Error> {
        if let Some(branch) = value.clone().as_branch() {
            Ok(Self::from(branch))
        } else {
            Err(value)
        }
    }
}

impl TryFrom<Out> for XmlElementRef {
    type Error = Out;

    fn try_from(value: Out) -> Result<Self, Self::Error> {
        match value {
            Out::YXmlElement(value) => Ok(value),
            other => Err(other),
        }
    }
}

impl AsPrelim for XmlElementRef {
    type Prelim = XmlElementPrelim;

    fn as_prelim<T: ReadTxn>(&self, txn: &T) -> Self::Prelim {
        let attributes: HashMap<Arc<str>, String> = self
            .0
            .map
            .iter()
            .flat_map(|(k, ptr)| {
                if ptr.is_deleted() {
                    None
                } else if let Some(value) = ptr.content.get_last() {
                    Some((k.clone(), value.to_string(txn)))
                } else {
                    None
                }
            })
            .collect();
        let children: Vec<_> = self
            .children(txn)
            .map(|v| match v {
                XmlOut::Element(v) => XmlIn::Element(v.as_prelim(txn)),
                XmlOut::Fragment(v) => XmlIn::Fragment(v.as_prelim(txn)),
                XmlOut::Text(v) => XmlIn::Text(v.as_prelim(txn)),
            })
            .collect();
        XmlElementPrelim {
            tag: self.tag().clone(),
            attributes,
            children,
        }
    }
}

/// A preliminary type that will be materialized into an [XmlElementRef] once it will be integrated
/// into Yrs document.
#[derive(Debug, Clone, PartialEq)]
pub struct XmlElementPrelim {
    pub tag: Arc<str>,
    pub attributes: HashMap<Arc<str>, String>,
    pub children: Vec<XmlIn>,
}

impl XmlElementPrelim {
    pub fn new<S, I>(tag: S, iter: I) -> Self
    where
        S: Into<Arc<str>>,
        I: IntoIterator<Item = XmlIn>,
    {
        XmlElementPrelim {
            tag: tag.into(),
            attributes: HashMap::default(),
            children: iter.into_iter().collect(),
        }
    }

    pub fn empty<S>(tag: S) -> Self
    where
        S: Into<Arc<str>>,
    {
        XmlElementPrelim {
            tag: tag.into(),
            attributes: HashMap::default(),
            children: Vec::default(),
        }
    }
}

impl XmlPrelim for XmlElementPrelim {}

impl Prelim for XmlElementPrelim {
    type Return = XmlElementRef;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let inner = Branch::new(TypeRef::XmlElement(self.tag.clone()));
        (ItemContent::Type(inner), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        let xml = XmlElementRef::from(inner_ref);
        for (key, value) in self.attributes {
            xml.insert_attribute(txn, key, value);
        }
        for value in self.children {
            xml.push_back(txn, value);
        }
    }
}

impl Into<EmbedPrelim<XmlElementPrelim>> for XmlElementPrelim {
    #[inline]
    fn into(self) -> EmbedPrelim<XmlElementPrelim> {
        EmbedPrelim::Shared(self)
    }
}

impl From<XmlElementPrelim> for In {
    #[inline]
    fn from(value: XmlElementPrelim) -> Self {
        In::XmlElement(value)
    }
}

/// A shared data type used for collaborative text editing, that can be used in a context of
/// [XmlElementRef] node. It enables multiple users to add and remove chunks of text in efficient
/// manner. This type is internally represented as a mutable double-linked list of text chunks
/// - an optimization occurs during [Transaction::commit], which allows to squash multiple
/// consecutively inserted characters together as a single chunk of text even between transaction
/// boundaries in order to preserve more efficient memory model.
///
/// Just like [XmlElementRef], [XmlTextRef] can be marked with extra metadata in form of attributes.
///
/// [XmlTextRef] structure internally uses UTF-8 encoding and its length is described in a number of
/// bytes rather than individual characters (a single UTF-8 code point can consist of many bytes).
///
/// Like all Yrs shared data types, [XmlTextRef] is resistant to the problem of interleaving (situation
/// when characters inserted one after another may interleave with other peers concurrent inserts
/// after merging all updates together). In case of Yrs conflict resolution is solved by using
/// unique document id to determine correct and consistent ordering.
///
/// [XmlTextRef] offers a rich text editing capabilities (it's not limited to simple text operations).
/// Actions like embedding objects, binaries (eg. images) and formatting attributes are all possible
/// using [XmlTextRef].
///
/// Keep in mind that [XmlTextRef::get_string] method returns a raw string, while rendering
/// formatting attrbitues as XML tags in-text. However it doesn't include embedded elements.
/// If there's a need to include them, use [XmlTextRef::diff] method instead.
///
/// Another note worth reminding is that human-readable numeric indexes are not good for maintaining
/// cursor positions in rich text documents with real-time collaborative capabilities. In such cases
/// any concurrent update incoming and applied from the remote peer may change the order of elements
/// in current [XmlTextRef], invalidating numeric index. For such cases you can take advantage of fact
/// that [XmlTextRef] implements [IndexedSequence::sticky_index] method that returns a
/// [permanent index](crate::StickyIndex) position that sticks to the same place even when concurrent
/// updates are being made.
///
/// # Example
///
/// ```rust
/// use yrs::{Any, Array, ArrayPrelim, Doc, GetString, Text, Transact, WriteTxn, XmlFragment, XmlTextPrelim};
/// use yrs::types::Attrs;
///
/// let doc = Doc::new();
/// let mut txn = doc.transact_mut();
/// let f = txn.get_or_insert_xml_fragment("article");
/// let text = f.insert(&mut txn, 0, XmlTextPrelim::new(""));
///
/// let bold = Attrs::from([("b".into(), true.into())]);
/// let italic = Attrs::from([("i".into(), true.into())]);
///
/// text.insert(&mut txn, 0, "hello ");
/// text.insert_with_attributes(&mut txn, 6, "world", italic);
/// text.format(&mut txn, 0, 5, bold);
///
/// assert_eq!(text.get_string(&txn), "<b>hello</b> <i>world</i>");
///
/// // remove formatting
/// let remove_italic = Attrs::from([("i".into(), Any::Null)]);
/// text.format(&mut txn, 6, 5, remove_italic);
///
/// assert_eq!(text.get_string(&txn), "<b>hello</b> world");
///
/// // insert binary payload eg. images
/// let image = b"deadbeaf".to_vec();
/// text.insert_embed(&mut txn, 1, image);
///
/// // insert nested shared type eg. table as ArrayRef of ArrayRefs
/// let table = text.insert_embed(&mut txn, 5, ArrayPrelim::default());
/// let header = table.insert(&mut txn, 0, ArrayPrelim::from(["Book title", "Author"]));
/// let row = table.insert(&mut txn, 1, ArrayPrelim::from(["\"Moby-Dick\"", "Herman Melville"]));
/// ```
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct XmlTextRef(BranchPtr);

impl XmlTextRef {
    pub(crate) fn get_string_fragment(
        head: Option<ItemPtr>,
        start: Option<&StickyIndex>,
        end: Option<&StickyIndex>,
    ) -> String {
        let mut buf = String::new();
        for d in diff_between(head, start, end, YChange::identity) {
            let mut attrs = Vec::new();
            if let Some(attributes) = d.attributes.as_ref() {
                for (key, value) in attributes.iter() {
                    attrs.push((key, value));
                }
                attrs.sort_by(|x, y| x.0.cmp(y.0))
            }

            // write attributes as xml opening tags
            for (node, at) in attrs.iter() {
                write!(buf, "<{}", node).unwrap();
                if let Any::Map(at) = at {
                    for (k, v) in at.iter() {
                        write!(buf, " {}=\"{}\"", k, v).unwrap();
                    }
                }
                buf.push('>');
            }

            // write string content of delta
            if let Out::Any(any) = d.insert {
                write!(buf, "{}", any).unwrap();
            }

            // write attributes as xml closing tags
            attrs.reverse();
            for (key, _) in attrs {
                write!(buf, "</{}>", key).unwrap();
            }
        }
        buf
    }
}

impl SharedRef for XmlTextRef {}
impl Xml for XmlTextRef {}
impl Text for XmlTextRef {}
impl IndexedSequence for XmlTextRef {}
#[cfg(feature = "weak")]
impl crate::Quotable for XmlTextRef {}

impl AsRef<TextRef> for XmlTextRef {
    #[inline]
    fn as_ref(&self) -> &TextRef {
        unsafe { std::mem::transmute(self) }
    }
}

impl DeepObservable for XmlTextRef {}
impl Observable for XmlTextRef {
    type Event = XmlTextEvent;
}

impl GetString for XmlTextRef {
    fn get_string<T: ReadTxn>(&self, _txn: &T) -> String {
        XmlTextRef::get_string_fragment(self.0.start, None, None)
    }
}

impl AsRef<Branch> for XmlTextRef {
    fn as_ref(&self) -> &Branch {
        &self.0
    }
}

impl Eq for XmlTextRef {}
impl PartialEq for XmlTextRef {
    fn eq(&self, other: &Self) -> bool {
        self.0.id() == other.0.id()
    }
}

impl From<BranchPtr> for XmlTextRef {
    fn from(inner: BranchPtr) -> Self {
        XmlTextRef(inner)
    }
}

impl TryFrom<ItemPtr> for XmlTextRef {
    type Error = ItemPtr;

    fn try_from(value: ItemPtr) -> Result<Self, Self::Error> {
        if let Some(branch) = value.clone().as_branch() {
            Ok(Self::from(branch))
        } else {
            Err(value)
        }
    }
}

impl TryFrom<Out> for XmlTextRef {
    type Error = Out;

    fn try_from(value: Out) -> Result<Self, Self::Error> {
        match value {
            Out::YXmlText(value) => Ok(value),
            other => Err(other),
        }
    }
}

impl AsPrelim for XmlTextRef {
    type Prelim = XmlDeltaPrelim;

    fn as_prelim<T: ReadTxn>(&self, txn: &T) -> Self::Prelim {
        let attributes: HashMap<Arc<str>, String> = self
            .0
            .map
            .iter()
            .flat_map(|(k, ptr)| {
                if ptr.is_deleted() {
                    None
                } else if let Some(value) = ptr.content.get_last() {
                    Some((k.clone(), value.to_string(txn)))
                } else {
                    None
                }
            })
            .collect();
        let delta: Vec<Delta<In>> = self
            .diff(txn, YChange::identity)
            .into_iter()
            .map(|diff| Delta::Inserted(diff.insert.as_prelim(txn), diff.attributes))
            .collect();
        XmlDeltaPrelim { attributes, delta }
    }
}

impl DefaultPrelim for XmlTextRef {
    type Prelim = XmlTextPrelim;

    #[inline]
    fn default_prelim() -> Self::Prelim {
        XmlTextPrelim::default()
    }
}

/// A preliminary type that will be materialized into an [XmlTextRef] once it will be integrated
/// into Yrs document.
#[repr(transparent)]
#[derive(Debug, Clone, PartialEq, Hash, Default)]
pub struct XmlTextPrelim(String);

impl Deref for XmlTextPrelim {
    type Target = String;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for XmlTextPrelim {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl XmlTextPrelim {
    #[inline]
    pub fn new<S: Into<String>>(str: S) -> Self {
        XmlTextPrelim(str.into())
    }
}

impl XmlPrelim for XmlTextPrelim {}

impl Prelim for XmlTextPrelim {
    type Return = XmlTextRef;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let inner = Branch::new(TypeRef::XmlText);
        (ItemContent::Type(inner), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        if !self.is_empty() {
            let text = XmlTextRef::from(inner_ref);
            text.push(txn, &self.0);
        }
    }
}

impl Into<EmbedPrelim<XmlTextPrelim>> for XmlTextPrelim {
    #[inline]
    fn into(self) -> EmbedPrelim<XmlTextPrelim> {
        EmbedPrelim::Shared(self)
    }
}

impl From<XmlTextPrelim> for In {
    #[inline]
    fn from(value: XmlTextPrelim) -> Self {
        In::XmlText(XmlDeltaPrelim::from(value))
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct XmlDeltaPrelim {
    pub attributes: HashMap<Arc<str>, String>,
    pub delta: Vec<Delta<In>>,
}

impl Deref for XmlDeltaPrelim {
    type Target = [Delta<In>];

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.delta
    }
}

impl Prelim for XmlDeltaPrelim {
    type Return = XmlTextRef;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        (ItemContent::Type(Branch::new(TypeRef::XmlText)), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        let text_ref = XmlTextRef::from(inner_ref);
        for (key, value) in self.attributes {
            text_ref.insert_attribute(txn, key, value);
        }
        text_ref.apply_delta(txn, self.delta);
    }
}

impl From<XmlTextPrelim> for XmlDeltaPrelim {
    fn from(value: XmlTextPrelim) -> Self {
        XmlDeltaPrelim {
            attributes: HashMap::default(),
            delta: vec![Delta::Inserted(In::Any(Any::from(value.0)), None)],
        }
    }
}

impl From<XmlDeltaPrelim> for In {
    #[inline]
    fn from(value: XmlDeltaPrelim) -> Self {
        In::XmlText(value)
    }
}

/// A XML fragment, which works as an untagged collection of XML nodes.
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct XmlFragmentRef(BranchPtr);

impl RootRef for XmlFragmentRef {
    fn type_ref() -> TypeRef {
        TypeRef::XmlFragment
    }
}
impl SharedRef for XmlFragmentRef {}
impl XmlFragment for XmlFragmentRef {}
impl IndexedSequence for XmlFragmentRef {}

impl XmlFragmentRef {
    pub fn parent(&self) -> Option<XmlOut> {
        let item = self.0.item?;
        let parent = item.parent.as_branch()?;
        XmlOut::try_from(*parent).ok()
    }
}

impl AsRef<ArrayRef> for XmlFragmentRef {
    #[inline]
    fn as_ref(&self) -> &ArrayRef {
        unsafe { std::mem::transmute(self) }
    }
}

impl GetString for XmlFragmentRef {
    /// Converts current XML node into a textual representation. This representation if flat, it
    /// doesn't include any indentation.
    fn get_string<T: ReadTxn>(&self, txn: &T) -> String {
        let inner = self.0;
        let mut s = String::new();
        for i in inner.iter(txn) {
            if !i.is_deleted() {
                for content in i.content.get_content() {
                    write!(&mut s, "{}", content.to_string(txn)).unwrap();
                }
            }
        }
        s
    }
}

impl DeepObservable for XmlFragmentRef {}
impl Observable for XmlFragmentRef {
    type Event = XmlEvent;
}

impl AsRef<Branch> for XmlFragmentRef {
    fn as_ref(&self) -> &Branch {
        self.0.deref()
    }
}

impl Eq for XmlFragmentRef {}
impl PartialEq for XmlFragmentRef {
    fn eq(&self, other: &Self) -> bool {
        self.0.id() == other.0.id()
    }
}

impl From<BranchPtr> for XmlFragmentRef {
    fn from(inner: BranchPtr) -> Self {
        XmlFragmentRef(inner)
    }
}

impl TryFrom<ItemPtr> for XmlFragmentRef {
    type Error = ItemPtr;

    fn try_from(value: ItemPtr) -> Result<Self, Self::Error> {
        if let Some(branch) = value.clone().as_branch() {
            Ok(Self::from(branch))
        } else {
            Err(value)
        }
    }
}

impl TryFrom<Out> for XmlFragmentRef {
    type Error = Out;

    fn try_from(value: Out) -> Result<Self, Self::Error> {
        match value {
            Out::YXmlFragment(value) => Ok(value),
            other => Err(other),
        }
    }
}

impl AsPrelim for XmlFragmentRef {
    type Prelim = XmlFragmentPrelim;

    fn as_prelim<T: ReadTxn>(&self, txn: &T) -> Self::Prelim {
        let children: Vec<_> = self
            .children(txn)
            .map(|v| match v {
                XmlOut::Element(v) => XmlIn::from(v.as_prelim(txn)),
                XmlOut::Fragment(v) => XmlIn::from(v.as_prelim(txn)),
                XmlOut::Text(v) => XmlIn::from(v.as_prelim(txn)),
            })
            .collect();
        XmlFragmentPrelim(children)
    }
}

impl DefaultPrelim for XmlFragmentRef {
    type Prelim = XmlFragmentPrelim;

    #[inline]
    fn default_prelim() -> Self::Prelim {
        XmlFragmentPrelim::default()
    }
}

/// A preliminary type that will be materialized into an [XmlFragmentRef] once it will be integrated
/// into Yrs document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct XmlFragmentPrelim(Vec<XmlIn>);

impl XmlFragmentPrelim {
    pub fn new<I, T>(iter: I) -> Self
    where
        I: IntoIterator<Item = XmlIn>,
    {
        XmlFragmentPrelim(iter.into_iter().collect())
    }
}

impl Prelim for XmlFragmentPrelim {
    type Return = XmlFragmentRef;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let inner = Branch::new(TypeRef::XmlFragment);
        (ItemContent::Type(inner), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        let xml = XmlFragmentRef::from(inner_ref);
        for value in self.0 {
            xml.push_back(txn, value);
        }
    }
}

impl Into<EmbedPrelim<XmlFragmentPrelim>> for XmlFragmentPrelim {
    #[inline]
    fn into(self) -> EmbedPrelim<XmlFragmentPrelim> {
        EmbedPrelim::Shared(self)
    }
}

impl From<XmlFragmentPrelim> for In {
    #[inline]
    fn from(value: XmlFragmentPrelim) -> Self {
        In::XmlFragment(value)
    }
}

/// (Obsolete) an Yjs-compatible XML node used for nesting Map elements.
#[derive(Debug, Clone)]
pub struct XmlHookRef(BranchPtr);

impl Map for XmlHookRef {}

impl ToJson for XmlHookRef {
    fn to_json<T: ReadTxn>(&self, txn: &T) -> Any {
        let map: &MapRef = self.as_ref();
        map.to_json(txn)
    }
}

impl AsRef<Branch> for XmlHookRef {
    fn as_ref(&self) -> &Branch {
        self.0.deref()
    }
}

impl Eq for XmlHookRef {}
impl PartialEq for XmlHookRef {
    fn eq(&self, other: &Self) -> bool {
        self.0.id() == other.0.id()
    }
}

impl From<BranchPtr> for XmlHookRef {
    fn from(inner: BranchPtr) -> Self {
        XmlHookRef(inner)
    }
}

impl AsRef<MapRef> for XmlHookRef {
    #[inline]
    fn as_ref(&self) -> &MapRef {
        unsafe { std::mem::transmute(self) }
    }
}

pub trait Xml: AsRef<Branch> {
    fn parent(&self) -> Option<XmlOut> {
        let item = self.as_ref().item?;
        let parent = item.parent.as_branch()?;
        XmlOut::try_from(*parent).ok()
    }

    /// Removes an attribute recognized by an `attr_name` from a current XML element.
    fn remove_attribute<K>(&self, txn: &mut TransactionMut, attr_name: &K)
    where
        K: AsRef<str>,
    {
        self.as_ref().remove(txn, attr_name.as_ref());
    }

    /// Inserts an attribute entry into current XML element.
    fn insert_attribute<K, V>(&self, txn: &mut TransactionMut, key: K, value: V) -> V::Return
    where
        K: Into<Arc<str>>,
        V: Prelim,
    {
        let key = key.into();
        let pos = {
            let inner = self.as_ref();
            let left = inner.map.get(&key);
            ItemPosition {
                parent: BranchPtr::from(inner).into(),
                left: left.cloned(),
                right: None,
                index: 0,
                current_attrs: None,
            }
        };

        let ptr = txn
            .create_item(&pos, value, Some(key))
            .expect("Cannot insert empty value");
        if let Ok(integrated) = ptr.try_into() {
            integrated
        } else {
            panic!("Defect: unexpected integrated type")
        }
    }

    /// Returns a value of an attribute given its `attr_name`. Returns `None` if no such attribute
    /// can be found inside of a current XML element.
    fn get_attribute<T: ReadTxn>(&self, txn: &T, attr_name: &str) -> Option<Out> {
        let branch = self.as_ref();
        branch.get(txn, attr_name)
    }

    /// Returns an unordered iterator over all attributes (key-value pairs), that can be found
    /// inside of a current XML element.
    fn attributes<'a, T: ReadTxn>(&'a self, txn: &'a T) -> Attributes<'a, &'a T, T> {
        Attributes(Entries::new(&self.as_ref().map, txn))
    }

    fn siblings<'a, T: ReadTxn>(&self, txn: &'a T) -> Siblings<'a, T> {
        let ptr = BranchPtr::from(self.as_ref());
        Siblings::new(ptr.item, txn)
    }
}

pub trait XmlFragment: AsRef<Branch> {
    fn first_child(&self) -> Option<XmlOut> {
        let first = self.as_ref().first()?;
        match &first.content {
            ItemContent::Type(c) => {
                let ptr = BranchPtr::from(c);
                XmlOut::try_from(ptr).ok()
            }
            _ => None,
        }
    }

    /// Returns an iterator over all children of a current XML fragment.
    /// It does NOT include nested children of its children - for such cases use [Self::successors]
    /// iterator.
    fn children<'a, T: ReadTxn>(&self, txn: &'a T) -> XmlNodes<'a, T> {
        let iter = BlockIter::new(BranchPtr::from(self.as_ref()));
        XmlNodes::new(iter, txn)
    }

    /// Returns a number of elements stored in current array.
    fn len<T: ReadTxn>(&self, _txn: &T) -> u32 {
        self.as_ref().len()
    }

    /// Inserts a `value` at the given `index`. Inserting at index `0` is equivalent to prepending
    /// current array with given `value`, while inserting at array length is equivalent to appending
    /// that value at the end of it.
    ///
    /// Using `index` value that's higher than current array length results in panic.
    fn insert<V>(&self, txn: &mut TransactionMut, index: u32, xml_node: V) -> V::Return
    where
        V: XmlPrelim,
    {
        let ptr = self.as_ref().insert_at(txn, index, xml_node).unwrap(); // XML node is never empty
        if let Ok(integrated) = V::Return::try_from(ptr) {
            integrated
        } else {
            panic!("Defect: inserted XML element returned primitive value block")
        }
    }

    /// Inserts given `value` at the end of the current array.
    fn push_back<V>(&self, txn: &mut TransactionMut, xml_node: V) -> V::Return
    where
        V: XmlPrelim,
    {
        let len = self.len(txn);
        self.insert(txn, len, xml_node)
    }

    /// Inserts given `value` at the beginning of the current array.
    fn push_front<V>(&self, txn: &mut TransactionMut, xml_node: V) -> V::Return
    where
        V: XmlPrelim,
    {
        self.insert(txn, 0, xml_node)
    }

    /// Removes a single element at provided `index`.
    fn remove(&self, txn: &mut TransactionMut, index: u32) {
        self.remove_range(txn, index, 1)
    }

    /// Removes a range of elements from current array, starting at given `index` up until
    /// a particular number described by `len` has been deleted. This method panics in case when
    /// not all expected elements were removed (due to insufficient number of elements in an array)
    /// or `index` is outside the bounds of an array.
    fn remove_range(&self, txn: &mut TransactionMut, index: u32, len: u32) {
        let mut walker = BlockIter::new(BranchPtr::from(self.as_ref()));
        if walker.try_forward(txn, index) {
            walker.delete(txn, len)
        } else {
            panic!("Index {} is outside of the range of an array", index);
        }
    }

    /// Retrieves a value stored at a given `index`. Returns `None` when provided index was out
    /// of the range of a current array.
    fn get<T: ReadTxn>(&self, _txn: &T, index: u32) -> Option<XmlOut> {
        let branch = self.as_ref();
        let (content, _) = branch.get_at(index)?;
        if let ItemContent::Type(inner) = content {
            let ptr: BranchPtr = inner.into();
            XmlOut::try_from(ptr).ok()
        } else {
            None
        }
    }

    /// Returns an iterator that can be used to traverse over the successors of a current
    /// XML element. This includes recursive step over children of its children. The recursive
    /// iteration is depth-first.
    ///
    /// Example:
    /// ```
    /// /* construct node with a shape:
    ///    <div>
    ///       <p>Hello <b>world</b></p>
    ///       again
    ///    </div>
    /// */
    /// use yrs::{Doc, Text, Xml, XmlOut, Transact, XmlFragment, XmlElementPrelim, XmlTextPrelim, GetString};
    ///
    /// let doc = Doc::new();
    /// let mut html = doc.get_or_insert_xml_fragment("div");
    /// let mut txn = doc.transact_mut();
    /// let p = html.push_back(&mut txn, XmlElementPrelim::empty("p"));
    /// let txt = p.push_back(&mut txn, XmlTextPrelim::new("Hello "));
    /// let b = p.push_back(&mut txn, XmlElementPrelim::empty("b"));
    /// let txt = b.push_back(&mut txn, XmlTextPrelim::new("world"));
    /// let txt = html.push_back(&mut txn, XmlTextPrelim::new("again"));
    ///
    /// let mut result = Vec::new();
    /// for node in html.successors(&txn) {
    ///   let value = match node {
    ///       XmlOut::Element(elem) => elem.tag().to_string(),
    ///       XmlOut::Text(txt) => txt.get_string(&txn),
    ///       _ => panic!("shouldn't be the case here")
    ///   };
    ///   result.push(value);
    /// }
    /// assert_eq!(result, vec![
    ///   "p".to_string(),
    ///   "Hello ".to_string(),
    ///   "b".to_string(),
    ///   "world".to_string(),
    ///   "again".to_string()
    /// ]);
    /// ```
    fn successors<'a, T: ReadTxn>(&'a self, txn: &'a T) -> TreeWalker<'a, &'a T, T> {
        TreeWalker::new(self.as_ref(), txn)
    }
}

/// Iterator over the attributes (key-value pairs represented as a strings) of an [XmlElement].
pub struct Attributes<'a, B, T>(Entries<'a, B, T>);

impl<'a, B, T> Attributes<'a, B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
    pub fn new(branch: &'a Branch, txn: B) -> Self {
        let entries = Entries::new(&branch.map, txn);
        Attributes(entries)
    }
}

impl<'a, B, T> Iterator for Attributes<'a, B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
    type Item = (&'a str, Out);

    fn next(&mut self) -> Option<Self::Item> {
        let (key, block) = self.0.next()?;
        match block.content.get_last() {
            Some(value) => Some((key.as_ref(), value)),
            None => self.next(),
        }
    }
}

pub struct XmlNodes<'a, T> {
    iter: BlockIter,
    txn: &'a T,
}

impl<'a, T: ReadTxn> XmlNodes<'a, T> {
    fn new(iter: BlockIter, txn: &'a T) -> Self {
        XmlNodes { iter, txn }
    }
}

impl<'a, T: ReadTxn> Iterator for XmlNodes<'a, T> {
    type Item = XmlOut;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.iter.read_value(self.txn)?;
        XmlOut::try_from(value).ok()
    }
}

/// An iterator over [XmlElement] successors, working in a recursive depth-first manner.
pub struct TreeWalker<'a, B, T> {
    current: Option<&'a Item>,
    root: TypePtr,
    first_call: bool,
    _txn: B,
    _marker: PhantomData<T>,
}

impl<'a, B, T: ReadTxn> TreeWalker<'a, B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
    pub fn new(root: &'a Branch, txn: B) -> Self {
        TreeWalker {
            current: root.start.as_deref(),
            root: TypePtr::Branch(BranchPtr::from(root)),
            first_call: true,
            _txn: txn,
            _marker: PhantomData::default(),
        }
    }
}

impl<'a, B, T: ReadTxn> Iterator for TreeWalker<'a, B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
    type Item = XmlOut;

    /// Tree walker used depth-first search to move over the xml tree.
    fn next(&mut self) -> Option<Self::Item> {
        fn try_descend(item: &Item) -> Option<&Item> {
            if let ItemContent::Type(t) = &item.content {
                let inner = t.as_ref();
                match inner.type_ref() {
                    TypeRef::XmlElement(_) | TypeRef::XmlFragment if !item.is_deleted() => {
                        return inner.start.as_deref();
                    }
                    _ => { /* do nothing */ }
                }
            }

            None
        }

        let mut result = None;
        let mut n = self.current.take();
        if let Some(current) = n {
            if !self.first_call || current.is_deleted() {
                while {
                    if let Some(current) = n {
                        if let Some(ptr) = try_descend(current) {
                            // depth-first search - try walk down the tree first
                            n = Some(ptr);
                        } else {
                            // walk right or up in the tree
                            while let Some(current) = n {
                                if let Some(right) = current.right.as_ref() {
                                    n = Some(right);
                                    break;
                                } else if current.parent == self.root {
                                    n = None;
                                } else {
                                    let ptr = current.parent.as_branch().unwrap();
                                    n = ptr.item.as_deref();
                                }
                            }
                        }
                    }
                    if let Some(current) = n {
                        current.is_deleted()
                    } else {
                        false
                    }
                } {}
            }
            self.first_call = false;
            self.current = n;
        }
        if let Some(current) = self.current {
            if let ItemContent::Type(t) = &current.content {
                result = XmlOut::try_from(BranchPtr::from(t)).ok();
            }
        }
        result
    }
}

/// Event generated by [XmlText::observe] method. Emitted during transaction commit phase.
pub struct XmlTextEvent {
    pub(crate) current_target: BranchPtr,
    target: XmlTextRef,
    delta: UnsafeCell<Option<Vec<Delta>>>,
    keys: UnsafeCell<Result<HashMap<Arc<str>, EntryChange>, HashSet<Option<Arc<str>>>>>,
}

impl XmlTextEvent {
    pub(crate) fn new(branch_ref: BranchPtr, key_changes: HashSet<Option<Arc<str>>>) -> Self {
        let current_target = branch_ref.clone();
        let target = XmlTextRef::from(branch_ref);
        XmlTextEvent {
            target,
            current_target,
            delta: UnsafeCell::new(None),
            keys: UnsafeCell::new(Err(key_changes)),
        }
    }

    /// Returns a [XmlText] instance which emitted this event.
    pub fn target(&self) -> &XmlTextRef {
        &self.target
    }

    /// Returns a path from root type down to [XmlText] instance which emitted this event.
    pub fn path(&self) -> Path {
        Branch::path(self.current_target, self.target.0)
    }

    /// Returns a summary of text changes made over corresponding [XmlText] collection within
    /// bounds of current transaction.
    pub fn delta(&self, txn: &TransactionMut) -> &[Delta] {
        let delta = unsafe { self.delta.get().as_mut().unwrap() };
        delta
            .get_or_insert_with(|| TextEvent::get_delta(self.target.0, txn))
            .as_slice()
    }

    /// Returns a summary of attribute changes made over corresponding [XmlText] collection within
    /// bounds of current transaction.
    pub fn keys(&self, txn: &TransactionMut) -> &HashMap<Arc<str>, EntryChange> {
        let keys = unsafe { self.keys.get().as_mut().unwrap() };

        match keys {
            Ok(keys) => {
                return keys;
            }
            Err(subs) => {
                let subs = event_keys(txn, self.target.0, subs);
                *keys = Ok(subs);
                if let Ok(keys) = keys {
                    keys
                } else {
                    panic!("Defect: should not happen");
                }
            }
        }
    }
}

pub struct Siblings<'a, T> {
    current: Option<ItemPtr>,
    _txn: &'a T,
}

impl<'a, T> Siblings<'a, T> {
    fn new(current: Option<ItemPtr>, txn: &'a T) -> Self {
        Siblings { current, _txn: txn }
    }
}

impl<'a, T> Iterator for Siblings<'a, T> {
    type Item = XmlOut;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(item) = self.current.as_deref() {
            self.current = item.right;
            if let Some(right) = self.current.as_deref() {
                if !right.is_deleted() {
                    if let ItemContent::Type(inner) = &right.content {
                        let ptr = BranchPtr::from(inner);
                        return XmlOut::try_from(ptr).ok();
                    }
                }
            }
        }

        None
    }
}

impl<'a, T> DoubleEndedIterator for Siblings<'a, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        while let Some(item) = self.current.as_deref() {
            self.current = item.left;
            if let Some(left) = self.current.as_deref() {
                if !left.is_deleted() {
                    if let ItemContent::Type(inner) = &left.content {
                        let ptr = BranchPtr::from(inner);
                        return XmlOut::try_from(ptr).ok();
                    }
                }
            }
        }

        None
    }
}

/// Event generated by [XmlElement::observe] method. Emitted during transaction commit phase.
pub struct XmlEvent {
    pub(crate) current_target: BranchPtr,
    target: XmlOut,
    change_set: UnsafeCell<Option<Box<ChangeSet<Change>>>>,
    keys: UnsafeCell<Result<HashMap<Arc<str>, EntryChange>, HashSet<Option<Arc<str>>>>>,
    children_changed: bool,
}

impl XmlEvent {
    pub(crate) fn new(branch_ref: BranchPtr, key_changes: HashSet<Option<Arc<str>>>) -> Self {
        let current_target = branch_ref.clone();
        let children_changed = key_changes.iter().any(Option::is_none);
        XmlEvent {
            target: XmlOut::try_from(branch_ref).unwrap(),
            current_target,
            change_set: UnsafeCell::new(None),
            keys: UnsafeCell::new(Err(key_changes)),
            children_changed,
        }
    }

    /// True if any child XML nodes have been changed within bounds of current transaction.
    pub fn children_changed(&self) -> bool {
        self.children_changed
    }

    /// Returns a [XmlElement] instance which emitted this event.
    pub fn target(&self) -> &XmlOut {
        &self.target
    }

    /// Returns a path from root type down to [XmlElement] instance which emitted this event.
    pub fn path(&self) -> Path {
        Branch::path(self.current_target, self.target.as_ptr())
    }

    /// Returns a summary of XML child nodes changed within corresponding [XmlElement] collection
    /// within bounds of current transaction.
    pub fn delta(&self, txn: &TransactionMut) -> &[Change] {
        self.changes(txn).delta.as_slice()
    }

    /// Returns a collection of block identifiers that have been added within a bounds of
    /// current transaction.
    pub fn added(&self, txn: &TransactionMut) -> &HashSet<ID> {
        &self.changes(txn).added
    }

    /// Returns a collection of block identifiers that have been removed within a bounds of
    /// current transaction.
    pub fn deleted(&self, txn: &TransactionMut) -> &HashSet<ID> {
        &self.changes(txn).deleted
    }

    /// Returns a summary of attribute changes made over corresponding [XmlElement] collection
    /// within bounds of current transaction.
    pub fn keys(&self, txn: &TransactionMut) -> &HashMap<Arc<str>, EntryChange> {
        let keys = unsafe { self.keys.get().as_mut().unwrap() };

        match keys {
            Ok(keys) => keys,
            Err(subs) => {
                let subs = event_keys(txn, self.target.as_ptr(), subs);
                *keys = Ok(subs);
                if let Ok(keys) = keys {
                    keys
                } else {
                    panic!("Defect: should not happen");
                }
            }
        }
    }

    fn changes(&self, txn: &TransactionMut) -> &ChangeSet<Change> {
        let change_set = unsafe { self.change_set.get().as_mut().unwrap() };
        change_set
            .get_or_insert_with(|| Box::new(event_change_set(txn, self.target.as_ptr().start)))
    }
}

#[cfg(test)]
mod test {
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    use arc_swap::ArcSwapOption;

    use crate::test_utils::exchange_updates;
    use crate::transaction::ReadTxn;
    use crate::types::xml::{Xml, XmlFragment, XmlOut};
    use crate::types::{Attrs, Change, EntryChange, Out};
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::{Encoder, EncoderV1};
    use crate::{
        Any, Doc, GetString, Observable, SharedRef, StateVector, Text, Transact, Update,
        XmlElementPrelim, XmlTextPrelim, XmlTextRef,
    };

    #[test]
    fn insert_attribute() {
        let d1 = Doc::with_client_id(1);
        let f = d1.get_or_insert_xml_fragment("xml");
        let mut t1 = d1.transact_mut();
        let xml1 = f.push_back(&mut t1, XmlElementPrelim::empty("div"));
        xml1.insert_attribute(&mut t1, "height", 10.to_string());
        assert_eq!(xml1.get_attribute(&t1, "height"), Some(Out::from("10")));

        let d2 = Doc::with_client_id(1);
        let f = d2.get_or_insert_xml_fragment("xml");
        let mut t2 = d2.transact_mut();
        let xml2 = f.push_back(&mut t2, XmlElementPrelim::empty("div"));
        let u = t1.encode_state_as_update_v1(&StateVector::default());
        let u = Update::decode_v1(u.as_slice()).unwrap();
        t2.apply_update(u).unwrap();
        assert_eq!(xml2.get_attribute(&t2, "height"), Some(Out::from("10")));
    }

    #[test]
    fn tree_walker() {
        let doc = Doc::with_client_id(1);
        let root = doc.get_or_insert_xml_fragment("xml");
        let mut txn = doc.transact_mut();
        /*
            <UNDEFINED>
                <p>{txt1}{txt2}</p>
                <p></p>
                <img/>
            </UNDEFINED>
        */
        let p1 = root.push_back(&mut txn, XmlElementPrelim::empty("p"));
        p1.push_back(&mut txn, XmlTextPrelim::new(""));
        p1.push_back(&mut txn, XmlTextPrelim::new(""));
        let p2 = root.push_back(&mut txn, XmlElementPrelim::empty("p"));
        root.push_back(&mut txn, XmlElementPrelim::empty("img"));

        let all_paragraphs = root.successors(&txn).filter_map(|n| match n {
            XmlOut::Element(e) if e.tag() == &"p".into() => Some(e),
            _ => None,
        });
        let actual: Vec<_> = all_paragraphs.collect();

        assert_eq!(
            actual.len(),
            2,
            "query selector should found two paragraphs"
        );
        assert_eq!(
            actual[0].hook(),
            p1.hook(),
            "query selector found 1st paragraph"
        );
        assert_eq!(
            actual[1].hook(),
            p2.hook(),
            "query selector found 2nd paragraph"
        );
    }

    #[test]
    fn text_attributes() {
        let doc = Doc::with_client_id(1);
        let f = doc.get_or_insert_xml_fragment("test");
        let mut txn = doc.transact_mut();
        let txt = f.push_back(&mut txn, XmlTextPrelim::new(""));
        txt.insert_attribute(&mut txn, "test", 42.to_string());

        assert_eq!(txt.get_attribute(&txn, "test"), Some(Out::from("42")));
        let actual: Vec<_> = txt.attributes(&txn).collect();
        let expected: Vec<_> = vec![("test", Out::from("42"))].into_iter().collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn text_attributes_any() {
        let doc = Doc::with_client_id(1);
        let f = doc.get_or_insert_xml_fragment("test");
        let mut txn = doc.transact_mut();
        let txt = f.push_back(&mut txn, XmlTextPrelim::new(""));
        txt.insert_attribute(&mut txn, "test", Any::BigInt(42));
        txt.insert_attribute(&mut txn, "test_true", true);
        txt.insert_attribute(&mut txn, "test_null", Any::Null);

        assert_eq!(
            txt.get_attribute(&txn, "test"),
            Some(Out::Any(Any::BigInt(42)))
        );
        assert_eq!(
            txt.get_attribute(&txn, "test_true"),
            Some(Out::Any(Any::Bool(true)))
        );
        assert_eq!(
            txt.get_attribute(&txn, "test_null"),
            Some(Out::Any(Any::Null))
        );

        // Collect attributes into a HashSet of keys to verify all expected keys are present
        let actual_keys: HashSet<&str> = txt.attributes(&txn).map(|(k, _)| k).collect();
        let expected_keys: HashSet<&str> =
            vec!["test", "test_true", "test_null"].into_iter().collect();
        assert_eq!(actual_keys, expected_keys);
    }

    #[test]
    fn siblings() {
        let doc = Doc::with_client_id(1);
        let root = doc.get_or_insert_xml_fragment("root");
        let mut txn = doc.transact_mut();
        let first = root.push_back(&mut txn, XmlTextPrelim::new("hello"));
        let second = root.push_back(&mut txn, XmlElementPrelim::empty("p"));

        assert_eq!(
            &first.siblings(&txn).next().unwrap().id(),
            second.hook().id(),
            "first.next_sibling should point to second"
        );
        assert_eq!(
            &second.siblings(&txn).next_back().unwrap().id(),
            first.hook().id(),
            "second.prev_sibling should point to first"
        );
        assert_eq!(
            &first.parent().unwrap().id(),
            root.hook().id(),
            "first.parent should point to root"
        );
        assert!(root.parent().is_none(), "root parent should not exist");
        assert_eq!(
            &root.first_child().unwrap().id(),
            first.hook().id(),
            "root.first_child should point to first"
        );
    }

    #[test]
    fn serialization() {
        let d1 = Doc::with_client_id(1);
        let r1 = d1.get_or_insert_xml_fragment("root");
        let mut t1 = d1.transact_mut();
        let _first = r1.push_back(&mut t1, XmlTextPrelim::new("hello"));
        r1.push_back(&mut t1, XmlElementPrelim::empty("p"));

        let expected = "hello<p></p>";
        assert_eq!(r1.get_string(&t1), expected);

        let u1 = t1.encode_state_as_update_v1(&StateVector::default());

        let d2 = Doc::with_client_id(2);
        let r2 = d2.get_or_insert_xml_fragment("root");
        let mut t2 = d2.transact_mut();

        let u1 = Update::decode_v1(u1.as_slice()).unwrap();
        t2.apply_update(u1).unwrap();
        assert_eq!(r2.get_string(&t2), expected);
    }

    #[test]
    fn serialization_compatibility() {
        let d1 = Doc::with_client_id(1);
        let r1 = d1.get_or_insert_xml_fragment("root");
        let mut t1 = d1.transact_mut();
        let _first = r1.push_back(&mut t1, XmlTextPrelim::new("hello"));
        r1.push_back(&mut t1, XmlElementPrelim::empty("p"));

        /* This binary is result of following Yjs code (matching Rust code above):
        ```js
            let d1 = new Y.Doc()
            d1.clientID = 1
            let root = d1.get('root', Y.XmlElement)
            let first = new Y.XmlText()
            first.insert(0, 'hello')
            let second = new Y.XmlElement('p')
            root.insert(0, [first,second])

            let expected = Y.encodeStateAsUpdate(d1)
        ``` */
        let expected = &[
            1, 3, 1, 0, 7, 1, 4, 114, 111, 111, 116, 6, 4, 0, 1, 0, 5, 104, 101, 108, 108, 111,
            135, 1, 0, 3, 1, 112, 0,
        ];
        let u1 = t1.encode_state_as_update_v1(&StateVector::default());
        assert_eq!(u1.as_slice(), expected);
    }

    #[test]
    fn event_observers() {
        let d1 = Doc::with_client_id(1);
        let f = d1.get_or_insert_xml_fragment("xml");
        let xml = f.insert(&mut d1.transact_mut(), 0, XmlElementPrelim::empty("test"));

        let d2 = Doc::with_client_id(2);
        let f = d2.get_or_insert_xml_fragment("xml");
        exchange_updates(&[&d1, &d2]);
        let xml2 = f
            .get(&d2.transact(), 0)
            .unwrap()
            .into_xml_element()
            .unwrap();

        let attributes = Arc::new(ArcSwapOption::default());
        let nodes = Arc::new(ArcSwapOption::default());
        let attributes_c = attributes.clone();
        let nodes_c = nodes.clone();
        let _sub = xml.observe(move |txn, e| {
            attributes_c.store(Some(Arc::new(e.keys(txn).clone())));
            nodes_c.store(Some(Arc::new(e.delta(txn).to_vec())));
        });

        // insert attribute
        {
            let mut txn = d1.transact_mut();
            xml.insert_attribute(&mut txn, "key1", "value1");
            xml.insert_attribute(&mut txn, "key2", "value2");
        }
        assert!(nodes.swap(None).unwrap().is_empty());
        assert_eq!(
            attributes.swap(None),
            Some(Arc::new(HashMap::from([
                (
                    "key1".into(),
                    EntryChange::Inserted(Any::String("value1".into()).into())
                ),
                (
                    "key2".into(),
                    EntryChange::Inserted(Any::String("value2".into()).into())
                )
            ])))
        );

        // change and remove attribute
        {
            let mut txn = d1.transact_mut();
            xml.insert_attribute(&mut txn, "key1", "value11");
            xml.remove_attribute(&mut txn, &"key2");
        }
        assert!(nodes.swap(None).unwrap().is_empty());
        assert_eq!(
            attributes.swap(None),
            Some(Arc::new(HashMap::from([
                (
                    "key1".into(),
                    EntryChange::Updated(
                        Any::String("value1".into()).into(),
                        Any::String("value11".into()).into()
                    )
                ),
                (
                    "key2".into(),
                    EntryChange::Removed(Any::String("value2".into()).into())
                )
            ])))
        );

        // add xml elements
        let (nested_txt, nested_xml) = {
            let mut txn = d1.transact_mut();
            let txt = xml.insert(&mut txn, 0, XmlTextPrelim::new(""));
            let xml2 = xml.insert(&mut txn, 1, XmlElementPrelim::empty("div"));
            (txt, xml2)
        };
        assert_eq!(
            nodes.swap(None),
            Some(Arc::new(vec![Change::Added(vec![
                Out::YXmlText(nested_txt.clone()),
                Out::YXmlElement(nested_xml.clone())
            ])]))
        );
        assert_eq!(attributes.swap(None), Some(HashMap::new().into()));

        // remove and add
        let nested_xml2 = {
            let mut txn = d1.transact_mut();
            xml.remove_range(&mut txn, 1, 1);
            xml.insert(&mut txn, 1, XmlElementPrelim::empty("p"))
        };
        assert_eq!(
            nodes.swap(None),
            Some(Arc::new(vec![
                Change::Retain(1),
                Change::Added(vec![Out::YXmlElement(nested_xml2.clone())]),
                Change::Removed(1),
            ]))
        );
        assert_eq!(attributes.swap(None), Some(HashMap::new().into()));

        // copy updates over
        let attributes = Arc::new(ArcSwapOption::default());
        let nodes = Arc::new(ArcSwapOption::default());
        let attributes_c = attributes.clone();
        let nodes_c = nodes.clone();
        let _sub = xml2.observe(move |txn, e| {
            attributes_c.store(Some(Arc::new(e.keys(txn).clone())));
            nodes_c.store(Some(Arc::new(e.delta(txn).to_vec())));
        });

        {
            let t1 = d1.transact_mut();
            let mut t2 = d2.transact_mut();
            let sv = t2.state_vector();
            let mut encoder = EncoderV1::new();
            t1.encode_diff(&sv, &mut encoder);
            let update = Update::decode_v1(encoder.to_vec().as_slice()).unwrap();
            t2.apply_update(update).unwrap();
        }
        assert_eq!(
            nodes.swap(None),
            Some(Arc::new(vec![Change::Added(vec![
                Out::YXmlText(nested_txt),
                Out::YXmlElement(nested_xml2)
            ])]))
        );
        assert_eq!(
            attributes.swap(None),
            Some(Arc::new(HashMap::from([(
                "key1".into(),
                EntryChange::Inserted(Any::String("value11".into()).into())
            )])))
        );
    }

    #[test]
    fn xml_to_string() {
        let doc = Doc::new();
        let f = doc.get_or_insert_xml_fragment("test");
        let mut txn = doc.transact_mut();
        let div = f.push_back(&mut txn, XmlElementPrelim::empty("div"));
        div.insert_attribute(&mut txn, "class", "t-button");
        let text = div.push_back(&mut txn, XmlTextPrelim::new("hello world"));
        text.format(
            &mut txn,
            6,
            5,
            Attrs::from([(
                "a".into(),
                HashMap::from([("href".into(), "http://domain.org")]).into(),
            )]),
        );
        drop(txn);

        let str = f.get_string(&doc.transact());
        assert_eq!(
            str.as_str(),
            "<div class=\"t-button\">hello <a href=\"http://domain.org\">world</a></div>"
        )
    }

    #[test]
    fn xml_to_string_2() {
        let doc = Doc::new();
        let f = doc.get_or_insert_xml_fragment("article");
        let xml = f.insert(&mut doc.transact_mut(), 0, XmlTextPrelim::new(""));
        let mut txn = doc.transact_mut();

        let bold = Attrs::from([("b".into(), true.into())]);
        let italic = Attrs::from([("i".into(), true.into())]);

        xml.insert(&mut txn, 0, "hello ");
        xml.insert_with_attributes(&mut txn, 6, "world", italic);
        xml.format(&mut txn, 0, 5, bold);

        assert_eq!(xml.get_string(&txn), "<b>hello</b> <i>world</i>");

        let remove_italic = Attrs::from([("i".into(), Any::Null)]);
        xml.format(&mut txn, 6, 5, remove_italic);

        assert_eq!(xml.get_string(&txn), "<b>hello</b> world");
    }

    #[test]
    fn format_attributes_decode_compatibility_v1() {
        let data = &[
            1, 6, 1, 0, 6, 1, 4, 116, 101, 115, 116, 1, 105, 4, 116, 114, 117, 101, 132, 1, 0, 6,
            104, 101, 108, 108, 111, 32, 132, 1, 6, 5, 119, 111, 114, 108, 100, 134, 1, 11, 1, 105,
            4, 110, 117, 108, 108, 198, 1, 6, 1, 7, 1, 98, 4, 116, 114, 117, 101, 134, 1, 12, 1,
            98, 4, 110, 117, 108, 108, 0,
        ];
        let update = Update::decode_v1(data).unwrap();
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let txt: &XmlTextRef = txt.as_ref();
        let mut txn = doc.transact_mut();

        txn.apply_update(update).unwrap();
        assert_eq!(txt.get_string(&txn), "<i>hello </i><b><i>world</i></b>");

        let actual = txn.encode_state_as_update_v1(&StateVector::default());
        assert_eq!(actual, data);
    }

    #[test]
    fn format_attributes_decode_compatibility_v2() {
        let data = &[
            0, 3, 0, 3, 1, 2, 65, 5, 5, 0, 12, 10, 74, 12, 1, 14, 9, 6, 0, 132, 1, 134, 0, 198, 0,
            134, 26, 19, 116, 101, 115, 116, 105, 104, 101, 108, 108, 111, 32, 119, 111, 114, 108,
            100, 105, 98, 98, 4, 1, 6, 5, 65, 1, 1, 1, 0, 0, 1, 6, 0, 120, 126, 120, 126, 0,
        ];
        let update = Update::decode_v2(data).unwrap();
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let txt: &XmlTextRef = txt.as_ref();
        let mut txn = doc.transact_mut();

        txn.apply_update(update).unwrap();
        assert_eq!(txt.get_string(&txn), "<i>hello </i><b><i>world</i></b>");

        let actual = txn.encode_state_as_update_v2(&StateVector::default());
        assert_eq!(actual, data);
    }
}
