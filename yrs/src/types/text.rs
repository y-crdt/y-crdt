use crate::block::{Block, BlockPtr, EmbedPrelim, Item, ItemContent, ItemPosition, Prelim};
use crate::block_store::Snapshot;
use crate::transaction::TransactionMut;
use crate::types::{
    Attrs, Branch, BranchPtr, Delta, EventHandler, Observers, Path, Value, TYPE_REFS_TEXT,
};
use crate::utils::OptionExt;
use crate::*;
use lib0::any::Any;
use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::fmt::Formatter;
use std::ops::{Deref, DerefMut};

/// A shared data type used for collaborative text editing. It enables multiple users to add and
/// remove chunks of text in efficient manner. This type is internally represented as a mutable
/// double-linked list of text chunks - an optimization occurs during [Transaction::commit], which
/// allows to squash multiple consecutively inserted characters together as a single chunk of text
/// even between transaction boundaries in order to preserve more efficient memory model.
///
/// [TextRef] structure internally uses UTF-8 encoding and its length is described in a number of
/// bytes rather than individual characters (a single UTF-8 code point can consist of many bytes).
///
/// Like all Yrs shared data types, [TextRef] is resistant to the problem of interleaving (situation
/// when characters inserted one after another may interleave with other peers concurrent inserts
/// after merging all updates together). In case of Yrs conflict resolution is solved by using
/// unique document id to determine correct and consistent ordering.
///
/// [TextRef] offers a rich text editing capabilities (it's not limited to simple text operations).
/// Actions like embedding objects, binaries (eg. images) and formatting attributes are all possible
/// using [TextRef].
///
/// Keep in mind that [TextRef::get_string] method returns a raw string, stripped of formatting
/// attributes or embedded objects. If there's a need to include them, use [TextRef::diff] method
/// instead.
///
/// Another note worth reminding is that human-readable numeric indexes are not good for maintaining
/// cursor positions in rich text documents with real-time collaborative capabilities. In such cases
/// any concurrent update incoming and applied from the remote peer may change the order of elements
/// in current [TextRef], invalidating numeric index. For such cases you can take advantage of fact
/// that [TextRef] implements [IndexedSequence::sticky_index] method that returns a
/// [permanent index](StickyIndex) position that sticks to the same place even when concurrent
/// updates are being made.
///
/// # Example
///
/// ```rust
/// use lib0::any::Any;
/// use yrs::{Array, ArrayPrelim, Doc, GetString, Text, Transact};
/// use yrs::types::Attrs;
/// use yrs::types::text::{Diff, YChange};
///
/// let doc = Doc::new();
/// let text = doc.get_or_insert_text("article");
/// let mut txn = doc.transact_mut();
///
/// let bold = Attrs::from([("b".into(), true.into())]);
/// let italic = Attrs::from([("i".into(), true.into())]);
///
/// text.insert(&mut txn, 0, "hello ");
/// text.insert_with_attributes(&mut txn, 6, "world", italic.clone());
/// text.format(&mut txn, 0, 5, bold.clone());
///
/// let chunks = text.diff(&txn, YChange::identity);
/// assert_eq!(chunks, vec![
///     Diff::new("hello".into(), Some(Box::new(bold.clone()))),
///     Diff::new(" ".into(), None),
///     Diff::new("world".into(), Some(Box::new(italic))),
/// ]);
///
/// // remove formatting
/// let remove_italic = Attrs::from([("i".into(), Any::Null)]);
/// text.format(&mut txn, 6, 5, remove_italic);
///
/// let chunks = text.diff(&txn, YChange::identity);
/// assert_eq!(chunks, vec![
///     Diff::new("hello".into(), Some(Box::new(bold.clone()))),
///     Diff::new(" world".into(), None),
/// ]);
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
pub struct TextRef(BranchPtr);

impl Text for TextRef {}
impl IndexedSequence for TextRef {}

impl Into<XmlTextRef> for TextRef {
    fn into(self) -> XmlTextRef {
        XmlTextRef::from(self.0)
    }
}

impl Observable for TextRef {
    type Event = TextEvent;

    fn try_observer(&self) -> Option<&EventHandler<Self::Event>> {
        if let Some(Observers::Text(eh)) = self.0.observers.as_ref() {
            Some(eh)
        } else {
            None
        }
    }

    fn try_observer_mut(&mut self) -> Option<&mut EventHandler<Self::Event>> {
        if let Observers::Text(eh) = self.0.observers.get_or_insert_with(Observers::text) {
            Some(eh)
        } else {
            None
        }
    }
}

impl GetString for TextRef {
    /// Converts context of this text data structure into a single string value. This method doesn't
    /// render formatting attributes or embedded content. In order to retrieve it, use
    /// [TextRef::diff] method.
    fn get_string<T: ReadTxn>(&self, txn: &T) -> String {
        let mut start = self.as_ref().start;
        let mut s = String::new();
        while let Some(Block::Item(item)) = start.as_deref() {
            if !item.is_deleted() {
                if let ItemContent::String(item_string) = &item.content {
                    s.push_str(item_string);
                }
            }
            start = item.right.clone();
        }
        s
    }
}

impl TryFrom<BlockPtr> for TextRef {
    type Error = BlockPtr;

    fn try_from(value: BlockPtr) -> Result<Self, Self::Error> {
        if let Some(branch) = value.clone().as_branch() {
            Ok(TextRef::from(branch))
        } else {
            Err(value)
        }
    }
}

pub trait Text: AsRef<Branch> {
    /// Returns a number of characters visible in a current text data structure.
    fn len<T: ReadTxn>(&self, txn: &T) -> u32 {
        self.as_ref().content_len
    }

    /// Inserts a `chunk` of text at a given `index`.
    /// If `index` is `0`, this `chunk` will be inserted at the beginning of a current text.
    /// If `index` is equal to current data structure length, this `chunk` will be appended at
    /// the end of it.
    ///
    /// This method will panic if provided `index` is greater than the length of a current text.
    ///
    /// # Examples
    ///
    /// By default Document uses byte offset:
    ///
    /// ```
    /// use yrs::{Doc, Text, GetString, Transact};
    ///
    /// let doc = Doc::new();
    /// let ytext = doc.get_or_insert_text("text");
    /// let txn = &mut doc.transact_mut();
    /// ytext.push(txn, "Hi ★ to you");
    ///
    /// // The same as `String::len()`
    /// assert_eq!(ytext.len(txn), 13);
    ///
    /// // To insert you have to count bytes and not chars.
    /// ytext.insert(txn, 6, "!");
    /// assert_eq!(ytext.get_string(txn), "Hi ★! to you");
    /// ```
    ///
    /// You can override how Yrs calculates the index with [OffsetKind]:
    ///
    /// ```
    /// use yrs::{Doc, Options, Text, GetString, Transact, OffsetKind};
    ///
    /// let doc = Doc::with_options(Options {
    ///     offset_kind: OffsetKind::Utf32,
    ///     ..Default::default()
    /// });
    /// let ytext = doc.get_or_insert_text("text");
    /// let txn = &mut doc.transact_mut();
    /// ytext.push(txn, "Hi ★ to you");
    ///
    /// // The same as `String::chars()::count()`
    /// assert_eq!(ytext.len(txn), 11);
    ///
    /// // To insert you have to count chars.
    /// ytext.insert(txn, 4, "!");
    /// assert_eq!(ytext.get_string(txn), "Hi ★! to you");
    /// ```
    ///
    fn insert(&self, txn: &mut TransactionMut, index: u32, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        let this = BranchPtr::from(self.as_ref());
        if let Some(mut pos) = find_position(this, txn, index) {
            let value = crate::block::PrelimString(chunk.into());
            while let Some(right) = pos.right.as_ref() {
                if right.is_deleted() {
                    // skip over deleted blocks, just like Yjs does
                    pos.forward();
                } else {
                    break;
                }
            }
            txn.create_item(&pos, value, None);
        } else {
            panic!("The type or the position doesn't exist!");
        }
    }

    /// Inserts a `chunk` of text at a given `index`.
    /// If `index` is `0`, this `chunk` will be inserted at the beginning of a current text.
    /// If `index` is equal to current data structure length, this `chunk` will be appended at
    /// the end of it.
    /// Collection of supplied `attributes` will be used to wrap provided text `chunk` range with a
    /// formatting blocks.
    ///
    /// This method will panic if provided `index` is greater than the length of a current text.
    fn insert_with_attributes(
        &self,
        txn: &mut TransactionMut,
        index: u32,
        chunk: &str,
        mut attributes: Attrs,
    ) {
        let this = BranchPtr::from(self.as_ref());
        if let Some(mut pos) = find_position(this, txn, index) {
            pos.unset_missing(&mut attributes);
            minimize_attr_changes(&mut pos, &attributes);
            let negated_attrs = insert_attributes(this, txn, &mut pos, attributes);

            let value = block::PrelimString(chunk.into());
            let item = txn.create_item(&pos, value, None);

            pos.right = Some(item);
            pos.forward();

            insert_negated_attributes(this, txn, &mut pos, negated_attrs);
        } else {
            panic!("The type or the position doesn't exist!");
        }
    }

    /// Inserts an embed `content` at a given `index`.
    ///
    /// If `index` is `0`, this `content` will be inserted at the beginning of a current text.
    /// If `index` is equal to current data structure length, this `embed` will be appended at
    /// the end of it.
    ///
    /// This method will panic if provided `index` is greater than the length of a current text.
    fn insert_embed<V>(&self, txn: &mut TransactionMut, index: u32, content: V) -> V::Return
    where
        V: Into<EmbedPrelim<V>> + Prelim,
    {
        let this = BranchPtr::from(self.as_ref());
        if let Some(pos) = find_position(this, txn, index) {
            let ptr = txn.create_item(&pos, content.into(), None);
            if let Ok(integrated) = ptr.try_into() {
                integrated
            } else {
                panic!("Defect: embedded return type doesn't match.")
            }
        } else {
            panic!("The type or the position doesn't exist!");
        }
    }

