use std::collections::hash_map::Entry;
use std::collections::{Bound, HashSet};
use std::convert::TryFrom;
use std::marker::PhantomData;
use std::ops::{DerefMut, RangeBounds};
use std::sync::Arc;

use thiserror::Error;

use crate::block::{EmbedPrelim, ItemContent, ItemPtr, Prelim};
use crate::iter::{
    AsIter, BlockIterator, BlockSliceIterator, IntoBlockIter, MoveIter, RangeIter, TxnIterator,
    Values,
};
use crate::out::FromOut;
use crate::transaction::TransactionState;
use crate::types::{AsPrelim, Branch, BranchPtr, Out, Path, SharedRef, TypeRef};
use crate::{
    Array, Assoc, DeepObservable, Doc, GetString, In, IndexScope, Map, Observable, StickyIndex,
    TextRef, Transaction, TransactionMut, XmlTextRef, ID,
};

/// Weak link reference represents a reference to a single element or consecutive range of elements
/// stored in another collection in the same document.
///
/// The same element may be linked by many [WeakRef]s, however the ownership still belongs to
/// a collection, where referenced elements were originally inserted in. For this reason removing
/// [WeakRef] doesn't affect linked elements. [WeakRef] can also be outdated when the linked
/// reference has been removed.
///
/// In order to create a [WeakRef], a preliminary [WeakPrelim] element must be obtained first. This
/// can be done via either:
///
/// - [Map::link] to pick a reference to key-value entry of map. As entry is being updated, so will
/// be the referenced value.
/// - [Array::quote] to take a reference to a consecutive range of array's elements. Any elements
/// inserted in an originally quoted range will later on appear when [WeakRef::unquote] is called.
/// - [Text::quote] to take a reference to a slice of text. When materialized, quoted slice will
/// contain any changes that happened within the quoted slice. It will also contain formatting
/// information about the quotation.
///
/// [WeakPrelim] can be used like any preliminary type (ie. inserted into array, map or as embedded
/// value in text), producing [WeakRef] in a result. [WeakRef] can be also cloned and converted back
/// into [WeakPrelim], allowing to reference the same element(s) in many different places.
///
/// [WeakRef] can also be observed on via [WeakRef::observe]/[WeakRef::observe_deep]. These enable
/// to react to changes which happen in other parts of the document tree.
///
/// # Example
///
/// ```rust
/// use yrs::{Array, Doc, Map, Quotable, Assoc};
///
/// let mut doc = Doc::new();
/// let array = doc.get_or_insert_array("array");
/// let map = doc.get_or_insert_map("map");
/// let mut txn = doc.transact_mut();
///
/// // insert values
/// array.insert_range(&mut txn, 0, ["A", "B", "C", "D"]);
///
/// // link the reference for value in another collection
/// let link = array.quote(&txn, 1..=2).unwrap(); // [B, C]
/// let link = map.insert(&mut txn, "key", link);
///
/// // evaluate quoted range
/// let values: Vec<_> = link.unquote(&txn).map(|v| v.to_string(&txn)).collect();
/// assert_eq!(values, vec!["B".to_string(), "C".to_string()]);
///
/// // update quoted range
/// array.insert(&mut txn, 2, "E"); // [A, B, E, C, D]
///
/// // evaluate quoted range (updated)
/// let values: Vec<_> = link.unquote(&txn).map(|v| v.to_string(&txn)).collect();
/// assert_eq!(values, vec!["B".to_string(), "E".to_string(), "C".to_string()]);
/// ```
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct WeakRef<P>(P);

impl<P: SharedRef> SharedRef for WeakRef<P> {}
impl SharedRef for WeakRef<BranchPtr> {}
impl<P: SharedRef> From<WeakRef<BranchPtr>> for WeakRef<P> {
    fn from(value: WeakRef<BranchPtr>) -> Self {
        WeakRef(P::from(value.0))
    }
}
impl<P: AsRef<Branch>> AsRef<Branch> for WeakRef<P> {
    fn as_ref(&self) -> &Branch {
        self.0.as_ref()
    }
}
impl<P: AsRef<Branch>> WeakRef<P> {
    /// Returns a [LinkSource] corresponding with current [WeakRef].
    /// Returns `None` if underlying branch reference was not meant to be used as [WeakRef].
    pub fn try_source(&self) -> Option<&Arc<LinkSource>> {
        let branch = self.as_ref();
        if let TypeRef::WeakLink(source) = &branch.type_ref {
            Some(source)
        } else {
            None
        }
    }

    /// Returns a [LinkSource] corresponding with current [WeakRef].
    ///
    /// # Panics
    ///
    /// This method panic if an underlying branch was not meant to be used as [WeakRef]. This can
    /// happen if a different shared type was forcibly casted to [WeakRef]. To avoid panic, use
    /// [WeakRef::try_source] instead.
    pub fn source(&self) -> &Arc<LinkSource> {
        self.try_source()
            .expect("Defect: called WeakRef-specific method over non-WeakRef shared type")
    }

    /// Returns a block [ID] to a beginning of a quoted range.
    /// For quotes linking to a single elements this is equal to [WeakRef::end_id].
    pub fn start_id(&self) -> Option<&ID> {
        self.source().quote_start.id()
    }

    /// Returns a block [ID] to an ending of a quoted range.
    /// For quotes linking to a single elements this is equal to [WeakRef::start_id].
    pub fn end_id(&self) -> Option<&ID> {
        self.source().quote_end.id()
    }
}

impl<P: From<BranchPtr>> From<BranchPtr> for WeakRef<P> {
    fn from(inner: BranchPtr) -> Self {
        WeakRef(P::from(inner))
    }
}

impl<P> FromOut for WeakRef<P>
where
    P: FromOut + From<BranchPtr>,
{
    fn from_out(value: Out, txn: &Transaction) -> Result<Self, Out>
    where
        Self: Sized,
    {
        match value {
            Out::WeakLink(value) => Ok(WeakRef(value.0.into())),
            other => Err(other),
        }
    }

    fn from_item(item: ItemPtr, txn: &Transaction) -> Option<Self>
    where
        Self: Sized,
    {
        let inner = P::from_item(item, txn)?;
        Some(WeakRef(inner))
    }
}

impl<P: AsRef<Branch>> Eq for WeakRef<P> {}
impl<P: AsRef<Branch>> PartialEq for WeakRef<P> {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref().id() == other.as_ref().id()
    }
}

impl<P> DeepObservable for WeakRef<P> where P: AsRef<Branch> {}
impl<P> Observable for WeakRef<P>
where
    P: AsRef<Branch>,
{
    type Event = WeakEvent;
}

impl GetString for WeakRef<TextRef> {
    /// Returns a plain string representation of an underlying range of a quoted [TextRef].
    ///
    /// # Example
    ///
    /// ```rust
    /// use yrs::{Assoc, Doc, GetString, Map, Quotable, Text};
    ///
    /// let mut doc = Doc::new();
    /// let text = doc.get_or_insert_text("text");
    /// let map = doc.get_or_insert_map("map");
    /// let mut txn = doc.transact_mut();
    ///
    /// // initialize text
    /// text.insert(&mut txn, 0, "hello world!");
    ///
    /// // link fragment of text
    /// let link = text.quote(&mut txn, 0..=5).unwrap(); // 'hello '
    /// let link = map.insert(&mut txn, "key", link);
    ///
    /// // check the quoted fragment
    /// assert_eq!(link.get_string(&txn), "hello ".to_string());
    /// ```
    fn get_string(&self, txn: &Transaction) -> String {
        self.source().to_string(txn.doc())
    }
}

impl GetString for WeakRef<XmlTextRef> {
    /// Returns a XML-formatted string representation of an underlying range of a quoted [XmlTextRef].
    ///
    /// # Example
    ///
    /// ```rust
    /// use yrs::{Assoc, Doc, GetString, Map, Quotable, Text, XmlFragment, XmlTextPrelim};
    /// use yrs::types::Attrs;
    ///
    /// let mut doc = Doc::new();
    /// let f = doc.get_or_insert_xml_fragment("xml");
    /// let map = doc.get_or_insert_map("map");
    /// let mut txn = doc.transact_mut();
    /// let text = f.insert(&mut txn, 0, XmlTextPrelim::new("Bold, italic text"));
    ///
    /// // add formatting
    /// let italic = Attrs::from([("i".into(), true.into())]);
    /// let bold = Attrs::from([("b".into(), true.into())]);
    /// text.format(&mut txn, 0, 4, bold); // '<b>Bold</b>, italic text'
    /// text.format(&mut txn, 6, 6, italic); // '<b>Bold</b>, <i>italic</i> text'
    ///
    /// // link fragment of text
    /// let link = text.quote(&mut txn, 1..=10).unwrap(); // '<b>old</b>, <i>itali</i>'
    /// let link = map.insert(&mut txn, "key", link);
    ///
    /// // check the quoted fragment
    /// assert_eq!(link.get_string(&txn), "<b>old</b>, <i>itali</i>".to_string());
    /// ```
    fn get_string(&self, txn: &Transaction) -> String {
        self.source().to_xml_string(txn.doc())
    }
}

impl<P: AsRef<Branch>> WeakRef<P> {
    pub fn into_inner(self) -> WeakRef<BranchPtr> {
        WeakRef(BranchPtr::from(self.0.as_ref()))
    }
}

