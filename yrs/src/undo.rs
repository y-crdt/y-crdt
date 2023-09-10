use crate::block::{Block, BlockPtr};
use crate::doc::{AfterTransactionSubscription, TransactionAcqError};
use crate::transaction::Origin;
use crate::types::{Branch, BranchPtr};
use crate::{
    DeleteSet, DestroySubscription, Doc, Observer, Subscription, SubscriptionId, Transact,
    TransactionMut, ID,
};
use std::cell::Cell;
use std::collections::HashSet;
use std::fmt::Formatter;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[repr(transparent)]
/// Undo manager is a structure used to perform undo/redo operations over the associated shared
/// type(s).
///
/// Undo-/redo-able actions (a.k.a. [StackItem]s) are not equivalent to [TransactionMut]
/// unit of work, but rather a series of updates batched within specified time intervals
/// (see: [Options::capture_timeout_millis]) and their corresponding origins
/// (see: [Doc::transact_mut_with] and [UndoManager::include_origin]).
///
/// Individual stack item boundaries can be also specified explicitly by calling [UndoManager::reset],
/// which denotes the end of the batch.
///
/// In order to revert an operation, call [UndoManager::undo], then [UndoManager::redo] to bring it
/// back.
///
/// Users can also subscribe to change notifications observed by undo manager:
/// - [UndoManager::observe_item_added], which is fired every time a new [StackItem] is created.
/// - [UndoManager::observe_item_updated], which is fired every time when an existing [StackItem]
///    had been extended due to new document changes arriving before capture timeout for that stack
///    item finished.
/// - [UndoManager::observe_item_popped], which is fired whenever [StackItem] is being from undo
///    manager as a result of calling either [UndoManager::undo] or [UndoManager::redo] method.
pub struct UndoManager(Box<Inner>);

struct Inner {
    doc: Doc,
    scope: HashSet<BranchPtr>,
    options: Options,
    undo_stack: Vec<StackItem>,
    redo_stack: Vec<StackItem>,
    undoing: Cell<bool>,
    redoing: Cell<bool>,
    last_change: u64,
    on_after_transaction: Option<AfterTransactionSubscription>,
    on_destroy: Option<DestroySubscription>,
    observer_added: Observer<Arc<dyn Fn(&TransactionMut, &Event) -> ()>>,
    observer_updated: Observer<Arc<dyn Fn(&TransactionMut, &Event) -> ()>>,
    observer_popped: Observer<Arc<dyn Fn(&TransactionMut, &Event) -> ()>>,
}

impl UndoManager {
    /// Creates a new instance of the [UndoManager] working in a `scope` of a particular shared
    /// type and document. While it's possible for undo manager to observe multiple shared types
    /// (see: [UndoManager::expand_scope]), it can only work with a single document at the same time.
    pub fn new<T>(doc: &Doc, scope: &T) -> Self
    where
        T: AsRef<Branch>,
    {
        Self::with_options(doc, scope, Options::default())
    }

    /// Creates a new instance of the [UndoManager] working in a `scope` of a particular shared
    /// type and document. While it's possible for undo manager to observe multiple shared types
    /// (see: [UndoManager::expand_scope]), it can only work with a single document at the same time.
    pub fn with_options<T>(doc: &Doc, scope: &T, options: Options) -> Self
    where
        T: AsRef<Branch>,
    {
        let scope = BranchPtr::from(scope.as_ref());
        let mut inner = Box::new(Inner {
            doc: doc.clone(),
            scope: HashSet::from([scope]),
            options,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            undoing: Cell::new(false),
            redoing: Cell::new(false),
            last_change: 0,
            on_after_transaction: None,
            on_destroy: None,
            observer_added: Observer::new(),
            observer_updated: Observer::new(),
            observer_popped: Observer::new(),
        });
        let inner_ptr = inner.as_mut() as *mut Inner;
        inner
            .options
            .tracked_origins
            .insert(Origin::from(inner_ptr as usize));
        inner.on_destroy = Some(
            doc.observe_destroy(move |_, _| Self::handle_destroy(inner_ptr))
                .unwrap(),
        );
        inner.on_after_transaction = Some(
            doc.observe_after_transaction(move |txn| {
                let inner = unsafe { inner_ptr.as_mut().unwrap() };
                Self::handle_after_transaction(inner, txn);
            })
            .unwrap(),
        );

        UndoManager(inner)
    }

    fn should_skip(inner: &Inner, txn: &TransactionMut) -> bool {
        !(inner.options.capture_transaction)(txn)
            || !inner
                .scope
                .iter()
                .any(|parent| txn.changed_parent_types.contains(parent))
            || !txn
                .origin()
                .map(|o| inner.options.tracked_origins.contains(o))
                .unwrap_or(inner.options.tracked_origins.len() == 1) // tracked origins contain only undo manager itself
    }

    fn handle_after_transaction(inner: &mut Inner, txn: &mut TransactionMut) {
        if Self::should_skip(inner, txn) {
            return;
        }
        let undoing = inner.undoing.get();
        let redoing = inner.redoing.get();
        if undoing {
            inner.last_change = 0; // next undo should not be appended to last stack item
        } else if !redoing {
            // neither undoing nor redoing: delete redoStack
            let len = inner.redo_stack.len();
            for item in inner.redo_stack.drain(0..len) {
                Self::clear_item(&inner.scope, txn, item);
            }
        }

        let mut insertions = DeleteSet::new();
        for (client, &end_clock) in txn.after_state().iter() {
            let start_clock = txn.before_state.get(client);
            let diff = end_clock - start_clock;
            if diff != 0 {
                insertions.insert(ID::new(*client, start_clock), diff);
            }
        }
        let now = (inner.options.timestamp)();
        let stack = if undoing {
            &mut inner.redo_stack
        } else {
            &mut inner.undo_stack
        };
        let extend = !undoing
            && !redoing
            && !stack.is_empty()
            && inner.last_change > 0
            && now - inner.last_change < inner.options.capture_timeout_millis;

        if extend {
            // append change to last stack op
            if let Some(last_op) = stack.last_mut() {
                // always true - we checked if stack is empty above
                last_op.deletions.merge(txn.delete_set.clone());
                last_op.insertions.merge(insertions);
            }
        } else {
            // create a new stack op
            let item = StackItem::new(txn.delete_set.clone(), insertions);
            stack.push(item);
        }

        if !undoing && !redoing {
            inner.last_change = now;
        }
        // make sure that deleted structs are not gc'd
        for ptr in txn.delete_set.clone().deleted_blocks(txn) {
            if ptr.is_item() && inner.scope.iter().any(|b| b.is_parent_of(Some(ptr))) {
                ptr.keep(true);
            }
        }

        let last_op = stack.last().unwrap().clone();
        let event = if undoing {
            Event::undo(
                last_op,
                txn.origin.clone(),
                txn.changed_parent_types.clone(),
            )
        } else {
            Event::redo(
                last_op,
                txn.origin.clone(),
                txn.changed_parent_types.clone(),
            )
        };
        if !extend {
            for cb in inner.observer_added.callbacks() {
                cb(txn, &event);
            }
        } else {
            for cb in inner.observer_updated.callbacks() {
                cb(txn, &event);
            }
        }
    }