    /// Inserts an embed `content` of text at a given `index`.
    /// If `index` is `0`, this `content` will be inserted at the beginning of a current text.
    /// If `index` is equal to current data structure length, this `chunk` will be appended at
    /// the end of it.
    /// Collection of supplied `attributes` will be used to wrap provided text `content` range with
    /// a formatting blocks.
    ///
    /// This method will panic if provided `index` is greater than the length of a current text.
    fn insert_embed_with_attributes<V>(
        &self,
        txn: &mut TransactionMut,
        index: u32,
        embed: V,
        mut attributes: Attrs,
    ) -> V::Return
    where
        V: Into<EmbedPrelim<V>> + Prelim,
    {
        let this = BranchPtr::from(self.as_ref());
        if let Some(mut pos) = find_position(this, txn, index) {
            pos.unset_missing(&mut attributes);
            minimize_attr_changes(&mut pos, &attributes);
            let negated_attrs = insert_attributes(this, txn, &mut pos, attributes);

            let item = txn.create_item(&pos, embed.into(), None);

            pos.right = Some(item.clone());
            pos.forward();

            insert_negated_attributes(this, txn, &mut pos, negated_attrs);
            if let Ok(integrated) = item.try_into() {
                integrated
            } else {
                panic!("Defect: unexpected returned integrated type")
            }
        } else {
            panic!("The type or the position doesn't exist!");
        }
    }

    /// Appends a given `chunk` of text at the end of a current text structure.
    fn push(&self, txn: &mut TransactionMut, chunk: &str) {
        let idx = self.len(txn);
        self.insert(txn, idx, chunk)
    }

    /// Removes up to a `len` characters from a current text structure, starting at given `index`.
    /// This method panics in case when not all expected characters were removed (due to
    /// insufficient number of characters to remove) or `index` is outside of the bounds of text.
    fn remove_range(&self, txn: &mut TransactionMut, index: u32, len: u32) {
        let this = BranchPtr::from(self.as_ref());
        if let Some(pos) = find_position(this, txn, index) {
            remove(txn, pos, len)
        } else {
            panic!("The type or the position doesn't exist!");
        }
    }

    /// Wraps an existing piece of text within a range described by `index`-`len` parameters with
    /// formatting blocks containing provided `attributes` metadata.
    fn format(&self, txn: &mut TransactionMut, index: u32, len: u32, attributes: Attrs) {
        let this = BranchPtr::from(self.as_ref());
        if let Some(pos) = find_position(this, txn, index) {
            insert_format(this, txn, pos, len, attributes)
        } else {
            panic!("Index {} is outside of the range.", index);
        }
    }

    /// Returns an ordered sequence of formatted chunks, current [Text] corresponds of. These chunks
    /// may contain inserted pieces of text or more complex elements like embedded binaries of
    /// shared objects. Chunks are organized by type of inserted value and formatting attributes
    /// wrapping it around. If formatting attributes are nested into each other, they will be split
    /// into separate [Diff] chunks.
    ///
    /// `compute_ychange` callback is used to attach custom data to produced chunks.
    ///
    /// # Example
    ///
    /// ```rust
    /// use yrs::{Doc, Text, Transact};
    /// use yrs::types::Attrs;
    /// use yrs::types::text::{Diff, YChange};
    ///
    /// let doc = Doc::new();
    /// let text = doc.get_or_insert_text("article");
    /// let mut txn = doc.transact_mut();
    ///
    /// let bold = Attrs::from([("b".into(), true.into())]);
    /// let italic = Attrs::from([("i".into(), true.into())]);
    ///
    /// text.insert_with_attributes(&mut txn, 0, "hello world", italic.clone()); // "<i>hello world</i>"
    /// text.format(&mut txn, 6, 5, bold.clone()); // "<i>hello <b>world</b></i>"
    /// let image = vec![0, 0, 0, 0];
    /// text.insert_embed(&mut txn, 5, image.clone()); // insert binary after "hello"
    ///
    /// let italic_and_bold = Attrs::from([
    ///   ("b".into(), true.into()),
    ///   ("i".into(), true.into())
    /// ]);
    /// let chunks = text.diff(&txn, YChange::identity);
    /// assert_eq!(chunks, vec![
    ///     Diff::new("hello".into(), Some(Box::new(italic.clone()))),
    ///     Diff::new(image.into(), Some(Box::new(italic.clone()))),
    ///     Diff::new(" ".into(), Some(Box::new(italic))),
    ///     Diff::new("world".into(), Some(Box::new(italic_and_bold))),
    /// ]);
    /// ```
    fn diff<T, D, F>(&self, txn: &T, compute_ychange: F) -> Vec<Diff<D>>
    where
        T: ReadTxn,
        F: Fn(YChange) -> D,
    {
        let mut asm = DiffAssembler::new(compute_ychange);
        asm.process(self.as_ref().start, None, None);
        asm.finish()
    }

    /// Returns the Delta representation of this YText type.
    fn diff_range<D, F>(
        &self,
        txn: &mut TransactionMut,
        hi: Option<&Snapshot>,
        lo: Option<&Snapshot>,
        compute_ychange: F,
    ) -> Vec<Diff<D>>
    where
        F: Fn(YChange) -> D,
    {
        if let Some(snapshot) = hi {
            txn.split_by_snapshot(snapshot);
        }
        if let Some(snapshot) = lo {
            txn.split_by_snapshot(snapshot);
        }

        let mut asm = DiffAssembler::new(compute_ychange);
        asm.process(self.as_ref().start, hi, lo);
        asm.finish()
    }
}

impl From<BranchPtr> for TextRef {
    fn from(inner: BranchPtr) -> Self {
        TextRef(inner)
    }
}

impl AsRef<Branch> for TextRef {
    fn as_ref(&self) -> &Branch {
        self.0.deref()
    }
}

impl AsMut<Branch> for TextRef {
    fn as_mut(&mut self) -> &mut Branch {
        self.0.deref_mut()
    }
}

struct DiffAssembler<D, F>
where
    F: Fn(YChange) -> D,
{
    ops: Vec<Diff<D>>,
    buf: String,
    curr_attrs: Attrs,
    curr_ychange: Option<YChange>,
    compute_ychange: F,
}

impl<T, F> DiffAssembler<T, F>
where
    F: Fn(YChange) -> T,
{
    fn new(compute_ychange: F) -> Self {
        DiffAssembler {
            ops: Vec::new(),
            buf: String::new(),
            curr_attrs: HashMap::new(),
            curr_ychange: None,
            compute_ychange,
        }
    }
    fn pack_str(&mut self) {
        if !self.buf.is_empty() {
            let attrs = self.attrs_boxed();
            let mut buf = std::mem::replace(&mut self.buf, String::new());
            buf.shrink_to_fit();
            let change = if let Some(ychange) = self.curr_ychange.take() {
                Some((self.compute_ychange)(ychange))
            } else {
                None
            };
            let op = Diff::with_change(Value::Any(buf.into()), attrs, change);
            self.ops.push(op);
        }
    }

    fn finish(self) -> Vec<Diff<T>> {
        self.ops
    }

    fn attrs_boxed(&mut self) -> Option<Box<Attrs>> {
        if self.curr_attrs.is_empty() {
            None
        } else {
            Some(Box::new(self.curr_attrs.clone()))
        }
    }

    fn process(&mut self, mut n: Option<BlockPtr>, hi: Option<&Snapshot>, lo: Option<&Snapshot>) {
        fn seen(snapshot: Option<&Snapshot>, item: &Item) -> bool {
            if let Some(s) = snapshot {
                s.is_visible(&item.id)
            } else {
                !item.is_deleted()
            }
        }

        while let Some(Block::Item(item)) = n.as_deref() {
            if seen(hi, item) || (lo.is_some() && seen(lo, item)) {
                match &item.content {
                    ItemContent::String(s) => {
                        if let Some(snapshot) = hi {
                            if !snapshot.is_visible(&item.id) {
                                self.pack_str();
                                self.curr_ychange =
                                    Some(YChange::new(ChangeKind::Removed, item.id));
                            } else if let Some(snapshot) = lo {
                                if !snapshot.is_visible(&item.id) {
                                    self.pack_str();
                                    self.curr_ychange =
                                        Some(YChange::new(ChangeKind::Added, item.id));
                                } else if self.curr_ychange.is_some() {
                                    self.pack_str();
                                }
                            }
                        }
                        self.buf.push_str(s.as_str());
                    }
                    ItemContent::Type(_) | ItemContent::Embed(_) => {
                        self.pack_str();
                        if let Some(value) = item.content.get_first() {
                            let attrs = self.attrs_boxed();
                            self.ops.push(Diff::new(value, attrs));
                        }
                    }
                    ItemContent::Format(key, value) => {
                        if seen(hi, item) {
                            self.pack_str();
                            update_current_attributes(&mut self.curr_attrs, key, value.as_ref());
                        }
                    }
                    _ => {}
                }
            }
            n = item.right;
        }

        self.pack_str();
    }
}

pub(crate) fn update_current_attributes(attrs: &mut Attrs, key: &str, value: &Any) {
    if let Any::Null = value {
        attrs.remove(key);
    } else {
        attrs.insert(key.into(), value.clone());
    }
}