impl<P> WeakRef<P>
where
    P: SharedRef + Map,
{
    /// Tries to dereference a value for linked [Map] entry, performing automatic conversion if
    /// possible. If conversion was not possible or element didn't exist, an error case will be
    /// returned.
    ///
    /// Use [WeakRef::try_deref_value] if conversion is not possible or desired at the current moment.
    pub fn try_deref<V>(&self, txn: &Transaction) -> Option<V>
    where
        V: FromOut,
    {
        let value = self.try_deref_value(txn)?;
        V::from_out(value, txn).ok()
    }

    /// Tries to dereference a value for linked [Map] entry. If element didn't exist, `None` will
    /// be returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// use yrs::{Doc, Map};
    ///
    /// let mut doc = Doc::new();
    /// let map = doc.get_or_insert_map("map");
    /// let mut txn = doc.transact_mut();
    ///
    /// // insert a value and the link referencing it
    /// map.insert(&mut txn, "A", "value");
    /// let link = map.link(&txn, "A").unwrap();
    /// let link = map.insert(&mut txn, "B", link);
    ///
    /// assert_eq!(link.try_deref_value(&txn), Some("value".into()));
    ///
    /// // update entry and check if link has been updated
    /// map.insert(&mut txn, "A", "other");
    /// assert_eq!(link.try_deref_value(&txn), Some("other".into()));
    /// ```
    pub fn try_deref_value(&self, txn: &Transaction) -> Option<Out> {
        let source = self.try_source()?;
        let item = source.quote_start.get_item(txn.doc());
        let last = item.to_iter().last()?;
        if last.is_deleted() {
            None
        } else {
            last.content.get_last()
        }
    }
}

impl<P> WeakRef<P>
where
    P: SharedRef + Array,
{
    /// Returns an iterator over [Out]s existing in a scope of the current [WeakRef] quotation
    /// range.
    pub fn unquote<'a>(&self, txn: &'a Transaction) -> Unquote<'a> {
        if let Some(source) = self.try_source() {
            source.unquote(txn.doc())
        } else {
            Unquote::empty()
        }
    }
}

impl<V> AsPrelim for WeakRef<V>
where
    V: From<BranchPtr> + AsRef<Branch> + FromOut,
{
    type Prelim = WeakPrelim<V>;

    fn as_prelim(&self, _txn: &Transaction) -> Self::Prelim {
        let source = self.try_source().unwrap();
        WeakPrelim::with_source(source.clone())
    }
}

/// A preliminary type for [WeakRef]. Once inserted into document it can be used as a weak reference
/// link to another value living inside of the document store.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WeakPrelim<P> {
    source: Arc<LinkSource>,
    _marker: PhantomData<P>,
}

impl<P> WeakPrelim<P> {
    pub(crate) fn new(start: StickyIndex, end: StickyIndex) -> Self {
        let source = Arc::new(LinkSource::new(start, end));
        WeakPrelim {
            source,
            _marker: PhantomData::default(),
        }
    }
    pub(crate) fn with_source(source: Arc<LinkSource>) -> Self {
        WeakPrelim {
            source,
            _marker: PhantomData::default(),
        }
    }

    pub fn into_inner(&self) -> WeakPrelim<BranchPtr> {
        WeakPrelim {
            source: self.source.clone(),
            _marker: PhantomData::default(),
        }
    }

    pub fn source(&self) -> &Arc<LinkSource> {
        &self.source
    }
}

impl<P> WeakPrelim<P>
where
    P: SharedRef + Array,
{
    /// Returns an iterator over [Out]s existing in a scope of the current [WeakPrelim] quotation
    /// range.
    pub fn unquote<'a>(&self, txn: &'a Transaction) -> Unquote<'a> {
        self.source.unquote(txn.doc())
    }
}

impl<P> WeakPrelim<P>
where
    P: SharedRef + Map,
{
    pub fn try_deref_raw(&self, txn: &Transaction) -> Option<Out> {
        self.source.unquote(txn.doc()).next()
    }

    pub fn try_deref<V>(&self, txn: &Transaction) -> Result<V, Option<V::Error>>
    where
        V: TryFrom<Out>,
    {
        if let Some(value) = self.try_deref_raw(txn) {
            match V::try_from(value) {
                Ok(value) => Ok(value),
                Err(value) => Err(Some(value)),
            }
        } else {
            Err(None)
        }
    }
}

impl GetString for WeakPrelim<TextRef> {
    fn get_string(&self, txn: &Transaction) -> String {
        self.source.to_string(txn.doc())
    }
}

impl GetString for WeakPrelim<XmlTextRef> {
    fn get_string(&self, txn: &Transaction) -> String {
        self.source.to_xml_string(txn.doc())
    }
}

impl<P: AsRef<Branch>> From<WeakRef<P>> for WeakPrelim<P> {
    fn from(value: WeakRef<P>) -> Self {
        let branch = value.0.as_ref();
        if let TypeRef::WeakLink(source) = &branch.type_ref {
            WeakPrelim {
                source: source.clone(),
                _marker: PhantomData::default(),
            }
        } else {
            panic!("Defect: WeakRef's underlying branch is not matching expected weak ref.")
        }
    }
}

impl<P: AsRef<Branch>> WeakPrelim<P> {
    pub fn upcast(self) -> WeakPrelim<BranchPtr> {
        WeakPrelim {
            source: self.source,
            _marker: Default::default(),
        }
    }
}

impl<P> Prelim for WeakPrelim<P>
where
    P: FromOut + From<BranchPtr>,
{
    type Return = WeakRef<P>;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let inner = Branch::new(TypeRef::WeakLink(self.source.clone()));
        (ItemContent::Type(inner), Some(self))
    }
}

impl<P: SharedRef> From<WeakPrelim<BranchPtr>> for WeakPrelim<P> {
    fn from(value: WeakPrelim<BranchPtr>) -> Self {
        WeakPrelim {
            source: value.source,
            _marker: Default::default(),
        }
    }
}

impl<P> Into<EmbedPrelim<WeakPrelim<P>>> for WeakPrelim<P> {
    fn into(self) -> EmbedPrelim<WeakPrelim<P>> {
        EmbedPrelim::Shared(self)
    }
}

impl<T> From<WeakPrelim<T>> for In {
    #[inline]
    fn from(value: WeakPrelim<T>) -> Self {
        In::WeakLink(value.into_inner())
    }
}

pub struct WeakEvent {
    pub(crate) current_target: BranchPtr,
    target: BranchPtr,
}

impl WeakEvent {
    pub(crate) fn new(branch_ref: BranchPtr) -> Self {
        let current_target = branch_ref.clone();
        WeakEvent {
            target: branch_ref,
            current_target,
        }
    }

    pub fn as_target<T: From<BranchPtr>>(&self) -> WeakRef<T> {
        WeakRef(T::from(self.target))
    }

    /// Returns a path from root type down to [Text] instance which emitted this event.
    pub fn path(&self) -> Path {
        Branch::path(self.current_target, self.target)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct LinkSource {
    pub(crate) quote_start: StickyIndex,
    pub(crate) quote_end: StickyIndex,
}

impl LinkSource {
    pub fn new(start: StickyIndex, end: StickyIndex) -> Self {
        LinkSource {
            quote_start: start,
            quote_end: end,
        }
    }

    #[inline]
    pub fn is_single(&self) -> bool {
        match (self.quote_start.scope(), self.quote_end.scope()) {
            (IndexScope::Relative(x), IndexScope::Relative(y)) => x == y,
            _ => false,
        }
    }

    /// Remove reference to current weak link from all items it quotes.
    pub(crate) fn unlink_all(
        &self,
        state: &mut TransactionState,
        doc: &mut Doc,
        branch_ptr: BranchPtr,
    ) {
        let item = self.quote_start.get_item(doc);
        let mut i = item.to_iter().moved();
        while let Some(item) = i.next(doc) {
            if item.info.is_linked() {
                state.unlink(doc, item, branch_ptr);
            }
        }
    }

    pub(crate) fn unquote<'a>(&self, doc: &'a Doc) -> Unquote<'a> {
        let mut current = self.quote_start.get_item(doc);
        if let Some(ptr) = &mut current {
            if Self::try_right_most(ptr) {
                current = Some(*ptr);
            }
        }
        if let Some(item) = current.as_deref() {
            let parent = *item.parent.as_branch().unwrap();
            Unquote::new(
                doc,
                parent,
                self.quote_start.clone(),
                self.quote_end.clone(),
            )
        } else {
            Unquote::empty()
        }
    }

    /// If provided ref is pointing to map type which has been updated, we may want to invalidate
    /// current pointer to point to its right most neighbor.
    fn try_right_most(item: &mut ItemPtr) -> bool {
        if item.parent_sub.is_some() {
            // for map types go to the most recent one
            if let Some(curr_block) = item.right.to_iter().last() {
                *item = curr_block;
                return true;
            }
        }
        false
    }

    pub(crate) fn materialize(&self, doc: &mut Doc, inner_ref: BranchPtr) {
        let curr = if let Some(ptr) = self.quote_start.get_item(doc) {
            ptr
        } else {
            // referenced element has already been GCed
            return;
        };
        if curr.parent_sub.is_some() {
            // for maps, advance to most recent item
            if let Some(mut last) = Some(curr).to_iter().last() {
                last.info.set_linked();
                let linked_by = doc.linked_by.entry(last).or_default();
                linked_by.insert(inner_ref);
            }
        } else {
            let mut first = true;
            let from = self.quote_start.clone();
            let to = self.quote_end.clone();
            let mut i = Some(curr).to_iter().moved().within_range(from, to);
            while let Some(slice) = i.next(doc) {
                let mut item = if !slice.adjacent() {
                    doc.materialize(slice)
                } else {
                    slice.ptr
                };
                if first {
                    first = false;
                }
                item.info.set_linked();
                let linked_by = doc.linked_by.entry(item).or_default();
                linked_by.insert(inner_ref);
            }
        }
    }