    fn handle_destroy(inner: *mut Inner) {
        let origin = Origin::from(inner as usize);
        let inner = unsafe { inner.as_mut().unwrap() };
        if inner.options.tracked_origins.remove(&origin) {
            inner.on_destroy.take();
            inner.on_after_transaction.take();
        }
    }

    /// Registers a callback function to be called every time a new [StackItem] is created. This
    /// usually happens when a new update over an tracked shared type happened after capture timeout
    /// threshold from the previous stack item occurence has been reached or [UndoManager::reset]
    /// has been called.
    ///
    /// Returns a subscription object which - when dropped - will unregister provided callback.
    pub fn observe_item_added<F>(&self, f: F) -> UndoEventSubscription
    where
        F: Fn(&TransactionMut, &Event) -> () + 'static,
    {
        self.0.observer_added.subscribe(Arc::new(f))
    }

    /// Unsubscribes a callback previously registered using [UndoManager::observe_item_added].
    pub fn unobserve_item_added(&self, subscription_id: SubscriptionId) {
        self.0.observer_added.unsubscribe(subscription_id)
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// extended as a result of updates from tracked types which happened before a capture timeout
    /// has passed.
    ///
    /// Returns a subscription object which - when dropped - will unregister provided callback.
    pub fn observe_item_updated<F>(&self, f: F) -> UndoEventSubscription
    where
        F: Fn(&TransactionMut, &Event) -> () + 'static,
    {
        self.0.observer_updated.subscribe(Arc::new(f))
    }

    /// Unsubscribes a callback previously registered using [UndoManager::observe_item_updated].
    pub fn unobserve_item_updated(&self, subscription_id: SubscriptionId) {
        self.0.observer_updated.unsubscribe(subscription_id)
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// removed as a result of [UndoManager::undo] or [UndoManager::redo] method.
    ///
    /// Returns a subscription object which - when dropped - will unregister provided callback.
    pub fn observe_item_popped<F>(&self, f: F) -> UndoEventSubscription
    where
        F: Fn(&TransactionMut, &Event) -> () + 'static,
    {
        self.0.observer_popped.subscribe(Arc::new(f))
    }

    /// Unsubscribes a callback previously registered using [UndoManager::observe_item_popped].
    pub fn unobserve_item_popped(&self, subscription_id: SubscriptionId) {
        self.0.observer_popped.unsubscribe(subscription_id)
    }

    /// Extends a list of shared types tracked by current undo manager by a given `scope`.
    pub fn expand_scope<T>(&mut self, scope: &T)
    where
        T: AsRef<Branch>,
    {
        let ptr = BranchPtr::from(scope.as_ref());
        self.0.scope.insert(ptr);
    }

    /// Extends a list of origins tracked by current undo manager by given `origin`. Origin markers
    /// can be assigned to updates executing in a scope of a particular transaction
    /// (see: [Doc::transact_mut_with]).
    pub fn include_origin<O>(&mut self, origin: O)
    where
        O: Into<Origin>,
    {
        self.0.options.tracked_origins.insert(origin.into());
    }

    /// Removes an `origin` from the list of origins tracked by a current undo manager.
    pub fn exclude_origin<O>(&mut self, origin: O)
    where
        O: Into<Origin>,
    {
        self.0.options.tracked_origins.remove(&origin.into());
    }

    /// Clears all [StackItem]s stored within current UndoManager, effectively resetting its state.
    pub fn clear(&mut self) -> Result<(), TransactionAcqError> {
        let mut txn = self.0.doc.try_transact_mut()?;

        let len = self.0.undo_stack.len();
        for item in self.0.undo_stack.drain(0..len) {
            Self::clear_item(&self.0.scope, &mut txn, item);
        }

        let len = self.0.redo_stack.len();
        for item in self.0.redo_stack.drain(0..len) {
            Self::clear_item(&self.0.scope, &mut txn, item);
        }

        Ok(())
    }

    fn clear_item(scope: &HashSet<BranchPtr>, txn: &mut TransactionMut, stack_item: StackItem) {
        for ptr in stack_item.deletions.deleted_blocks(txn) {
            if ptr.is_item() && scope.iter().any(|b| b.is_parent_of(Some(ptr))) {
                ptr.keep(false);
            }
        }
    }

    pub fn as_origin(&self) -> Origin {
        let mgr_ptr: *const Inner = &*self.0;
        Origin::from(mgr_ptr as usize)
    }

    /// [UndoManager] merges undo stack items if they were created withing the time gap smaller than
    /// [Options::capture_timeout_millis]. You can call this method so that the next stack item won't be
    /// merged.
    ///
    /// Example:
    /// ```rust
    /// use yrs::{Doc, GetString, Text, Transact, UndoManager};
    /// let doc = Doc::new();
    ///
    /// // without UndoManager::stop
    /// let txt = doc.get_or_insert_text("no-stop");
    /// let mut mgr = UndoManager::new(&doc, &txt);
    /// txt.insert(&mut doc.transact_mut(), 0, "a");
    /// txt.insert(&mut doc.transact_mut(), 1, "b");
    /// mgr.undo().unwrap();
    /// txt.get_string(&doc.transact()); // => "" (note that 'ab' was removed)
    ///
    /// // with UndoManager::stop
    /// let txt = doc.get_or_insert_text("with-stop");
    /// let mut mgr = UndoManager::new(&doc, &txt);
    /// txt.insert(&mut doc.transact_mut(), 0, "a");
    /// mgr.reset();
    /// txt.insert(&mut doc.transact_mut(), 1, "b");
    /// mgr.undo().unwrap();
    /// txt.get_string(&doc.transact()); // => "a" (note that only 'b' was removed)
    /// ```
    pub fn reset(&mut self) {
        self.0.last_change = 0;
    }

    /// Are there any undo steps available?
    pub fn can_undo(&self) -> bool {
        !self.0.undo_stack.is_empty()
    }

    /// Undo last action tracked by current undo manager. Actions (a.k.a. [StackItem]s) are groups
    /// of updates performed in a given time range - they also can be separated explicitly by
    /// calling [UndoManager::reset].
    ///
    /// Successful execution returns a boolean value telling if an undo call has performed any changes.
    ///
    /// # Errors
    ///
    /// This method requires an exclusive access to underlying document store. This means that
    /// no other transaction on that same document can be active while calling this method.
    /// Otherwise an error will be returned.
    pub fn undo(&mut self) -> Result<bool, TransactionAcqError> {
        let mut txn = self.0.doc.try_transact_mut_with(self.as_origin())?;
        self.0.undoing.set(true);
        let result = Self::pop(&mut self.0.undo_stack, &mut txn, &self.0.scope);
        txn.commit();
        let changed = if let Some(item) = result {
            let e = Event::undo(
                item,
                Some(self.as_origin()),
                txn.changed_parent_types.clone(),
            );
            for cb in self.0.observer_popped.callbacks() {
                cb(&txn, &e);
            }
            true
        } else {
            false
        };
        self.0.undoing.set(false);
        Ok(changed)
    }

    /// Are there any redo steps available?
    pub fn can_redo(&self) -> bool {
        !self.0.redo_stack.is_empty()
    }

    /// Redo'es last action previously undo'ed by current undo manager. Actions
    /// (a.k.a. [StackItem]s) are groups of updates performed in a given time range - they also can
    /// be separated explicitly by calling [UndoManager::reset].
    ///
    /// Successful execution returns a boolean value telling if an undo call has performed any changes.
    ///
    /// # Errors
    ///
    /// This method requires an exclusive access to underlying document store. This means that
    /// no other transaction on that same document can be active while calling this method.
    /// Otherwise an error will be returned.
    pub fn redo(&mut self) -> Result<bool, TransactionAcqError> {
        let mut txn = self.0.doc.try_transact_mut_with(self.as_origin())?;
        self.0.redoing.set(true);
        let result = Self::pop(&mut self.0.redo_stack, &mut txn, &self.0.scope);
        txn.commit();
        let changed = if let Some(item) = result {
            let e = Event::redo(
                item,
                Some(self.as_origin()),
                txn.changed_parent_types.clone(),
            );
            for cb in self.0.observer_popped.callbacks() {
                cb(&txn, &e);
            }
            true
        } else {
            false
        };
        self.0.redoing.set(false);
        Ok(changed)
    }

    fn pop(
        stack: &mut Vec<StackItem>,
        txn: &mut TransactionMut,
        scope: &HashSet<BranchPtr>,
    ) -> Option<StackItem> {
        let mut result = None;
        while let Some(item) = stack.pop() {
            let mut to_redo = HashSet::<BlockPtr>::new();
            let mut to_delete = Vec::<BlockPtr>::new();
            let mut change_performed = false;

            let deleted: Vec<_> = item.insertions.deleted_blocks(txn).collect();
            for ptr in deleted {
                let mut ptr = ptr;
                if let Block::Item(item) = ptr.clone().deref() {
                    if item.redone.is_some() {
                        let mut id = *ptr.id();
                        let (block, diff) = txn.store().follow_redone(&id);
                        ptr = if diff > 0 {
                            id.clock += diff;
                            let slice = txn.store.blocks.get_item_clean_start(&id).unwrap();
                            txn.store.materialize(slice)
                        } else {
                            block
                        };
                    }
                }
                if let Block::Item(item) = ptr.clone().deref() {
                    if !item.is_deleted() && scope.iter().any(|b| b.is_parent_of(Some(ptr.clone())))
                    {
                        to_delete.push(ptr);
                    }
                }
            }

            for ptr in item.deletions.deleted_blocks(txn) {
                if ptr.is_item()
                    && scope.iter().any(|b| b.is_parent_of(Some(ptr.clone())))
                    && !item.insertions.is_deleted(ptr.id())
                // Never redo structs in stackItem.insertions because they were created and deleted in the same capture interval.
                {
                    to_redo.insert(ptr);
                }
            }

            let redo_copy = to_redo.clone();
            for ptr in to_redo {
                let mut ptr = ptr;
                change_performed |= ptr.redo(txn, &redo_copy, &item.insertions).is_some();
            }

            // We want to delete in reverse order so that children are deleted before
            // parents, so we have more information available when items are filtered.
            for &item in to_delete.iter().rev() {
                // if self.options.delete_filter(item) {
                txn.delete(item);
                change_performed = true;
            }

            if change_performed {
                result = Some(item);
                break;
            }
        }
        result
    }
}

impl std::fmt::Debug for UndoManager {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("UndoManager");
        s.field("scope", &self.0.scope);
        s.field("tracked_origins", &self.0.options.tracked_origins);
        if !self.0.undo_stack.is_empty() {
            s.field("undo", &self.0.undo_stack);
        }
        if !self.0.redo_stack.is_empty() {
            s.field("redo", &self.0.redo_stack);
        }
        s.finish()
    }
}

pub type UndoEventSubscription = Subscription<Arc<dyn Fn(&TransactionMut, &Event) -> ()>>;

/// Set of options used to configure [UndoManager].
#[derive(Clone)]
pub struct Options {
    /// Undo-/redo-able updates are grouped together in time-constrained snapshots. This field
    /// determines the period of time, every snapshot will be automatically made in.
    pub capture_timeout_millis: u64,