fn find_position(this: BranchPtr, txn: &mut TransactionMut, index: u32) -> Option<ItemPosition> {
    let mut pos = {
        ItemPosition {
            parent: this.into(),
            left: None,
            right: this.start,
            index: 0,
            current_attrs: None,
        }
    };

    let mut format_ptrs = HashMap::new();
    let store = txn.store_mut();
    let encoding = store.options.offset_kind;
    let mut remaining = index;
    while let Some(mut right_ptr) = pos.right {
        if remaining == 0 {
            break;
        }

        if let Block::Item(right) = right_ptr.deref_mut() {
            if !right.is_deleted() {
                match &right.content {
                    ItemContent::Format(key, value) => {
                        if let Any::Null = value.as_ref() {
                            format_ptrs.remove(key);
                        } else {
                            format_ptrs.insert(key.clone(), pos.right.clone());
                        }
                    }
                    _ => {
                        let mut block_len = right.len();
                        let content_len = right.content_len(encoding);
                        if remaining < content_len {
                            // split right item
                            let offset = if let ItemContent::String(str) = &right.content {
                                str.block_offset(remaining, encoding)
                            } else {
                                remaining
                            };
                            store
                                .blocks
                                .split_block(right_ptr, offset, OffsetKind::Utf16)
                                .unwrap();
                            block_len -= offset;
                            remaining = 0;
                        } else {
                            remaining -= content_len;
                        }
                        pos.index += block_len;
                    }
                }
            }
            pos.left = pos.right.take();
            pos.right = if let Some(Block::Item(item)) = pos.left.as_deref() {
                item.right
            } else {
                None
            };
        } else {
            return None;
        }
    }

    for (_, block_ptr) in format_ptrs {
        if let Some(mut ptr) = block_ptr {
            if let Block::Item(item) = ptr.deref_mut() {
                if let ItemContent::Format(key, value) = &item.content {
                    let attrs = pos.current_attrs.get_or_init();
                    update_current_attributes(attrs, key, value.as_ref());
                }
            }
        }
    }

    Some(pos)
}

fn remove(txn: &mut TransactionMut, mut pos: ItemPosition, len: u32) {
    let encoding = txn.store().options.offset_kind;
    let mut remaining = len;
    let start = pos.right.clone();
    let start_attrs = pos.current_attrs.clone();
    while let Some(Block::Item(item)) = pos.right.as_deref() {
        if remaining == 0 {
            break;
        }

        if !item.is_deleted() {
            match &item.content {
                ItemContent::Embed(_) | ItemContent::String(_) | ItemContent::Type(_) => {
                    let content_len = item.content_len(encoding);
                    let ptr = pos.right.unwrap();
                    if remaining < content_len {
                        // split block
                        let offset = if let ItemContent::String(s) = &item.content {
                            s.block_offset(remaining, encoding)
                        } else {
                            len
                        };
                        remaining = 0;
                        txn.store_mut()
                            .blocks
                            .split_block(ptr, offset, OffsetKind::Utf16);
                    } else {
                        remaining -= content_len;
                    };
                    txn.delete(ptr);
                }
                _ => {}
            }
        }

        pos.forward();
    }

    if remaining > 0 {
        panic!(
            "Couldn't remove {} elements from an array. Only {} of them were successfully removed.",
            len,
            len - remaining
        );
    }

    if let (Some(start), Some(start_attrs), Some(end_attrs)) =
        (start, start_attrs, pos.current_attrs.as_mut())
    {
        clean_format_gap(
            txn,
            Some(start),
            pos.right,
            start_attrs.as_ref(),
            end_attrs.as_mut(),
        );
    }
}

fn is_valid_target(ptr: BlockPtr) -> bool {
    if ptr.is_deleted() {
        true
    } else if let Block::Item(item) = ptr.deref() {
        if let ItemContent::Format(_, _) = &item.content {
            true
        } else {
            false
        }
    } else {
        true
    }
}

fn insert_format(
    this: BranchPtr,
    txn: &mut TransactionMut,
    mut pos: ItemPosition,
    mut len: u32,
    attrs: Attrs,
) {
    minimize_attr_changes(&mut pos, &attrs);
    let mut negated_attrs = insert_attributes(this, txn, &mut pos, attrs.clone()); //TODO: remove `attrs.clone()`
    let encoding = txn.store().options.offset_kind;
    // iterate until first non-format or null is found
    // delete all formats with attributes[format.key] != null
    // also check the attributes after the first non-format as we do not want to insert redundant
    // negated attributes there
    while let Some(right) = pos.right {
        if !(len > 0 || (!negated_attrs.is_empty() && is_valid_target(right))) {
            break;
        }

        if let Block::Item(item) = right.deref() {
            if !item.is_deleted() {
                match &item.content {
                    ItemContent::Format(key, value) => {
                        if let Some(v) = attrs.get(key) {
                            if v == value.as_ref() {
                                negated_attrs.remove(key);
                            } else {
                                negated_attrs.insert(key.clone(), *value.clone());
                            }
                            txn.delete(right);
                        }
                    }
                    ItemContent::String(s) => {
                        let content_len = item.content_len(encoding);
                        if len < content_len {
                            // split block
                            let offset = s.block_offset(len, encoding);
                            let new_right = txn.store_mut().blocks.split_block(
                                right,
                                offset,
                                OffsetKind::Utf16,
                            );
                            pos.left = Some(right);
                            pos.right = new_right;
                            break;
                        }
                        len -= content_len;
                    }
                    _ => {
                        let content_len = item.len();
                        if len < content_len {
                            let new_right =
                                txn.store_mut()
                                    .blocks
                                    .split_block(right, len, OffsetKind::Utf16);
                            pos.left = Some(right);
                            pos.right = new_right;
                            break;
                        }
                        len -= content_len;
                    }
                }
            }
        }

        if !pos.forward() {
            break;
        }
    }

    insert_negated_attributes(this, txn, &mut pos, negated_attrs);
}

fn minimize_attr_changes(pos: &mut ItemPosition, attrs: &Attrs) {
    // go right while attrs[right.key] === right.value (or right is deleted)
    while let Some(Block::Item(i)) = pos.right.as_deref() {
        if !i.is_deleted() {
            if let ItemContent::Format(k, v) = &i.content {
                if let Some(v2) = attrs.get(k) {
                    if (v.as_ref()).eq(v2) {
                        pos.forward();
                        continue;
                    }
                }
            }

            break;
        } else {
            pos.forward();
        }
    }
}

fn insert_attributes(
    this: BranchPtr,
    txn: &mut TransactionMut,
    pos: &mut ItemPosition,
    attrs: Attrs,
) -> Attrs {
    let mut negated_attrs = HashMap::with_capacity(attrs.len());
    let mut store = txn.store_mut();
    for (k, v) in attrs {
        let current_value = pos
            .current_attrs
            .as_ref()
            .and_then(|a| a.get(&k))
            .unwrap_or(&Any::Null);
        if &v != current_value {
            // save negated attribute (set null if currentVal undefined)
            negated_attrs.insert(k.clone(), current_value.clone());

            let client_id = store.options.client_id;
            let parent = this.into();
            let mut item = Item::new(
                ID::new(client_id, store.blocks.get_state(&client_id)),
                pos.left.clone(),
                pos.left.map(|ptr| ptr.last_id()),
                pos.right.clone(),
                pos.right.map(|ptr| ptr.id().clone()),
                parent,
                None,
                ItemContent::Format(k, v.into()),
            );
            let mut item_ptr = BlockPtr::from(&mut item);
            pos.right = Some(item_ptr);
            item_ptr.integrate(txn, 0);
            let local_block_list = txn
                .store_mut()
                .blocks
                .get_client_blocks_mut(item.id().client);
            local_block_list.push(item);

            pos.forward();
            store = txn.store_mut();
        }
    }
    negated_attrs
}

fn insert_negated_attributes(
    this: BranchPtr,
    txn: &mut TransactionMut,
    pos: &mut ItemPosition,
    mut attrs: Attrs,
) {
    while let Some(Block::Item(item)) = pos.right.as_deref() {
        if !item.is_deleted() {
            if let ItemContent::Format(key, value) = &item.content {
                if let Some(curr_val) = attrs.get(key) {
                    if curr_val == value.as_ref() {
                        attrs.remove(key);
                        pos.forward();
                        continue;
                    }
                }
            }

            break;
        } else {
            pos.forward();
        }
    }

    let mut store = txn.store_mut();
    for (k, v) in attrs {
        let client_id = store.options.client_id;
        let parent = this.into();
        let mut item = Item::new(
            ID::new(client_id, store.blocks.get_state(&client_id)),
            pos.left.clone(),
            pos.left.map(|ptr| ptr.last_id()),
            pos.right.clone(),
            pos.right.map(|ptr| ptr.id().clone()),
            parent,
            None,
            ItemContent::Format(k, v.into()),
        );
        let mut item_ptr = BlockPtr::from(&mut item);
        pos.right = Some(item_ptr);
        item_ptr.integrate(txn, 0);

        let local_block_list = txn
            .store_mut()
            .blocks
            .get_client_blocks_mut(item.id().client);
        local_block_list.push(item);

        pos.forward();
        store = txn.store_mut();
    }
}

fn clean_format_gap(
    txn: &mut TransactionMut,
    mut start: Option<BlockPtr>,
    mut end: Option<BlockPtr>,
    start_attrs: &Attrs,
    end_attrs: &mut Attrs,
) -> u32 {
    while let Some(Block::Item(item)) = end.as_deref() {
        match &item.content {
            ItemContent::String(_) | ItemContent::Embed(_) => break,
            ItemContent::Format(key, value) if !item.is_deleted() => {
                update_current_attributes(end_attrs, key.as_ref(), value);
            }
            _ => {}
        }
        end = item.right.clone();
    }

    let mut cleanups = 0;
    while start != end {
        if let Some(Block::Item(item)) = start.as_deref() {
            let right = item.right.clone();
            if !item.is_deleted() {
                if let ItemContent::Format(key, value) = &item.content {
                    let e = end_attrs.get(key).unwrap_or(&Any::Null);
                    let s = start_attrs.get(key).unwrap_or(&Any::Null);
                    if e != value.as_ref() || s == value.as_ref() {
                        txn.delete(start.unwrap());
                        cleanups += 1;
                    }
                }
            }
            start = right;
        } else {
            break;
        }
    }
    cleanups
}

