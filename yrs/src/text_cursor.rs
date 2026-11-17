//! A remembered insertion point in a [Text] or [XmlText].
//!
//! [Text::insert] takes an integer and walks to that character. A cursor keeps the
//! gap, so the next insert does not walk the characters in front of it. An edit
//! made somewhere else leaves the gap on the same character.
//!
//! ```
//! use yrs::{Doc, GetString, Text, Transact};
//!
//! let doc = Doc::new();
//! let text = doc.get_or_insert_text("t");
//! let mut cursor = {
//!     let mut txn = doc.transact_mut();
//!     text.insert(&mut txn, 0, "hello");
//!     text.cursor(&mut txn, 5)
//! };
//! {
//!     let mut txn = doc.transact_mut();
//!     cursor.insert(&mut txn, "!");
//! }
//! assert_eq!(text.get_string(&doc.transact()), "hello!");
//! ```

use crate::block::{ItemContent, ItemPosition, ItemPtr, PrelimString};
use crate::branch::BranchPtr;
use crate::doc::OffsetKind;
use crate::types::text::{chunk_len, find_position, insert, remove, update_current_attributes};
use crate::types::{Attrs, TypePtr};
use crate::{TransactionMut, ID};

/// A place to insert in a text, remembered across transactions.
///
/// Create one with [Text::cursor]. The cursor must not outlive the document that
/// owns the text.
pub struct TextCursor {
    branch: BranchPtr,
    /// The character this cursor sits after. `None` is the start of the text.
    anchor: Option<ID>,
    /// [Store::commit_gen] when `left` and `right` were last resolved.
    commit_gen: u64,
    /// [Branch::edit_epoch] when `attrs` and `index` were last known to be right.
    edit_epoch: u64,
    left: Option<ItemPtr>,
    right: Option<ItemPtr>,
    attrs: Option<Box<Attrs>>,
    index: u32,
    /// `index` counts visible characters. It is stale after an edit this cursor
    /// did not make, until the next method that needs it.
    index_trusted: bool,
}

impl TextCursor {
    pub(crate) fn at(branch: BranchPtr, txn: &mut TransactionMut, index: u32) -> Self {
        let mut cursor = TextCursor {
            branch,
            anchor: None,
            commit_gen: 0,
            edit_epoch: 0,
            left: None,
            right: None,
            attrs: None,
            index: 0,
            index_trusted: false,
        };
        cursor.seek_index(txn, index);
        cursor
    }

    /// Move this cursor to `index`, counting visible characters from the start.
    ///
    /// Panics if `index` is greater than the length of the text.
    pub fn seek(&mut self, txn: &mut TransactionMut, index: u32) {
        self.seek_index(txn, index);
    }

    /// Visible index of the gap, counting the same way as [Text::len].
    ///
    /// After another edit, this walks once to recount. Inserts made through this
    /// cursor update the number in place.
    pub fn index(&mut self, txn: &mut TransactionMut) -> u32 {
        self.ensure(txn);
        if !self.index_trusted {
            self.refresh_index(txn);
        }
        self.index
    }

    /// Move by `delta` visible characters. Positive moves toward the end.
    ///
    /// The walk covers `delta` characters, not the whole text. Moving past either
    /// end stops there.
    pub fn advance(&mut self, txn: &mut TransactionMut, delta: i32) {
        self.ensure(txn);
        if delta == 0 {
            return;
        }
        let kind = txn.store().offset_kind;
        let (consumed, forward) = if delta > 0 {
            (self.advance_forward(txn, delta as u32, kind), true)
        } else {
            let steps = (delta as i64).unsigned_abs() as u32;
            (self.advance_backward(txn, steps, kind), false)
        };
        self.skip_deleted();
        self.anchor = self.left.map(|item| item.last_id());
        if self.branch.has_formatting {
            self.refresh_index(txn);
        } else if self.index_trusted {
            if forward {
                self.index = self.index.saturating_add(consumed);
            } else {
                self.index = self.index.saturating_sub(consumed);
            }
        }
    }