    /// List of origins tracked by corresponding [UndoManager].
    /// If provided, it will track only updates made within transactions of specific origin.
    /// If not provided, it will track only updates made within transaction with no origin defined.
    pub tracked_origins: HashSet<Origin>,

    /// Custom logic decider, that along with [tracked_origins] can be used to determine if
    /// transaction changes should be captured or not.
    pub capture_transaction: Rc<dyn Fn(&TransactionMut) -> bool>,

    /// Custom clock function, that can be used to generate timestamps used by
    /// [Options::capture_timeout_millis].
    pub timestamp: Rc<dyn Fn() -> u64>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            capture_timeout_millis: 500,
            tracked_origins: HashSet::new(),
            capture_transaction: Rc::new(|_txn| true),
            timestamp: Rc::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64
            }),
        }
    }
}

/// A unit of work for the [UndoManager]. It contains a compressed information about all updates and
/// deletions tracked by a corresponding undo manager. Whenever an [UndoManger::undo] or
/// [UndoManager::redo] methods are called a last [StackItem] is being used to modify a state of
/// the document.
///
/// Stack items are stored internally by undo manager and created automatically whenever a new
/// update from tracked shared type and transaction of tracked origin has been committed within
/// a threshold specified by [Options::capture_timeout_millis] time window since the previous stack
/// item creation. They can also be created explicitly by calling [UndoManager::reset], which marks
/// the end of the last stack item batch.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct StackItem {
    deletions: DeleteSet,
    insertions: DeleteSet,
}