    pub fn to_string(&self, doc: &Doc) -> String {
        let mut result = String::new();
        let mut curr = self.quote_start.get_item(doc);
        let end = self.quote_end.id();
        while let Some(item) = curr.as_deref() {
            if let Some(end) = end {
                if self.quote_end.assoc == Assoc::Before && &item.id == end {
                    // right side is open (last item excluded)
                    break;
                }
            }
            if !item.is_deleted() {
                if let ItemContent::String(s) = &item.content {
                    result.push_str(s.as_str());
                }
            }
            if let Some(end) = end {
                if self.quote_end.assoc == Assoc::After && &item.last_id() == end {
                    // right side is closed (last item included)
                    break;
                }
            }
            curr = item.right;
        }
        result
    }

    pub fn to_xml_string(&self, doc: &Doc) -> String {
        let curr = self.quote_start.get_item(doc);
        if let Some(item) = curr.as_deref() {
            if let Some(branch) = item.parent.as_branch() {
                return XmlTextRef::get_string_fragment(
                    branch.start,
                    Some(&self.quote_start),
                    Some(&self.quote_end),
                );
            }
        }
        String::new()
    }
}

/// Iterator over non-deleted items, bounded by the given ID range.
pub struct Unquote<'a>(Option<AsIter<'a, Values<RangeIter<MoveIter>>>>);

impl<'a> Unquote<'a> {
    fn new(doc: &'a Doc, parent: BranchPtr, from: StickyIndex, to: StickyIndex) -> Self {
        let iter = parent
            .start
            .to_iter()
            .moved()
            .within_range(from, to)
            .values();
        Unquote(Some(AsIter::new(iter, doc)))
    }

    fn empty() -> Self {
        Unquote(None)
    }
}

impl<'a> Iterator for Unquote<'a> {
    type Item = Out;

    fn next(&mut self) -> Option<Self::Item> {
        let iter = self.0.as_mut()?;
        iter.next()
    }
}

/// Trait which defines a capability to quote a range of elements from implementing collection
/// and referencing them later in other collections.
pub trait Quotable: AsRef<Branch> + Sized {
    /// Returns [WeakPrelim] to a given range of elements, if it's in a boundaries of a current
    /// quotable collection.
    ///
    /// Quoted ranges inclusivity define behavior of quote in face of concurrent inserts that might
    /// have happened, example:
    /// - Inclusive range (eg. `1..=2`) means, that any concurrent inserts that happen between
    ///   indexes 2 and 3 will **not** be part of the quoted range.
    /// - Exclusive range (eg. `1..3`) theoretically being similar to an upper one, will behave
    ///   differently as for concurrent inserts on 2nd and 3rd index boundary, these inserts will be
    ///   counted as a part of quoted range.
    ///
    /// # Errors
    ///
    /// This method may return an [QuoteError::OutOfBounds] if passed range params span beyond
    /// the boundaries of a current collection ie. `0..yarray.len()` will error, as the upper index
    /// refers to position that's not present in current collection - even though the position
    /// itself is not included in range it still has to exists as a point of reference.
    ///
    /// Currently this method doesn't support unbounded ranges (ie. `..n`, `n..`). Passing such
    /// range will cause [QuoteError::UnboundedRange] error.
    ///
    /// # Example
    /// ```
    /// use yrs::{Doc, Array, Assoc, Quotable};
    /// let mut doc = Doc::new();
    /// let array = doc.get_or_insert_array("array");
    /// array.insert_range(&mut doc.transact_mut(), 0, [1,2,3,4]);
    /// // quote elements 2 and 3
    /// let prelim = array.quote(&doc.transact(), 1..3).unwrap();
    /// let quote = array.insert(&mut doc.transact_mut(), 0, prelim);
    /// // retrieve quoted values
    /// let quoted: Vec<_> = quote.unquote(&doc.transact()).collect();
    /// assert_eq!(quoted, vec![2.into(), 3.into()]);
    /// ```
    fn quote<R>(&self, txn: &Transaction, range: R) -> Result<WeakPrelim<Self>, QuoteError>
    where
        R: RangeBounds<u32>,
    {
        let this = BranchPtr::from(self.as_ref());
        let start = match range.start_bound() {
            Bound::Included(&i) => Some((i, Assoc::Before)),
            Bound::Excluded(&i) => Some((i, Assoc::After)),
            Bound::Unbounded => None,
        };
        let end = match range.end_bound() {
            Bound::Included(&i) => Some((i, Assoc::After)),
            Bound::Excluded(&i) => Some((i, Assoc::Before)),
            Bound::Unbounded => None,
        };
        let doc = txn.doc();
        let encoding = doc.offset_kind();
        let mut start_index = 0;
        let mut remaining = start_index;
        let mut curr = None;
        let mut i = this.start.to_iter().moved();

        let start = if let Some((start_i, assoc_start)) = start {
            start_index = start_i;
            remaining = start_index;
            // figure out the first ID
            curr = i.next(doc);
            while let Some(item) = curr.as_deref() {
                if remaining == 0 {
                    break;
                }
                if !item.is_deleted() && item.is_countable() {
                    let len = item.content_len(encoding);
                    if remaining < len {
                        break;
                    }
                    remaining -= len;
                }
                curr = i.next(doc);
            }
            let start_id = if let Some(item) = curr.as_deref() {
                let mut id = item.id.clone();
                id.clock += if let ItemContent::String(s) = &item.content {
                    s.block_offset(remaining, encoding)
                } else {
                    remaining
                };
                id
            } else {
                return Err(QuoteError::OutOfBounds);
            };
            StickyIndex::new(IndexScope::Relative(start_id), assoc_start)
        } else {
            curr = i.next(doc);
            StickyIndex::new(IndexScope::from_branch(this), Assoc::Before)
        };

        let end = if let Some((end_index, assoc_end)) = end {
            // figure out the last ID
            remaining = end_index - start_index + remaining;
            while let Some(item) = curr.as_deref() {
                if !item.is_deleted() && item.is_countable() {
                    let len = item.content_len(encoding);
                    if remaining < len {
                        break;
                    }
                    remaining -= len;
                }
                curr = i.next(doc);
            }
            let end_id = if let Some(item) = curr.as_deref() {
                let mut id = item.id.clone();
                id.clock += if let ItemContent::String(s) = &item.content {
                    s.block_offset(remaining, encoding)
                } else {
                    remaining
                };
                id
            } else {
                return Err(QuoteError::OutOfBounds);
            };
            StickyIndex::new(IndexScope::Relative(end_id), assoc_end)
        } else {
            StickyIndex::new(IndexScope::from_branch(this), Assoc::After)
        };

        let source = LinkSource::new(start, end);
        Ok(WeakPrelim::with_source(Arc::new(source)))
    }
}

/// Error that may appear in result of [Quotable::quote] method call.
#[derive(Debug, Error)]
pub enum QuoteError {
    /// Range lower or upper indexes passed to [Quotable::quote] were beyond scope of quoted
    /// collection.
    ///
    /// Remember: even though range itself may not include index (ie. `1..n`), that index still
    /// needs to point to existing value within quoted collection (`n < ytype.len()`) as a point
    /// of reference.
    #[error("Quoted range spans beyond the bounds of current collection")]
    OutOfBounds,
}