    /// Insert `chunk` at the gap and leave the cursor after it.
    pub fn insert(&mut self, txn: &mut TransactionMut, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        self.prepare_edit(txn);
        let kind = txn.store().offset_kind;
        let added = chunk_len(chunk, kind);
        self.branch.bump_edit_epoch();
        self.skip_deleted();
        let pos = self.position();
        if let Some(item) = txn.create_item(&pos, PrelimString(chunk.into()), None) {
            self.left = Some(item);
            self.right = item.right;
            self.anchor = Some(item.last_id());
            if self.index_trusted {
                self.index = self.index.saturating_add(added as u32);
            }
        }
        self.note_local(txn);
    }

    /// Insert `chunk` wrapped in `attributes`, and leave the cursor on that
    /// text, before the closing format marker.
    ///
    /// The next call with the same map extends this span. Sitting after the
    /// closer would open a new one on every call. Another edit to this text
    /// drops the cached map.
    pub fn insert_with_attributes(
        &mut self,
        txn: &mut TransactionMut,
        chunk: &str,
        attributes: Attrs,
    ) {
        if chunk.is_empty() {
            return;
        }
        self.prepare_edit(txn);
        let kind = txn.store().offset_kind;
        let added = chunk_len(chunk, kind);
        self.branch.bump_edit_epoch();
        self.skip_deleted();
        let mut pos = self.position();
        if let Some(inserted) = insert(
            self.branch,
            txn,
            &mut pos,
            PrelimString(chunk.into()),
            attributes,
        ) {
            let item = inserted.item;
            self.left = Some(item);
            self.right = item.right;
            self.anchor = Some(item.last_id());
            self.attrs = inserted.attrs;
            if self.index_trusted {
                self.index = self.index.saturating_add(added as u32);
            }
        }
        self.note_local(txn);
    }

    /// Delete `len` visible characters to the right of the gap. The cursor stays
    /// where it is.
    ///
    /// Panics if fewer than `len` characters are left, same as [Text::remove_range].
    pub fn remove(&mut self, txn: &mut TransactionMut, len: u32) {
        if len == 0 {
            return;
        }
        self.prepare_edit(txn);
        if !self.index_trusted {
            self.refresh_index(txn);
        }
        let available = self.branch.content_len.saturating_sub(self.index);
        if self.index as u64 + len as u64 > self.branch.content_len as u64 {
            panic!(
                "Couldn't remove {} elements from an array. Only {} of them were successfully removed.",
                len, available
            );
        }
        self.branch.bump_edit_epoch();
        self.skip_deleted();
        let mut pos = self.position();
        remove(txn, &mut pos, len);
        self.left = pos.left;
        self.right = pos.right;
        self.anchor = pos.left.map(|item| item.last_id());
        if self.branch.has_formatting {
            self.attrs = pos.current_attrs;
        }
        self.note_local(txn);
    }

    fn seek_index(&mut self, txn: &mut TransactionMut, index: u32) {
        if index > self.branch.content_len {
            panic!("The type or the position doesn't exist!");
        }
        let pos = find_position(self.branch, txn, index).unwrap();
        self.left = pos.left;
        self.right = pos.right;
        self.attrs = pos.current_attrs;
        self.index = index;
        self.index_trusted = true;
        self.skip_deleted();
        self.anchor = self.left.map(|item| item.last_id());
        self.commit_gen = txn.store().commit_gen;
        self.edit_epoch = self.branch.edit_epoch;
    }

    fn prepare_edit(&mut self, txn: &mut TransactionMut) {
        self.ensure(txn);
    }

    fn ensure(&mut self, txn: &mut TransactionMut) {
        let gen = txn.store().commit_gen;
        let epoch = self.branch.edit_epoch;
        if self.commit_gen != gen || self.edit_epoch != epoch {
            self.resolve_anchor(txn);
            self.commit_gen = txn.store().commit_gen;
        }
        if self.edit_epoch != epoch {
            if self.branch.has_formatting {
                self.refresh_index(txn);
            } else {
                self.attrs = None;
                self.index_trusted = false;
            }
            self.edit_epoch = epoch;
        }
    }