impl StackItem {
    fn new(deletions: DeleteSet, insertions: DeleteSet) -> StackItem {
        StackItem {
            deletions,
            insertions,
        }
    }

    /// A descriptor of all IDs and ranges of the updates deleted within a scope of a current [StackItem].
    pub fn deletions(&self) -> &DeleteSet {
        &self.deletions
    }

    /// A descriptor of all IDs and ranges of the updates created within a scope of a current [StackItem].
    pub fn insertions(&self) -> &DeleteSet {
        &self.insertions
    }
}

impl std::fmt::Display for StackItem {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "StackItem(")?;
        if !self.deletions.is_empty() {
            write!(f, "-{}", self.deletions)?;
        }
        if !self.insertions.is_empty() {
            write!(f, "+{}", self.insertions)?;
        }
        write!(f, ")")
    }
}

#[derive(Debug, Clone)]
pub struct Event {
    pub item: StackItem,
    pub origin: Option<Origin>,
    pub kind: EventKind,
    pub changed_parent_types: Vec<BranchPtr>,
}

impl Event {
    fn undo(item: StackItem, origin: Option<Origin>, changed_parent_types: Vec<BranchPtr>) -> Self {
        Event {
            item,
            origin,
            changed_parent_types,
            kind: EventKind::Undo,
        }
    }

    fn redo(item: StackItem, origin: Option<Origin>, changed_parent_types: Vec<BranchPtr>) -> Self {
        Event {
            item,
            origin,
            changed_parent_types,
            kind: EventKind::Redo,
        }
    }

    pub fn has_changed<T: AsRef<Branch>>(&self, target: &T) -> bool {
        let ptr = BranchPtr::from(target.as_ref());
        self.changed_parent_types.contains(&ptr)
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq)]
pub enum EventKind {
    Undo,
    Redo,
}

#[cfg(test)]
mod test {
    use crate::test_utils::exchange_updates;
    use crate::types::text::{Diff, YChange};
    use crate::types::{Attrs, ToJson};
    use crate::undo::{Options, UndoManager};
    use crate::updates::decoder::Decode;
    use crate::{Array, Doc, GetString, Map, MapPrelim, MapRef, ReadTxn, StateVector, Text, TextPrelim, TextRef, Transact, Update, Xml, XmlElementPrelim, XmlElementRef, XmlFragment, XmlTextPrelim, Any, any};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::convert::TryInto;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn undo_text() {
        let d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("test");
        let mut mgr = UndoManager::new(&d1, &txt1);

        let d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("test");

        // items that are added & deleted in the same transaction won't be undo
        txt1.insert(&mut d1.transact_mut(), 0, "test");
        txt1.remove_range(&mut d1.transact_mut(), 0, 4);
        mgr.undo().unwrap();
        assert_eq!(txt1.get_string(&d1.transact()), "");

        // follow redone items
        txt1.insert(&mut d1.transact_mut(), 0, "a");
        mgr.reset();
        txt1.remove_range(&mut d1.transact_mut(), 0, 1);
        mgr.reset();
        mgr.undo().unwrap();
        assert_eq!(txt1.get_string(&d1.transact()), "a");
        mgr.undo().unwrap();
        assert_eq!(txt1.get_string(&d1.transact()), "");

        txt1.insert(&mut d1.transact_mut(), 0, "abc");
        txt2.insert(&mut d2.transact_mut(), 0, "xyz");

        exchange_updates(&[&d1, &d2]);
        mgr.undo().unwrap();
        assert_eq!(txt1.get_string(&d1.transact()), "xyz");
        mgr.redo().unwrap();
        assert_eq!(txt1.get_string(&d1.transact()), "abcxyz");

        exchange_updates(&[&d1, &d2]);

        txt2.remove_range(&mut d2.transact_mut(), 0, 1);

        exchange_updates(&[&d1, &d2]);

        mgr.undo().unwrap();
        assert_eq!(txt1.get_string(&d1.transact()), "xyz");
        mgr.redo().unwrap();
        assert_eq!(txt1.get_string(&d1.transact()), "bcxyz");

        // test marks
        let attrs = Attrs::from([("bold".into(), true.into())]);
        txt1.format(&mut d1.transact_mut(), 1, 3, attrs.clone());
        let diff = txt1.diff(&d1.transact(), YChange::identity);
        assert_eq!(
            diff,
            vec![
                Diff::new("b".into(), None),
                Diff::new("cxy".into(), Some(Box::new(attrs.clone()))),
                Diff::new("z".into(), None),
            ]
        );

        mgr.undo().unwrap();
        let diff = txt1.diff(&d1.transact(), YChange::identity);
        assert_eq!(diff, vec![Diff::new("bcxyz".into(), None)]);

        mgr.redo().unwrap();
        let diff = txt1.diff(&d1.transact(), YChange::identity);
        assert_eq!(
            diff,
            vec![
                Diff::new("b".into(), None),
                Diff::new("cxy".into(), Some(Box::new(attrs.clone()))),
                Diff::new("z".into(), None),
            ]
        );
    }

