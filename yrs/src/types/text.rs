use crate::block::{BlockPtr, Item, ItemContent, ItemPosition};
use crate::block_store::Snapshot;
use crate::event::Subscription;
use crate::transaction::Transaction;
use crate::types::{Attrs, Branch, BranchRef, Delta, Observers, Path, Value};
use crate::*;
use lib0::any::Any;
use std::cell::{Ref, RefMut, UnsafeCell};
use std::collections::HashMap;

/// A shared data type used for collaborative text editing. It enables multiple users to add and
/// remove chunks of text in efficient manner. This type is internally represented as a mutable
/// double-linked list of text chunks - an optimization occurs during [Transaction::commit], which
/// allows to squash multiple consecutively inserted characters together as a single chunk of text
/// even between transaction boundaries in order to preserve more efficient memory model.
///
/// [Text] structure internally uses UTF-8 encoding and its length is described in a number of
/// bytes rather than individual characters (a single UTF-8 code point can consist of many bytes).
///
/// Like all Yrs shared data types, [Text] is resistant to the problem of interleaving (situation
/// when characters inserted one after another may interleave with other peers concurrent inserts
/// after merging all updates together). In case of Yrs conflict resolution is solved by using
/// unique document id to determine correct and consistent ordering.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Text(BranchRef);