    fn resolve_anchor(&mut self, txn: &mut TransactionMut) {
        let anchor = match self.anchor {
            Some(id) => id,
            None => {
                self.left = None;
                self.right = self.branch.start;
                self.skip_deleted();
                self.anchor = self.left.map(|item| item.last_id());
                return;
            }
        };
        let slice = match txn.store().follow_redone(&anchor) {
            Some(slice) => slice,
            None => {
                let index = self.index.min(self.branch.content_len);
                self.seek_index(txn, index);
                return;
            }
        };
        let item = slice.ptr;
        let offset = slice.start + 1;
        if !item.is_deleted() && offset < item.len() {
            let right = txn
                .store_mut()
                .blocks
                .split_block(item, offset, OffsetKind::Utf16);
            self.left = Some(item);
            self.right = right.or(item.right);
        } else {
            self.left = Some(item);
            self.right = item.right;
        }
        self.skip_deleted();
        self.anchor = self.left.map(|ptr| ptr.last_id());
    }

    fn refresh_index(&mut self, txn: &TransactionMut) {
        let kind = txn.store().offset_kind;
        let (index, attrs) = measure(self.branch, self.left, kind);
        self.index = index;
        if self.branch.has_formatting {
            self.attrs = attrs;
        }
        self.index_trusted = true;
    }

    fn note_local(&mut self, txn: &TransactionMut) {
        self.edit_epoch = self.branch.edit_epoch;
        self.commit_gen = txn.store().commit_gen;
    }

    fn position(&self) -> ItemPosition {
        ItemPosition {
            parent: TypePtr::Branch(self.branch),
            left: self.left,
            right: self.right,
            index: 0,
            current_attrs: self.attrs.clone(),
        }
    }

    fn skip_deleted(&mut self) {
        while let Some(right) = self.right {
            if !right.is_deleted() {
                break;
            }
            self.left = Some(right);
            self.right = right.right;
        }
    }

    fn advance_forward(
        &mut self,
        txn: &mut TransactionMut,
        mut remaining: u32,
        kind: OffsetKind,
    ) -> u32 {
        let start = remaining;
        while remaining > 0 {
            let right = match self.right {
                Some(right) => right,
                None => break,
            };
            if right.is_deleted() || !right.is_countable() {
                self.left = Some(right);
                self.right = right.right;
                continue;
            }
            let len = right.content_len(kind);
            if remaining < len {
                let offset = match &right.content {
                    ItemContent::String(s) => s.block_offset(remaining, kind),
                    _ => remaining,
                };
                let block_len = right.len();
                if offset > 0 && offset < block_len {
                    let split =
                        txn.store_mut()
                            .blocks
                            .split_block(right, offset, OffsetKind::Utf16);
                    self.left = Some(right);
                    self.right = split.or(right.right);
                }
                remaining = 0;
            } else {
                remaining -= len;
                self.left = Some(right);
                self.right = right.right;
            }
        }
        start - remaining
    }

    fn advance_backward(
        &mut self,
        txn: &mut TransactionMut,
        mut remaining: u32,
        kind: OffsetKind,
    ) -> u32 {
        let start = remaining;
        while remaining > 0 {
            let left = match self.left {
                Some(left) => left,
                None => break,
            };
            if left.is_deleted() || !left.is_countable() {
                self.right = Some(left);
                self.left = left.left;
                continue;
            }
            let len = left.content_len(kind);
            if remaining < len {
                let keep = len - remaining;
                let offset = match &left.content {
                    ItemContent::String(s) => s.block_offset(keep, kind),
                    _ => keep,
                };
                let block_len = left.len();
                if offset > 0 && offset < block_len {
                    let split = txn
                        .store_mut()
                        .blocks
                        .split_block(left, offset, OffsetKind::Utf16);
                    self.left = Some(left);
                    self.right = split.or(left.right);
                }
                remaining = 0;
            } else {
                remaining -= len;
                self.right = Some(left);
                self.left = left.left;
            }
        }
        start - remaining
    }
}