    #[test]
    fn double_undo() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        txt.insert(&mut doc.transact_mut(), 0, "1221");

        let mut mgr = UndoManager::new(&doc, &txt);
        txt.insert(&mut doc.transact_mut(), 2, "3");
        txt.insert(&mut doc.transact_mut(), 3, "3");

        mgr.undo().unwrap();
        mgr.undo().unwrap();

        txt.insert(&mut doc.transact_mut(), 2, "3");
        assert_eq!(txt.get_string(&doc.transact()), "12321");
    }

    #[test]
    fn undo_map() {
        let d1 = Doc::with_client_id(1);
        let map1 = d1.get_or_insert_map("test");

        let d2 = Doc::with_client_id(2);
        let map2 = d2.get_or_insert_map("test");

        map1.insert(&mut d1.transact_mut(), "a", 0);
        let mut mgr = UndoManager::new(&d1, &map1);
        map1.insert(&mut d1.transact_mut(), "a", 1);
        mgr.undo().unwrap();
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 0.into());
        mgr.redo().unwrap();
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 1.into());

        // testing sub-types and if it can restore a whole type
        let sub_type = map1.insert(&mut d1.transact_mut(), "a", MapPrelim::<u32>::new());
        sub_type.insert(&mut d1.transact_mut(), "x", 42);
        let actual = map1.to_json(&d1.transact());
        let expected = Any::from_json(r#"{ "a": { "x": 42 } }"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo().unwrap();
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 1.into());
        mgr.redo().unwrap();
        let actual = map1.to_json(&d1.transact());
        let expected = Any::from_json(r#"{ "a": { "x": 42 } }"#).unwrap();
        assert_eq!(actual, expected);

        exchange_updates(&[&d1, &d2]);

        // if content is overwritten by another user, undo operations should be skipped
        map2.insert(&mut d2.transact_mut(), "a", 44);

        exchange_updates(&[&d1, &d2]);
        exchange_updates(&[&d1, &d2]);

        mgr.undo().unwrap();
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 44.into());
        mgr.redo().unwrap();
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 44.into());

        // test setting value multiple times
        map1.insert(&mut d1.transact_mut(), "b", "initial");
        mgr.reset();
        map1.insert(&mut d1.transact_mut(), "b", "val1");
        map1.insert(&mut d1.transact_mut(), "b", "val2");
        mgr.reset();
        mgr.undo().unwrap();
        assert_eq!(map1.get(&d1.transact(), "b").unwrap(), "initial".into());
    }

    #[test]
    fn undo_array() {
        let d1 = Doc::with_client_id(1);
        let array1 = d1.get_or_insert_array("test");

        let d2 = Doc::with_client_id(2);
        let array2 = d2.get_or_insert_array("test");

        let mut mgr = UndoManager::new(&d1, &array1);
        array1.insert_range(&mut d1.transact_mut(), 0, [1, 2, 3]);
        array2.insert_range(&mut d2.transact_mut(), 0, [4, 5, 6]);

        exchange_updates(&[&d1, &d2]);

        assert_eq!(
            array1.to_json(&d1.transact()),
            vec![1, 2, 3, 4, 5, 6].into()
        );
        mgr.undo().unwrap();
        assert_eq!(array1.to_json(&d1.transact()), vec![4, 5, 6].into());
        mgr.redo().unwrap();
        assert_eq!(
            array1.to_json(&d1.transact()),
            vec![1, 2, 3, 4, 5, 6].into()
        );

        exchange_updates(&[&d1, &d2]);

        array2.remove_range(&mut d2.transact_mut(), 0, 1); // user2 deletes [1]

        exchange_updates(&[&d1, &d2]);

        mgr.undo().unwrap();
        assert_eq!(array1.to_json(&d1.transact()), vec![4, 5, 6].into());
        mgr.redo().unwrap();
        assert_eq!(array1.to_json(&d1.transact()), vec![2, 3, 4, 5, 6].into());
        array1.remove_range(&mut d1.transact_mut(), 0, 5);

        // test nested structure
        let map = array1.insert(&mut d1.transact_mut(), 0, MapPrelim::<u32>::new());
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{}]"#).unwrap();
        assert_eq!(actual, expected);
        mgr.reset();
        map.insert(&mut d1.transact_mut(), "a", 1);
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"a":1}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo().unwrap();
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo().unwrap();
        assert_eq!(array1.to_json(&d1.transact()), vec![2, 3, 4, 5, 6].into());

        mgr.redo().unwrap();
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.redo().unwrap();
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"a":1}]"#).unwrap();
        assert_eq!(actual, expected);

        exchange_updates(&[&d1, &d2]);

        let map2 = array2
            .get(&d2.transact(), 0)
            .unwrap()
            .cast::<MapRef>()
            .unwrap();
        map2.insert(&mut d2.transact_mut(), "b", 2);
        exchange_updates(&[&d1, &d2]);

        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"a":1,"b":2}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo().unwrap();
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"b":2}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo().unwrap();
        assert_eq!(array1.to_json(&d1.transact()), vec![2, 3, 4, 5, 6].into());

        mgr.redo().unwrap();
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"b":2}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.redo().unwrap();
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"a":1,"b":2}]"#).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn undo_xml() {
        let d1 = Doc::with_client_id(1);
        let xml1 = d1.get_or_insert_xml_element("undefined");

        let mut mgr = UndoManager::new(&d1, &xml1);
        let child = xml1.insert(&mut d1.transact_mut(), 0, XmlElementPrelim::empty("p"));
        let text_child = child.insert(&mut d1.transact_mut(), 0, XmlTextPrelim::new("content"));

        assert_eq!(
            xml1.get_string(&d1.transact()),
            "<undefined><p>content</p></undefined>"
        );
        // format textchild and revert that change
        mgr.reset();
        text_child.format(
            &mut d1.transact_mut(),
            3,
            4,
            Attrs::from([("bold".into(), any!({}))]),
        );
        assert_eq!(
            xml1.get_string(&d1.transact()),
            "<undefined><p>con<bold>tent</bold></p></undefined>"
        );
        mgr.undo().unwrap();
        assert_eq!(
            xml1.get_string(&d1.transact()),
            "<undefined><p>content</p></undefined>"
        );
        mgr.redo().unwrap();
        assert_eq!(
            xml1.get_string(&d1.transact()),
            "<undefined><p>con<bold>tent</bold></p></undefined>"
        );
        xml1.remove_range(&mut d1.transact_mut(), 0, 1);
        assert_eq!(xml1.get_string(&d1.transact()), "<undefined></undefined>");
        mgr.undo().unwrap();
        assert_eq!(
            xml1.get_string(&d1.transact()),
            "<undefined><p>con<bold>tent</bold></p></undefined>"
        );
    }

    #[test]
    fn undo_events() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        let mut mgr = UndoManager::new(&doc, &txt);

        let result = Arc::new(AtomicUsize::new(0));
        let counter = AtomicUsize::new(1);
        let data = Rc::new(RefCell::new(HashMap::new()));

        let txt_clone = txt.clone();
        let data_clone = data.clone();
        let _sub1 = mgr.observe_item_added(move |_, e| {
            assert!(e.has_changed(&txt_clone));
            let c = counter.fetch_add(1, Ordering::SeqCst);
            let mut data = data_clone.borrow_mut();
            data.insert(e.item.clone(), c);
        });

        let txt_clone = txt.clone();
        let data_clone = data.clone();
        let result_clone = result.clone();
        let _sub2 = mgr.observe_item_popped(move |_, e| {
            assert!(e.has_changed(&txt_clone));
            let data = data_clone.borrow();
            if let Some(&v) = data.get(&e.item) {
                result_clone.store(v, Ordering::Relaxed);
            }
        });

        txt.insert(&mut doc.transact_mut(), 0, "abc");
        mgr.undo().unwrap();
        assert_eq!(result.load(Ordering::SeqCst), 1);
        mgr.redo().unwrap();
        assert_eq!(result.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn undo_until_change_performed() {
        let d1 = Doc::with_client_id(1);
        let arr1 = d1.get_or_insert_array("array");

        let d2 = Doc::with_client_id(2);
        let arr2 = d2.get_or_insert_array("array");

        let map1a = arr1.push_back(
            &mut d1.transact_mut(),
            MapPrelim::from([("hello".to_owned(), "world".to_owned())]),
        );

        let map1b = arr1.push_back(
            &mut d1.transact_mut(),
            MapPrelim::from([("key".to_owned(), "value".to_owned())]),
        );

        exchange_updates(&[&d1, &d2]);

        let mut mgr1 = UndoManager::new(&d1, &arr1);
        mgr1.include_origin(d1.client_id());

        let mut mgr2 = UndoManager::new(&d2, &arr2);
        mgr2.include_origin(d2.client_id());

        map1b.insert(
            &mut d1.transact_mut_with(d1.client_id()),
            "key",
            "value modified",
        );

        exchange_updates(&[&d1, &d2]);
        mgr1.reset();

        map1a.insert(
            &mut d1.transact_mut_with(d1.client_id()),
            "hello",
            "world modified",
        );

        exchange_updates(&[&d1, &d2]);

        arr2.remove_range(&mut d2.transact_mut_with(d2.client_id()), 0, 1);

        exchange_updates(&[&d1, &d2]);
        mgr2.undo().unwrap();

        exchange_updates(&[&d1, &d2]);
        mgr1.undo().unwrap();
        exchange_updates(&[&d1, &d2]);

        assert_eq!(map1b.get(&d1.transact(), "key"), Some("value".into()));
    }

    #[test]
    fn nested_undo() {
        // This issue has been reported in https://github.com/yjs/yjs/issues/317
        let doc = Doc::with_options(crate::doc::Options {
            skip_gc: true,
            client_id: 1,
            ..crate::doc::Options::default()
        });
        let design = doc.get_or_insert_map("map");
        let mut mgr = UndoManager::with_options(&doc, &design, {
            let mut o = Options::default();
            o.capture_timeout_millis = 0;
            o
        });
        {
            let mut txn = doc.transact_mut();
            design.insert(
                &mut txn,
                "text",
                MapPrelim::from([(
                    "blocks",
                    MapPrelim::from([("text", "Type something".to_owned())]),
                )]),
            );
        }
        let text = design
            .get(&doc.transact(), "text")
            .unwrap()
            .cast::<MapRef>()
            .unwrap();

        {
            let mut txn = doc.transact_mut();
            text.insert(&mut txn, "blocks", MapPrelim::from([("text", "Something")]));
        }

        {
            let mut txn = doc.transact_mut();
            text.insert(
                &mut txn,
                "blocks",
                MapPrelim::from([("text", "Something else")]),
            );
        }

        assert_eq!(
            design.to_json(&doc.transact()),
            Any::from_json(r#"{ "text": { "blocks": { "text": "Something else" } } }"#).unwrap()
        );
        mgr.undo().unwrap();
        assert_eq!(
            design.to_json(&doc.transact()),
            Any::from_json(r#"{ "text": { "blocks": { "text": "Something" } } }"#).unwrap()
        );
        mgr.undo().unwrap();
        assert_eq!(
            design.to_json(&doc.transact()),
            Any::from_json(r#"{ "text": { "blocks": { "text": "Type something" } } }"#).unwrap()
        );
        mgr.undo().unwrap();
        assert_eq!(
            design.to_json(&doc.transact()),
            Any::from_json(r#"{}"#).unwrap()
        );
        mgr.redo().unwrap();
        assert_eq!(
            design.to_json(&doc.transact()),
            Any::from_json(r#"{ "text": { "blocks": { "text": "Type something" } } }"#).unwrap()
        );
        mgr.redo().unwrap();
        assert_eq!(
            design.to_json(&doc.transact()),
            Any::from_json(r#"{ "text": { "blocks": { "text": "Something" } } }"#).unwrap()
        );
        mgr.redo().unwrap();
        assert_eq!(
            design.to_json(&doc.transact()),
            Any::from_json(r#"{ "text": { "blocks": { "text": "Something else" } } }"#).unwrap()
        );
    }

    #[test]
    fn consecutive_redo_bug() {
        // https://github.com/yjs/yjs/issues/355
        let doc = Doc::with_client_id(1);
        let root = doc.get_or_insert_map("root");
        let mut mgr = UndoManager::new(&doc, &root);

        root.insert(
            &mut doc.transact_mut(),
            "a",
            MapPrelim::<i32>::from(HashMap::from([
                ("x".to_owned(), 0.into()),
                ("y".to_owned(), 0.into()),
            ])),
        );
        let point = root
            .get(&doc.transact(), "a")
            .unwrap()
            .cast::<MapRef>()
            .unwrap();
        mgr.reset();

        point.insert(&mut doc.transact_mut(), "x", 100);
        point.insert(&mut doc.transact_mut(), "y", 100);
        mgr.reset();

        point.insert(&mut doc.transact_mut(), "x", 200);
        point.insert(&mut doc.transact_mut(), "y", 200);
        mgr.reset();

        point.insert(&mut doc.transact_mut(), "x", 300);
        point.insert(&mut doc.transact_mut(), "y", 300);
        mgr.reset();

        let actual = point.to_json(&doc.transact());
        assert_eq!(actual, Any::from_json(r#"{"x":300,"y":300}"#).unwrap());

        mgr.undo().unwrap(); // x=200, y=200
        let actual = point.to_json(&doc.transact());
        assert_eq!(actual, Any::from_json(r#"{"x":200,"y":200}"#).unwrap());

        mgr.undo().unwrap(); // x=100, y=100
        let actual = point.to_json(&doc.transact());
        assert_eq!(actual, Any::from_json(r#"{"x":100,"y":100}"#).unwrap());

        mgr.undo().unwrap(); // x=0, y=0
        let actual = point.to_json(&doc.transact());
        assert_eq!(actual, Any::from_json(r#"{"x":0,"y":0}"#).unwrap());

        mgr.undo().unwrap(); // null
        assert_eq!(root.get(&doc.transact(), "a"), None);

        mgr.redo().unwrap(); // x=0, y=0
        let point = root
            .get(&doc.transact(), "a")
            .unwrap()
            .cast::<MapRef>()
            .unwrap();

        assert_eq!(actual, Any::from_json(r#"{"x":0,"y":0}"#).unwrap());

        mgr.redo().unwrap(); // x=100, y=100
        let actual = point.to_json(&doc.transact());
        assert_eq!(actual, Any::from_json(r#"{"x":100,"y":100}"#).unwrap());

        mgr.redo().unwrap(); // x=200, y=200
        let actual = point.to_json(&doc.transact());
        assert_eq!(actual, Any::from_json(r#"{"x":200,"y":200}"#).unwrap());

        mgr.redo().unwrap(); // x=300, y=300
        let actual = point.to_json(&doc.transact());
        assert_eq!(actual, Any::from_json(r#"{"x":300,"y":300}"#).unwrap());
    }

    #[test]
    fn undo_xml_bug() {
        // https://github.com/yjs/yjs/issues/304
        const ORIGIN: &str = "origin";
        let doc = Doc::with_client_id(1);
        let f = doc.get_or_insert_xml_fragment("t");
        let mut mgr = UndoManager::with_options(&doc, &f, {
            let mut o = Options::default();
            o.capture_timeout_millis = 0;
            o
        });
        mgr.include_origin(ORIGIN.clone());

        // create element
        {
            let mut txn = doc.transact_mut_with(ORIGIN.clone());
            let e = f.insert(&mut txn, 0, XmlElementPrelim::empty("test-node"));
            e.insert_attribute(&mut txn, "a", "100");
            e.insert_attribute(&mut txn, "b", "0");
        }

        // change one attribute
        {
            let mut txn = doc.transact_mut_with(ORIGIN.clone());
            let e: XmlElementRef = f.get(&txn, 0).unwrap().try_into().unwrap();
            e.insert_attribute(&mut txn, "a", "200");
        }

        // change both attributes
        {
            let mut txn = doc.transact_mut_with(ORIGIN.clone());
            let e: XmlElementRef = f.get(&txn, 0).unwrap().try_into().unwrap();
            e.insert_attribute(&mut txn, "a", "180");
            e.insert_attribute(&mut txn, "b", "50");
        }

        mgr.undo().unwrap();
        mgr.undo().unwrap();
        mgr.undo().unwrap();

        mgr.redo().unwrap();
        mgr.redo().unwrap();
        mgr.redo().unwrap();

        let str = f.get_string(&doc.transact());
        assert!(
            str == r#"<test-node a="180" b="50"></test-node>"#
                || str == r#"<test-node b="50" a="180"></test-node>"#
        );
    }

    #[test]
    fn undo_block_bug() {
        // https://github.com/yjs/yjs/issues/343
        let doc = Doc::with_options({
            let mut o = crate::doc::Options::default();
            o.client_id = 1;
            o.skip_gc = true;
            o
        });
        let design = doc.get_or_insert_map("map");
        let mut mgr = UndoManager::with_options(&doc, &design, {
            let mut o = Options::default();
            o.capture_timeout_millis = 0;
            o
        });
        let text = {
            let mut txn = doc.transact_mut();
            design.insert(
                &mut txn,
                "text",
                MapPrelim::from([(
                    "blocks",
                    MapPrelim::from([("text".to_owned(), "1".to_owned())]),
                )]),
            )
        };
        {
            let mut txn = doc.transact_mut();
            text.insert(
                &mut txn,
                "blocks",
                MapPrelim::from([("text".to_owned(), "1".to_owned())]),
            );
        }
        {
            let mut txn = doc.transact_mut();
            text.insert(
                &mut txn,
                "blocks",
                MapPrelim::from([("text".to_owned(), "3".to_owned())]),
            );
        }
        {
            let mut txn = doc.transact_mut();
            text.insert(
                &mut txn,
                "blocks",
                MapPrelim::from([("text".to_owned(), "4".to_owned())]),
            );
        }
        // {"text":{"blocks":{"text":"4"}}}
        mgr.undo().unwrap(); // {"text":{"blocks":{"3"}}}
        mgr.undo().unwrap(); // {"text":{"blocks":{"text":"2"}}}
        mgr.undo().unwrap(); // {"text":{"blocks":{"text":"1"}}}
        mgr.undo().unwrap(); // {}
        mgr.redo().unwrap(); // {"text":{"blocks":{"text":"1"}}}
        mgr.redo().unwrap(); // {"text":{"blocks":{"text":"2"}}}
        mgr.redo().unwrap(); // {"text":{"blocks":{"text":"3"}}}
        mgr.redo().unwrap(); // {"text":{}}
        let actual = design.to_json(&doc.transact());
        assert_eq!(
            actual,
            Any::from_json(r#"{"text":{"blocks":{"text":"4"}}}"#).unwrap()
        );
    }

    #[test]
    fn undo_delete_text_format() {
        // https://github.com/yjs/yjs/issues/392
        fn send(src: &Doc, dst: &Doc) {
            let update = Update::decode_v1(
                &src.transact()
                    .encode_state_as_update_v1(&StateVector::default()),
            )
            .unwrap();
            dst.transact_mut().apply_update(update)
        }

        let doc1 = Doc::with_client_id(1);
        let txt = doc1.get_or_insert_text("test");
        txt.insert(
            &mut doc1.transact_mut(),
            0,
            "Attack ships on fire off the shoulder of Orion.",
        ); // D1: 'Attack ships on fire off the shoulder of Orion.'
        let doc2 = Doc::with_client_id(2);
        let txt2 = doc2.get_or_insert_text("test");

        send(&doc1, &doc2); // D2: 'Attack ships on fire off the shoulder of Orion.'
        let mut mgr = UndoManager::new(&doc1, &txt);

        let attrs = Attrs::from([("bold".into(), true.into())]);
        txt.format(&mut doc1.transact_mut(), 13, 7, attrs.clone()); // D1: 'Attack ships <b>on fire</b> off the shoulder of Orion.'

        mgr.reset();

        send(&doc1, &doc2); // D2: 'Attack ships <b>on fire</b> off the shoulder of Orion.'

        let attrs2 = Attrs::from([("bold".into(), Any::Null)]);
        txt.format(&mut doc1.transact_mut(), 16, 4, attrs2.clone()); // D1: 'Attack ships <b>on </b>fire off the shoulder of Orion.'

        let expected = vec![
            Diff::new("Attack ships ".into(), None),
            Diff::new("on ".into(), Some(Box::new(attrs.clone()))),
            Diff::new("fire off the shoulder of Orion.".into(), None),
        ];
        let actual = txt.diff(&doc1.transact(), YChange::identity);
        assert_eq!(actual, expected);

        mgr.reset();
        send(&doc1, &doc2); // D2: 'Attack ships <b>on </b>fire off the shoulder of Orion.'

        mgr.undo().unwrap(); // D1: 'Attack ships <b>on fire</b> off the shoulder of Orion.'
        send(&doc1, &doc2); // D2: 'Attack ships <b>on fire</b> off the shoulder of Orion.'

        let expected = vec![
            Diff::new("Attack ships ".into(), None),
            Diff::new("on fire".into(), Some(Box::new(attrs))),
            Diff::new(" off the shoulder of Orion.".into(), None),
        ];
        let actual = txt.diff(&doc1.transact(), YChange::identity);
        assert_eq!(actual, expected);
        let actual = txt2.diff(&doc2.transact(), YChange::identity);
        assert_eq!(actual, expected);
    }

    #[test]
    fn special_deletion_case() {
        // https://github.com/yjs/yjs/issues/447
        const ORIGIN: &str = "undoable";
        let doc = Doc::with_client_id(1);
        let f = doc.get_or_insert_xml_fragment("test");
        let mut mgr = UndoManager::new(&doc, &f);
        mgr.include_origin(ORIGIN.clone());
        {
            let mut txn = doc.transact_mut();
            let e = f.insert(&mut txn, 0, XmlElementPrelim::empty("test"));
            e.insert_attribute(&mut txn, "a", "1");
            e.insert_attribute(&mut txn, "b", "2");
        }
        let s = f.get_string(&doc.transact());
        assert!(s == r#"<test a="1" b="2"></test>"# || s == r#"<test b="2" a="1"></test>"#);
        {
            // change attribute "b" and delete test-node
            let mut txn = doc.transact_mut_with(ORIGIN);
            let e: XmlElementRef = f.get(&txn, 0).unwrap().try_into().unwrap();
            e.insert_attribute(&mut txn, "b", "3");
            f.remove_range(&mut txn, 0, 1);
        }
        assert_eq!(f.get_string(&doc.transact()), "");

        mgr.undo().unwrap();
        let s = f.get_string(&doc.transact());
        assert!(s == r#"<test a="1" b="2"></test>"# || s == r#"<test b="2" a="1"></test>"#);
    }

    #[test]
    fn undo_in_embed() {
        let d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("test");
        let mut mgr = UndoManager::new(&d1, &txt1);

        let d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("test");

        let attrs = Attrs::from([("bold".into(), true.into())]);
        let nested = txt1.insert_embed_with_attributes(
            &mut d1.transact_mut(),
            0,
            TextPrelim::new("initial text"),
            attrs,
        );
        assert_eq!(
            nested.get_string(&d1.transact()),
            "initial text".to_string()
        );
        mgr.reset();
        {
            let mut txn = d1.transact_mut();
            let len = nested.len(&txn);
            nested.remove_range(&mut txn, 0, len);
        }
        nested.insert(&mut d1.transact_mut(), 0, "other text");
        assert_eq!(nested.get_string(&d1.transact()), "other text".to_string());
        mgr.undo().unwrap();
        assert_eq!(
            nested.get_string(&d1.transact()),
            "initial text".to_string()
        );

        exchange_updates(&[&d1, &d2]);
        let diff = txt2.diff(&d1.transact(), YChange::identity);
        let nested2 = diff[0].insert.clone().cast::<TextRef>().unwrap();
        assert_eq!(
            nested2.get_string(&d2.transact()),
            "initial text".to_string()
        );

        mgr.undo().unwrap();
        assert_eq!(nested.len(&d1.transact()), 0);
    }
}