pub(crate) fn join_linked_range(mut block: ItemPtr, txn: &mut TransactionMut) {
    let item = block.deref_mut();
    // this item may exists within a quoted range
    item.info.set_linked();
    // we checked if left and right exists before this method call
    let left = item.left.unwrap();
    let right = item.right.unwrap();
    let all_links = &mut txn.doc_mut().linked_by;
    let left_links = all_links.get(&left);
    let right_links = all_links.get(&right);
    let mut common = HashSet::new();
    if let Some(llinks) = left_links {
        for link in llinks.iter() {
            match right_links {
                Some(rlinks) if rlinks.contains(link) => {
                    // new item existing in a quoted range in between two elements
                    common.insert(*link);
                }
                _ => {
                    if let TypeRef::WeakLink(source) = &link.type_ref {
                        if source.quote_end.assoc == Assoc::Before {
                            // We're at the right edge of quoted range - right neighbor is not included
                            // but the left one is. Since quotation is open on the right side, we need to
                            // include current item.
                            common.insert(*link);
                        }
                    }
                }
            }
        }
    }
    if let Some(rlinks) = right_links {
        for link in rlinks.iter() {
            match left_links {
                Some(llinks) if llinks.contains(link) => {
                    /* already visited by previous if-loop */
                }
                _ => {
                    if let TypeRef::WeakLink(source) = &link.type_ref {
                        if source.quote_start.assoc == Assoc::After {
                            let start_id = source.quote_start.id().cloned();
                            let prev_id = item.left.map(|i| i.last_id());
                            if start_id == prev_id {
                                // even though current boundary if left-side exclusive, current item
                                // has been inserted on the right of it, therefore it's within range
                                common.insert(*link);
                            }
                        }
                    }
                }
            }
        }
    }
    if !common.is_empty() {
        match all_links.entry(block) {
            Entry::Occupied(mut e) => {
                let links = e.get_mut();
                for link in common {
                    links.insert(link);
                }
            }
            Entry::Vacant(e) => {
                e.insert(common);
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::collections::{Bound, HashMap};
    use std::ops::RangeBounds;
    use std::sync::{Arc, Mutex};

    use arc_swap::ArcSwapOption;

    use crate::test_utils::exchange_updates;
    use crate::types::text::YChange;
    use crate::types::weak::{WeakPrelim, WeakRef};
    use crate::types::{Attrs, EntryChange, Event, Out, ToJson};
    use crate::Assoc::{After, Before};
    use crate::{
        Array, ArrayRef, DeepObservable, Doc, GetString, Map, MapPrelim, MapRef, Observable,
        Quotable, Text, TextRef, XmlTextRef,
    };

    #[test]
    fn basic_map_link() {
        let mut doc = Doc::new();
        let map = doc.get_or_insert_map("map");
        let mut txn = doc.transact_mut();
        let nested = MapPrelim::from([("a1".to_owned(), "hello".to_owned())]);
        let nested = map.insert(&mut txn, "a", nested);
        let link = map.link(&txn, "a").unwrap();
        map.insert(&mut txn, "b", link);

        let link: WeakRef<MapRef> = map.get(&txn, "b").unwrap();

        let expected = nested.to_json(&txn);
        let deref: MapRef = link.try_deref(&txn).unwrap();
        let actual = deref.to_json(&txn);

        assert_eq!(actual, expected);
    }

    #[test]
    fn basic_array_link() {
        let mut d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");
        {
            let mut txn = d1.transact_mut();

            a1.insert_range(&mut txn, 0, [1, 2, 3]);
            let link = a1.quote(&txn, 1..2).unwrap();
            a1.insert(&mut txn, 3, link);

            assert_eq!(a1.get(&txn, 0), Some(1));
            assert_eq!(a1.get(&txn, 1), Some(2));
            assert_eq!(a1.get(&txn, 2), Some(3));
            let mut u: WeakRef<ArrayRef> = a1.get(&txn, 3).unwrap();
            let mut u = u.unquote(&txn);
            assert_eq!(u.next(), Some(2.into()));
            assert_eq!(u.next(), None);
        }

        let mut d2 = Doc::new();
        let a2 = d2.get_or_insert_array("array");

        exchange_updates([&mut d1, &mut d2]);
        let txn = d2.transact_mut();

        assert_eq!(a2.get(&txn, 0), Some(1));
        assert_eq!(a2.get(&txn, 1), Some(2));
        assert_eq!(a2.get(&txn, 2), Some(3));
        let weak: WeakRef<ArrayRef> = a2.get(&txn, 3).unwrap();
        let actual: Vec<_> = weak.unquote(&txn).collect();
        assert_eq!(actual, vec![2.into()]);
    }

    #[test]
    fn array_quote_multi_elements() {
        let mut d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");
        let mut d2 = Doc::with_client_id(2);
        let a2 = d2.get_or_insert_array("array");

        let nested = {
            let mut txn = d1.transact_mut();
            a1.insert_range(&mut txn, 0, [1, 2]);
            let nested = a1.push_back(&mut txn, MapPrelim::from([("key", "value")]));
            a1.push_back(&mut txn, 3);
            nested
        };
        let l1 = {
            let mut t1 = d1.transact_mut();
            let prelim = a1.quote(&t1, 1..=3).unwrap();
            a1.insert(&mut t1, 0, prelim)
        };

        let t1 = d1.transact();
        assert_eq!(
            l1.unquote(&t1).collect::<Vec<Out>>(),
            vec![2.into(), Out::Map(nested.clone()), 3.into()]
        );
        assert_eq!(a1.get(&t1, 1), Some(1));
        assert_eq!(a1.get(&t1, 2), Some(2));
        assert_eq!(a1.get(&t1, 3), Some(Out::Map(nested.clone())));
        assert_eq!(a1.get(&t1, 4), Some(3));
        drop(t1);

        exchange_updates([&mut d1, &mut d2]);

        let t2 = d2.transact();
        let l2: WeakRef<ArrayRef> = a2.get(&t2, 0).unwrap();
        let unquoted: Vec<_> = l2.unquote(&t2).map(|v| v.to_string(&t2)).collect();
        assert_eq!(
            unquoted,
            vec![
                "2".to_string(),
                r#"{key: value}"#.to_string(),
                "3".to_string()
            ]
        );
        assert_eq!(a2.get(&t2, 1), Some(1));
        assert_eq!(a2.get(&t2, 2), Some(2));
        assert_eq!(
            a2.get(&t2, 3).map(|v: Out| v.to_string(&t2)),
            Some(r#"{key: value}"#.to_string())
        );
        assert_eq!(a2.get(&t2, 4), Some(3));
        drop(t2);

        a2.insert_range(&mut d2.transact_mut(), 3, ["A", "B"]);

        let t2 = d2.transact();
        let unquoted: Vec<_> = l2.unquote(&t2).map(|v| v.to_string(&t2)).collect();
        assert_eq!(
            unquoted,
            vec![
                "2".to_string(),
                "A".to_string(),
                "B".to_string(),
                r#"{key: value}"#.to_string(),
                "3".to_string()
            ]
        );
        drop(t2);

        exchange_updates([&mut d1, &mut d2]);

        assert_eq!(
            l1.unquote(&d1.transact()).collect::<Vec<Out>>(),
            vec![
                2.into(),
                "A".into(),
                "B".into(),
                Out::Map(nested.clone()),
                3.into()
            ]
        );
    }

    #[test]
    fn self_quotation() {
        let mut d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");
        let mut d2 = Doc::with_client_id(2);
        let a2 = d2.get_or_insert_array("array");

        a1.insert_range(&mut d1.transact_mut(), 0, [1, 2, 3, 4]);
        let l1 = a1.quote(&d1.transact(), 0..3).unwrap();
        // link is inserted into its own range
        let l1 = a1.insert(&mut d1.transact_mut(), 1, l1);
        let t1 = d1.transact();
        let mut u = l1.unquote(&t1);
        assert_eq!(u.next(), Some(1.into()));
        assert_eq!(u.next(), Some(Out::WeakLink(l1.clone().into_inner())));
        assert_eq!(u.next(), Some(2.into()));
        assert_eq!(u.next(), Some(3.into()));

        assert_eq!(a1.get(&t1, 0), Some(1));
        assert_eq!(a1.get(&t1, 1), Some(Out::WeakLink(l1.clone().into_inner())));
        assert_eq!(a1.get(&t1, 2), Some(2));
        assert_eq!(a1.get(&t1, 3), Some(3));
        assert_eq!(a1.get(&t1, 4), Some(4));
        drop(t1);

        exchange_updates([&mut d1, &mut d2]);

        let t2 = d2.transact();
        let l2: WeakRef<ArrayRef> = a2.get(&t2, 1).unwrap();
        let unquote: Vec<_> = l2.unquote(&t2).collect();
        assert_eq!(
            unquote,
            vec![
                1.into(),
                Out::WeakLink(l2.clone().into_inner()),
                2.into(),
                3.into()
            ]
        );
        assert_eq!(a2.get(&t2, 0), Some(1));
        assert_eq!(a2.get(&t2, 1), Some(Out::WeakLink(l2.into_inner())));
        assert_eq!(a2.get(&t2, 2), Some(2));
        assert_eq!(a2.get(&t2, 3), Some(3));
        assert_eq!(a2.get(&t2, 4), Some(4));
    }

    #[test]
    fn update() {
        let mut d1 = Doc::new();
        let m1 = d1.get_or_insert_map("map");

        let mut d2 = Doc::new();
        let m2 = d2.get_or_insert_map("map");

        let link1 = {
            let mut txn = d1.transact_mut();
            let nested = MapPrelim::from([("a1".to_owned(), "hello".to_owned())]);
            m1.insert(&mut txn, "a", nested);
            let link = m1.link(&txn, "a").unwrap();
            m1.insert(&mut txn, "b", link)
        };

        exchange_updates([&mut d1, &mut d2]);

        let link2: WeakRef<MapRef> = m2.get(&d2.transact(), "b").unwrap();
        let l1: MapRef = link1.try_deref(&d1.transact()).unwrap();
        let l2: MapRef = link2.try_deref(&d2.transact()).unwrap();
        assert_eq!(
            l1.get::<Out>(&d1.transact(), "a1"),
            l2.get::<Out>(&d2.transact(), "a1")
        );

        m2.insert(&mut d2.transact_mut(), "a2", "world");

        exchange_updates([&mut d1, &mut d2]);

        let l1: MapRef = link1.try_deref(&d1.transact()).unwrap();
        let l2: MapRef = link2.try_deref(&d2.transact()).unwrap();
        assert_eq!(
            l1.get::<Out>(&d1.transact(), "a2"),
            l2.get::<Out>(&d2.transact(), "a2")
        );
    }

    #[test]
    #[cfg_attr(target_os = "windows", ignore)]
    fn delete_weak_link() {
        let mut d1 = Doc::new();
        let m1 = d1.get_or_insert_map("map");

        let mut d2 = Doc::new();
        let m2 = d2.get_or_insert_map("map");

        let link1 = {
            let mut txn = d1.transact_mut();
            let nested = MapPrelim::from([("a1".to_owned(), "hello".to_owned())]);
            m1.insert(&mut txn, "a", nested);
            let link = m1.link(&txn, "a").unwrap();
            m1.insert(&mut txn, "b", link)
        };

        exchange_updates([&mut d1, &mut d2]);

        let link2: WeakRef<MapRef> = m2.get(&d2.transact(), "b").unwrap();
        let l1: MapRef = link1.try_deref(&d1.transact()).unwrap();
        let l2: MapRef = link2.try_deref(&d2.transact()).unwrap();
        assert_eq!(
            l1.get::<Out>(&d1.transact(), "a1"),
            l2.get::<Out>(&d2.transact(), "a1")
        );

        m2.remove(&mut d2.transact_mut(), "b"); // delete links

        exchange_updates([&mut d1, &mut d2]);

        // since links have been deleted, they no longer refer to any content
        assert_eq!(link1.try_deref_value(&d1.transact()), None);
        assert_eq!(link2.try_deref_value(&d2.transact()), None);
    }

    #[test]
    fn delete_source() {
        let mut d1 = Doc::new();
        let m1 = d1.get_or_insert_map("map");

        let mut d2 = Doc::new();
        let m2 = d2.get_or_insert_map("map");

        let link1 = {
            let mut txn = d1.transact_mut();
            let nested = MapPrelim::from([("a1".to_owned(), "hello".to_owned())]);
            m1.insert(&mut txn, "a", nested);
            let link = m1.link(&txn, "a").unwrap();
            m1.insert(&mut txn, "b", link)
        };

        exchange_updates([&mut d1, &mut d2]);

        let link2: WeakRef<MapRef> = m2.get(&d2.transact(), "b").unwrap();
        let l1: MapRef = link1.try_deref(&d1.transact()).unwrap();
        let l2: MapRef = link2.try_deref(&d2.transact()).unwrap();
        assert_eq!(
            l1.get::<Out>(&d1.transact(), "a1"),
            l2.get::<Out>(&d2.transact(), "a1")
        );

        m2.remove(&mut d2.transact_mut(), "a"); // delete source of the link

        exchange_updates([&mut d1, &mut d2]);

        // since links have been deleted, they no longer refer to any content
        assert_eq!(link1.try_deref_value(&d1.transact()), None);
        assert_eq!(link2.try_deref_value(&d2.transact()), None);
    }

    #[test]
    fn observe_map_update() {
        let mut d1 = Doc::new();
        let m1 = d1.get_or_insert_map("map");
        let mut d2 = Doc::new();
        let m2 = d2.get_or_insert_map("map");

        let link1 = {
            let mut txn = d1.transact_mut();
            m1.insert(&mut txn, "a", "value");
            let link1 = m1.link(&txn, "a").unwrap();
            m1.insert(&mut txn, "b", link1)
        };

        let target1 = Arc::new(ArcSwapOption::default());
        let _sub1 = {
            let target = target1.clone();
            link1.observe(move |_, e| target.store(Some(Arc::new(e.target.clone()))))
        };

        exchange_updates([&mut d1, &mut d2]);

        let link2: WeakRef<MapRef> = m2.get(&d2.transact(), "b").unwrap();
        assert_eq!(link2.try_deref_value(&d2.transact()), Some("value".into()));

        let target2 = Arc::new(ArcSwapOption::default());
        let _sub2 = {
            let target = target2.clone();
            link2.observe(move |_, e| target.store(Some(Arc::new(e.target.clone()))))
        };

        m1.insert(&mut d1.transact_mut(), "a", "value2");
        assert_eq!(link1.try_deref_value(&d1.transact()), Some("value2".into()));

        exchange_updates([&mut d1, &mut d2]);
        assert_eq!(link2.try_deref_value(&d2.transact()), Some("value2".into()));
    }

    #[test]
    fn observe_map_delete() {
        let mut d1 = Doc::new();
        let m1 = d1.get_or_insert_map("map");
        let mut d2 = Doc::new();
        let m2 = d2.get_or_insert_map("map");

        let link1 = {
            let mut txn = d1.transact_mut();
            m1.insert(&mut txn, "a", "value");
            let link1 = m1.link(&txn, "a").unwrap();
            m1.insert(&mut txn, "b", link1)
        };

        let target1 = Arc::new(ArcSwapOption::default());
        let _sub1 = {
            let target = target1.clone();
            link1.observe(move |_, e| target.store(Some(Arc::new(e.as_target::<MapRef>()))))
        };

        exchange_updates([&mut d1, &mut d2]);

        let link2: WeakRef<MapRef> = m2.get(&d2.transact(), "b").unwrap();
        assert_eq!(link2.try_deref_value(&d2.transact()), Some("value".into()));

        let target2 = Arc::new(ArcSwapOption::default());
        let _sub2 = {
            let target = target2.clone();
            link2.observe(move |_, e| target.store(Some(Arc::new(e.as_target::<MapRef>()))))
        };

        m1.remove(&mut d1.transact_mut(), "a");
        let l1 = target1.swap(None).unwrap();
        assert_eq!(l1.try_deref_value(&d1.transact()), None);

        exchange_updates([&mut d1, &mut d2]);
        let l2 = target2.swap(None).unwrap();
        assert_eq!(l2.try_deref_value(&d2.transact()), None);
    }

    #[test]
    fn observe_array() {
        let mut d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");
        let mut d2 = Doc::with_client_id(2);
        let a2 = d2.get_or_insert_array("array");

        let link1 = {
            let mut txn = d1.transact_mut();
            a1.insert_range(&mut txn, 0, ["A", "B", "C"]);
            let link1 = a1.quote(&txn, 1..=2).unwrap();
            a1.insert(&mut txn, 0, link1)
        };

        let target1 = Arc::new(ArcSwapOption::default());
        let _sub1 = {
            let target = target1.clone();
            link1.observe(move |_, e| target.store(Some(Arc::new(e.as_target::<ArrayRef>()))))
        };

        exchange_updates([&mut d1, &mut d2]);

        let link2: WeakRef<ArrayRef> = a2.get(&d2.transact(), 0).unwrap();
        let actual: Vec<_> = link2.unquote(&d2.transact()).collect();
        assert_eq!(actual, vec!["B".into(), "C".into()]);

        let target2 = Arc::new(ArcSwapOption::default());
        let _sub2 = {
            let target = target2.clone();
            link2.observe(move |_, e| target.store(Some(Arc::new(e.as_target::<ArrayRef>()))))
        };

        a1.remove(&mut d1.transact_mut(), 2);
        let actual: Vec<_> = link1.unquote(&d1.transact()).collect();
        assert_eq!(actual, vec!["C".into()]);

        exchange_updates([&mut d1, &mut d2]);
        let l2 = target2.swap(None).unwrap();
        let actual: Vec<_> = l2.unquote(&d2.transact()).collect();
        assert_eq!(actual, vec!["C".into()]);

        a2.remove(&mut d2.transact_mut(), 2);
        let l2 = target2.swap(None).unwrap();
        let actual: Vec<_> = l2.unquote(&d2.transact()).collect();
        assert_eq!(actual, vec![]);

        exchange_updates([&mut d1, &mut d2]);
        let l1 = target1.swap(None).unwrap();
        let actual: Vec<_> = l1.unquote(&d1.transact()).collect();
        assert_eq!(actual, vec![]);

        a1.remove(&mut d1.transact_mut(), 1);
        assert_eq!(target1.swap(None), None);
    }

    #[test]
    fn deep_observe_transitive() {
        /*
          Structure:
            - map1
              - link-key: <=+-+
            - map2:         | |
              - key: value1-+ |
              - link-link: <--+
        */
        let mut doc = Doc::new();
        let m1 = doc.get_or_insert_map("map1");
        let m2 = doc.get_or_insert_map("map2");
        let mut txn = doc.transact_mut();

        // test observers in a face of linked chains of values
        m2.insert(&mut txn, "key", "value1");
        let link1 = m2.link(&txn, "key").unwrap();
        m1.insert(&mut txn, "link-key", link1);
        let link2 = m1.link(&txn, "link-key").unwrap();
        let link2 = m2.insert(&mut txn, "link-link", link2);
        drop(txn);

        let events = Arc::new(Mutex::new(vec![]));
        let _sub1 = {
            let events = events.clone();
            link2.observe_deep(move |_, evts| {
                let mut er = events.lock().unwrap();
                for e in evts.iter() {
                    er.push(e.target());
                }
            })
        };
        m2.insert(&mut doc.transact_mut(), "key", "value2");
        let actual: Vec<_> = events
            .lock()
            .unwrap()
            .iter()
            .flat_map(|v| {
                v.clone()
                    .cast::<WeakRef<MapRef>>(&doc.transact())
                    .unwrap()
                    .try_deref_value(&doc.transact())
            })
            .collect();
        assert_eq!(actual, vec!["value2".into()])
    }

    #[test]
    fn deep_observe_transitive2() {
        /*
          Structure:
            - map1
              - link-key: <=+-+
            - map2:         | |
              - key: value1-+ |
              - link-link: <==+--+
            - map3:              |
              - link-link-link:<-+
        */
        let mut doc = Doc::new();
        let m1 = doc.get_or_insert_map("map1");
        let m2 = doc.get_or_insert_map("map2");
        let m3 = doc.get_or_insert_map("map3");
        let mut txn = doc.transact_mut();

        // test observers in a face of multi-layer linked chains of values
        m2.insert(&mut txn, "key", "value1");
        let link1 = m2.link(&txn, "key").unwrap();
        m1.insert(&mut txn, "link-key", link1);
        let link2 = m1.link(&txn, "link-key").unwrap();
        m2.insert(&mut txn, "link-link", link2);
        let link3 = m2.link(&txn, "link-link").unwrap();
        let link3 = m3.insert(&mut txn, "link-link-link", link3);
        drop(txn);

        let events = Arc::new(Mutex::new(vec![]));
        let _sub1 = {
            let events = events.clone();
            link3.observe_deep(move |_, evts| {
                let mut er = events.lock().unwrap();
                for e in evts.iter() {
                    er.push(e.target());
                }
            })
        };
        m2.insert(&mut doc.transact_mut(), "key", "value2");
        let mut guard = events.lock().unwrap();
        let actual = std::mem::take(&mut *guard);
        let actual: Vec<_> = actual
            .into_iter()
            .flat_map(|v| {
                v.cast::<WeakRef<MapRef>>(&doc.transact())
                    .unwrap()
                    .try_deref_value(&doc.transact())
            })
            .collect();
        assert_eq!(actual, vec!["value2".into()])
    }

    #[test]
    fn deep_observe_map() {
        /*
          Structure:
            - map (observed):
              - link:<----+
            - array:      |
               0: nested:-+
                 - key: value
        */
        let mut doc = Doc::with_client_id(1);
        let map = doc.get_or_insert_map("map");
        let array = doc.get_or_insert_array("array");

        let events = Arc::new(Mutex::new(vec![]));
        let _sub = {
            let events = events.clone();
            map.observe_deep(move |_, e| {
                let mut rs = events.lock().unwrap();
                for e in e.iter() {
                    match e {
                        Event::Map(e) => {
                            let value = Out::Map(e.target().clone());
                            rs.push((value, Some(e.keys().clone())));
                        }
                        Event::Weak(e) => {
                            let value = Out::WeakLink(e.as_target());
                            rs.push((value, None));
                        }
                        _ => {}
                    }
                }
            })
        };

        let mut txn = doc.transact_mut();
        let nested = array.insert(&mut txn, 0, MapPrelim::default());
        let link = array.quote(&txn, 0..=0).unwrap();
        let link = map.insert(&mut txn, "link", link);
        drop(txn);

        // update entry in linked map
        events.lock().unwrap().clear();
        nested.insert(&mut doc.transact_mut(), "key", "value");
        let actual = {
            let mut guard = events.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        assert_eq!(
            actual,
            vec![(
                Out::Map(nested.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Inserted("value".into())
                )]))
            )]
        );

        // delete entry in linked map
        nested.remove(&mut doc.transact_mut(), "key");
        let actual = {
            let mut guard = events.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        assert_eq!(
            actual,
            vec![(
                Out::Map(nested.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Removed("value".into())
                )]))
            )]
        );

        // delete linked map
        array.remove(&mut doc.transact_mut(), 0);
        let actual = {
            let mut guard = events.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        assert_eq!(actual, vec![(Out::WeakLink(link.into_inner()), None)]);
    }

    #[test]
    fn deep_observe_array() {
        // test observers in a face of linked chains of values
        /*
          Structure:
            - map:
              - nested: --------+
                - key: value    |
            - array (observed): |
              0: <--------------+
        */
        let mut doc = Doc::with_client_id(1);
        let map = doc.get_or_insert_map("map");
        let array = doc.get_or_insert_array("array");

        let nested = map.insert(&mut doc.transact_mut(), "nested", MapPrelim::default());
        let link = map.link(&doc.transact(), "nested").unwrap();
        let link = array.insert(&mut doc.transact_mut(), 0, link);

        let events = Arc::new(Mutex::new(vec![]));
        let _sub = {
            let events = events.clone();
            array.observe_deep(move |_, e| {
                let mut events = events.lock().unwrap();
                for e in e.iter() {
                    match e {
                        Event::Map(e) => {
                            events.push((Out::Map(e.target().clone()), Some(e.keys().clone())))
                        }
                        Event::Weak(e) => events.push((Out::WeakLink(e.as_target()), None)),
                        _ => {}
                    }
                }
            })
        };
        nested.insert(&mut doc.transact_mut(), "key", "value");
        let actual = {
            let mut guard = events.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        assert_eq!(
            actual,
            vec![(
                Out::Map(nested.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Inserted("value".into())
                )]))
            )]
        );
        // update existing entry
        nested.insert(&mut doc.transact_mut(), "key", "value2");
        let actual = {
            let mut guard = events.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        assert_eq!(
            actual,
            vec![(
                Out::Map(nested.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Updated("value".into(), "value2".into())
                )]))
            )]
        );

        // delete entry in linked map
        nested.remove(&mut doc.transact_mut(), "key");
        let actual = {
            let mut guard = events.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        assert_eq!(
            actual,
            vec![(
                Out::Map(nested.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Removed("value2".into())
                )]))
            )]
        );

        // delete linked map
        map.remove(&mut doc.transact_mut(), "nested");
        let actual = {
            let mut guard = events.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        assert_eq!(actual, vec![(Out::WeakLink(link.into_inner()), None)]);
    }

    #[test]
    fn deep_observe_new_element_within_quoted_range() {
        let mut d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");
        let mut d2 = Doc::with_client_id(2);
        let a2 = d2.get_or_insert_array("array");

        {
            let mut t1 = d1.transact_mut();
            a1.push_back(&mut t1, 1);
            a1.push_back(&mut t1, MapPrelim::default());
            a1.push_back(&mut t1, MapPrelim::default());
            a1.push_back(&mut t1, 2);
        }
        let l1 = {
            let mut t1 = d1.transact_mut();
            let link = a1.quote(&t1, 1..=2).unwrap();
            a1.insert(&mut t1, 0, link)
        };

        exchange_updates([&mut d1, &mut d2]);

        let e1 = Arc::new(Mutex::new(vec![]));
        let _s1 = {
            let events = e1.clone();
            l1.observe_deep(move |_, e| {
                let mut events = events.lock().unwrap();
                events.clear();
                for e in e.iter() {
                    match e {
                        Event::Map(e) => {
                            events.push((Out::Map(e.target().clone()), Some(e.keys().clone())))
                        }
                        Event::Weak(e) => events.push((Out::WeakLink(e.as_target()), None)),
                        _ => {}
                    }
                }
            })
        };

        let l2: WeakRef<ArrayRef> = a2.get(&d2.transact(), 0).unwrap();
        let e2 = Arc::new(Mutex::new(vec![]));
        let _s2 = {
            let events = e2.clone();
            l2.observe_deep(move |_, e| {
                let mut events = events.lock().unwrap();
                events.clear();
                for e in e.iter() {
                    match e {
                        Event::Map(e) => {
                            events.push((Out::Map(e.target().clone()), Some(e.keys().clone())))
                        }
                        Event::Weak(e) => events.push((Out::WeakLink(e.as_target()), None)),
                        _ => {}
                    }
                }
            })
        };

        let m20 = a1.insert(&mut d1.transact_mut(), 3, MapPrelim::default());
        exchange_updates([&mut d1, &mut d2]);
        m20.insert(&mut d1.transact_mut(), "key", "value");
        assert_eq!(
            &*e1.lock().unwrap(),
            &vec![(
                Out::Map(m20.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Inserted("value".into())
                )]))
            )]
        );

        exchange_updates([&mut d1, &mut d2]);

        let m21: MapRef = a2.get(&d2.transact(), 3).unwrap();
        assert_eq!(
            &*e2.lock().unwrap(),
            &vec![(
                Out::Map(m21.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Inserted("value".into())
                )]))
            )]
        );
    }

    #[test]
    fn deep_observe_recursive() {
        // test observers in a face of cycled chains of values
        /*
          Structure:
           array (observed):
             m0:--------+
              - k1:<-+  |
                     |  |
             m1------+  |
              - k2:<-+  |
                     |  |
             m2------+  |
              - k0:<----+
        */
        let mut doc = Doc::new();
        let root = doc.get_or_insert_array("array");
        let mut txn = doc.transact_mut();

        let m0 = root.insert(&mut txn, 0, MapPrelim::default());
        let m1 = root.insert(&mut txn, 1, MapPrelim::default());
        let m2 = root.insert(&mut txn, 2, MapPrelim::default());

        let l0 = root.quote(&txn, 0..=0).unwrap();
        let l1 = root.quote(&txn, 1..=1).unwrap();
        let l2 = root.quote(&txn, 2..=2).unwrap();

        // create cyclic reference between links
        m0.insert(&mut txn, "k1", l1);
        m1.insert(&mut txn, "k2", l2);
        m2.insert(&mut txn, "k0", l0);
        drop(txn);

        let events = Arc::new(Mutex::new(vec![]));
        let _sub = {
            let events = events.clone();
            m0.observe_deep(move |_, e| {
                let mut rs = events.lock().unwrap();
                for e in e.iter() {
                    if let Event::Map(e) = e {
                        let value = e.target().clone();
                        rs.push((value, e.keys().clone()));
                    }
                }
            })
        };

        m1.insert(&mut doc.transact_mut(), "test-key1", "value1");
        let actual = {
            let mut guard = events.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        assert_eq!(
            actual,
            vec![(
                m1.clone(),
                HashMap::from([(
                    Arc::from("test-key1"),
                    EntryChange::Inserted("value1".into())
                )])
            )]
        );

        m2.insert(&mut doc.transact_mut(), "test-key2", "value2");
        let actual = {
            let mut guard = events.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        assert_eq!(
            actual,
            vec![(
                m2.clone(),
                HashMap::from([(
                    Arc::from("test-key2"),
                    EntryChange::Inserted("value2".into())
                )])
            )]
        );

        m1.remove(&mut doc.transact_mut(), "test-key1");
        let actual = {
            let mut guard = events.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        assert_eq!(
            actual,
            vec![(
                m1.clone(),
                HashMap::from([(
                    Arc::from("test-key1"),
                    EntryChange::Removed("value1".into())
                )])
            )]
        );
    }

    #[test]
    fn remote_map_update() {
        let mut d1 = Doc::with_client_id(1);
        let m1 = d1.get_or_insert_map("map");
        let mut d2 = Doc::with_client_id(2);
        let m2 = d2.get_or_insert_map("map");
        let mut d3 = Doc::with_client_id(3);
        let m3 = d3.get_or_insert_map("map");

        m1.insert(&mut d1.transact_mut(), "key", 1);

        exchange_updates([&mut d1, &mut d2, &mut d3]);

        let l2 = m2.link(&d2.transact(), "key").unwrap();
        m2.insert(&mut d2.transact_mut(), "link", l2);
        m1.insert(&mut d1.transact_mut(), "key", 2);
        m1.insert(&mut d1.transact_mut(), "key", 3);

        // apply updated content first, link second
        exchange_updates([&mut d3, &mut d1]);
        exchange_updates([&mut d3, &mut d2]);

        // make sure that link can find the most recent block
        let l3: WeakRef<MapRef> = m3.get(&d3.transact(), "link").unwrap();
        assert_eq!(l3.try_deref_value(&d3.transact()), Some(3.into()));

        exchange_updates([&mut d1, &mut d2, &mut d3]);

        let l1: WeakRef<MapRef> = m1.get(&d1.transact(), "link").unwrap();
        let l2: WeakRef<MapRef> = m2.get(&d2.transact(), "link").unwrap();

        assert_eq!(l1.try_deref_value(&d1.transact()), Some(3.into()));
        assert_eq!(l2.try_deref_value(&d2.transact()), Some(3.into()));
        assert_eq!(l3.try_deref_value(&d3.transact()), Some(3.into()));
    }

    #[test]
    fn basic_text() {
        let mut d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("text");
        let a1 = d1.get_or_insert_array("array");
        let mut d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("text");

        txt1.insert(&mut d1.transact_mut(), 0, "abcd"); // 'abcd'
        let l1 = {
            let mut txn = d1.transact_mut();
            let q = txt1.quote(&mut txn, 1..=2); // quote: [bc]
            a1.insert(&mut txn, 0, q.unwrap())
        };
        assert_eq!(l1.get_string(&d1.transact()), "bc".to_string());

        txt1.insert(&mut d1.transact_mut(), 2, "ef"); // 'abefcd', quote: [befc]
        assert_eq!(l1.get_string(&d1.transact()), "befc".to_string());

        txt1.remove_range(&mut d1.transact_mut(), 3, 3); // 'abe', quote: [be]
        assert_eq!(l1.get_string(&d1.transact()), "be".to_string());

        txt1.insert_embed(&mut d1.transact_mut(), 3, WeakPrelim::from(l1.clone())); // 'abe[be]'

        exchange_updates([&mut d1, &mut d2]);

        let txn = d2.transact();
        let diff = txt2.diff(&txn, YChange::identity);
        let l2: WeakRef<TextRef> = diff[1].insert.clone().cast(&txn).unwrap();
        assert_eq!(l2.get_string(&txn), "be".to_string());
    }

    #[test]
    fn basic_xml_text() {
        let mut d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("text");
        let txt1: &XmlTextRef = txt1.as_ref();
        let a1 = d1.get_or_insert_array("array");
        let mut d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("text");
        let txt2: &XmlTextRef = txt2.as_ref();

        txt1.insert(&mut d1.transact_mut(), 0, "abcd"); // 'abcd'
        let l1 = {
            let mut txn = d1.transact_mut();
            let q = txt1.quote(&mut txn, 1..=2); // quote: [bc]
            a1.insert(&mut txn, 0, q.unwrap())
        };
        assert_eq!(l1.get_string(&d1.transact()), "bc".to_string());

        txt1.insert(&mut d1.transact_mut(), 2, "ef"); // 'abefcd', quote: [befc]
        assert_eq!(l1.get_string(&d1.transact()), "befc".to_string());

        txt1.remove_range(&mut d1.transact_mut(), 3, 3); // 'abe', quote: [be]
        assert_eq!(l1.get_string(&d1.transact()), "be".to_string());

        txt1.insert_embed(&mut d1.transact_mut(), 3, WeakPrelim::from(l1.clone())); // 'abe[be]'

        exchange_updates([&mut d1, &mut d2]);

        let txn = d2.transact();
        let diff = txt2.diff(&txn, YChange::identity);
        let l2: WeakRef<TextRef> = diff[1].insert.clone().cast(&txn).unwrap();
        assert_eq!(l2.get_string(&txn), "be".to_string());
    }

    #[test]
    fn quote_formatted_text() {
        let mut doc = Doc::with_client_id(1);
        let txt1 = doc.get_or_insert_text("text1");
        let txt1: &XmlTextRef = txt1.as_ref();
        let txt2 = doc.get_or_insert_text("text2");
        let txt2: &XmlTextRef = txt2.as_ref();
        let array = doc.get_or_insert_array("array");
        txt1.insert(&mut doc.transact_mut(), 0, "abcde");
        let b = Attrs::from([("b".into(), true.into())]);
        let i = Attrs::from([("i".into(), true.into())]);
        txt1.format(&mut doc.transact_mut(), 0, 1, b.clone()); // '<b>a</b>bcde'
        txt1.format(&mut doc.transact_mut(), 1, 3, i.clone()); // '<b>a</b><i>bcd</i>e'
        let l1 = {
            let mut txn = doc.transact_mut();
            let l = txt1.quote(&mut txn, 0..=1).unwrap();
            array.insert(&mut txn, 0, l) // <b>a</b><i>b</i>
        };
        let l2 = {
            let mut txn = doc.transact_mut();
            let l = txt1.quote(&mut txn, 2..=2).unwrap();
            array.insert(&mut txn, 0, l) // <i>c</i>
        };
        let l3 = {
            let mut txn = doc.transact_mut();
            let l = txt1.quote(&mut txn, 3..=4).unwrap();
            array.insert(&mut txn, 0, l) // <i>d</i>e
        };
        assert_eq!(l1.get_string(&doc.transact()), "<b>a</b><i>b</i>");
        assert_eq!(l2.get_string(&doc.transact()), "<i>c</i>");
        assert_eq!(l3.get_string(&doc.transact()), "<i>d</i>e");

        txt2.insert_embed(&mut doc.transact_mut(), 0, WeakPrelim::from(l1.clone()));
        txt2.insert_embed(&mut doc.transact_mut(), 1, WeakPrelim::from(l2.clone()));
        txt2.insert_embed(&mut doc.transact_mut(), 2, WeakPrelim::from(l3.clone()));

        let txn = doc.transact();
        let diff: Vec<_> = txt2
            .diff(&txn, YChange::identity)
            .into_iter()
            .map(|d| {
                d.insert
                    .cast::<WeakRef<XmlTextRef>>(&txn)
                    .unwrap()
                    .get_string(&txn)
            })
            .collect();
        assert_eq!(
            diff,
            vec![
                "<b>a</b><i>b</i>".to_string(),
                "<i>c</i>".to_string(),
                "<i>d</i>e".to_string()
            ]
        );
    }

    #[test]
    fn quote_moved_elements() {
        let mut doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("values");
        let quotes = doc.get_or_insert_array("quotes");
        let mut txn = doc.transact_mut();

        array.insert_range(&mut txn, 0, [2, 3, 1, 7, 4, 6, 5]);
        array.move_to(&mut txn, 2, 0); // [1, 2, 3, 7, 4, 6, 5]
        array.move_to(&mut txn, 3, 7); // [1, 2, 3, 4, 6, 5, 7]
        array.move_to(&mut txn, 4, 6); // [1, 2, 3, 4, 5, 6, 7]

        let values: Vec<u32> = array.iter(&txn).map(|v| v.cast(&txn).unwrap()).collect();
        assert_eq!(values, vec![1, 2, 3, 4, 5, 6, 7]);

        let mut assert_quote = |start: u32, len: u32, expected: Vec<u32>| {
            let end = start + len - 1;
            let q = array.quote(&mut txn, start..=end).unwrap();
            let q = quotes.push_back(&mut txn, q);
            let values: Vec<u32> = q.unquote(&txn).map(|v| v.cast(&txn).unwrap()).collect();
            assert_eq!(values, expected)
        };

        assert_quote(0, 1, vec![1]);
        assert_quote(0, 3, vec![1, 2, 3]);
        assert_quote(1, 3, vec![2, 3, 4]);
        assert_quote(2, 1, vec![3]);
        assert_quote(2, 3, vec![3, 4, 5]);
        assert_quote(3, 4, vec![4, 5, 6, 7]);
    }

    #[test]
    fn quote_moved_range_elements() {
        let mut doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("values");
        let quotes = doc.get_or_insert_array("quotes");
        let mut txn = doc.transact_mut();

        array.insert_range(&mut txn, 0, [1, 5, 6, 2, 3, 4, 7]);
        array.move_range_to(&mut txn, 3, Before, 5, After, 1);

        let values: Vec<u32> = array.iter(&txn).map(|v| v.cast(&txn).unwrap()).collect();
        assert_eq!(values, vec![1, 2, 3, 4, 5, 6, 7]);

        let mut assert_quote = |start: u32, len: u32, expected: Vec<u32>| {
            let end = start + len - 1;
            let q = array.quote(&mut txn, start..=end).unwrap();
            let q = quotes.push_back(&mut txn, q);
            let values: Vec<u32> = q.unquote(&txn).map(|v| v.cast(&txn).unwrap()).collect();
            assert_eq!(values, expected)
        };

        assert_quote(0, 1, vec![1]);
        assert_quote(0, 3, vec![1, 2, 3]);
        assert_quote(1, 3, vec![2, 3, 4]);
        assert_quote(2, 1, vec![3]);
        assert_quote(2, 3, vec![3, 4, 5]);
        assert_quote(3, 4, vec![4, 5, 6, 7]);
    }

    #[ignore]
    #[test]
    fn move_range_of_quoted_elements() {
        let mut doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("values");
        let quotes = doc.get_or_insert_array("quotes");
        let mut txn = doc.transact_mut();

        array.insert_range(&mut txn, 0, [1, 2, 3, 4, 5, 6, 7]);

        let mut quote = |start: u32, len: u32| {
            let end = start + len - 1;
            let q = array.quote(&mut txn, start..=end).unwrap();
            quotes.push_back(&mut txn, q)
        };
        let q1 = quote(0, 3); // [1,2,3]
        let q2 = quote(1, 3); // [2,3,4]
        let q3 = quote(2, 3); // [3,4,5]
        let q4 = quote(3, 3); // [4,5,6]
        let q5 = quote(4, 3); // [5,6,7]

        array.move_range_to(&mut txn, 3, Before, 5, After, 1);
        let values: Vec<u32> = array.iter(&txn).map(|v| v.cast(&txn).unwrap()).collect();
        assert_eq!(values, vec![1, 4, 5, 6, 2, 3, 7]);

        let actual: Vec<u32> = q1.unquote(&txn).map(|v| v.cast(&txn).unwrap()).collect();
        assert_eq!(actual, vec![1, 4, 5, 6, 2, 3]);

        let actual: Vec<u32> = q2.unquote(&txn).map(|v| v.cast(&txn).unwrap()).collect();
        assert_eq!(actual, vec![2, 3]);

        let actual: Vec<u32> = q3.unquote(&txn).map(|v| v.cast(&txn).unwrap()).collect();
        assert_eq!(actual, vec![3]);

        let actual: Vec<u32> = q4.unquote(&txn).map(|v| v.cast(&txn).unwrap()).collect();
        assert_eq!(actual, vec![4, 5, 6]);

        let actual: Vec<u32> = q5.unquote(&txn).map(|v| v.cast(&txn).unwrap()).collect();
        assert_eq!(actual, vec![5, 6, 2, 3, 7]);
    }

    fn to_weak_xml_text(weak: &WeakRef<TextRef>) -> WeakRef<XmlTextRef> {
        WeakRef::from(weak.clone().into_inner())
    }

    #[test]
    fn quoted_text_start_boundary_inserts() {
        let mut d1 = Doc::with_client_id(1);
        let arr1 = d1.get_or_insert_array("array");
        let txt1 = d1.get_or_insert_text("text");
        {
            let mut txn = d1.transact_mut();
            txt1.insert(&mut txn, 0, "abcdef"); // t1: 'abcdef'
        }

        let mut d2 = Doc::with_client_id(2);
        let _arr2 = d2.get_or_insert_array("array");
        let txt2 = d2.get_or_insert_text("text");

        exchange_updates([&mut d1, &mut d2]); // t2: 'abcdef'

        txt2.insert(&mut d2.transact_mut(), 1, "xyz"); // t2: 'axyzbcdef'

        let link_excl = {
            struct RangeLeftExclusive(u32, u32);
            impl RangeBounds<u32> for RangeLeftExclusive {
                fn start_bound(&self) -> Bound<&u32> {
                    Bound::Excluded(&self.0)
                }

                fn end_bound(&self) -> Bound<&u32> {
                    Bound::Excluded(&self.1)
                }
            }

            let mut txn = d1.transact_mut();
            let q = txt1.quote(&txn, RangeLeftExclusive(0, 5)).unwrap(); // [bcde]
            arr1.insert(&mut txn, 0, q)
        };
        let link_incl = {
            let mut txn = d1.transact_mut();
            let q = txt1.quote(&txn, 1..5).unwrap(); // [bcde]
            arr1.insert(&mut txn, 0, q)
        };
        {
            let txn = d1.transact();
            let str = link_excl.get_string(&txn);
            assert_eq!(&str, "bcde");
            let str = to_weak_xml_text(&link_excl).get_string(&txn);
            assert_eq!(&str, "bcde");
            let str = link_incl.get_string(&txn);
            assert_eq!(&str, "bcde");
            let str = to_weak_xml_text(&link_incl).get_string(&txn);
            assert_eq!(&str, "bcde");
        }

        exchange_updates([&mut d1, &mut d2]);

        {
            let txn = d1.transact();
            let str = link_excl.get_string(&txn);
            assert_eq!(&str, "xyzbcde");
            let str = to_weak_xml_text(&link_excl).get_string(&txn);
            assert_eq!(&str, "xyzbcde");
            let str = link_incl.get_string(&txn);
            assert_eq!(&str, "bcde");
            let str = to_weak_xml_text(&link_incl).get_string(&txn);
            assert_eq!(&str, "bcde");
        }
    }

    #[test]
    fn quoted_text_end_boundary_inserts() {
        let mut d1 = Doc::with_client_id(1);
        let arr1 = d1.get_or_insert_array("array");
        let txt1 = d1.get_or_insert_text("text");
        {
            let mut txn = d1.transact_mut();
            txt1.insert(&mut txn, 0, "abcdef");
        }

        let mut d2 = Doc::with_client_id(2);
        let _arr2 = d2.get_or_insert_array("array");
        let txt2 = d2.get_or_insert_text("text");

        exchange_updates([&mut d1, &mut d2]);

        txt2.insert(&mut d2.transact_mut(), 5, "xyz");

        let link_excl = {
            let mut txn = d1.transact_mut();
            let q = txt1.quote(&txn, 1..5).unwrap();
            arr1.insert(&mut txn, 0, q)
        };
        let link_incl = {
            let mut txn = d1.transact_mut();
            let q = txt1.quote(&txn, 1..=4).unwrap();
            arr1.insert(&mut txn, 0, q)
        };

        {
            let txn = d1.transact();
            let str = link_excl.get_string(&txn);
            assert_eq!(&str, "bcde");
            let str = to_weak_xml_text(&link_excl).get_string(&txn);
            assert_eq!(&str, "bcde");
            let str = link_incl.get_string(&txn);
            assert_eq!(&str, "bcde");
            let str = to_weak_xml_text(&link_incl).get_string(&txn);
            assert_eq!(&str, "bcde");
        }

        exchange_updates([&mut d1, &mut d2]);

        {
            let txn = d1.transact();
            let str = link_excl.get_string(&txn);
            assert_eq!(&str, "bcdexyz");
            let str = to_weak_xml_text(&link_excl).get_string(&txn);
            assert_eq!(&str, "bcdexyz");
            let str = link_incl.get_string(&txn);
            assert_eq!(&str, "bcde");
            let str = to_weak_xml_text(&link_incl).get_string(&txn);
            assert_eq!(&str, "bcde");
        }
    }

    #[test]
    fn quote_end_unbounded_text() {
        let mut d1 = Doc::with_client_id(1);
        let mut txn = d1.transact_mut();
        let txt1 = txn.get_or_insert_text("text");
        let arr1 = txn.get_or_insert_array("array");
        txt1.insert(&mut txn, 0, "abc");
        let link1 = txt1.quote(&txn, 1..).unwrap();
        let link1 = arr1.insert(&mut txn, 0, link1);
        let str = link1.get_string(&txn);
        assert_eq!(str, "bc");

        txt1.push(&mut txn, "def");
        let str = link1.get_string(&txn);
        assert_eq!(str, "bcdef");
        drop(txn);

        let mut d2 = Doc::with_client_id(2);

        exchange_updates([&mut d1, &mut d2]);

        let mut txn = d2.transact_mut();
        let _txt2 = txn.get_or_insert_text("text");
        let arr2 = txn.get_or_insert_array("array");

        let link2: WeakRef<TextRef> = arr2.get(&txn, 0).unwrap();
        let str = link2.get_string(&txn);
        assert_eq!(str, "bcdef");
    }

    #[test]
    fn quote_start_unbounded_text() {
        let mut d1 = Doc::with_client_id(1);
        let mut txn = d1.transact_mut();
        let txt1 = txn.get_or_insert_text("text");
        let arr1 = txn.get_or_insert_array("array");
        txt1.insert(&mut txn, 0, "xyz");
        let link1 = txt1.quote(&txn, ..=1).unwrap();
        let link1 = arr1.insert(&mut txn, 0, link1);
        let str = link1.get_string(&txn);
        assert_eq!(str, "xy");

        txt1.insert(&mut txn, 0, "uwv"); // 'uwvxyz'
        let str = link1.get_string(&txn);
        assert_eq!(str, "uwvxy");
        drop(txn);

        let mut d2 = Doc::with_client_id(2);

        exchange_updates([&mut d1, &mut d2]);

        let mut txn = d2.transact_mut();
        let _txt2 = txn.get_or_insert_text("text");
        let arr2 = txn.get_or_insert_array("array");

        let link2: WeakRef<TextRef> = arr2.get(&txn, 0).unwrap();
        let str = link2.get_string(&txn);
        assert_eq!(str, "uwvxy");
    }

    #[test]
    fn quote_both_sides_unbounded_text() {
        let mut d1 = Doc::with_client_id(1);
        let mut txn = d1.transact_mut();
        let txt1 = txn.get_or_insert_text("text");
        let arr1 = txn.get_or_insert_array("array");
        txt1.insert(&mut txn, 0, "xyz");
        let link1 = txt1.quote(&txn, ..).unwrap();
        let link1 = arr1.insert(&mut txn, 0, link1);
        let str = link1.get_string(&txn);
        assert_eq!(str, "xyz");

        txt1.insert(&mut txn, 0, "uwv"); // 'uwvxyz'
        txt1.push(&mut txn, "abc"); // 'uwvxyzabc'
        let str = link1.get_string(&txn);
        assert_eq!(str, "uwvxyzabc");
        drop(txn);

        let mut d2 = Doc::with_client_id(2);

        exchange_updates([&mut d1, &mut d2]);

        let mut txn = d2.transact_mut();
        let _txt2 = txn.get_or_insert_text("text");
        let arr2 = txn.get_or_insert_array("array");

        let link2: WeakRef<TextRef> = arr2.get(&txn, 0).unwrap();
        let str = link2.get_string(&txn);
        assert_eq!(str, "uwvxyzabc");
    }
}