/// A representation of an uniformly-formatted chunk of rich context stored by [TextRef] or
/// [XmlTextRef]. It contains a value (which could be a string, embedded object or another shared
/// type) with optional formatting attributes wrapping around this chunk. It can also contain some
/// custom data generated by caller as part of [TextRef::diff] callback.
#[derive(Debug, PartialEq)]
pub struct Diff<T> {
    /// Inserted chunk of data. It can be (usually) piece of text, but possibly also embedded value
    /// or another shared type.
    pub insert: Value,

    /// Optional formatting attributes wrapping inserted chunk of data.
    pub attributes: Option<Box<Attrs>>,

    /// Custom user data attached to this chunk of data.
    pub ychange: Option<T>,
}

impl<T> Diff<T> {
    pub fn new(insert: Value, attributes: Option<Box<Attrs>>) -> Self {
        Self::with_change(insert, attributes, None)
    }

    pub fn with_change(insert: Value, attributes: Option<Box<Attrs>>, ychange: Option<T>) -> Self {
        Diff {
            insert,
            attributes,
            ychange,
        }
    }
}

impl<T> std::fmt::Display for Diff<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{ insert: '{}'", self.insert)?;
        if let Some(attrs) = self.attributes.as_ref() {
            write!(f, ", attributes: {{")?;
            let mut i = attrs.iter();
            if let Some((k, v)) = i.next() {
                write!(f, " {}={}", k, v)?;
            }
            for (k, v) in i {
                write!(f, ", {}={}", k, v)?;
            }
            write!(f, " }}")?;
        }
        write!(f, " }}")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct YChange {
    pub kind: ChangeKind,
    pub id: ID,
}

impl YChange {
    pub fn new(kind: ChangeKind, id: ID) -> Self {
        YChange { kind, id }
    }

    #[inline]
    pub fn identity(change: YChange) -> YChange {
        change
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Removed,
}

/// Event generated by [Text::observe] method. Emitted during transaction commit phase.
pub struct TextEvent {
    pub(crate) current_target: BranchPtr,
    target: TextRef,
    delta: UnsafeCell<Option<Vec<Delta>>>,
}

impl TextEvent {
    pub(crate) fn new(branch_ref: BranchPtr) -> Self {
        let current_target = branch_ref.clone();
        let target = TextRef::from(branch_ref);
        TextEvent {
            target,
            current_target,
            delta: UnsafeCell::new(None),
        }
    }

    /// Returns a [Text] instance which emitted this event.
    pub fn target(&self) -> &TextRef {
        &self.target
    }

    /// Returns a path from root type down to [Text] instance which emitted this event.
    pub fn path(&self) -> Path {
        Branch::path(self.current_target, self.target.0)
    }

    /// Returns a summary of text changes made over corresponding [Text] collection within
    /// bounds of current transaction.
    pub fn delta(&self, txn: &TransactionMut) -> &[Delta] {
        let delta = unsafe { self.delta.get().as_mut().unwrap() };
        delta
            .get_or_insert_with(|| Self::get_delta(self.target.0, txn))
            .as_slice()
    }

    pub(crate) fn get_delta(target: BranchPtr, txn: &TransactionMut) -> Vec<Delta> {
        #[derive(Debug, Clone, Copy, Eq, PartialEq)]
        enum Action {
            Insert,
            Retain,
            Delete,
        }

        #[derive(Debug, Default)]
        struct DeltaAssembler {
            action: Option<Action>,
            insert: Option<Value>,
            insert_string: Option<String>,
            retain: u32,
            delete: u32,
            attrs: Attrs,
            current_attrs: Attrs,
            delta: Vec<Delta>,
        }

        impl DeltaAssembler {
            fn add_op(&mut self) {
                match self.action.take() {
                    None => {}
                    Some(Action::Delete) => {
                        let len = self.delete;
                        self.delete = 0;
                        self.delta.push(Delta::Deleted(len))
                    }
                    Some(Action::Insert) => {
                        let value = if let Some(str) = self.insert.take() {
                            str
                        } else {
                            let value = self.insert_string.take().unwrap().into_boxed_str();
                            Any::String(value).into()
                        };
                        let attrs = if self.current_attrs.is_empty() {
                            None
                        } else {
                            Some(Box::new(self.current_attrs.clone()))
                        };
                        self.delta.push(Delta::Inserted(value, attrs))
                    }
                    Some(Action::Retain) => {
                        let len = self.retain;
                        self.retain = 0;
                        let attrs = if self.attrs.is_empty() {
                            None
                        } else {
                            Some(Box::new(self.attrs.clone()))
                        };
                        self.delta.push(Delta::Retain(len, attrs));
                    }
                }
            }

            fn finish(mut self) -> Vec<Delta> {
                while let Some(last) = self.delta.pop() {
                    match last {
                        Delta::Retain(_, None) => {
                            // retain delta's if they don't assign attributes
                        }
                        other => {
                            self.delta.push(other);
                            return self.delta;
                        }
                    }
                }
                self.delta
            }
        }

        let encoding = txn.store().options.offset_kind;
        let mut old_attrs = HashMap::new();
        let mut asm = DeltaAssembler::default();
        let mut current = target.start;

        while let Some(Block::Item(item)) = current.as_deref() {
            match &item.content {
                ItemContent::Type(_) | ItemContent::Embed(_) => {
                    if txn.has_added(&item.id) {
                        if !txn.has_deleted(&item.id) {
                            asm.add_op();
                            asm.action = Some(Action::Insert);
                            asm.insert = item.content.get_last();
                            asm.add_op();
                        }
                    } else if txn.has_deleted(&item.id) {
                        if asm.action != Some(Action::Delete) {
                            asm.add_op();
                            asm.action = Some(Action::Delete);
                        }
                        asm.delete += 1;
                    } else if !item.is_deleted() {
                        if asm.action != Some(Action::Retain) {
                            asm.add_op();
                            asm.action = Some(Action::Retain);
                        }
                        asm.retain += 1;
                    }
                }
                ItemContent::String(s) => {
                    if txn.has_added(&item.id) {
                        if !txn.has_deleted(&item.id) {
                            if asm.action != Some(Action::Insert) {
                                asm.add_op();
                                asm.action = Some(Action::Insert);
                            }
                            let buf = asm.insert_string.get_or_insert_with(String::default);
                            buf.push_str(s.as_str());
                        }
                    } else if txn.has_deleted(&item.id) {
                        if asm.action != Some(Action::Delete) {
                            asm.add_op();
                            asm.action = Some(Action::Delete);
                        }
                        let content_len = item.content_len(encoding);
                        asm.delete += content_len;
                    } else if !item.is_deleted() {
                        if asm.action != Some(Action::Retain) {
                            asm.add_op();
                            asm.action = Some(Action::Retain);
                        }
                        asm.retain += item.content_len(encoding);
                    }
                }
                ItemContent::Format(key, value) => {
                    if txn.has_added(&item.id) {
                        if !txn.has_deleted(&item.id) {
                            let current_val = asm.current_attrs.get(key);
                            if current_val != Some(value) {
                                if asm.action == Some(Action::Retain) {
                                    asm.add_op();
                                }
                                match old_attrs.get(key) {
                                    None if value.as_ref() == &Any::Null => {
                                        asm.attrs.remove(key);
                                    }
                                    Some(v) if v == value => {
                                        asm.attrs.remove(key);
                                    }
                                    _ => {
                                        asm.attrs.insert(key.clone(), *value.clone());
                                    }
                                }
                            } else {
                                // item.delete(transaction)
                            }
                        }
                    } else if txn.has_deleted(&item.id) {
                        old_attrs.insert(key.clone(), value.clone());
                        let current_val = asm.current_attrs.get(key).unwrap_or(&Any::Null);
                        if current_val != value.as_ref() {
                            let curr_val_clone = current_val.clone();
                            if asm.action == Some(Action::Retain) {
                                asm.add_op();
                            }
                            asm.attrs.insert(key.clone(), curr_val_clone);
                        }
                    } else if !item.is_deleted() {
                        old_attrs.insert(key.clone(), value.clone());
                        let attr = asm.attrs.get(key);
                        if let Some(attr) = attr {
                            if attr != value.as_ref() {
                                if asm.action == Some(Action::Retain) {
                                    asm.add_op();
                                }
                                if value.as_ref() == &Any::Null {
                                    asm.attrs.remove(key);
                                } else {
                                    asm.attrs.insert(key.clone(), *value.clone());
                                }
                            } else {
                                // item.delete(transaction)
                            }
                        }
                    }

                    if !item.is_deleted() {
                        if asm.action == Some(Action::Insert) {
                            asm.add_op();
                        }
                        update_current_attributes(&mut asm.current_attrs, key, value.as_ref());
                    }
                }
                _ => {}
            }

            current = item.right;
        }

        asm.add_op();
        asm.finish()
    }
}

/// A preliminary text. It's can be used to initialize a [TextRef], when it's about to be nested
/// into another Yrs data collection, such as [Map] or [Array].
#[derive(Debug)]
pub struct TextPrelim<T: Borrow<str>>(T);

impl<T: Borrow<str>> TextPrelim<T> {
    pub fn new(value: T) -> Self {
        TextPrelim(value)
    }
}

impl<T: Borrow<str>> Prelim for TextPrelim<T> {
    type Return = TextRef;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let inner = Branch::new(TYPE_REFS_TEXT, None);
        (ItemContent::Type(inner), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        let borrowed = self.0.borrow();
        if !borrowed.is_empty() {
            let text = TextRef::from(inner_ref);
            text.push(txn, borrowed);
        }
    }
}

impl<T: Borrow<str>> Into<EmbedPrelim<TextPrelim<T>>> for TextPrelim<T> {
    #[inline]
    fn into(self) -> EmbedPrelim<TextPrelim<T>> {
        EmbedPrelim::Shared(self)
    }
}

#[cfg(test)]
mod test {
    use crate::doc::{OffsetKind, Options};
    use crate::test_utils::{exchange_updates, run_scenario, RngExt};
    use crate::transaction::ReadTxn;
    use crate::types::text::{Attrs, ChangeKind, Delta, Diff, YChange};
    use crate::types::Value;
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::{Encode, Encoder, EncoderV1};
    use crate::{
        ArrayPrelim, Doc, GetString, Observable, StateVector, Text, Transact, Update, XmlTextRef,
        ID,
    };
    use lib0::any;
    use lib0::any::Any;
    use rand::prelude::StdRng;
    use rand::Rng;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::time::Duration;