fn measure(
    branch: BranchPtr,
    left: Option<ItemPtr>,
    kind: OffsetKind,
) -> (u32, Option<Box<Attrs>>) {
    let left = match left {
        Some(left) => left,
        None => return (0, None),
    };
    let mut index = 0u32;
    let mut attrs = Attrs::new();
    let mut current = branch.start;
    while let Some(item) = current {
        if !item.is_deleted() {
            if item.is_countable() {
                index += item.content_len(kind);
            } else if let ItemContent::Format(key, value) = &item.content {
                update_current_attributes(&mut attrs, key, value.as_ref());
            }
        }
        if item == left {
            break;
        }
        current = item.right;
    }
    let boxed = if attrs.is_empty() {
        None
    } else {
        Some(Box::new(attrs))
    };
    (index, boxed)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_utils::exchange_updates;
    use crate::types::text::YChange;
    use crate::{
        Any, Doc, GetString, OffsetKind, Options, Out, Text, Transact, XmlFragment, XmlTextPrelim,
    };

    #[test]
    fn appends_across_transactions_without_losing_characters() {
        let doc = Doc::new();
        let text = doc.get_or_insert_text("t");
        let mut cursor = {
            let mut txn = doc.transact_mut();
            text.cursor(&mut txn, 0)
        };
        for _ in 0..30 {
            let mut txn = doc.transact_mut();
            cursor.insert(&mut txn, "a");
        }
        {
            let mut txn = doc.transact_mut();
            cursor.seek(&mut txn, 10);
            cursor.insert(&mut txn, "X");
        }
        let txn = doc.transact();
        assert_eq!(
            text.get_string(&txn),
            format!("{}X{}", "a".repeat(10), "a".repeat(20))
        );
    }

    #[test]
    fn inserts_in_the_middle() {
        let doc = Doc::new();
        let text = doc.get_or_insert_text("t");
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, &"a".repeat(100));
        let mut cursor = text.cursor(&mut txn, 50);
        cursor.insert(&mut txn, "X");
        assert_eq!(
            text.get_string(&txn),
            format!("{}X{}", "a".repeat(50), "a".repeat(50))
        );
    }

    #[test]
    fn remove_and_advance() {
        let doc = Doc::new();
        let text = doc.get_or_insert_text("t");
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "hello world");
        let mut cursor = text.cursor(&mut txn, 5);
        cursor.remove(&mut txn, 6);
        assert_eq!(text.get_string(&txn), "hello");
        cursor.advance(&mut txn, -5);
        cursor.insert(&mut txn, "X");
        assert_eq!(text.get_string(&txn), "Xhello");
        assert_eq!(cursor.index(&mut txn), 1);
    }

    #[test]
    fn remote_insert_elsewhere_keeps_the_same_character() {
        let local = Doc::with_client_id(1);
        let remote = Doc::with_client_id(2);
        let local_text = local.get_or_insert_text("t");
        let remote_text = remote.get_or_insert_text("t");
        {
            let mut txn = local.transact_mut();
            local_text.insert(&mut txn, 0, "hello");
        }
        exchange_updates(&[&local, &remote]);
        let mut cursor = {
            let mut txn = local.transact_mut();
            let end = local_text.len(&txn);
            local_text.cursor(&mut txn, end)
        };
        {
            let mut txn = remote.transact_mut();
            remote_text.insert(&mut txn, 0, "X");
        }
        exchange_updates(&[&local, &remote]);
        {
            let mut txn = local.transact_mut();
            assert_eq!(cursor.index(&mut txn), 6);
            cursor.insert(&mut txn, "!");
        }
        let txn = local.transact();
        assert_eq!(local_text.get_string(&txn), "Xhello!");
    }

    #[test]
    fn attributed_inserts_reuse_the_gap_across_transactions() {
        let bold = Attrs::from([("bold".into(), Any::Bool(true))]);
        let doc = Doc::new();
        let text = doc.get_or_insert_text("t");
        let mut cursor = {
            let mut txn = doc.transact_mut();
            text.cursor(&mut txn, 0)
        };
        {
            let mut txn = doc.transact_mut();
            cursor.insert_with_attributes(&mut txn, "ab", bold.clone());
        }
        {
            let mut txn = doc.transact_mut();
            cursor.insert_with_attributes(&mut txn, "cd", bold);
        }
        let txn = doc.transact();
        assert_eq!(text.get_string(&txn), "abcd");
        let mut combined = String::new();
        for chunk in text.diff(&txn, YChange::identity) {
            match chunk.insert {
                Out::Any(Any::String(s)) => combined.push_str(s.as_ref()),
                other => panic!("unexpected chunk {:?}", other),
            }
            let attrs = chunk.attributes.expect("bold formatting");
            assert_eq!(attrs.get("bold"), Some(&Any::Bool(true)));
        }
        assert_eq!(combined, "abcd");
    }

    #[test]
    fn utf16_offsets() {
        let doc = Doc::with_options(Options {
            offset_kind: OffsetKind::Utf16,
            ..Default::default()
        });
        let text = doc.get_or_insert_text("t");
        let mut cursor = {
            let mut txn = doc.transact_mut();
            text.insert(&mut txn, 0, "Hi ★ ");
            let end = text.len(&txn);
            text.cursor(&mut txn, end)
        };
        {
            let mut txn = doc.transact_mut();
            cursor.insert(&mut txn, "you");
        }
        let txn = doc.transact();
        assert_eq!(text.get_string(&txn), "Hi ★ you");
        assert_eq!(text.len(&txn), 8);
    }

    #[test]
    fn inserts_where_the_anchored_character_was_deleted() {
        let doc = Doc::new();
        let text = doc.get_or_insert_text("t");
        let mut cursor = {
            let mut txn = doc.transact_mut();
            text.insert(&mut txn, 0, "hello");
            text.cursor(&mut txn, 1)
        };
        {
            let mut txn = doc.transact_mut();
            text.remove_range(&mut txn, 0, 1);
        }
        {
            let mut txn = doc.transact_mut();
            cursor.insert(&mut txn, "X");
        }
        assert_eq!(text.get_string(&doc.transact()), "Xello");
    }

    #[test]
    fn concurrent_inserts_at_the_same_gap() {
        let local = Doc::with_client_id(1);
        let remote = Doc::with_client_id(2);
        let local_text = local.get_or_insert_text("t");
        let remote_text = remote.get_or_insert_text("t");
        {
            let mut txn = local.transact_mut();
            local_text.insert(&mut txn, 0, "hello");
        }
        exchange_updates(&[&local, &remote]);
        let mut local_cursor = {
            let mut txn = local.transact_mut();
            let end = local_text.len(&txn);
            local_text.cursor(&mut txn, end)
        };
        let mut remote_cursor = {
            let mut txn = remote.transact_mut();
            let end = remote_text.len(&txn);
            remote_text.cursor(&mut txn, end)
        };
        {
            let mut txn = local.transact_mut();
            local_cursor.insert(&mut txn, "A");
        }
        {
            let mut txn = remote.transact_mut();
            remote_cursor.insert(&mut txn, "B");
        }
        exchange_updates(&[&local, &remote]);
        let local_txn = local.transact();
        let remote_txn = remote.transact();
        assert_eq!(local_text.get_string(&local_txn), "helloAB");
        assert_eq!(remote_text.get_string(&remote_txn), "helloAB");
    }

    #[test]
    fn xml_text_cursor_inserts_across_transactions() {
        let doc = Doc::new();
        let fragment = doc.get_or_insert_xml_fragment("f");
        let (text, mut cursor) = {
            let mut txn = doc.transact_mut();
            let text = fragment.insert(&mut txn, 0, XmlTextPrelim::new(""));
            let cursor = text.cursor(&mut txn, 0);
            (text, cursor)
        };
        {
            let mut txn = doc.transact_mut();
            cursor.insert(&mut txn, "ab");
        }
        {
            let mut txn = doc.transact_mut();
            cursor.insert(&mut txn, "c");
        }
        assert_eq!(text.get_string(&doc.transact()), "abc");
    }

    #[test]
    fn plain_retain_keeps_an_end_cursor_in_the_same_transaction() {
        use crate::types::Delta;
        let doc = Doc::new();
        let text = doc.get_or_insert_text("t");
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "hello");
        let mut cursor = text.cursor(&mut txn, 5);
        text.apply_delta(&mut txn, [Delta::retain(2)]);
        cursor.insert(&mut txn, "X");
        assert_eq!(text.get_string(&txn), "helloX");
    }

    #[test]
    fn a_cursor_at_the_end_survives_another_cursor_splitting_the_block() {
        let doc = Doc::new();
        let text = doc.get_or_insert_text("t");
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "abcdefghij");
        let mut end = text.cursor(&mut txn, 10);
        let mut mid = text.cursor(&mut txn, 5);
        end.insert(&mut txn, "X");
        mid.insert(&mut txn, "Y");
        assert_eq!(text.get_string(&txn), "abcdeYfghijX");
    }

    #[test]
    fn attributed_inserts_in_one_transaction_stay_one_span() {
        let bold = Attrs::from([("bold".into(), Any::Bool(true))]);
        let doc = Doc::new();
        let text = doc.get_or_insert_text("t");
        let during = {
            let mut txn = doc.transact_mut();
            let mut cursor = text.cursor(&mut txn, 0);
            for _ in 0..5 {
                cursor.insert_with_attributes(&mut txn, "a", bold.clone());
            }
            text.diff(&txn, YChange::identity).len()
        };
        assert_eq!(during, 1, "during {}", during);
        let txn = doc.transact();
        let after = text.diff(&txn, YChange::identity).len();
        assert_eq!(after, 1, "after {}", after);
        assert_eq!(text.get_string(&txn), "aaaaa");
    }

    #[test]
    fn an_update_in_the_same_transaction_keeps_the_cursor_on_its_character() {
        use crate::updates::decoder::Decode;
        use crate::updates::encoder::Encode;
        use crate::{ReadTxn, StateVector, Update};

        let local = Doc::with_client_id(1);
        let remote = Doc::with_client_id(2);
        let local_text = local.get_or_insert_text("t");
        let remote_text = remote.get_or_insert_text("t");
        local_text.insert(&mut local.transact_mut(), 0, "hello");
        exchange_updates(&[&local, &remote]);
        remote_text.insert(&mut remote.transact_mut(), 1, "X");
        let update = {
            let state = local.transact().state_vector().encode_v1();
            let diff = remote
                .transact()
                .encode_diff_v1(&StateVector::decode_v1(&state).unwrap());
            Update::decode_v1(&diff).unwrap()
        };
        let mut txn = local.transact_mut();
        let mut cursor = local_text.cursor(&mut txn, 4);
        txn.apply_update(update).unwrap();
        cursor.insert(&mut txn, "!");
        assert_eq!(local_text.get_string(&txn), "hXell!o");
    }

    #[test]
    fn undo_puts_the_cursor_index_back() {
        use crate::UndoManager;
        let doc = Doc::new();
        let text = doc.get_or_insert_text("t");
        let mut mgr = UndoManager::new();
        mgr.expand_scope(&doc, &text);
        text.insert(&mut doc.transact_mut(), 0, "abcdefghij");
        let mut cursor = {
            let mut txn = doc.transact_mut();
            text.cursor(&mut txn, 10)
        };
        mgr.reset();
        text.insert(&mut doc.transact_mut(), 0, "Q");
        assert_eq!(cursor.index(&mut doc.transact_mut()), 11);
        mgr.undo_blocking();
        assert_eq!(cursor.index(&mut doc.transact_mut()), 10);
        cursor.insert(&mut doc.transact_mut(), "X");
        assert_eq!(text.get_string(&doc.transact()), "abcdefghijX");
    }

    #[test]
    fn cursor_remove_past_the_end_leaves_the_text_unchanged() {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let doc = Doc::new();
        let text = doc.get_or_insert_text("t");
        {
            let mut txn = doc.transact_mut();
            text.insert(&mut txn, 0, "hello");
            text.insert(&mut txn, 5, "!");
        }
        let panicked = catch_unwind(AssertUnwindSafe(|| {
            let mut txn = doc.transact_mut();
            let mut cursor = text.cursor(&mut txn, 2);
            cursor.remove(&mut txn, 100);
        }));
        assert!(panicked.is_err());
        assert_eq!(text.get_string(&doc.transact()), "hello!");
        {
            let mut txn = doc.transact_mut();
            text.insert(&mut txn, 5, "X");
            assert_eq!(text.get_string(&txn), "helloX!");
        }
    }
}