impl Text {
    /// Converts context of this text data structure into a single string value.
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self, txn: &Transaction) -> String {
        let inner = self.0.as_ref();
        let mut start = inner.start;
        let mut s = String::new();
        let store = txn.store();
        while let Some(a) = start.as_ref() {
            if let Some(item) = store.blocks.get_item(&a) {
                if !item.is_deleted() {
                    if let block::ItemContent::String(item_string) = &item.content {
                        s.push_str(item_string);
                    }
                }
                start = item.right.clone();
            } else {
                break;
            }
        }
        s
    }

    /// Returns a number of characters visible in a current text data structure.
    pub fn len(&self) -> u32 {
        self.0.borrow().content_len
    }

    pub(crate) fn inner(&self) -> Ref<Branch> {
        self.0.borrow()
    }

    pub(crate) fn inner_mut(&self) -> RefMut<Branch> {
        self.0.borrow_mut()
    }

    pub(crate) fn find_position(
        &self,
        txn: &mut Transaction,
        index: u32,
    ) -> Option<block::ItemPosition> {
        let mut pos = {
            let inner = self.0.borrow();
            block::ItemPosition {
                parent: inner.ptr.clone(),
                left: None,
                right: inner.start,
                index: 0,
                current_attrs: None,
            }
        };

        let store = txn.store_mut();
        let encoding = store.options.offset_kind;
        let mut remaining = index;
        while let Some(right_ptr) = pos.right.as_ref() {
            if remaining == 0 {
                break;
            }

            if let Some(mut right) = store.blocks.get_item(right_ptr) {
                if !right.is_deleted() {
                    if let ItemContent::Format(key, value) = &right.content {
                        let attrs = pos
                            .current_attrs
                            .get_or_insert_with(|| Box::new(Attrs::new()));
                        Text::update_current_attributes(attrs, key, value.as_ref());
                    } else {
                        let mut block_len = right.len();
                        let content_len = right.content_len(encoding);
                        if remaining < content_len {
                            // split right item
                            let offset = if let ItemContent::String(str) = &right.content {
                                str.block_offset(remaining, encoding)
                            } else {
                                remaining
                            };
                            let split_ptr = BlockPtr::new(
                                ID::new(right.id.client, right.id.clock + offset),
                                right_ptr.pivot() as u32,
                            );
                            let (_, _) = store.blocks.split_block(&split_ptr);
                            right = store.blocks.get_item(right_ptr).unwrap();
                            block_len = right.len();
                            remaining = 0;
                        } else {
                            remaining -= content_len;
                        }
                        pos.index += block_len;
                    }
                }
                pos.left = pos.right.take();
                pos.right = right.right.clone();
            } else {
                return None;
            }
        }

        Some(pos)
    }

    /// Inserts a `chunk` of text at a given `index`.
    /// If `index` is `0`, this `chunk` will be inserted at the beginning of a current text.
    /// If `index` is equal to current data structure length, this `chunk` will be appended at
    /// the end of it.
    ///
    /// This method will panic if provided `index` is greater than the length of a current text.
    pub fn insert(&self, txn: &mut Transaction, index: u32, chunk: &str) {
        if let Some(pos) = self.find_position(txn, index) {
            let value = crate::block::PrelimText(chunk.into());
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
    pub fn insert_with_attributes(
        &self,
        txn: &mut Transaction,
        index: u32,
        chunk: &str,
        mut attributes: Attrs,
    ) {
        if let Some(mut pos) = self.find_position(txn, index) {
            pos.unset_missing(&mut attributes);
            Text::minimize_attr_changes(&mut pos, txn, &attributes);
            let negated_attrs = self.insert_attributes(txn, &mut pos, attributes);

            let value = crate::block::PrelimText(chunk.into());
            let item = txn.create_item(&pos, value, None);

            pos.right = Some(BlockPtr::from(item.id));
            pos.forward(txn);

            self.insert_negated_attributes(txn, &mut pos, negated_attrs);
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
    pub fn insert_embed(&self, txn: &mut Transaction, index: u32, content: Any) {
        if let Some(pos) = self.find_position(txn, index) {
            let value = crate::block::PrelimEmbed(content);
            txn.create_item(&pos, value, None);
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
    pub fn insert_embed_with_attributes(
        &self,
        txn: &mut Transaction,
        index: u32,
        embed: Any,
        mut attributes: Attrs,
    ) {
        if let Some(mut pos) = self.find_position(txn, index) {
            pos.unset_missing(&mut attributes);
            Text::minimize_attr_changes(&mut pos, txn, &attributes);
            let negated_attrs = self.insert_attributes(txn, &mut pos, attributes);

            let value = crate::block::PrelimEmbed(embed);
            let item = txn.create_item(&pos, value, None);

            pos.right = Some(BlockPtr::from(item.id));
            pos.forward(txn);

            self.insert_negated_attributes(txn, &mut pos, negated_attrs);
        } else {
            panic!("The type or the position doesn't exist!");
        }
    }

    /// Appends a given `chunk` of text at the end of a current text structure.
    pub fn push(&self, txn: &mut Transaction, chunk: &str) {
        let idx = self.len();
        self.insert(txn, idx, chunk)
    }

    /// Removes up to a `len` characters from a current text structure, starting at given `index`.
    /// This method panics in case when not all expected characters were removed (due to
    /// insufficient number of characters to remove) or `index` is outside of the bounds of text.
    pub fn remove_range(&self, txn: &mut Transaction, index: u32, len: u32) {
        if let Some(pos) = self.find_position(txn, index) {
            Self::remove(txn, pos, len)
        } else {
            panic!("The type or the position doesn't exist!");
        }
    }

    fn remove(txn: &mut Transaction, mut pos: ItemPosition, len: u32) {
        let encoding = txn.store().options.offset_kind;
        let mut remaining = len;
        let start = pos.right.clone();
        let start_attrs = pos.current_attrs.clone();
        while let Some(item) = pos
            .right
            .as_ref()
            .and_then(|p| txn.store().blocks.get_item_mut(p))
        {
            if remaining == 0 {
                break;
            }

            if !item.is_deleted() {
                match &item.content {
                    ItemContent::Embed(_) | ItemContent::String(_) | ItemContent::Type(_) => {
                        let content_len = item.content_len(encoding);
                        let mut ptr = pos.right.unwrap();
                        if remaining < content_len {
                            // split block
                            let offset = if let ItemContent::String(s) = &item.content {
                                s.block_offset(remaining, encoding)
                            } else {
                                len
                            };
                            ptr.id.clock += offset;
                            remaining = 0;
                            let (l, _) = txn.store_mut().blocks.split_block(&ptr);
                            ptr = l.unwrap();
                        } else {
                            remaining -= content_len;
                        };
                        txn.delete(&ptr);
                    }
                    _ => {}
                }
            }

            pos.forward(txn);
        }

        if remaining > 0 {
            panic!("Couldn't remove {} elements from an array. Only {} of them were successfully removed.", len, len - remaining);
        }

        if let (Some(start), Some(start_attrs), Some(end_attrs)) =
            (start, start_attrs, pos.current_attrs.as_mut())
        {
            Self::clean_format_gap(
                txn,
                Some(start),
                pos.right,
                start_attrs.as_ref(),
                end_attrs.as_mut(),
            );
        }
    }

    fn clean_format_gap(
        txn: &mut Transaction,
        mut start: Option<BlockPtr>,
        mut end: Option<BlockPtr>,
        start_attrs: &Attrs,
        end_attrs: &mut Attrs,
    ) -> u32 {
        let store = txn.store();
        while let Some(item) = end.as_ref().and_then(|ptr| store.blocks.get_item(ptr)) {
            match &item.content {
                ItemContent::String(_) | ItemContent::Embed(_) => break,
                ItemContent::Format(key, value) if !item.is_deleted() => {
                    Self::update_current_attributes(end_attrs, key.as_ref(), value);
                }
                _ => {}
            }
            end = item.right.clone();
        }

        let mut cleanups = 0;
        while start != end {
            if let Some(item) = start
                .as_ref()
                .and_then(|ptr| txn.store().blocks.get_item(ptr))
            {
                let right = item.right.clone();
                if !item.is_deleted() {
                    if let ItemContent::Format(key, value) = &item.content {
                        let e = end_attrs.get(key).unwrap_or(&Any::Null);
                        let s = start_attrs.get(key).unwrap_or(&Any::Null);
                        if e != value.as_ref() || s == value.as_ref() {
                            txn.delete(&start.unwrap());
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

    /// Wraps an existing piece of text within a range described by `index`-`len` parameters with
    /// formatting blocks containing provided `attributes` metadata.
    pub fn format(&self, txn: &mut Transaction, index: u32, len: u32, attributes: Attrs) {
        if let Some(pos) = self.find_position(txn, index) {
            self.insert_format(txn, pos, len, attributes)
        } else {
            panic!("Index {} is outside of the range.", index);
        }
    }

    fn insert_format(
        &self,
        txn: &mut Transaction,
        mut pos: ItemPosition,
        mut len: u32,
        attrs: Attrs,
    ) {
        Self::minimize_attr_changes(&mut pos, txn, &attrs);
        let mut negated_attrs = self.insert_attributes(txn, &mut pos, attrs.clone()); //TODO: remove `attrs.clone()`
        let encoding = txn.store().options.offset_kind;
        while let Some(right) = pos.right.as_ref() {
            if len <= 0 {
                break;
            }

            if let Some(block) = txn.store().blocks.get_item_mut(right) {
                if !block.is_deleted() {
                    match &block.content {
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
                        _ => {
                            let content_len = block.content_len(encoding);
                            if len < content_len {
                                let mut split_ptr = right.clone();
                                split_ptr.id.clock += len;
                                let (_, r) = txn.store_mut().blocks.split_block(&split_ptr);
                                pos.right = r;
                                break;
                            }
                            len -= content_len;
                        }
                    }
                }
            }

            if !pos.forward(txn) {
                break;
            }
        }

        self.insert_negated_attributes(txn, &mut pos, negated_attrs);
    }

    fn minimize_attr_changes(pos: &mut ItemPosition, txn: &mut Transaction, attrs: &Attrs) {
        // go right while attrs[right.key] === right.value (or right is deleted)
        while let Some(right) = pos.right.as_ref() {
            if let Some(i) = txn.store().blocks.get_item(right) {
                if !i.is_deleted() {
                    if let ItemContent::Format(k, v) = &i.content {
                        if let Some(v2) = attrs.get(k) {
                            if (v.as_ref()).eq(v2) {
                                pos.forward(txn);
                                continue;
                            }
                        }
                    }
                } else {
                    pos.forward(txn);
                    continue;
                }
            }
            break;
        }
    }

    fn insert_attributes(
        &self,
        txn: &mut Transaction,
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
                let parent = { self.0.borrow().ptr.clone() };
                let mut item = Item::new(
                    ID::new(client_id, store.blocks.get_state(&client_id)),
                    pos.left.clone(),
                    pos.left
                        .map(|ptr| store.blocks.get_block(&ptr).unwrap().last_id()),
                    pos.right.clone(),
                    pos.right.map(|ptr| ptr.id.clone()),
                    parent,
                    None,
                    ItemContent::Format(k, v.into()),
                );
                pos.right = Some(BlockPtr::from(item.id));
                item.integrate(txn, 0, 0);

                let local_block_list = txn.store_mut().blocks.get_client_blocks_mut(item.id.client);
                local_block_list.push(block::Block::Item(item));

                pos.forward(txn);
                store = txn.store_mut();
            }
        }
        negated_attrs
    }

    fn insert_negated_attributes(
        &self,
        txn: &mut Transaction,
        pos: &mut ItemPosition,
        mut attrs: Attrs,
    ) {
        while let Some(right) = pos.right.as_ref() {
            if let Some(item) = txn.store().blocks.get_item(right) {
                if !item.is_deleted() {
                    if let ItemContent::Format(key, value) = &item.content {
                        if let Some(curr_val) = attrs.get(key) {
                            if curr_val == value.as_ref() {
                                attrs.remove(key);

                                pos.forward(txn);
                                continue;
                            }
                        }
                    }
                } else {
                    pos.forward(txn);
                    continue;
                }
            }
            break;
        }

        let mut store = txn.store_mut();
        for (k, v) in attrs {
            let client_id = store.options.client_id;
            let parent = { self.0.borrow().ptr.clone() };
            let mut item = Item::new(
                ID::new(client_id, store.blocks.get_state(&client_id)),
                pos.left.clone(),
                pos.left
                    .map(|ptr| store.blocks.get_block(&ptr).unwrap().last_id()),
                pos.right.clone(),
                pos.right.map(|ptr| ptr.id.clone()),
                parent,
                None,
                ItemContent::Format(k, v.into()),
            );
            pos.right = Some(BlockPtr::from(item.id));
            item.integrate(txn, 0, 0);

            let local_block_list = txn.store_mut().blocks.get_client_blocks_mut(item.id.client);
            local_block_list.push(block::Block::Item(item));

            pos.forward(txn);
            store = txn.store_mut();
        }
    }

    /// Subscribes a given callback to be triggered whenever current text is changed.
    /// A callback is triggered whenever a transaction gets committed. This function does not
    /// trigger if changes have been observed by nested shared collections.
    ///
    /// All text changes can be tracked by using [TextEvent::delta] method: keep in mind that delta
    /// contains collection of individual characters rather than strings.
    ///
    /// Returns an [Observer] which, when dropped, will unsubscribe current callback.
    pub fn observe<F>(&self, f: F) -> Subscription<TextEvent>
    where
        F: Fn(&Transaction, &TextEvent) -> () + 'static,
    {
        let mut branch = self.0.borrow_mut();
        if let Observers::Text(eh) = branch.observers.get_or_insert_with(Observers::text) {
            eh.subscribe(f)
        } else {
            panic!("Observed collection is of different type") //TODO: this should be Result::Err
        }
    }

    /// Unsubscribes a previously subscribed event callback identified by given `subscription_id`.
    pub fn unobserve(&self, subscription_id: SubscriptionId) {
        let mut branch = self.0.borrow_mut();
        if let Some(Observers::Text(eh)) = branch.observers.as_mut() {
            eh.unsubscribe(subscription_id);
        }
    }

    pub(crate) fn update_current_attributes(attrs: &mut Attrs, key: &str, value: &Any) {
        if let Any::Null = value {
            attrs.remove(key);
        } else {
            attrs.insert(key.into(), value.clone());
        }
    }

    pub fn diff(&self, txn: &mut Transaction) -> Vec<Diff> {
        self.diff_range(txn, None, None)
    }

    /// Returns the Delta representation of this YText type.
    pub fn diff_range(
        &self,
        txn: &mut Transaction,
        hi: Option<&Snapshot>,
        lo: Option<&Snapshot>,
    ) -> Vec<Diff> {
        #[derive(Default)]
        struct DiffAssembler {
            ops: Vec<Diff>,
            buf: String,
            curr_attrs: Attrs,
        }

        impl DiffAssembler {
            fn pack_str(&mut self) {
                if !self.buf.is_empty() {
                    let attrs = self.attrs_boxed();
                    let mut buf = std::mem::replace(&mut self.buf, String::new());
                    buf.shrink_to_fit();
                    let op = Diff::Insert(Value::Any(buf.into()), attrs);
                    self.ops.push(op);
                }
            }

            fn finish(self) -> Vec<Diff> {
                self.ops
            }

            fn attrs_boxed(&mut self) -> Option<Box<Attrs>> {
                if self.curr_attrs.is_empty() {
                    None
                } else {
                    let attrs = std::mem::replace(&mut self.curr_attrs, Attrs::new());
                    Some(Box::new(attrs))
                }
            }
        }

        fn seen(snapshot: Option<&Snapshot>, item: &Item) -> bool {
            if let Some(s) = snapshot {
                s.is_visible(&item.id)
            } else {
                !item.is_deleted()
            }
        }

        if let Some(snapshot) = hi {
            txn.split_by_snapshot(snapshot);
        }

        if let Some(snapshot) = lo {
            txn.split_by_snapshot(snapshot);
        }

        let mut asm = DiffAssembler::default();
        let mut n = self
            .0
            .borrow()
            .start
            .as_ref()
            .and_then(|ptr| txn.store().blocks.get_item(ptr));
        while let Some(item) = n {
            if seen(hi, item) || (lo.is_some() && seen(lo, item)) {
                match &item.content {
                    ItemContent::String(s) => {
                        /*TODO:
                        const cur = currentAttributes.get('ychange')
                        if (snapshot !== undefined && !isVisible(n, snapshot)) {
                          if (cur === undefined || cur.user !== n.id.client || cur.state !== 'removed') {
                            packStr()
                            currentAttributes.set('ychange', computeYChange ? computeYChange('removed', n.id) : { type: 'removed' })
                          }
                        } else if (prevSnapshot !== undefined && !isVisible(n, prevSnapshot)) {
                          if (cur === undefined || cur.user !== n.id.client || cur.state !== 'added') {
                            packStr()
                            currentAttributes.set('ychange', computeYChange ? computeYChange('added', n.id) : { type: 'added' })
                          }
                        } else if (cur !== undefined) {
                          packStr()
                          currentAttributes.delete('ychange')
                        }
                         */
                        asm.buf.push_str(s.as_str());
                    }
                    ItemContent::Type(_) | ItemContent::Embed(_) => {
                        asm.pack_str();
                        if let Some(value) = item.content.get_content_last(&txn) {
                            let attrs = asm.attrs_boxed();
                            asm.ops.push(Diff::Insert(value, attrs));
                        }
                    }
                    ItemContent::Format(key, value) => {
                        if seen(hi, item) {
                            asm.pack_str();
                            Self::update_current_attributes(
                                &mut asm.curr_attrs,
                                key,
                                value.as_ref(),
                            );
                        }
                    }
                    _ => {}
                }
            }
            n = item
                .right
                .as_ref()
                .and_then(|ptr| txn.store().blocks.get_item(ptr));
        }

        asm.pack_str();
        asm.finish()
    }
}

impl Into<ItemContent> for Text {
    fn into(self) -> ItemContent {
        ItemContent::Type(self.0.clone())
    }
}

impl From<BranchRef> for Text {
    fn from(inner: BranchRef) -> Self {
        Text(inner)
    }
}

#[derive(Debug, PartialEq)]
pub enum Diff {
    Insert(Value, Option<Box<Attrs>>),
}

/// Event generated by [Text::observe] method. Emitted during transaction commit phase.
pub struct TextEvent {
    target: Text,
    current_target: BranchRef,
    delta: UnsafeCell<Option<Vec<Delta>>>,
}

impl TextEvent {
    pub(crate) fn new(branch_ref: BranchRef) -> Self {
        let current_target = branch_ref.clone();
        let target = Text::from(branch_ref);
        TextEvent {
            target,
            current_target,
            delta: UnsafeCell::new(None),
        }
    }

    /// Returns a [Text] instance which emitted this event.
    pub fn target(&self) -> &Text {
        &self.target
    }

    /// Returns a path from root type down to [Text] instance which emitted this event.
    pub fn path(&self, txn: &Transaction) -> Path {
        Branch::path(self.current_target.borrow(), self.target.0.borrow(), txn)
    }

    /// Returns a summary of text changes made over corresponding [Text] collection within
    /// bounds of current transaction.
    pub fn delta(&self, txn: &Transaction) -> &[Delta] {
        let delta = unsafe { self.delta.get().as_mut().unwrap() };
        delta
            .get_or_insert_with(|| Self::get_delta(self.target.0.borrow(), txn))
            .as_slice()
    }

    pub(crate) fn get_delta(target: Ref<Branch>, txn: &Transaction) -> Vec<Delta> {
        #[derive(Clone, Copy, Eq, PartialEq)]
        enum Action {
            Insert,
            Retain,
            Delete,
        }

        #[derive(Default)]
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
                        self.delta.push(Delta::Retain(len, attrs))
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
        let mut current = target
            .start
            .as_ref()
            .and_then(|ptr| txn.store().blocks.get_item(ptr));

        while let Some(item) = current {
            match &item.content {
                ItemContent::Type(_) | ItemContent::Embed(_) => {
                    if txn.has_added(&item.id) {
                        if !txn.has_deleted(&item.id) {
                            asm.add_op();
                            asm.action = Some(Action::Insert);
                            asm.insert = item.content.get_content_last(txn);
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
                        asm.delete += item.content_len(encoding);
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
                        Text::update_current_attributes(
                            &mut asm.current_attrs,
                            key,
                            value.as_ref(),
                        );
                    }
                }
                _ => {}
            }

            current = item
                .right
                .as_ref()
                .and_then(|ptr| txn.store().blocks.get_item(ptr));
        }

        asm.add_op();
        asm.finish()
    }
}

#[cfg(test)]
mod test {
    use crate::doc::{OffsetKind, Options};
    use crate::test_utils::{exchange_updates, run_scenario, RngExt};
    use crate::types::text::{Attrs, Delta, Diff};
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::{Encoder, EncoderV1};
    use crate::{Doc, Update};
    use lib0::any::Any;
    use rand::prelude::StdRng;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    #[test]
    fn append_single_character_blocks() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "a");
        txt.insert(&mut txn, 1, "b");
        txt.insert(&mut txn, 2, "c");

        assert_eq!(txt.to_string(&txn).as_str(), "abc");
    }

    #[test]
    fn append_mutli_character_blocks() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "hello");
        txt.insert(&mut txn, 5, " ");
        txt.insert(&mut txn, 6, "world");

        assert_eq!(txt.to_string(&txn).as_str(), "hello world");
    }

    #[test]
    fn prepend_single_character_blocks() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "a");
        txt.insert(&mut txn, 0, "b");
        txt.insert(&mut txn, 0, "c");

        assert_eq!(txt.to_string(&txn).as_str(), "cba");
    }

    #[test]
    fn prepend_mutli_character_blocks() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "hello");
        txt.insert(&mut txn, 0, " ");
        txt.insert(&mut txn, 0, "world");

        assert_eq!(txt.to_string(&txn).as_str(), "world hello");
    }

    #[test]
    fn insert_after_block() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "hello");
        txt.insert(&mut txn, 5, " ");
        txt.insert(&mut txn, 6, "world");
        txt.insert(&mut txn, 6, "beautiful ");

        assert_eq!(txt.to_string(&txn).as_str(), "hello beautiful world");
    }

    #[test]
    fn insert_inside_of_block() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "it was expected");
        txt.insert(&mut txn, 6, " not");

        assert_eq!(txt.to_string(&txn).as_str(), "it was not expected");
    }

    #[test]
    fn insert_concurrent_root() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let txt1 = t1.get_text("test");

        txt1.insert(&mut t1, 0, "hello ");

        let d2 = Doc::with_client_id(2);
        let mut t2 = d2.transact();
        let txt2 = t2.get_text("test");

        txt2.insert(&mut t2, 0, "world");

        let d1_sv = d1.get_state_vector(&t1);
        let d2_sv = d2.get_state_vector(&t2);

        let u1 = d1.encode_delta_as_update_v1(&t1, &d2_sv);
        let u2 = d2.encode_delta_as_update_v1(&t2, &d1_sv);

        d1.apply_update_v1(&mut t1, u2.as_slice());
        d2.apply_update_v1(&mut t2, u1.as_slice());

        let a = txt1.to_string(&t1);
        let b = txt2.to_string(&t2);

        assert_eq!(a, b);
        assert_eq!(a.as_str(), "hello world");
    }

    #[test]
    fn insert_concurrent_in_the_middle() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let txt1 = t1.get_text("test");

        txt1.insert(&mut t1, 0, "I expect that");
        assert_eq!(txt1.to_string(&t1).as_str(), "I expect that");

        let d2 = Doc::with_client_id(2);
        let mut t2 = d2.transact();

        let d2_sv = d2.get_state_vector(&t2);
        let u1 = d1.encode_delta_as_update_v1(&t1, &d2_sv);
        d2.apply_update_v1(&mut t2, u1.as_slice());

        let txt2 = t2.get_text("test");
        assert_eq!(txt2.to_string(&t2).as_str(), "I expect that");

        txt2.insert(&mut t2, 1, " have");
        txt2.insert(&mut t2, 13, "ed");
        assert_eq!(txt2.to_string(&t2).as_str(), "I have expected that");

        txt1.insert(&mut t1, 1, " didn't");
        assert_eq!(txt1.to_string(&t1).as_str(), "I didn't expect that");

        let d2_sv = d2.get_state_vector(&t2);
        let d1_sv = d1.get_state_vector(&t1);
        let u1 = d1.encode_delta_as_update_v1(&t1, &d2_sv);
        let u2 = d2.encode_delta_as_update_v1(&t2, &d1_sv);
        d1.apply_update_v1(&mut t1, u2.as_slice());
        d2.apply_update_v1(&mut t2, u1.as_slice());

        let a = txt1.to_string(&t1);
        let b = txt2.to_string(&t2);

        assert_eq!(a, b);
        assert_eq!(a.as_str(), "I didn't have expected that");
    }

    #[test]
    fn append_concurrent() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let txt1 = t1.get_text("test");

        txt1.insert(&mut t1, 0, "aaa");
        assert_eq!(txt1.to_string(&t1).as_str(), "aaa");

        let d2 = Doc::with_client_id(2);
        let mut t2 = d2.transact();

        let d2_sv = d2.get_state_vector(&t2);
        let u1 = d1.encode_delta_as_update_v1(&t1, &d2_sv);
        d2.apply_update_v1(&mut t2, u1.as_slice());

        let txt2 = t2.get_text("test");
        assert_eq!(txt2.to_string(&t2).as_str(), "aaa");

        txt2.insert(&mut t2, 3, "bbb");
        txt2.insert(&mut t2, 6, "bbb");
        assert_eq!(txt2.to_string(&t2).as_str(), "aaabbbbbb");

        txt1.insert(&mut t1, 3, "aaa");
        assert_eq!(txt1.to_string(&t1).as_str(), "aaaaaa");

        let d2_sv = d2.get_state_vector(&t2);
        let d1_sv = d1.get_state_vector(&t1);
        let u1 = d1.encode_delta_as_update_v1(&t1, &d2_sv);
        let u2 = d2.encode_delta_as_update_v1(&t2, &d1_sv);

        d1.apply_update_v1(&mut t1, u2.as_slice());
        d2.apply_update_v1(&mut t2, u1.as_slice());

        let a = txt1.to_string(&t1);
        let b = txt2.to_string(&t2);

        assert_eq!(a.as_str(), "aaaaaabbbbbb");
        assert_eq!(a, b);
    }

    #[test]
    fn delete_single_block_start() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "bbb");
        txt.insert(&mut txn, 0, "aaa");
        txt.remove_range(&mut txn, 0, 3);

        assert_eq!(txt.len(), 3);
        assert_eq!(txt.to_string(&txn).as_str(), "bbb");
    }

    #[test]
    fn delete_single_block_end() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "bbb");
        txt.insert(&mut txn, 0, "aaa");
        txt.remove_range(&mut txn, 3, 3);

        assert_eq!(txt.to_string(&txn).as_str(), "aaa");
    }

    #[test]
    fn delete_multiple_whole_blocks() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "a");
        txt.insert(&mut txn, 1, "b");
        txt.insert(&mut txn, 2, "c");

        txt.remove_range(&mut txn, 1, 1);
        assert_eq!(txt.to_string(&txn).as_str(), "ac");

        txt.remove_range(&mut txn, 1, 1);
        assert_eq!(txt.to_string(&txn).as_str(), "a");

        txt.remove_range(&mut txn, 0, 1);
        assert_eq!(txt.to_string(&txn).as_str(), "");
    }

    #[test]
    fn delete_slice_of_block() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "abc");
        txt.remove_range(&mut txn, 1, 1);

        assert_eq!(txt.to_string(&txn).as_str(), "ac");
    }

    #[test]
    fn delete_multiple_blocks_with_slicing() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "hello ");
        txt.insert(&mut txn, 6, "beautiful");
        txt.insert(&mut txn, 15, " world");

        txt.remove_range(&mut txn, 5, 11);
        assert_eq!(txt.to_string(&txn).as_str(), "helloworld");
    }

    #[test]
    fn insert_after_delete() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "hello ");
        txt.remove_range(&mut txn, 0, 5);
        txt.insert(&mut txn, 1, "world");

        assert_eq!(txt.to_string(&txn).as_str(), " world");
    }

    #[test]
    fn concurrent_insert_delete() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let txt1 = t1.get_text("test");

        txt1.insert(&mut t1, 0, "hello world");
        assert_eq!(txt1.to_string(&t1).as_str(), "hello world");

        let u1 = d1.encode_state_as_update_v1(&t1);

        let d2 = Doc::with_client_id(2);
        let mut t2 = d2.transact();
        d2.apply_update_v1(&mut t2, u1.as_slice());
        let txt2 = t2.get_text("test");
        assert_eq!(txt2.to_string(&t2).as_str(), "hello world");

        txt1.insert(&mut t1, 5, " beautiful");
        txt1.insert(&mut t1, 21, "!");
        txt1.remove_range(&mut t1, 0, 5);
        assert_eq!(txt1.to_string(&t1).as_str(), " beautiful world!");

        txt2.remove_range(&mut t2, 5, 5);
        txt2.remove_range(&mut t2, 0, 1);
        txt2.insert(&mut t2, 0, "H");
        assert_eq!(txt2.to_string(&t2).as_str(), "Hellod");

        let sv1 = d1.get_state_vector(&t1);
        let sv2 = d2.get_state_vector(&t2);
        let u1 = d1.encode_delta_as_update_v1(&t1, &sv2);
        let u2 = d2.encode_delta_as_update_v1(&t2, &sv1);

        d1.apply_update_v1(&mut t1, u2.as_slice());
        d2.apply_update_v1(&mut t2, u1.as_slice());

        let a = txt1.to_string(&t1);
        let b = txt2.to_string(&t2);

        assert_eq!(a, b);
        assert_eq!(a, "H beautifuld!".to_owned());
    }

    #[test]
    fn insert_and_remove_event_changes() {
        let d1 = Doc::with_client_id(1);
        let txt = {
            let mut txn = d1.transact();
            txn.get_text("text")
        };
        let delta = Rc::new(RefCell::new(None));
        let delta_c = delta.clone();
        let _sub = txt.observe(move |txn, e| {
            *delta_c.borrow_mut() = Some(e.delta(txn).to_vec());
        });

        // insert initial string
        {
            let mut txn = d1.transact();
            txt.insert(&mut txn, 0, "abcd");
        }
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![Delta::Inserted("abcd".into(), None)])
        );

        // remove middle
        {
            let mut txn = d1.transact();
            txt.remove_range(&mut txn, 1, 2);
        }
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![Delta::Retain(1, None), Delta::Deleted(2)])
        );

        // insert again
        {
            let mut txn = d1.transact();
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
        let txt = {
            let mut txn = d2.transact();
            txn.get_text("text")
        };
        let delta_c = delta.clone();
        let _sub = txt.observe(move |txn, e| {
            *delta_c.borrow_mut() = Some(e.delta(txn).to_vec());
        });

        {
            let t1 = d1.transact();
            let mut t2 = d2.transact();

            let sv = t2.state_vector();
            let mut encoder = EncoderV1::new();
            t1.encode_diff(&sv, &mut encoder);
            t2.apply_update(Update::decode_v1(encoder.to_vec().as_slice()));
        }

        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![Delta::Inserted("aefd".into(), None)])
        );
    }

    #[test]
    fn unicode_support() {
        let d1 = {
            let mut options = Options::with_client_id(1);
            options.offset_kind = OffsetKind::Utf32;
            Doc::with_options(options)
        };
        let txt1 = {
            let mut txn = d1.transact();
            txn.get_text("test")
        };

        let d2 = {
            let mut options = Options::with_client_id(2);
            options.offset_kind = OffsetKind::Bytes;
            Doc::with_options(options)
        };
        let txt2 = {
            let mut txn = d2.transact();
            txn.get_text("test")
        };

        {
            let mut txn = d1.transact();

            txt1.insert(&mut txn, 0, "Zażółć gęślą jaźń");
            assert_eq!(txt1.to_string(&txn), "Zażółć gęślą jaźń");
            assert_eq!(txt1.len(), 17);
        }

        exchange_updates(&[&d1, &d2]);

        {
            let txn = d2.transact();
            assert_eq!(txt2.to_string(&txn), "Zażółć gęślą jaźń");
            assert_eq!(txt2.len(), 26);
        }

        {
            let mut txn = d1.transact();
            txt1.remove_range(&mut txn, 9, 3);
            txt1.insert(&mut txn, 9, "si");

            assert_eq!(txt1.to_string(&txn), "Zażółć gęsi jaźń");
            assert_eq!(txt1.len(), 16);
        }

        exchange_updates(&[&d1, &d2]);

        {
            let txn = d2.transact();
            assert_eq!(txt2.to_string(&txn), "Zażółć gęsi jaźń");
            assert_eq!(txt2.len(), 23);
        }
    }

    fn text_transactions() -> [Box<dyn Fn(&mut Doc, &mut StdRng)>; 2] {
        fn insert_text(doc: &mut Doc, rng: &mut StdRng) {
            let mut txn = doc.transact();
            let ytext = txn.get_text("text");
            let pos = rng.between(0, ytext.len());
            let word = rng.random_string();
            ytext.insert(&mut txn, pos, word.as_str());
        }

        fn delete_text(doc: &mut Doc, rng: &mut StdRng) {
            let mut txn = doc.transact();
            let ytext = txn.get_text("text");
            let len = ytext.len();
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
        let txt1 = {
            let mut txn = d1.transact();
            txn.get_text("text")
        };

        let delta1 = Rc::new(RefCell::new(None));
        let delta_clone = delta1.clone();
        let _sub1 = txt1.observe(move |txn, e| {
            delta_clone.replace(Some(e.delta(txn).to_vec()));
        });

        let d2 = Doc::with_client_id(2);
        let txt2 = {
            let mut txn = d2.transact();
            txn.get_text("text")
        };

        let delta2 = Rc::new(RefCell::new(None));
        let delta_clone = delta2.clone();
        let _sub2 = txt2.observe(move |txn, e| {
            delta_clone.replace(Some(e.delta(txn).to_vec()));
        });

        let a: Attrs = HashMap::from([("bold".into(), Any::Bool(true))]);

        // step 1
        {
            let mut txn = d1.transact();
            txt1.insert_with_attributes(&mut txn, 0, "abc", a.clone());
            let update = txn.encode_update_v1();
            txn.commit();

            let expected = Some(vec![Delta::Inserted(
                "abc".into(),
                Some(Box::new(a.clone())),
            )]);

            assert_eq!(txt1.to_string(&txn), "abc".to_string());
            assert_eq!(
                txt1.diff(&mut txn),
                vec![Diff::Insert("abc".into(), Some(Box::new(a.clone())))]
            );
            assert_eq!(delta1.take(), expected);

            let mut txn = d2.transact();
            d2.apply_update_v1(&mut txn, update.as_slice());
            txn.commit();

            assert_eq!(txt2.to_string(&txn), "abc".to_string());
            assert_eq!(delta2.take(), expected);
        }

        // step 2
        {
            let mut txn = d1.transact();
            txt1.remove_range(&mut txn, 0, 1);
            let update = txn.encode_update_v1();
            txn.commit();

            let expected = Some(vec![Delta::Deleted(1)]);

            assert_eq!(txt1.to_string(&txn), "bc".to_string());
            assert_eq!(
                txt1.diff(&mut txn),
                vec![Diff::Insert("bc".into(), Some(Box::new(a.clone())))]
            );
            assert_eq!(delta1.take(), expected);

            let mut txn = d2.transact();
            d2.apply_update_v1(&mut txn, update.as_slice());
            txn.commit();

            assert_eq!(txt2.to_string(&txn), "bc".to_string());
            assert_eq!(delta2.take(), expected);
        }

        // step 3
        {
            let mut txn = d1.transact();
            txt1.remove_range(&mut txn, 1, 1);
            let update = txn.encode_update_v1();
            txn.commit();

            let expected = Some(vec![Delta::Retain(1, None), Delta::Deleted(1)]);

            assert_eq!(txt1.to_string(&txn), "b".to_string());
            assert_eq!(
                txt1.diff(&mut txn),
                vec![Diff::Insert("b".into(), Some(Box::new(a.clone())))]
            );
            assert_eq!(delta1.take(), expected);

            let mut txn = d2.transact();
            d2.apply_update_v1(&mut txn, update.as_slice());
            txn.commit();

            assert_eq!(txt2.to_string(&txn), "b".to_string());
            assert_eq!(delta2.take(), expected);
        }

        // step 4
        {
            let mut txn = d1.transact();
            txt1.insert_with_attributes(&mut txn, 0, "z", a.clone());
            let update = txn.encode_update_v1();
            txn.commit();

            let expected = Some(vec![Delta::Inserted("z".into(), Some(Box::new(a.clone())))]);

            assert_eq!(txt1.to_string(&txn), "zb".to_string());
            assert_eq!(
                txt1.diff(&mut txn),
                vec![Diff::Insert("zb".into(), Some(Box::new(a.clone())))]
            );
            assert_eq!(delta1.take(), expected);

            let mut txn = d2.transact();
            d2.apply_update_v1(&mut txn, update.as_slice());
            txn.commit();

            assert_eq!(txt2.to_string(&txn), "zb".to_string());
            assert_eq!(delta2.take(), expected);
        }

        // step 5
        {
            let mut txn = d1.transact();
            txt1.insert(&mut txn, 0, "y");
            let update = txn.encode_update_v1();
            txn.commit();

            let expected = Some(vec![Delta::Inserted("y".into(), None)]);

            assert_eq!(txt1.to_string(&txn), "yzb".to_string());
            assert_eq!(
                txt1.diff(&mut txn),
                vec![
                    Diff::Insert("y".into(), None),
                    Diff::Insert("zb".into(), Some(Box::new(a.clone())))
                ]
            );
            assert_eq!(delta1.take(), expected);

            let mut txn = d2.transact();
            d2.apply_update_v1(&mut txn, update.as_slice());
            txn.commit();

            assert_eq!(txt2.to_string(&txn), "yzb".to_string());
            assert_eq!(delta2.take(), expected);
        }

        // step 6
        {
            let mut txn = d1.transact();
            let b: Attrs = HashMap::from([("bold".into(), Any::Null)]);
            txt1.format(&mut txn, 0, 2, b.clone());
            let update = txn.encode_update_v1();
            txn.commit();

            let expected = Some(vec![
                Delta::Retain(1, None),
                Delta::Retain(1, Some(Box::new(b))),
            ]);

            assert_eq!(txt1.to_string(&txn), "yzb".to_string());
            assert_eq!(
                txt1.diff(&mut txn),
                vec![
                    Diff::Insert("yz".into(), None),
                    Diff::Insert("b".into(), Some(Box::new(a.clone())))
                ]
            );
            assert_eq!(delta1.take(), expected);

            let mut txn = d2.transact();
            d2.apply_update_v1(&mut txn, update.as_slice());
            txn.commit();

            assert_eq!(txt2.to_string(&txn), "yzb".to_string());
            assert_eq!(delta2.take(), expected);
        }
    }

    #[test]
    fn embed_with_attributes() {
        let d1 = Doc::with_client_id(1);
        let txt1 = {
            let mut txn = d1.transact();
            txn.get_text("text")
        };

        let delta1 = Rc::new(RefCell::new(None));
        let delta_clone = delta1.clone();
        let _sub1 = txt1.observe(move |txn, e| {
            let delta = e.delta(txn).to_vec();
            delta_clone.replace(Some(delta));
        });

        {
            let mut txn = d1.transact();
            let a1: Attrs = HashMap::from([("bold".into(), true.into())]);
            txt1.insert_with_attributes(&mut txn, 0, "ab", a1.clone());

            let embed: Any = Any::Map(Box::new(HashMap::from([(
                "image".into(),
                "imageSrc.png".into(),
            )])));
            let a2: Attrs = HashMap::from([("width".into(), Any::BigInt(100))]);
            txt1.insert_embed_with_attributes(&mut txn, 1, embed.clone(), a2.clone());
            txn.commit();

            let a1 = Some(Box::new(a1));
            let a2 = Some(Box::new(a2));

            let expected = Some(vec![
                Delta::Inserted("a".into(), a1.clone()),
                Delta::Inserted(embed.clone().into(), a2.clone()),
                Delta::Inserted("b".into(), a1.clone()),
            ]);
            assert_eq!(delta1.take(), expected);

            let expected = vec![
                Diff::Insert("a".into(), a1.clone()),
                Diff::Insert(embed.into(), a2),
                Diff::Insert("b".into(), a1.clone()),
            ];
            let mut txn = d1.transact();
            assert_eq!(txt1.diff(&mut txn), expected);
        }
    }
}