    #[test]
    fn insert_empty_string() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        assert_eq!(txt.get_string(&txn).as_str(), "");

        txt.push(&mut txn, "");
        assert_eq!(txt.get_string(&txn).as_str(), "");

        txt.push(&mut txn, "abc");
        txt.push(&mut txn, "");
        assert_eq!(txt.get_string(&txn).as_str(), "abc");
    }

    #[test]
    fn append_single_character_blocks() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "a");
        txt.insert(&mut txn, 1, "b");
        txt.insert(&mut txn, 2, "c");

        assert_eq!(txt.get_string(&txn).as_str(), "abc");
    }

    #[test]
    fn append_mutli_character_blocks() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "hello");
        txt.insert(&mut txn, 5, " ");
        txt.insert(&mut txn, 6, "world");

        assert_eq!(txt.get_string(&txn).as_str(), "hello world");
    }

    #[test]
    fn prepend_single_character_blocks() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "a");
        txt.insert(&mut txn, 0, "b");
        txt.insert(&mut txn, 0, "c");

        assert_eq!(txt.get_string(&txn).as_str(), "cba");
    }

    #[test]
    fn prepend_mutli_character_blocks() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "hello");
        txt.insert(&mut txn, 0, " ");
        txt.insert(&mut txn, 0, "world");

        assert_eq!(txt.get_string(&txn).as_str(), "world hello");
    }

    #[test]
    fn insert_after_block() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "hello");
        txt.insert(&mut txn, 5, " ");
        txt.insert(&mut txn, 6, "world");
        txt.insert(&mut txn, 6, "beautiful ");

        assert_eq!(txt.get_string(&txn).as_str(), "hello beautiful world");
    }

    #[test]
    fn insert_inside_of_block() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "it was expected");
        txt.insert(&mut txn, 6, " not");

        assert_eq!(txt.get_string(&txn).as_str(), "it was not expected");
    }

    #[test]
    fn insert_concurrent_root() {
        let d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("test");
        let mut t1 = d1.transact_mut();

        txt1.insert(&mut t1, 0, "hello ");

        let d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("test");
        let mut t2 = d2.transact_mut();

        txt2.insert(&mut t2, 0, "world");

        let d1_sv = t1.state_vector().encode_v1().unwrap();
        let d2_sv = t2.state_vector().encode_v1().unwrap();

        let u1 = t1
            .encode_diff_v1(&StateVector::decode_v1(&d2_sv).unwrap())
            .unwrap();
        let u2 = t2
            .encode_diff_v1(&StateVector::decode_v1(&d1_sv).unwrap())
            .unwrap();

        t1.apply_update(Update::decode_v1(u2.as_slice()).unwrap());
        t2.apply_update(Update::decode_v1(u1.as_slice()).unwrap());

        let a = txt1.get_string(&t1);
        let b = txt2.get_string(&t2);

        assert_eq!(a, b);
        assert_eq!(a.as_str(), "hello world");
    }

    #[test]
    fn insert_concurrent_in_the_middle() {
        let d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("test");
        let mut t1 = d1.transact_mut();

        txt1.insert(&mut t1, 0, "I expect that");
        assert_eq!(txt1.get_string(&t1).as_str(), "I expect that");

        let d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("test");
        let mut t2 = d2.transact_mut();

        let d2_sv = t2.state_vector().encode_v1().unwrap();
        let u1 = t1
            .encode_diff_v1(&StateVector::decode_v1(&d2_sv).unwrap())
            .unwrap();
        t2.apply_update(Update::decode_v1(u1.as_slice()).unwrap());

        assert_eq!(txt2.get_string(&t2).as_str(), "I expect that");

        txt2.insert(&mut t2, 1, " have");
        txt2.insert(&mut t2, 13, "ed");
        assert_eq!(txt2.get_string(&t2).as_str(), "I have expected that");

        txt1.insert(&mut t1, 1, " didn't");
        assert_eq!(txt1.get_string(&t1).as_str(), "I didn't expect that");

        let d2_sv = t2.state_vector().encode_v1().unwrap();
        let d1_sv = t1.state_vector().encode_v1().unwrap();
        let u1 = t1
            .encode_diff_v1(&StateVector::decode_v1(&d2_sv.as_slice()).unwrap())
            .unwrap();
        let u2 = t2
            .encode_diff_v1(&StateVector::decode_v1(&d1_sv.as_slice()).unwrap())
            .unwrap();
        t1.apply_update(Update::decode_v1(u2.as_slice()).unwrap());
        t2.apply_update(Update::decode_v1(u1.as_slice()).unwrap());

        let a = txt1.get_string(&t1);
        let b = txt2.get_string(&t2);

        assert_eq!(a, b);
        assert_eq!(a.as_str(), "I didn't have expected that");
    }

    #[test]
    fn append_concurrent() {
        let d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("test");
        let mut t1 = d1.transact_mut();

        txt1.insert(&mut t1, 0, "aaa");
        assert_eq!(txt1.get_string(&t1).as_str(), "aaa");

        let d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("test");
        let mut t2 = d2.transact_mut();

        let d2_sv = t2.state_vector().encode_v1().unwrap();
        let u1 = t1
            .encode_diff_v1(&StateVector::decode_v1(&d2_sv.as_slice()).unwrap())
            .unwrap();
        t2.apply_update(Update::decode_v1(u1.as_slice()).unwrap());

        assert_eq!(txt2.get_string(&t2).as_str(), "aaa");

        txt2.insert(&mut t2, 3, "bbb");
        txt2.insert(&mut t2, 6, "bbb");
        assert_eq!(txt2.get_string(&t2).as_str(), "aaabbbbbb");

        txt1.insert(&mut t1, 3, "aaa");
        assert_eq!(txt1.get_string(&t1).as_str(), "aaaaaa");

        let d2_sv = t2.state_vector().encode_v1().unwrap();
        let d1_sv = t1.state_vector().encode_v1().unwrap();
        let u1 = t1
            .encode_diff_v1(&StateVector::decode_v1(&d2_sv.as_slice()).unwrap())
            .unwrap();
        let u2 = t2
            .encode_diff_v1(&StateVector::decode_v1(&d1_sv.as_slice()).unwrap())
            .unwrap();

        t1.apply_update(Update::decode_v1(u2.as_slice()).unwrap());
        t2.apply_update(Update::decode_v1(u1.as_slice()).unwrap());

        let a = txt1.get_string(&t1);
        let b = txt2.get_string(&t2);

        assert_eq!(a.as_str(), "aaaaaabbbbbb");
        assert_eq!(a, b);
    }

    #[test]
    fn delete_single_block_start() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "bbb");
        txt.insert(&mut txn, 0, "aaa");
        txt.remove_range(&mut txn, 0, 3);

        assert_eq!(txt.len(&txn), 3);
        assert_eq!(txt.get_string(&txn).as_str(), "bbb");
    }

    #[test]
    fn delete_single_block_end() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "bbb");
        txt.insert(&mut txn, 0, "aaa");
        txt.remove_range(&mut txn, 3, 3);

        assert_eq!(txt.get_string(&txn).as_str(), "aaa");
    }

    #[test]
    fn delete_multiple_whole_blocks() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "a");
        txt.insert(&mut txn, 1, "b");
        txt.insert(&mut txn, 2, "c");

        txt.remove_range(&mut txn, 1, 1);
        assert_eq!(txt.get_string(&txn).as_str(), "ac");

        txt.remove_range(&mut txn, 1, 1);
        assert_eq!(txt.get_string(&txn).as_str(), "a");

        txt.remove_range(&mut txn, 0, 1);
        assert_eq!(txt.get_string(&txn).as_str(), "");
    }

    #[test]
    fn delete_slice_of_block() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "abc");
        txt.remove_range(&mut txn, 1, 1);

        assert_eq!(txt.get_string(&txn).as_str(), "ac");
    }

    #[test]
    fn delete_multiple_blocks_with_slicing() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "hello ");
        txt.insert(&mut txn, 6, "beautiful");
        txt.insert(&mut txn, 15, " world");

        txt.remove_range(&mut txn, 5, 11);
        assert_eq!(txt.get_string(&txn).as_str(), "helloworld");
    }

    #[test]
    fn insert_after_delete() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "hello ");
        txt.remove_range(&mut txn, 0, 5);
        txt.insert(&mut txn, 1, "world");

        assert_eq!(txt.get_string(&txn).as_str(), " world");
    }

    #[test]
    fn concurrent_insert_delete() {
        let d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("test");
        let mut t1 = d1.transact_mut();

        txt1.insert(&mut t1, 0, "hello world");
        assert_eq!(txt1.get_string(&t1).as_str(), "hello world");

        let u1 = t1
            .encode_state_as_update_v1(&StateVector::default())
            .unwrap();

        let d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("test");
        let mut t2 = d2.transact_mut();
        t2.apply_update(Update::decode_v1(u1.as_slice()).unwrap());
        assert_eq!(txt2.get_string(&t2).as_str(), "hello world");

        txt1.insert(&mut t1, 5, " beautiful");
        txt1.insert(&mut t1, 21, "!");
        txt1.remove_range(&mut t1, 0, 5);
        assert_eq!(txt1.get_string(&t1).as_str(), " beautiful world!");

        txt2.remove_range(&mut t2, 5, 5);
        txt2.remove_range(&mut t2, 0, 1);
        txt2.insert(&mut t2, 0, "H");
        assert_eq!(txt2.get_string(&t2).as_str(), "Hellod");

        let sv1 = t1.state_vector().encode_v1().unwrap();
        let sv2 = t2.state_vector().encode_v1().unwrap();
        let u1 = t1
            .encode_diff_v1(&StateVector::decode_v1(&sv2.as_slice()).unwrap())
            .unwrap();
        let u2 = t2
            .encode_diff_v1(&StateVector::decode_v1(&sv1.as_slice()).unwrap())
            .unwrap();

        t1.apply_update(Update::decode_v1(u2.as_slice()).unwrap());
        t2.apply_update(Update::decode_v1(u1.as_slice()).unwrap());

        let a = txt1.get_string(&t1);
        let b = txt2.get_string(&t2);

        assert_eq!(a, b);
        assert_eq!(a, "H beautifuld!".to_owned());
    }

    #[test]
    fn observer() {
        let doc = Doc::with_client_id(1);
        let mut txt: XmlTextRef = doc.get_or_insert_text("text").into();
        let delta = Rc::new(RefCell::new(None));
        let delta_c = delta.clone();
        let sub = txt.observe(move |txn, e| {
            *delta_c.borrow_mut() = Some(e.delta(txn).to_vec());
        });

        // insert initial data to an empty YText
        txt.insert(&mut doc.transact_mut(), 0, "abcd"); // => 'abcd'
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![Delta::Inserted("abcd".into(), None)])
        );

        // remove 2 chars from the middle
        txt.remove_range(&mut doc.transact_mut(), 1, 2); // => 'ad'
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![Delta::Retain(1, None), Delta::Deleted(2)])
        );

        // insert new item in the middle
        let attrs = Attrs::from([("bold".into(), true.into())]);
        txt.insert_with_attributes(&mut doc.transact_mut(), 1, "e", attrs.clone()); // => 'a<bold>e</bold>d'
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![
                Delta::Retain(1, None),
                Delta::Inserted("e".into(), Some(Box::new(attrs)))
            ])
        );

        // remove formatting
        let attrs = Attrs::from([("bold".into(), Any::Null)]);
        txt.format(&mut doc.transact_mut(), 1, 1, attrs.clone()); // => 'aed'
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![
                Delta::Retain(1, None),
                Delta::Retain(2, Some(Box::new(attrs)))
            ])
        );

        // free the observer and make sure that callback is no longer called
        drop(sub);
        txt.insert(&mut doc.transact_mut(), 1, "fgh"); // => 'afghed'
        assert_eq!(delta.borrow_mut().take(), None);
    }

    #[test]
    fn insert_and_remove_event_changes() {
        let d1 = Doc::with_client_id(1);
        let mut txt = d1.get_or_insert_text("text");
        let delta = Rc::new(RefCell::new(None));
        let delta_c = delta.clone();
        let _sub = txt.observe(move |txn, e| {
            *delta_c.borrow_mut() = Some(e.delta(txn).to_vec());
        });

        // insert initial string
        {
            let mut txn = d1.transact_mut();
            txt.insert(&mut txn, 0, "abcd");
        }
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![Delta::Inserted("abcd".into(), None)])
        );

        // remove middle
        {
            let mut txn = d1.transact_mut();
            txt.remove_range(&mut txn, 1, 2);
        }
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![Delta::Retain(1, None), Delta::Deleted(2)])
        );

        // insert again
        {
            let mut txn = d1.transact_mut();
            txt.insert(&mut txn, 1, "ef");
        }
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![
                Delta::Retain(1, None),
                Delta::Inserted("ef".into(), None)
            ])
        );

        // replicate data to another peer
        let d2 = Doc::with_client_id(2);
        let mut txt = d2.get_or_insert_text("text");
        let delta_c = delta.clone();
        let _sub = txt.observe(move |txn, e| {
            *delta_c.borrow_mut() = Some(e.delta(txn).to_vec());
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
            delta.borrow_mut().take(),
            Some(vec![Delta::Inserted("aefd".into(), None)])
        );
    }

    #[test]
    fn utf32_encoding() {
        let mut options = Options::with_client_id(1);
        options.offset_kind = OffsetKind::Utf32;
        let doc = Doc::with_options(options);
        let txt = doc.get_or_insert_text("content");

        txt.insert(&mut doc.transact_mut(), 0, r#"“”"#); // these chars are 3B long each
        txt.insert(&mut doc.transact_mut(), 1, r#"test"#);

        assert_eq!(txt.get_string(&txt.transact()), r#"“test”"#);
    }

    #[test]
    fn unicode_support() {
        let d1 = {
            let mut options = Options::with_client_id(1);
            options.offset_kind = OffsetKind::Utf32;
            Doc::with_options(options)
        };
        let txt1 = d1.get_or_insert_text("test");

        let d2 = {
            let mut options = Options::with_client_id(2);
            options.offset_kind = OffsetKind::Bytes;
            Doc::with_options(options)
        };
        let txt2 = d2.get_or_insert_text("test");

        {
            let mut txn = d1.transact_mut();

            txt1.insert(&mut txn, 0, "Zażółć gęślą jaźń");
            assert_eq!(txt1.get_string(&txn), "Zażółć gęślą jaźń");
            assert_eq!(txt1.len(&txn), 17);
        }

        exchange_updates(&[&d1, &d2]);

        {
            let txn = txt2.transact();
            assert_eq!(txt2.get_string(&txn), "Zażółć gęślą jaźń");
            assert_eq!(txt2.len(&txn), 26);
        }

        {
            let mut txn = d1.transact_mut();
            txt1.remove_range(&mut txn, 9, 3);
            txt1.insert(&mut txn, 9, "si");

            assert_eq!(txt1.get_string(&txn), "Zażółć gęsi jaźń");
            assert_eq!(txt1.len(&txn), 16);
        }

        exchange_updates(&[&d1, &d2]);

        {
            let txn = txt2.transact();
            assert_eq!(txt2.get_string(&txn), "Zażółć gęsi jaźń");
            assert_eq!(txt2.len(&txn), 23);
        }
    }

    fn text_transactions() -> [Box<dyn Fn(&mut Doc, &mut StdRng)>; 2] {
        fn insert_text(doc: &mut Doc, rng: &mut StdRng) {
            let ytext = doc.get_or_insert_text("text");
            let mut txn = doc.transact_mut();
            let pos = rng.between(0, ytext.len(&txn));
            let word = rng.random_string();
            ytext.insert(&mut txn, pos, word.as_str());
        }

        fn delete_text(doc: &mut Doc, rng: &mut StdRng) {
            let ytext = doc.get_or_insert_text("text");
            let mut txn = doc.transact_mut();
            let len = ytext.len(&txn);
            if len > 0 {
                let pos = rng.between(0, len - 1);
                let to_delete = rng.between(2, len - pos);
                ytext.remove_range(&mut txn, pos, to_delete);
            }
        }

        [Box::new(insert_text), Box::new(delete_text)]
    }

    fn fuzzy(iterations: usize) {
        run_scenario(0, &text_transactions(), 5, iterations)
    }

    #[test]
    fn fuzzy_test_3() {
        fuzzy(3)
    }

    #[test]
    fn basic_format() {
        let d1 = Doc::with_client_id(1);
        let mut txt1 = d1.get_or_insert_text("text");

        let delta1 = Rc::new(RefCell::new(None));
        let delta_clone = delta1.clone();
        let _sub1 = txt1.observe(move |txn, e| {
            delta_clone.replace(Some(e.delta(txn).to_vec()));
        });

        let d2 = Doc::with_client_id(2);
        let mut txt2 = d2.get_or_insert_text("text");

        let delta2 = Rc::new(RefCell::new(None));
        let delta_clone = delta2.clone();
        let _sub2 = txt2.observe(move |txn, e| {
            delta_clone.replace(Some(e.delta(txn).to_vec()));
        });

        let a: Attrs = HashMap::from([("bold".into(), Any::Bool(true))]);

        // step 1
        {
            let mut txn = d1.transact_mut();
            txt1.insert_with_attributes(&mut txn, 0, "abc", a.clone());
            let update = txn.encode_update_v1().unwrap();
            drop(txn);

            let expected = Some(vec![Delta::Inserted(
                "abc".into(),
                Some(Box::new(a.clone())),
            )]);

            assert_eq!(txt1.get_string(&txt1.transact()), "abc".to_string());
            assert_eq!(
                txt1.diff(&txt1.transact(), YChange::identity),
                vec![Diff::new("abc".into(), Some(Box::new(a.clone())))]
            );
            assert_eq!(delta1.take(), expected);

            let mut txn = d2.transact_mut();
            txn.apply_update(Update::decode_v1(update.as_slice()).unwrap());
            drop(txn);

            assert_eq!(txt2.get_string(&txt2.transact()), "abc".to_string());
            assert_eq!(delta2.take(), expected);
        }

        // step 2
        {
            let mut txn = d1.transact_mut();
            txt1.remove_range(&mut txn, 0, 1);
            let update = txn.encode_update_v1().unwrap();
            drop(txn);

            let expected = Some(vec![Delta::Deleted(1)]);

            assert_eq!(txt1.get_string(&txt1.transact()), "bc".to_string());
            assert_eq!(
                txt1.diff(&txt1.transact(), YChange::identity),
                vec![Diff::new("bc".into(), Some(Box::new(a.clone())))]
            );
            assert_eq!(delta1.take(), expected);

            let mut txn = d2.transact_mut();
            txn.apply_update(Update::decode_v1(update.as_slice()).unwrap());
            drop(txn);

            assert_eq!(txt2.get_string(&txt2.transact()), "bc".to_string());
            assert_eq!(delta2.take(), expected);
        }

        // step 3
        {
            let mut txn = d1.transact_mut();
            txt1.remove_range(&mut txn, 1, 1);
            let update = txn.encode_update_v1().unwrap();
            drop(txn);

            let expected = Some(vec![Delta::Retain(1, None), Delta::Deleted(1)]);

            assert_eq!(txt1.get_string(&txt1.transact()), "b".to_string());
            assert_eq!(
                txt1.diff(&txt1.transact(), YChange::identity),
                vec![Diff::new("b".into(), Some(Box::new(a.clone())))]
            );
            assert_eq!(delta1.take(), expected);

            let mut txn = d2.transact_mut();
            txn.apply_update(Update::decode_v1(update.as_slice()).unwrap());
            drop(txn);

            assert_eq!(txt2.get_string(&txt2.transact()), "b".to_string());
            assert_eq!(delta2.take(), expected);
        }

        // step 4
        {
            let mut txn = d1.transact_mut();
            txt1.insert_with_attributes(&mut txn, 0, "z", a.clone());
            let update = txn.encode_update_v1().unwrap();
            drop(txn);

            let expected = Some(vec![Delta::Inserted("z".into(), Some(Box::new(a.clone())))]);

            assert_eq!(txt1.get_string(&txt1.transact()), "zb".to_string());
            assert_eq!(
                txt1.diff(&mut txt1.transact_mut(), YChange::identity),
                vec![Diff::new("zb".into(), Some(Box::new(a.clone())))]
            );
            assert_eq!(delta1.take(), expected);

            let mut txn = d2.transact_mut();
            txn.apply_update(Update::decode_v1(update.as_slice()).unwrap());
            drop(txn);

            assert_eq!(txt2.get_string(&txt2.transact()), "zb".to_string());
            assert_eq!(delta2.take(), expected);
        }

        // step 5
        {
            let mut txn = d1.transact_mut();
            txt1.insert(&mut txn, 0, "y");
            let update = txn.encode_update_v1().unwrap();
            drop(txn);

            let expected = Some(vec![Delta::Inserted("y".into(), None)]);

            assert_eq!(txt1.get_string(&txt1.transact()), "yzb".to_string());
            assert_eq!(
                txt1.diff(&mut txt1.transact_mut(), YChange::identity),
                vec![
                    Diff::new("y".into(), None),
                    Diff::new("zb".into(), Some(Box::new(a.clone())))
                ]
            );
            assert_eq!(delta1.take(), expected);

            let mut txn = d2.transact_mut();
            txn.apply_update(Update::decode_v1(update.as_slice()).unwrap());
            drop(txn);

            assert_eq!(txt2.get_string(&txt2.transact()), "yzb".to_string());
            assert_eq!(delta2.take(), expected);
        }

        // step 6
        {
            let mut txn = d1.transact_mut();
            let b: Attrs = HashMap::from([("bold".into(), Any::Null)]);
            txt1.format(&mut txn, 0, 2, b.clone());
            let update = txn.encode_update_v1().unwrap();
            drop(txn);

            let expected = Some(vec![
                Delta::Retain(1, None),
                Delta::Retain(1, Some(Box::new(b))),
            ]);

            assert_eq!(txt1.get_string(&txt1.transact()), "yzb".to_string());
            assert_eq!(
                txt1.diff(&mut txt1.transact_mut(), YChange::identity),
                vec![
                    Diff::new("yz".into(), None),
                    Diff::new("b".into(), Some(Box::new(a.clone())))
                ]
            );
            assert_eq!(delta1.take(), expected);

            let mut txn = d2.transact_mut();
            txn.apply_update(Update::decode_v1(update.as_slice()).unwrap());
            drop(txn);

            assert_eq!(txt2.get_string(&txt2.transact()), "yzb".to_string());
            assert_eq!(delta2.take(), expected);
        }
    }

    #[test]
    fn embed_with_attributes() {
        let d1 = Doc::with_client_id(1);
        let mut txt1 = d1.get_or_insert_text("text");

        let delta1 = Rc::new(RefCell::new(None));
        let delta_clone = delta1.clone();
        let _sub1 = txt1.observe(move |txn, e| {
            let delta = e.delta(txn).to_vec();
            delta_clone.replace(Some(delta));
        });

        let a1: Attrs = HashMap::from([("bold".into(), true.into())]);
        let embed = any!({
            "image": "imageSrc.png"
        });

        let (update_v1, update_v2) = {
            let mut txn = d1.transact_mut();
            txt1.insert_with_attributes(&mut txn, 0, "ab", a1.clone());

            let a2: Attrs = HashMap::from([("width".into(), Any::Number(100.0))]);

            txt1.insert_embed_with_attributes(&mut txn, 1, embed.clone(), a2.clone());
            drop(txn);

            let a1 = Some(Box::new(a1.clone()));
            let a2 = Some(Box::new(a2.clone()));

            let expected = Some(vec![
                Delta::Inserted("a".into(), a1.clone()),
                Delta::Inserted(embed.clone().into(), a2.clone()),
                Delta::Inserted("b".into(), a1.clone()),
            ]);
            assert_eq!(delta1.take(), expected);

            let expected = vec![
                Diff::new("a".into(), a1.clone()),
                Diff::new(embed.clone().into(), a2),
                Diff::new("b".into(), a1.clone()),
            ];
            let mut txn = d1.transact_mut();
            assert_eq!(txt1.diff(&mut txn, YChange::identity), expected);

            let update_v1 = txn
                .encode_state_as_update_v1(&StateVector::default())
                .unwrap();
            let update_v2 = txn
                .encode_state_as_update_v2(&StateVector::default())
                .unwrap();
            (update_v1, update_v2)
        };

        let a1 = Some(Box::new(a1));
        let a2 = Some(Box::new(HashMap::from([(
            "width".into(),
            Any::Number(100.0),
        )])));

        let expected = vec![
            Diff::new("a".into(), a1.clone()),
            Diff::new(embed.into(), a2),
            Diff::new("b".into(), a1.clone()),
        ];

        let d2 = Doc::new();
        let txt2 = d2.get_or_insert_text("text");
        {
            let txn = &mut d2.transact_mut();
            txn.apply_update(Update::decode_v1(&update_v1).unwrap());
            assert_eq!(txt2.diff(txn, YChange::identity), expected);
        }

        let d3 = Doc::new();
        let txt3 = d3.get_or_insert_text("text");
        {
            let txn = &mut d3.transact_mut();
            txn.apply_update(Update::decode_v2(&update_v2).unwrap());
            let actual = txt3.diff(txn, YChange::identity);
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn issue_101() {
        let d1 = Doc::with_client_id(1);
        let mut txt1 = d1.get_or_insert_text("text");
        let delta = Rc::new(RefCell::new(None));
        let delta_copy = delta.clone();

        let attrs: Attrs = HashMap::from([("bold".into(), true.into())]);

        txt1.insert(&mut d1.transact_mut(), 0, "abcd");

        let _sub = txt1.observe(move |txn, e| {
            let mut d = delta_copy.borrow_mut();
            *d = Some(e.delta(txn).to_vec());
        });
        txt1.format(&mut d1.transact_mut(), 1, 2, attrs.clone());

        let expected = vec![
            Delta::Retain(1, None),
            Delta::Retain(2, Some(Box::new(attrs))),
        ];
        let actual = delta.borrow();
        assert_eq!(actual.as_ref(), Some(&expected));
    }

    #[test]
    fn yrs_delete() {
        let doc = Doc::with_options(Options {
            offset_kind: OffsetKind::Utf32,
            ..Default::default()
        });

        let text1 = r#"
		Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Eleifend mi in nulla posuere sollicitudin. Lorem mollis aliquam ut porttitor. Enim ut sem viverra aliquet eget sit amet. Sed turpis tincidunt id aliquet risus feugiat in ante metus. Accumsan lacus vel facilisis volutpat. Non consectetur a erat nam at lectus urna. Enim diam vulputate ut pharetra sit amet. In dictum non consectetur a erat. Bibendum at varius vel pharetra vel turpis nunc eget lorem. Blandit cursus risus at ultrices. Sed lectus vestibulum mattis ullamcorper velit sed ullamcorper. Sagittis nisl rhoncus mattis rhoncus.

		Sed vulputate odio ut enim. Erat pellentesque adipiscing commodo elit at imperdiet dui. Ultricies tristique nulla aliquet enim tortor at auctor urna nunc. Tincidunt eget nullam non nisi est sit amet. Sed adipiscing diam donec adipiscing tristique risus nec. Risus commodo viverra maecenas accumsan lacus vel facilisis volutpat est. Donec enim diam vulputate ut pharetra sit amet aliquam id. Netus et malesuada fames ac turpis egestas sed tempus urna. Augue mauris augue neque gravida. Tellus orci ac auctor augue mauris augue. Ante metus dictum at tempor. Feugiat in ante metus dictum at. Vitae elementum curabitur vitae nunc sed velit dignissim. Non arcu risus quis varius quam quisque id diam vel. Fermentum leo vel orci porta non. Donec adipiscing tristique risus nec feugiat in fermentum posuere. Duis convallis convallis tellus id interdum velit laoreet id. Vel eros donec ac odio tempor orci dapibus ultrices in. At varius vel pharetra vel turpis nunc eget lorem. Blandit aliquam etiam erat velit scelerisque in.
		"#;

        let text2 = r#"test"#;

        {
            let text = doc.get_or_insert_text("content");
            let mut txn = doc.transact_mut();
            text.insert(&mut txn, 0, text1);
            txn.commit();
        }

        {
            let text = doc.get_or_insert_text("content");
            let mut txn = doc.transact_mut();
            text.insert(&mut txn, 100, text2);
            txn.commit();
        }

        {
            let mut text = doc.get_or_insert_text("content");
            let mut txn = doc.transact_mut();

            let c1 = text1.chars().count();
            let c2 = text2.chars().count();
            let count = c1 as u32 + c2 as u32;

            let _observer = text
                .observe(move |txn, edit| assert_eq!(edit.delta(txn)[0], Delta::Deleted(count)));

            text.remove_range(&mut txn, 0, count);
            txn.commit();
        }

        {
            let text = doc.get_or_insert_text("content");
            assert_eq!(text.get_string(&text.transact()), "");
        }
    }

    #[test]
    fn text_diff_adjacent() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("text");
        let mut txn = doc.transact_mut();
        let attrs1 = Attrs::from([("a".into(), "a".into())]);
        txt.insert_with_attributes(&mut txn, 0, "abc", attrs1.clone());
        let attrs2 = Attrs::from([("a".into(), "a".into()), ("b".into(), "b".into())]);
        txt.insert_with_attributes(&mut txn, 3, "def", attrs2.clone());

        let diff = txt.diff(&mut txn, YChange::identity);
        let expected = vec![
            Diff::new("abc".into(), Some(Box::new(attrs1))),
            Diff::new("def".into(), Some(Box::new(attrs2))),
        ];
        assert_eq!(diff, expected);
    }

    #[test]
    fn text_remove_4_byte_range() {
        let d1 = Doc::new();
        let txt = d1.get_or_insert_text("test");

        txt.insert(&mut d1.transact_mut(), 0, "😭😊");

        let d2 = Doc::new();
        exchange_updates(&[&d1, &d2]);

        txt.remove_range(&mut d1.transact_mut(), 0, "😭".len() as u32);
        assert_eq!(txt.get_string(&txt.transact()).as_str(), "😊");

        exchange_updates(&[&d1, &d2]);
        let txt = d2.get_or_insert_text("test");
        assert_eq!(txt.get_string(&txt.transact()).as_str(), "😊");
    }

    #[test]
    fn text_remove_3_byte_range() {
        let d1 = Doc::new();
        let txt = d1.get_or_insert_text("test");

        txt.insert(&mut d1.transact_mut(), 0, "⏰⏳");

        let d2 = Doc::new();
        exchange_updates(&[&d1, &d2]);

        txt.remove_range(&mut d1.transact_mut(), 0, "⏰".len() as u32);
        assert_eq!(txt.get_string(&txt.transact()).as_str(), "⏳");

        exchange_updates(&[&d1, &d2]);
        let txt = d2.get_or_insert_text("test");
        assert_eq!(txt.get_string(&txt.transact()).as_str(), "⏳");
    }
    #[test]
    fn delete_4_byte_character_from_middle() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "😊😭");
        // uncomment the following line will pass the test
        // txt.format(&mut txn, 0, "😊".len() as u32, HashMap::new());
        txt.remove_range(&mut txn, "😊".len() as u32, "😭".len() as u32);

        assert_eq!(txt.get_string(&txn).as_str(), "😊");
    }

    #[test]
    fn delete_3_byte_character_from_middle_1() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "⏰⏳");
        // uncomment the following line will pass the test
        // txt.format(&mut txn, 0, "⏰".len() as u32, HashMap::new());
        txt.remove_range(&mut txn, "⏰".len() as u32, "⏳".len() as u32);

        assert_eq!(txt.get_string(&txn).as_str(), "⏰");
    }

    #[test]
    fn delete_3_byte_character_from_middle_2() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "👯🙇‍♀️🙇‍♀️⏰👩‍❤️‍💋‍👨");

        txt.format(
            &mut txn,
            "👯".len() as u32,
            "🙇‍♀️🙇‍♀️".len() as u32,
            HashMap::new(),
        );
        txt.remove_range(&mut txn, "👯🙇‍♀️🙇‍♀️".len() as u32, "⏰".len() as u32); // will delete ⏰ and 👩‍❤️‍💋‍👨

        assert_eq!(txt.get_string(&txn).as_str(), "👯🙇‍♀️🙇‍♀️👩‍❤️‍💋‍👨");
    }

    #[test]
    fn delete_3_byte_character_from_middle_after_insert_and_format() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "🙇‍♀️🙇‍♀️⏰👩‍❤️‍💋‍👨");
        txt.insert(&mut txn, 0, "👯");
        txt.format(
            &mut txn,
            "👯".len() as u32,
            "🙇‍♀️🙇‍♀️".len() as u32,
            HashMap::new(),
        );

        // will delete ⏰ and 👩‍❤️‍💋‍👨
        txt.remove_range(&mut txn, "👯🙇‍♀️🙇‍♀️".len() as u32, "⏰".len() as u32); // will delete ⏰ and 👩‍❤️‍💋‍👨

        assert_eq!(&txt.get_string(&txn), "👯🙇‍♀️🙇‍♀️👩‍❤️‍💋‍👨");
    }

    #[test]
    fn delete_multi_byte_character_from_middle_after_insert_and_format() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        txt.insert(&mut txn, 0, "❤️❤️🙇‍♀️🙇‍♀️⏰👩‍❤️‍💋‍👨👩‍❤️‍💋‍👨");
        txt.insert(&mut txn, 0, "👯");
        txt.format(
            &mut txn,
            "👯".len() as u32,
            "❤️❤️🙇‍♀️🙇‍♀️⏰".len() as u32,
            HashMap::new(),
        );
        txt.insert(&mut txn, "👯❤️❤️🙇‍♀️🙇‍♀️⏰".len() as u32, "⏰");
        txt.format(
            &mut txn,
            "👯❤️❤️🙇‍♀️🙇‍♀️⏰⏰".len() as u32,
            "👩‍❤️‍💋‍👨".len() as u32,
            HashMap::new(),
        );
        txt.remove_range(
            &mut txn,
            "👯❤️❤️🙇‍♀️🙇‍♀️⏰⏰👩‍❤️‍💋‍👩".len() as u32,
            "👩‍❤️‍💋‍👨".len() as u32,
        );
        assert_eq!(txt.get_string(&txn).as_str(), "👯❤️❤️🙇‍♀️🙇‍♀️⏰⏰👩‍❤️‍💋‍👨");
    }

    #[test]
    fn insert_string_with_no_attribute() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();

        let attrs = Attrs::from([("a".into(), "a".into())]);
        txt.insert_with_attributes(&mut txn, 0, "ac", attrs.clone());
        txt.insert_with_attributes(&mut txn, 1, "b", Attrs::new());

        let expect = vec![
            Diff::new("a".into(), Some(Box::new(attrs.clone()))),
            Diff::new("b".into(), None),
            Diff::new("c".into(), Some(Box::new(attrs.clone()))),
        ];

        assert!(txt.diff(&mut txn, YChange::identity).eq(&expect))
    }

    #[test]
    fn snapshots() {
        let doc = Doc::with_client_id(1);
        let text = doc.get_or_insert_text("text");
        text.insert(&mut doc.transact_mut(), 0, "hello");
        let prev = doc.transact_mut().snapshot();
        text.insert(&mut doc.transact_mut(), 5, " world");
        let next = doc.transact_mut().snapshot();
        let diff = text.diff_range(
            &mut doc.transact_mut(),
            Some(&next),
            Some(&prev),
            YChange::identity,
        );

        assert_eq!(
            diff,
            vec![
                Diff::new("hello".into(), None),
                Diff::with_change(
                    " world".into(),
                    None,
                    Some(YChange::new(ChangeKind::Added, ID::new(1, 5)))
                )
            ]
        )
    }

    #[test]
    fn diff_with_embedded_items() {
        let doc = Doc::new();
        let text = doc.get_or_insert_text("article");
        let mut txn = doc.transact_mut();

        let bold = Attrs::from([("b".into(), true.into())]);
        let italic = Attrs::from([("i".into(), true.into())]);

        text.insert_with_attributes(&mut txn, 0, "hello world", italic.clone()); // "<i>hello world</i>"
        text.format(&mut txn, 6, 5, bold.clone()); // "<i>hello <b>world</b></i>"
        let image = vec![0, 0, 0, 0];
        text.insert_embed(&mut txn, 5, image.clone()); // insert binary after "hello"
        let array = text.insert_embed(&mut txn, 5, ArrayPrelim::default()); // insert array ref after "hello"

        let italic_and_bold = Attrs::from([("b".into(), true.into()), ("i".into(), true.into())]);
        let chunks = text.diff(&txn, YChange::identity);
        assert_eq!(
            chunks,
            vec![
                Diff::new("hello".into(), Some(Box::new(italic.clone()))),
                Diff::new(Value::YArray(array), Some(Box::new(italic.clone()))),
                Diff::new(image.into(), Some(Box::new(italic.clone()))),
                Diff::new(" ".into(), Some(Box::new(italic))),
                Diff::new("world".into(), Some(Box::new(italic_and_bold))),
            ]
        );
    }

    #[test]
    fn multi_threading() {
        use rand::thread_rng;
        use std::sync::{Arc, RwLock};
        use std::thread::{sleep, spawn};

        let doc = Arc::new(RwLock::new(Doc::with_client_id(1)));

        let d2 = doc.clone();
        let h2 = spawn(move || {
            for _ in 0..10 {
                let millis = thread_rng().gen_range(1, 20);
                sleep(Duration::from_millis(millis));

                let doc = d2.write().unwrap();
                let txt = doc.get_or_insert_text("test");
                let mut txn = doc.transact_mut();
                txt.push(&mut txn, "a");
            }
        });

        let d3 = doc.clone();
        let h3 = spawn(move || {
            for _ in 0..10 {
                let millis = thread_rng().gen_range(1, 20);
                sleep(Duration::from_millis(millis));

                let doc = d3.write().unwrap();
                let txt = doc.get_or_insert_text("test");
                let mut txn = txt.transact_mut();
                txt.push(&mut txn, "b");
            }
        });

        h3.join().unwrap();
        h2.join().unwrap();

        let doc = doc.read().unwrap();
        let txt = doc.get_or_insert_text("test");
        let len = txt.len(&doc.transact());
        assert_eq!(len, 20);
    }
}
