use crate::block::{Block, BlockPtr, EmbedPrelim, Item, ItemContent, ItemPosition, Prelim};
use crate::block_iter::BlockIter;
use crate::transaction::TransactionMut;
use crate::types::text::{TextEvent, YChange};
use crate::types::{
    event_change_set, event_keys, Branch, BranchPtr, Change, ChangeSet, Delta, Entries,
    EntryChange, EventHandler, MapRef, Observers, Path, ToJson, TypePtr, Value,
    TYPE_REFS_XML_ELEMENT, TYPE_REFS_XML_FRAGMENT, TYPE_REFS_XML_TEXT,
};
use crate::{
    ArrayRef, GetString, IndexedSequence, Map, Observable, ReadTxn, StickyIndex, Text, TextRef, ID,
};
use lib0::any::Any;
use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::collections::{HashMap, HashSet};
use std::convert::{TryFrom, TryInto};
use std::fmt::Write;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

/// Trait shared by preliminary types that can be used as XML nodes: [XmlElementPrelim],
/// [XmlFragmentPrelim] and [XmlTextPrelim].
pub trait XmlPrelim: Prelim {}

/// An return type from XML elements retrieval methods. It's an enum of all supported values, that
/// can be nested inside of [XmlElementRef]. These are other [XmlElementRef]s, [XmlFragmentRef]s
/// or [XmlTextRef] values.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum XmlNode {
    Element(XmlElementRef),
    Fragment(XmlFragmentRef),
    Text(XmlTextRef),
}

impl XmlNode {
    pub fn as_ptr(&self) -> BranchPtr {
        match self {
            XmlNode::Element(n) => n.0,
            XmlNode::Fragment(n) => n.0,
            XmlNode::Text(n) => n.0,
        }
    }
}

impl AsRef<Branch> for XmlNode {
    fn as_ref(&self) -> &Branch {
        match self {
            XmlNode::Element(n) => n.as_ref(),
            XmlNode::Fragment(n) => n.as_ref(),
            XmlNode::Text(n) => n.as_ref(),
        }
    }
}

impl TryInto<XmlElementRef> for XmlNode {
    type Error = XmlNode;

    fn try_into(self) -> Result<XmlElementRef, Self::Error> {
        match self {
            XmlNode::Element(xml) => Ok(xml),
            other => Err(other),
        }
    }
}

impl TryInto<XmlTextRef> for XmlNode {
    type Error = XmlNode;

    fn try_into(self) -> Result<XmlTextRef, Self::Error> {
        match self {
            XmlNode::Text(xml) => Ok(xml),
            other => Err(other),
        }
    }
}

impl TryInto<XmlFragmentRef> for XmlNode {
    type Error = XmlNode;

    fn try_into(self) -> Result<XmlFragmentRef, Self::Error> {
        match self {
            XmlNode::Fragment(xml) => Ok(xml),
            other => Err(other),
        }
    }
}

impl TryFrom<BranchPtr> for XmlNode {
    type Error = BranchPtr;

    fn try_from(value: BranchPtr) -> Result<Self, Self::Error> {
        let type_ref = { value.type_ref & 0b1111 };
        match type_ref {
            TYPE_REFS_XML_ELEMENT => Ok(XmlNode::Element(XmlElementRef::from(value))),
            TYPE_REFS_XML_TEXT => Ok(XmlNode::Text(XmlTextRef::from(value))),
            TYPE_REFS_XML_FRAGMENT => Ok(XmlNode::Fragment(XmlFragmentRef::from(value))),
            _ => Err(value),
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
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct XmlElementRef(BranchPtr);

impl Xml for XmlElementRef {}
impl XmlFragment for XmlElementRef {}
impl IndexedSequence for XmlElementRef {}

impl Into<XmlFragmentRef> for XmlElementRef {
    fn into(self) -> XmlFragmentRef {
        XmlFragmentRef(self.0)
    }
}

impl Into<ArrayRef> for XmlElementRef {
    fn into(self) -> ArrayRef {
        ArrayRef::from(self.0)
    }
}

impl Into<MapRef> for XmlElementRef {
    fn into(self) -> MapRef {
        MapRef::from(self.0)
    }
}

impl XmlElementRef {
    /// A tag name of a current top-level XML node, eg. node `<p></p>` has "p" as it's tag name.
    pub fn tag(&self) -> &str {
        let inner = &self.0;
        inner.name.as_ref().unwrap()
    }
}

impl GetString for XmlElementRef {
    /// Converts current XML node into a textual representation. This representation if flat, it
    /// doesn't include any indentation.
    fn get_string<T: ReadTxn>(&self, txn: &T) -> String {
        let inner = self.0;
        let mut s = String::new();
        let tag = inner
            .name
            .as_ref()
            .map(|s| s.as_ref())
            .unwrap_or(&"UNDEFINED");
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

impl Observable for XmlElementRef {
    type Event = XmlEvent;

    fn try_observer(&self) -> Option<&EventHandler<Self::Event>> {
        if let Some(Observers::XmlFragment(eh)) = self.0.observers.as_ref() {
            Some(eh)
        } else {
            None
        }
    }

    fn try_observer_mut(&mut self) -> Option<&mut EventHandler<Self::Event>> {
        if let Observers::XmlFragment(eh) =
            self.0.observers.get_or_insert_with(Observers::xml_fragment)
        {
            Some(eh)
        } else {
            None
        }
    }
}

impl AsRef<Branch> for XmlElementRef {
    fn as_ref(&self) -> &Branch {
        &self.0
    }
}

impl AsMut<Branch> for XmlElementRef {
    fn as_mut(&mut self) -> &mut Branch {
        &mut self.0
    }
}

impl From<BranchPtr> for XmlElementRef {
    fn from(inner: BranchPtr) -> Self {
        XmlElementRef(inner)
    }
}

impl TryFrom<BlockPtr> for XmlElementRef {
    type Error = BlockPtr;

    fn try_from(value: BlockPtr) -> Result<Self, Self::Error> {
        if let Some(branch) = value.clone().as_branch() {
            Ok(Self::from(branch))
        } else {
            Err(value)
        }
    }
}

/// A preliminary type that will be materialized into an [XmlElementRef] once it will be integrated
/// into Yrs document.
#[derive(Debug, Clone)]
pub struct XmlElementPrelim<I, T>(Rc<str>, I)
where
    I: IntoIterator<Item = T>,
    T: XmlPrelim;

impl<I, T> XmlElementPrelim<I, T>
where
    I: IntoIterator<Item = T>,
    T: XmlPrelim,
{
    pub fn new<S: Into<Rc<str>>>(tag: S, iter: I) -> Self {
        XmlElementPrelim(tag.into(), iter)
    }
}

impl XmlElementPrelim<Option<XmlTextPrelim<String>>, XmlTextPrelim<String>> {
    pub fn empty<S: Into<Rc<str>>>(tag: S) -> Self {
        XmlElementPrelim(tag.into(), None)
    }
}

impl<I, T> XmlPrelim for XmlElementPrelim<I, T>
where
    I: IntoIterator<Item = T>,
    T: XmlPrelim,
{
}

impl<I, T> Prelim for XmlElementPrelim<I, T>
where
    I: IntoIterator<Item = T>,
    T: XmlPrelim,
{
    type Return = XmlElementRef;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let inner = Branch::new(TYPE_REFS_XML_ELEMENT, Some(self.0.clone()));
        (ItemContent::Type(inner), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        let xml = XmlElementRef::from(inner_ref);
        for value in self.1 {
            xml.push_back(txn, value);
        }
    }
}

impl<I, T: Prelim> Into<EmbedPrelim<XmlElementPrelim<I, T>>> for XmlElementPrelim<I, T>
where
    I: IntoIterator<Item = T>,
    T: XmlPrelim,
{
    #[inline]
    fn into(self) -> EmbedPrelim<XmlElementPrelim<I, T>> {
        EmbedPrelim::Shared(self)
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
/// [permanent index](StickyIndex) position that sticks to the same place even when concurrent
/// updates are being made.
///
/// # Example
///
/// ```rust
/// use lib0::any::Any;
/// use yrs::{Array, ArrayPrelim, Doc, GetString, Text, Transact};
/// use yrs::types::Attrs;
///
/// let doc = Doc::new();
/// let text = doc.get_or_insert_xml_text("article");
/// let mut txn = doc.transact_mut();
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
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct XmlTextRef(BranchPtr);

impl Xml for XmlTextRef {}
impl Text for XmlTextRef {}
impl IndexedSequence for XmlTextRef {}

impl Into<TextRef> for XmlTextRef {
    fn into(self) -> TextRef {
        TextRef::from(self.0)
    }
}

impl Observable for XmlTextRef {
    type Event = XmlTextEvent;

    fn try_observer(&self) -> Option<&EventHandler<Self::Event>> {
        if let Some(Observers::XmlText(eh)) = self.0.observers.as_ref() {
            Some(eh)
        } else {
            None
        }
    }

    fn try_observer_mut(&mut self) -> Option<&mut EventHandler<Self::Event>> {
        if let Observers::XmlText(eh) = self.0.observers.get_or_insert_with(Observers::xml_text) {
            Some(eh)
        } else {
            None
        }
    }
}

impl GetString for XmlTextRef {
    fn get_string<T: ReadTxn>(&self, txn: &T) -> String {
        let mut buf = String::new();
        for d in self.diff(txn, YChange::identity) {
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
            if let Value::Any(any) = d.insert {
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

impl AsRef<Branch> for XmlTextRef {
    fn as_ref(&self) -> &Branch {
        &self.0
    }
}

impl AsMut<Branch> for XmlTextRef {
    fn as_mut(&mut self) -> &mut Branch {
        &mut self.0
    }
}

impl From<BranchPtr> for XmlTextRef {
    fn from(inner: BranchPtr) -> Self {
        XmlTextRef(inner)
    }
}

impl TryFrom<BlockPtr> for XmlTextRef {
    type Error = BlockPtr;

    fn try_from(value: BlockPtr) -> Result<Self, Self::Error> {
        if let Some(branch) = value.clone().as_branch() {
            Ok(Self::from(branch))
        } else {
            Err(value)
        }
    }
}

/// A preliminary type that will be materialized into an [XmlTextRef] once it will be integrated
/// into Yrs document.
#[derive(Debug)]
pub struct XmlTextPrelim<T: Borrow<str>>(T);

impl<T: Borrow<str>> XmlTextPrelim<T> {
    #[inline]
    pub fn new(str: T) -> Self {
        XmlTextPrelim(str)
    }
}

impl<T: Borrow<str>> XmlPrelim for XmlTextPrelim<T> {}

impl<T: Borrow<str>> Prelim for XmlTextPrelim<T> {
    type Return = XmlTextRef;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let inner = Branch::new(TYPE_REFS_XML_TEXT, None);
        (ItemContent::Type(inner), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        let s = self.0.borrow();
        if !s.is_empty() {
            let text = XmlTextRef::from(inner_ref);
            text.push(txn, s);
        }
    }
}

impl<T: Borrow<str>> Into<EmbedPrelim<XmlTextPrelim<T>>> for XmlTextPrelim<T> {
    #[inline]
    fn into(self) -> EmbedPrelim<XmlTextPrelim<T>> {
        EmbedPrelim::Shared(self)
    }
}

/// A XML fragment, which works as an untagged collection of XML nodes.
#[repr(transparent)]
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct XmlFragmentRef(BranchPtr);

impl XmlFragment for XmlFragmentRef {}
impl IndexedSequence for XmlFragmentRef {}

impl XmlFragmentRef {
    pub fn parent(&self) -> Option<XmlNode> {
        let block = self.as_ref().item?;
        let item = block.as_item()?;
        let parent = item.parent.as_branch()?;
        XmlNode::try_from(*parent).ok()
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

impl Observable for XmlFragmentRef {
    type Event = XmlEvent;

    fn try_observer(&self) -> Option<&EventHandler<Self::Event>> {
        if let Some(Observers::XmlFragment(eh)) = self.0.observers.as_ref() {
            Some(eh)
        } else {
            None
        }
    }

    fn try_observer_mut(&mut self) -> Option<&mut EventHandler<Self::Event>> {
        if let Observers::XmlFragment(eh) =
            self.0.observers.get_or_insert_with(Observers::xml_fragment)
        {
            Some(eh)
        } else {
            None
        }
    }
}

impl AsRef<Branch> for XmlFragmentRef {
    fn as_ref(&self) -> &Branch {
        self.0.deref()
    }
}

impl AsMut<Branch> for XmlFragmentRef {
    fn as_mut(&mut self) -> &mut Branch {
        self.0.deref_mut()
    }
}

impl From<BranchPtr> for XmlFragmentRef {
    fn from(inner: BranchPtr) -> Self {
        XmlFragmentRef(inner)
    }
}

impl TryFrom<BlockPtr> for XmlFragmentRef {
    type Error = BlockPtr;

    fn try_from(value: BlockPtr) -> Result<Self, Self::Error> {
        if let Some(branch) = value.clone().as_branch() {
            Ok(Self::from(branch))
        } else {
            Err(value)
        }
    }
}

/// A preliminary type that will be materialized into an [XmlFragmentRef] once it will be integrated
/// into Yrs document.
#[derive(Debug, Clone)]
pub struct XmlFragmentPrelim<I, T>(I)
where
    I: IntoIterator<Item = T>,
    T: XmlPrelim;

impl<I, T> XmlFragmentPrelim<I, T>
where
    I: IntoIterator<Item = T>,
    T: XmlPrelim,
{
    pub fn new(iter: I) -> Self {
        XmlFragmentPrelim(iter)
    }
}

impl<I, T> XmlPrelim for XmlFragmentPrelim<I, T>
where
    I: IntoIterator<Item = T>,
    T: XmlPrelim,
    <T as Prelim>::Return: TryFrom<BlockPtr>,
{
}

impl<I, T> Prelim for XmlFragmentPrelim<I, T>
where
    I: IntoIterator<Item = T>,
    T: XmlPrelim,
    <T as Prelim>::Return: TryFrom<BlockPtr>,
{
    type Return = XmlFragmentRef;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let inner = Branch::new(TYPE_REFS_XML_FRAGMENT, None);
        (ItemContent::Type(inner), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        let xml = XmlFragmentRef::from(inner_ref);
        for value in self.0 {
            xml.push_back(txn, value);
        }
    }
}

impl<I, T: Prelim> Into<EmbedPrelim<XmlFragmentPrelim<I, T>>> for XmlFragmentPrelim<I, T>
where
    I: IntoIterator<Item = T>,
    T: XmlPrelim,
{
    #[inline]
    fn into(self) -> EmbedPrelim<XmlFragmentPrelim<I, T>> {
        EmbedPrelim::Shared(self)
    }
}

/// (Obsolete) an Yjs-compatible XML node used for nesting Map elements.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct XmlHookRef(BranchPtr);

impl Map for XmlHookRef {}

impl ToJson for XmlHookRef {
    fn to_json<T: ReadTxn>(&self, txn: &T) -> Any {
        let map: MapRef = self.clone().into();
        map.to_json(txn)
    }
}

impl AsRef<Branch> for XmlHookRef {
    fn as_ref(&self) -> &Branch {
        self.0.deref()
    }
}

impl AsMut<Branch> for XmlHookRef {
    fn as_mut(&mut self) -> &mut Branch {
        self.0.deref_mut()
    }
}

impl From<BranchPtr> for XmlHookRef {
    fn from(inner: BranchPtr) -> Self {
        XmlHookRef(inner)
    }
}

impl Into<MapRef> for XmlHookRef {
    fn into(self) -> MapRef {
        MapRef::from(self.0)
    }
}

pub trait Xml: AsRef<Branch> {
    fn parent(&self) -> Option<XmlNode> {
        let block = self.as_ref().item?;
        let item = block.as_item()?;
        let parent = item.parent.as_branch()?;
        XmlNode::try_from(*parent).ok()
    }

    /// Removes an attribute recognized by an `attr_name` from a current XML element.
    fn remove_attribute<K>(&self, txn: &mut TransactionMut, attr_name: &K)
    where
        K: AsRef<str>,
    {
        self.as_ref().remove(txn, attr_name.as_ref());
    }

    /// Inserts an attribute entry into current XML element.
    fn insert_attribute<K, V>(&self, txn: &mut TransactionMut, attr_name: K, attr_value: V)
    where
        K: Into<Rc<str>>,
        V: AsRef<str>,
    {
        let key = attr_name.into();
        let value = crate::block::PrelimString(attr_value.as_ref().into());
        let pos = {
            let branch = self.as_ref();
            let left = branch.map.get(&key);
            ItemPosition {
                parent: BranchPtr::from(branch).into(),
                left: left.cloned(),
                right: None,
                index: 0,
                current_attrs: None,
            }
        };

        txn.create_item(&pos, value, Some(key));
    }

    /// Returns a value of an attribute given its `attr_name`. Returns `None` if no such attribute
    /// can be found inside of a current XML element.
    fn get_attribute<T: ReadTxn>(&self, txn: &T, attr_name: &str) -> Option<String> {
        let branch = self.as_ref();
        let value = branch.get(txn, attr_name)?;
        Some(value.to_string(txn))
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
    fn first_child(&self) -> Option<XmlNode> {
        let first = self.as_ref().first()?;
        match &first.content {
            ItemContent::Type(c) => {
                let ptr = BranchPtr::from(c);
                XmlNode::try_from(ptr).ok()
            }
            _ => None,
        }
    }
    /// Returns a number of elements stored in current array.
    fn len<T: ReadTxn>(&self, txn: &T) -> u32 {
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
        let ptr = self.as_ref().insert_at(txn, index, xml_node);
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
    /// or `index` is outside of the bounds of an array.
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
    fn get<T: ReadTxn>(&self, txn: &T, index: u32) -> Option<XmlNode> {
        let branch = self.as_ref();
        let (content, _) = branch.get_at(index)?;
        if let ItemContent::Type(inner) = content {
            let ptr: BranchPtr = inner.into();
            XmlNode::try_from(ptr).ok()
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
    /// use yrs::{Doc, Text, Xml, XmlNode, Transact, XmlFragment, XmlElementPrelim, XmlTextPrelim, GetString};
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
    ///       XmlNode::Element(elem) => elem.tag().to_string(),
    ///       XmlNode::Text(txt) => txt.get_string(&txn),
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
    type Item = (&'a str, String);

    fn next(&mut self) -> Option<Self::Item> {
        let (key, block) = self.0.next()?;
        let txn = self.0.txn.borrow();
        let value = block
            .content
            .get_last()
            .map(|v| v.to_string(txn))
            .unwrap_or(String::default());
        Some((key.as_ref(), value))
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
        let current = if let Some(Block::Item(item)) = root.start.as_deref() {
            Some(item)
        } else {
            None
        };

        TreeWalker {
            current,
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
    type Item = XmlNode;

    /// Tree walker used depth-first search to move over the xml tree.
    fn next(&mut self) -> Option<Self::Item> {
        fn try_descend(item: &Item) -> Option<&Block> {
            if let ItemContent::Type(t) = &item.content {
                let inner = t.as_ref();
                let type_ref = inner.type_ref();
                if !item.is_deleted()
                    && (type_ref == TYPE_REFS_XML_ELEMENT || type_ref == TYPE_REFS_XML_FRAGMENT)
                {
                    return inner.start.as_deref();
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
                            n = ptr.as_item();
                        } else {
                            // walk right or up in the tree
                            while let Some(current) = n {
                                if let Some(right) = current.right.as_ref() {
                                    n = right.as_item();
                                    break;
                                } else if current.parent == self.root {
                                    n = None;
                                } else {
                                    let ptr = current.parent.as_branch().unwrap();
                                    n = if let Some(Block::Item(item)) = ptr.item.as_deref() {
                                        Some(item)
                                    } else {
                                        None
                                    };
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
                result = XmlNode::try_from(BranchPtr::from(t)).ok();
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
    keys: UnsafeCell<Result<HashMap<Rc<str>, EntryChange>, HashSet<Option<Rc<str>>>>>,
}

impl XmlTextEvent {
    pub(crate) fn new(branch_ref: BranchPtr, key_changes: HashSet<Option<Rc<str>>>) -> Self {
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
    pub fn keys(&self, txn: &TransactionMut) -> &HashMap<Rc<str>, EntryChange> {
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
    current: Option<BlockPtr>,
    _txn: &'a T,
}

impl<'a, T> Siblings<'a, T> {
    fn new(current: Option<BlockPtr>, txn: &'a T) -> Self {
        Siblings { current, _txn: txn }
    }
}

impl<'a, T> Iterator for Siblings<'a, T> {
    type Item = XmlNode;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(Block::Item(item)) = self.current.as_deref() {
            self.current = item.right;
            if let Some(Block::Item(right)) = self.current.as_deref() {
                if !right.is_deleted() {
                    if let ItemContent::Type(inner) = &right.content {
                        let ptr = BranchPtr::from(inner);
                        return XmlNode::try_from(ptr).ok();
                    }
                }
            }
        }

        None
    }
}

impl<'a, T> DoubleEndedIterator for Siblings<'a, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        while let Some(Block::Item(item)) = self.current.as_deref() {
            self.current = item.left;
            if let Some(Block::Item(left)) = self.current.as_deref() {
                if !left.is_deleted() {
                    if let ItemContent::Type(inner) = &left.content {
                        let ptr = BranchPtr::from(inner);
                        return XmlNode::try_from(ptr).ok();
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
    target: XmlNode,
    change_set: UnsafeCell<Option<Box<ChangeSet<Change>>>>,
    keys: UnsafeCell<Result<HashMap<Rc<str>, EntryChange>, HashSet<Option<Rc<str>>>>>,
    children_changed: bool,
}

impl XmlEvent {
    pub(crate) fn new(branch_ref: BranchPtr, key_changes: HashSet<Option<Rc<str>>>) -> Self {
        let current_target = branch_ref.clone();
        let children_changed = key_changes.iter().any(Option::is_none);
        XmlEvent {
            target: XmlNode::try_from(branch_ref).unwrap(),
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
    pub fn target(&self) -> &XmlNode {
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
    pub fn keys(&self, txn: &TransactionMut) -> &HashMap<Rc<str>, EntryChange> {
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
    use crate::transaction::ReadTxn;
    use crate::types::xml::{Xml, XmlFragment, XmlNode};
    use crate::types::{Attrs, Change, EntryChange, Value};
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::{Encoder, EncoderV1};
    use crate::{
        Doc, GetString, Observable, Options, StateVector, Text, Transact, Update, XmlElementPrelim,
        XmlTextPrelim,
    };
    use lib0::any::Any;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::rc::Rc;

    #[test]
    fn insert_attribute() {
        let d1 = Doc::with_client_id(1);
        let f = d1.get_or_insert_xml_fragment("xml");
        let mut t1 = d1.transact_mut();
        let xml1 = f.push_back(&mut t1, XmlElementPrelim::empty("div"));
        xml1.insert_attribute(&mut t1, "height", 10.to_string());
        assert_eq!(xml1.get_attribute(&t1, "height"), Some("10".to_string()));

        let d2 = Doc::with_client_id(1);
        let f = d2.get_or_insert_xml_fragment("xml");
        let mut t2 = d2.transact_mut();
        let xml2 = f.push_back(&mut t2, XmlElementPrelim::empty("div"));
        let u = t1
            .encode_state_as_update_v1(&StateVector::default())
            .unwrap();
        t2.apply_update(Update::decode_v1(u.as_slice()).unwrap());
        assert_eq!(xml2.get_attribute(&t2, "height"), Some("10".to_string()));
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
            XmlNode::Element(e) if e.tag() == "p" => Some(e),
            _ => None,
        });
        let actual: Vec<_> = all_paragraphs.collect();

        assert_eq!(
            actual.len(),
            2,
            "query selector should found two paragraphs"
        );
        assert_eq!(actual[0], p1, "query selector found 1st paragraph");
        assert_eq!(actual[1], p2, "query selector found 2nd paragraph");
    }

    #[test]
    fn text_attributes() {
        let doc = Doc::with_client_id(1);
        let f = doc.get_or_insert_xml_fragment("test");
        let mut txn = doc.transact_mut();
        let txt = f.push_back(&mut txn, XmlTextPrelim::new(""));
        txt.insert_attribute(&mut txn, "test", 42.to_string());

        assert_eq!(txt.get_attribute(&txn, "test"), Some("42".to_string()));
        let actual: Vec<_> = txt.attributes(&txn).collect();
        assert_eq!(actual, vec![("test", "42".to_string())]);
    }

    #[test]
    fn siblings() {
        let doc = Doc::with_client_id(1);
        let root = doc.get_or_insert_xml_fragment("root");
        let mut txn = doc.transact_mut();
        let first = root.push_back(&mut txn, XmlTextPrelim::new("hello"));
        let second = root.push_back(&mut txn, XmlElementPrelim::empty("p"));

        assert_eq!(
            first.siblings(&txn).next().as_ref(),
            Some(&XmlNode::Element(second.clone())),
            "first.next_sibling should point to second"
        );
        assert_eq!(
            second.siblings(&txn).next_back().as_ref(),
            Some(&XmlNode::Text(first.clone())),
            "second.prev_sibling should point to first"
        );
        assert_eq!(
            first.parent().as_ref(),
            Some(&XmlNode::Fragment(root.clone())),
            "first.parent should point to root"
        );
        assert!(root.parent().is_none(), "root parent should not exist");
        assert_eq!(
            root.first_child().as_ref(),
            Some(&XmlNode::Text(first)),
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

        let u1 = t1
            .encode_state_as_update_v1(&StateVector::default())
            .unwrap();

        let d2 = Doc::with_client_id(2);
        let r2 = d2.get_or_insert_xml_fragment("root");
        let mut t2 = d2.transact_mut();

        t2.apply_update(Update::decode_v1(u1.as_slice()).unwrap());
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
        let u1 = t1
            .encode_state_as_update_v1(&StateVector::default())
            .unwrap();
        assert_eq!(u1.as_slice(), expected);
    }

    #[test]
    fn event_observers() {
        let d1 = Doc::with_client_id(1);
        let mut xml = d1.get_or_insert_xml_element("test");

        let attributes = Rc::new(RefCell::new(None));
        let nodes = Rc::new(RefCell::new(None));
        let attributes_c = attributes.clone();
        let nodes_c = nodes.clone();
        let _sub = xml.observe(move |txn, e| {
            *attributes_c.borrow_mut() = Some(e.keys(txn).clone());
            *nodes_c.borrow_mut() = Some(e.delta(txn).to_vec());
        });

        // insert attribute
        {
            let mut txn = d1.transact_mut();
            xml.insert_attribute(&mut txn, "key1", "value1");
            xml.insert_attribute(&mut txn, "key2", "value2");
        }
        assert!(nodes.borrow_mut().take().unwrap().is_empty());
        assert_eq!(
            attributes.borrow_mut().take(),
            Some(HashMap::from([
                (
                    "key1".into(),
                    EntryChange::Inserted(Any::String("value1".into()).into())
                ),
                (
                    "key2".into(),
                    EntryChange::Inserted(Any::String("value2".into()).into())
                )
            ]))
        );

        // change and remove attribute
        {
            let mut txn = d1.transact_mut();
            xml.insert_attribute(&mut txn, "key1", "value11");
            xml.remove_attribute(&mut txn, &"key2");
        }
        assert!(nodes.borrow_mut().take().unwrap().is_empty());
        assert_eq!(
            attributes.borrow_mut().take(),
            Some(HashMap::from([
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
            ]))
        );

        // add xml elements
        let (nested_txt, nested_xml) = {
            let mut txn = d1.transact_mut();
            let txt = xml.insert(&mut txn, 0, XmlTextPrelim::new(""));
            let xml2 = xml.insert(&mut txn, 1, XmlElementPrelim::empty("div"));
            (txt, xml2)
        };
        assert_eq!(
            nodes.borrow_mut().take(),
            Some(vec![Change::Added(vec![
                Value::YXmlText(nested_txt.clone()),
                Value::YXmlElement(nested_xml.clone())
            ])])
        );
        assert_eq!(attributes.borrow_mut().take(), Some(HashMap::new()));

        // remove and add
        let nested_xml2 = {
            let mut txn = d1.transact_mut();
            xml.remove_range(&mut txn, 1, 1);
            xml.insert(&mut txn, 1, XmlElementPrelim::empty("p"))
        };
        assert_eq!(
            nodes.borrow_mut().take(),
            Some(vec![
                Change::Retain(1),
                Change::Added(vec![Value::YXmlElement(nested_xml2.clone())]),
                Change::Removed(1),
            ])
        );
        assert_eq!(attributes.borrow_mut().take(), Some(HashMap::new()));

        // copy updates over
        let d2 = Doc::with_client_id(2);
        let mut xml2 = d2.get_or_insert_xml_element("test");

        let attributes = Rc::new(RefCell::new(None));
        let nodes = Rc::new(RefCell::new(None));
        let attributes_c = attributes.clone();
        let nodes_c = nodes.clone();
        let _sub = xml2.observe(move |txn, e| {
            *attributes_c.borrow_mut() = Some(e.keys(txn).clone());
            *nodes_c.borrow_mut() = Some(e.delta(txn).to_vec());
        });

        {
            let t1 = d1.transact_mut();
            let mut t2 = d2.transact_mut();
            let sv = t2.state_vector();
            let mut encoder = EncoderV1::new();
            t1.encode_diff(&sv, &mut encoder).unwrap();
            t2.apply_update(Update::decode_v1(encoder.to_vec().as_slice()).unwrap());
        }
        assert_eq!(
            nodes.borrow_mut().take(),
            Some(vec![Change::Added(vec![
                Value::YXmlText(nested_txt),
                Value::YXmlElement(nested_xml2)
            ])])
        );
        assert_eq!(
            attributes.borrow_mut().take(),
            Some(HashMap::from([(
                "key1".into(),
                EntryChange::Inserted(Any::String("value11".into()).into())
            )]))
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
        let xml = doc.get_or_insert_xml_text("article");
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
        let txt = doc.get_or_insert_xml_text("test");
        let mut txn = doc.transact_mut();

        txn.apply_update(update);
        assert_eq!(txt.get_string(&txn), "<i>hello </i><b><i>world</i></b>");

        let actual = txn
            .encode_state_as_update_v1(&StateVector::default())
            .unwrap();
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
        let txt = doc.get_or_insert_xml_text("test");
        let mut txn = doc.transact_mut();

        txn.apply_update(update);
        assert_eq!(txt.get_string(&txn), "<i>hello </i><b><i>world</i></b>");

        let actual = txn
            .encode_state_as_update_v2(&StateVector::default())
            .unwrap();
        assert_eq!(actual, data);
    }
}
