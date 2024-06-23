use std::collections::HashSet;
use std::fmt::Formatter;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

use crate::block::ItemPtr;
use crate::branch::{Branch, BranchPtr};
use crate::doc::TransactionAcqError;
use crate::iter::TxnIterator;
use crate::slice::BlockSlice;
use crate::sync::Clock;
use crate::transaction::Origin;
use crate::{DeleteSet, Doc, Observer, Subscription, Transact, TransactionMut, ID};

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
#[repr(transparent)]
#[derive(Clone)]
pub struct UndoManager<M>(Arc<Inner<M>>);

#[cfg(not(target_family = "wasm"))]
type UndoFn<M> = Box<dyn Fn(&TransactionMut, &mut Event<M>) + Send + Sync + 'static>;

#[cfg(target_family = "wasm")]
type UndoFn<M> = Box<dyn Fn(&TransactionMut, &mut Event<M>) + 'static>;

#[cfg(not(target_family = "wasm"))]
pub trait Meta: Default + Send + Sync {}
#[cfg(not(target_family = "wasm"))]
impl<M> Meta for M where M: Default + Send + Sync {}

#[cfg(target_family = "wasm")]
pub trait Meta: Default {}
#[cfg(target_family = "wasm")]
impl<M> Meta for M where M: Default {}

struct Inner<M> {
    doc: Doc,
    scope: HashSet<BranchPtr>,
    options: Options,
    undo_stack: UndoStack<M>,
    redo_stack: UndoStack<M>,
    undoing: bool,
    redoing: bool,
    last_change: u64,
    observer_added: Observer<UndoFn<M>>,
    observer_updated: Observer<UndoFn<M>>,
    observer_popped: Observer<UndoFn<M>>,
}

impl<M> UndoManager<M>
where
    M: Meta + 'static,
{
    /// Creates a new instance of the [UndoManager] working in a `scope` of a particular shared
    /// type and document. While it's possible for undo manager to observe multiple shared types
    /// (see: [UndoManager::expand_scope]), it can only work with a single document at the same time.
    #[cfg(not(target_family = "wasm"))]
    pub fn new<T>(doc: &Doc, scope: &T) -> Self
    where
        T: AsRef<Branch>,
    {
        Self::with_options(doc, scope, Options::default())
    }

    #[inline]
    pub fn doc(&self) -> &Doc {
        &self.0.doc
    }

    #[inline]
    fn inner(&mut self) -> &mut Inner<M> {
        Arc::get_mut(&mut self.0).unwrap()
    }

    /// Creates a new instance of the [UndoManager] working in a `scope` of a particular shared
    /// type and document. While it's possible for undo manager to observe multiple shared types
    /// (see: [UndoManager::expand_scope]), it can only work with a single document at the same time.
    pub fn with_options<T>(doc: &Doc, scope: &T, options: Options) -> Self
    where
        T: AsRef<Branch>,
    {
        let scope = BranchPtr::from(scope.as_ref());
        let mut inner = Arc::new(Inner {
            doc: doc.clone(),
            scope: HashSet::from([scope]),
            options,
            undo_stack: UndoStack::default(),
            redo_stack: UndoStack::default(),
            undoing: false,
            redoing: false,
            last_change: 0,
            observer_added: Observer::new(),
            observer_updated: Observer::new(),
            observer_popped: Observer::new(),
        });
        let origin = Origin::from(Arc::as_ptr(&inner) as usize);
        let inner_mut = Arc::get_mut(&mut inner).unwrap();
        inner_mut.options.tracked_origins.insert(origin.clone());
        let ptr = AtomicPtr::new(inner_mut as *mut Inner<M>);

        doc.observe_destroy_with(origin.clone(), move |txn, _| {
            let ptr = ptr.load(Ordering::Acquire);
            let inner = unsafe { ptr.as_mut().unwrap() };
            Self::handle_destroy(txn, inner)
        })
        .unwrap();
        let ptr = AtomicPtr::new(inner_mut as *mut Inner<M>);

        doc.observe_after_transaction_with(origin, move |txn| {
            let ptr = ptr.load(Ordering::Acquire);
            let inner = unsafe { ptr.as_mut().unwrap() };
            Self::handle_after_transaction(inner, txn);
        })
        .unwrap();

        UndoManager(inner)
    }

    fn should_skip(inner: &Inner<M>, txn: &TransactionMut) -> bool {
        if let Some(capture_transaction) = &inner.options.capture_transaction {
            if !capture_transaction(txn) {
                return true;
            }
        }
        !inner
            .scope
            .iter()
            .any(|parent| txn.changed_parent_types.contains(parent))
            || !txn
                .origin()
                .map(|o| inner.options.tracked_origins.contains(o))
                .unwrap_or(inner.options.tracked_origins.len() == 1) // tracked origins contain only undo manager itself
    }

    fn handle_after_transaction(inner: &mut Inner<M>, txn: &mut TransactionMut) {
        if Self::should_skip(inner, txn) {
            return;
        }
        let undoing = inner.undoing;
        let redoing = inner.redoing;
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
        let now = inner.options.timestamp.now();
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
        let ds = txn.delete_set.clone();
        let mut deleted = ds.deleted_blocks();
        while let Some(slice) = deleted.next(txn) {
            if let Some(item) = slice.as_item() {
                if inner.scope.iter().any(|b| b.is_parent_of(Some(item))) {
                    item.keep(true);
                }
            }
        }

        let last_op = stack.last_mut().unwrap();
        let meta = std::mem::take(&mut last_op.meta);
        let mut event = if undoing {
            Event::undo(meta, txn.origin.clone(), txn.changed_parent_types.clone())
        } else {
            Event::redo(meta, txn.origin.clone(), txn.changed_parent_types.clone())
        };
        if !extend {
            if inner.observer_added.has_subscribers() {
                inner.observer_added.trigger(|fun| fun(txn, &mut event));
            }
        } else {
            if inner.observer_updated.has_subscribers() {
                inner.observer_updated.trigger(|fun| fun(txn, &mut event));
            }
        }
        last_op.meta = event.meta;
    }

    fn handle_destroy(txn: &TransactionMut, inner: &mut Inner<M>) {
        let origin = Origin::from(inner as *mut Inner<M> as usize);
        if inner.options.tracked_origins.remove(&origin) {
            if let Some(events) = txn.events() {
                events.destroy_events.unsubscribe(&origin);
                events.after_transaction_events.unsubscribe(&origin);
            }
        }
    }

    /// Registers a callback function to be called every time a new [StackItem] is created. This
    /// usually happens when a new update over an tracked shared type happened after capture timeout
    /// threshold from the previous stack item occurence has been reached or [UndoManager::reset]
    /// has been called.
    ///
    /// Returns a subscription object which - when dropped - will unregister provided callback.
    #[cfg(not(target_family = "wasm"))]
    pub fn observe_item_added<F>(&self, f: F) -> Subscription
    where
        F: Fn(&TransactionMut, &mut Event<M>) + Send + Sync + 'static,
    {
        self.0.observer_added.subscribe(Box::new(f))
    }

    /// Registers a callback function to be called every time a new [StackItem] is created. This
    /// usually happens when a new update over an tracked shared type happened after capture timeout
    /// threshold from the previous stack item occurence has been reached or [UndoManager::reset]
    /// has been called.
    ///
    /// Provided `key` is used to identify the origin of the callback. It can be used to unregister
    /// the callback later on.
    #[cfg(not(target_family = "wasm"))]
    pub fn observe_item_added_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &mut Event<M>) + Send + Sync + 'static,
    {
        self.0
            .observer_added
            .subscribe_with(key.into(), Box::new(f))
    }

    /// Registers a callback function to be called every time a new [StackItem] is created. This
    /// usually happens when a new update over an tracked shared type happened after capture timeout
    /// threshold from the previous stack item occurence has been reached or [UndoManager::reset]
    /// has been called.
    ///
    /// Provided `key` is used to identify the origin of the callback. It can be used to unregister
    /// the callback later on.
    #[cfg(target_family = "wasm")]
    pub fn observe_item_added_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &mut Event<M>) + 'static,
    {
        self.0
            .observer_added
            .subscribe_with(key.into(), Box::new(f))
    }

    pub fn unobserve_item_added<K>(&self, key: K)
    where
        K: Into<Origin>,
    {
        self.0.observer_added.unsubscribe(&key.into())
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// extended as a result of updates from tracked types which happened before a capture timeout
    /// has passed.
    ///
    /// Returns a subscription object which - when dropped - will unregister provided callback.
    #[cfg(not(target_family = "wasm"))]
    pub fn observe_item_updated<F>(&self, f: F) -> Subscription
    where
        F: Fn(&TransactionMut, &mut Event<M>) + Send + Sync + 'static,
    {
        self.0.observer_updated.subscribe(Box::new(f))
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// extended as a result of updates from tracked types which happened before a capture timeout
    /// has passed.
    ///
    /// Provided `key` is used to identify the origin of the callback. It can be used to unregister
    /// the callback later on.
    #[cfg(not(target_family = "wasm"))]
    pub fn observe_item_updated_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &mut Event<M>) + Send + Sync + 'static,
    {
        self.0
            .observer_updated
            .subscribe_with(key.into(), Box::new(f))
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// extended as a result of updates from tracked types which happened before a capture timeout
    /// has passed.
    ///
    /// Provided `key` is used to identify the origin of the callback. It can be used to unregister
    /// the callback later on.
    #[cfg(target_family = "wasm")]
    pub fn observe_item_updated_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &mut Event<M>) + 'static,
    {
        self.0
            .observer_updated
            .subscribe_with(key.into(), Box::new(f))
    }

    pub fn unobserve_item_updated<K>(&self, key: K)
    where
        K: Into<Origin>,
    {
        self.0.observer_updated.unsubscribe(&key.into())
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// removed as a result of [UndoManager::undo] or [UndoManager::redo] method.
    ///
    /// Returns a subscription object which - when dropped - will unregister provided callback.
    #[cfg(not(target_family = "wasm"))]
    pub fn observe_item_popped<F>(&self, f: F) -> Subscription
    where
        F: Fn(&TransactionMut, &mut Event<M>) + Send + Sync + 'static,
    {
        self.0.observer_popped.subscribe(Box::new(f))
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// removed as a result of [UndoManager::undo] or [UndoManager::redo] method.
    ///
    /// Provided `key` is used to identify the origin of the callback. It can be used to unregister
    /// the callback later on.
    #[cfg(not(target_family = "wasm"))]
    pub fn observe_item_popped_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &mut Event<M>) + Send + Sync + 'static,
    {
        self.0
            .observer_popped
            .subscribe_with(key.into(), Box::new(f))
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// removed as a result of [UndoManager::undo] or [UndoManager::redo] method.
    ///
    /// Provided `key` is used to identify the origin of the callback. It can be used to unregister
    /// the callback later on.
    #[cfg(target_family = "wasm")]
    pub fn observe_item_popped_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &mut Event<M>) + 'static,
    {
        self.0
            .observer_popped
            .subscribe_with(key.into(), Box::new(f))
    }

    pub fn unobserve_item_popped<K>(&self, key: K)
    where
        K: Into<Origin>,
    {
        self.0.observer_popped.unsubscribe(&key.into())
    }

    /// Extends a list of shared types tracked by current undo manager by a given `scope`.
    pub fn expand_scope<T>(&mut self, scope: &T)
    where
        T: AsRef<Branch>,
    {
        let ptr = BranchPtr::from(scope.as_ref());
        let inner = self.inner();
        inner.scope.insert(ptr);
    }

    /// Extends a list of origins tracked by current undo manager by given `origin`. Origin markers
    /// can be assigned to updates executing in a scope of a particular transaction
    /// (see: [Doc::transact_mut_with]).
    pub fn include_origin<O>(&mut self, origin: O)
    where
        O: Into<Origin>,
    {
        let inner = self.inner();
        inner.options.tracked_origins.insert(origin.into());
    }

    /// Removes an `origin` from the list of origins tracked by a current undo manager.
    pub fn exclude_origin<O>(&mut self, origin: O)
    where
        O: Into<Origin>,
    {
        let inner = self.inner();
        inner.options.tracked_origins.remove(&origin.into());
    }

    /// Clears all [StackItem]s stored within current UndoManager, effectively resetting its state.
    pub fn clear(&mut self) -> Result<(), TransactionAcqError> {
        let inner = self.inner();
        let mut txn = inner.doc.try_transact_mut()?;

        let len = inner.undo_stack.len();
        for item in inner.undo_stack.drain(0..len) {
            Self::clear_item(&inner.scope, &mut txn, item);
        }

        let len = inner.redo_stack.len();
        for item in inner.redo_stack.drain(0..len) {
            Self::clear_item(&inner.scope, &mut txn, item);
        }

        Ok(())
    }

    fn clear_item(scope: &HashSet<BranchPtr>, txn: &mut TransactionMut, stack_item: StackItem<M>) {
        let mut deleted = stack_item.deletions.deleted_blocks();
        while let Some(slice) = deleted.next(txn) {
            if let Some(item) = slice.as_item() {
                if scope.iter().any(|b| b.is_parent_of(Some(item))) {
                    item.keep(false);
                }
            }
        }
    }

    pub fn as_origin(&self) -> Origin {
        let mgr_ptr: *const Inner<M> = &*self.0;
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
        let inner = self.inner();
        inner.last_change = 0;
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
        let origin = self.as_origin();
        let inner = self.inner();
        let mut txn = inner.doc.try_transact_mut_with(origin.clone())?;
        inner.undoing = true;
        let result = Self::pop(
            &mut inner.undo_stack,
            &inner.redo_stack,
            &mut txn,
            &inner.scope,
        );
        txn.commit();
        let changed = if let Some(item) = result {
            let mut e = Event::undo(item.meta, Some(origin), txn.changed_parent_types.clone());
            if inner.observer_popped.has_subscribers() {
                inner.observer_popped.trigger(|fun| fun(&txn, &mut e));
            }
            true
        } else {
            false
        };
        inner.undoing = false;
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
        let origin = self.as_origin();
        let inner = self.inner();
        let mut txn = inner.doc.try_transact_mut_with(origin.clone())?;
        inner.redoing = true;
        let result = Self::pop(
            &mut inner.redo_stack,
            &inner.undo_stack,
            &mut txn,
            &inner.scope,
        );
        txn.commit();
        let changed = if let Some(item) = result {
            let mut e = Event::redo(item.meta, Some(origin), txn.changed_parent_types.clone());
            if inner.observer_popped.has_subscribers() {
                inner.observer_popped.trigger(|fun| fun(&txn, &mut e));
            }
            true
        } else {
            false
        };
        inner.redoing = false;
        Ok(changed)
    }

    fn pop(
        stack: &mut UndoStack<M>,
        other: &UndoStack<M>,
        txn: &mut TransactionMut,
        scope: &HashSet<BranchPtr>,
    ) -> Option<StackItem<M>> {
        let mut result = None;
        while let Some(item) = stack.pop() {
            let mut to_redo = HashSet::<ItemPtr>::new();
            let mut to_delete = Vec::<ItemPtr>::new();
            let mut change_performed = false;

            let deleted: Vec<_> = item.insertions.deleted_blocks().collect(txn);
            for slice in deleted {
                if let BlockSlice::Item(slice) = slice {
                    let mut item = txn.store.materialize(slice);
                    if item.redone.is_some() {
                        let slice = txn.store_mut().follow_redone(item.id())?;
                        item = txn.store.materialize(slice);
                    }

                    if !item.is_deleted() && scope.iter().any(|b| b.is_parent_of(Some(item))) {
                        to_delete.push(item);
                    }
                }
            }

            let mut deleted = item.deletions.deleted_blocks();
            while let Some(slice) = deleted.next(txn) {
                if let BlockSlice::Item(slice) = slice {
                    let ptr = txn.store.materialize(slice);
                    if scope.iter().any(|b| b.is_parent_of(Some(ptr)))
                        && !item.insertions.is_deleted(ptr.id())
                    // Never redo structs in stackItem.insertions because they were created and deleted in the same capture interval.
                    {
                        to_redo.insert(ptr);
                    }
                }
            }

            for &ptr in to_redo.iter() {
                let mut ptr = ptr;
                change_performed |= ptr
                    .redo(txn, &to_redo, &item.insertions, stack, other)
                    .is_some();
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

impl<M: std::fmt::Debug> std::fmt::Debug for UndoManager<M> {
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

impl<M> Drop for UndoManager<M> {
    fn drop(&mut self) {
        let inner = &self.0;
        let origin = Origin::from(Arc::as_ptr(&inner) as usize);
        inner.doc.unobserve_destroy(origin.clone()).unwrap();
        inner.doc.unobserve_after_transaction(origin).unwrap();
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub(crate) struct UndoStack<M>(Vec<StackItem<M>>);

impl<M> Deref for UndoStack<M> {
    type Target = Vec<StackItem<M>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<M> DerefMut for UndoStack<M> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<M> UndoStack<M> {
    pub fn is_deleted(&self, id: &ID) -> bool {
        for item in self.0.iter() {
            if item.deletions.is_deleted(id) {
                return true;
            }
        }
        false
    }
}

/// Set of options used to configure [UndoManager].
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
    pub capture_transaction: Option<CaptureTransactionFn>,

    /// Custom clock function, that can be used to generate timestamps used by
    /// [Options::capture_timeout_millis].
    pub timestamp: Arc<dyn Clock>,
}

pub type CaptureTransactionFn = Arc<dyn Fn(&TransactionMut) -> bool + Send + Sync + 'static>;

#[cfg(not(target_family = "wasm"))]
impl Default for Options {
    fn default() -> Self {
        Options {
            capture_timeout_millis: 500,
            tracked_origins: HashSet::new(),
            capture_transaction: None,
            timestamp: Arc::new(crate::sync::time::SystemClock),
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
pub struct StackItem<T> {
    deletions: DeleteSet,
    insertions: DeleteSet,

    /// A custom user metadata that can be attached to a particular [StackItem]. It can be used
    /// to carry over the additional information (such as ie. user cursor position) between
    /// undo/redo operations.
    pub meta: T,
}

impl<M: Default> StackItem<M> {
    fn new(deletions: DeleteSet, insertions: DeleteSet) -> Self {
        StackItem {
            deletions,
            insertions,
            meta: M::default(),
        }
    }

    /// A set of identifiers of element deleted at part of the timeframe current [StackItem] is
    /// responsible for.
    pub fn deletions(&self) -> &DeleteSet {
        &self.deletions
    }

    /// A set of identifiers of element inserted at part of the timeframe current [StackItem] is
    /// responsible for.
    pub fn insertions(&self) -> &DeleteSet {
        &self.insertions
    }
}

impl<M> std::fmt::Display for StackItem<M> {
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

#[derive(Debug)]
pub struct Event<M> {
    meta: M,
    origin: Option<Origin>,
    kind: EventKind,
    changed_parent_types: Vec<BranchPtr>,
}

impl<M> Event<M> {
    fn undo(meta: M, origin: Option<Origin>, changed_parent_types: Vec<BranchPtr>) -> Self {
        Event {
            meta,
            origin,
            changed_parent_types,
            kind: EventKind::Undo,
        }
    }

    fn redo(meta: M, origin: Option<Origin>, changed_parent_types: Vec<BranchPtr>) -> Self {
        Event {
            meta,
            origin,
            changed_parent_types,
            kind: EventKind::Redo,
        }
    }

    pub fn meta(&self) -> &M {
        &self.meta
    }

    pub fn meta_mut(&mut self) -> &mut M {
        &mut self.meta
    }

    /// Checks if given shared collection has changed in the scope of currently notified update.
    pub fn has_changed<T: AsRef<Branch>>(&self, target: &T) -> bool {
        let ptr = BranchPtr::from(target.as_ref());
        self.changed_parent_types.contains(&ptr)
    }

    /// Returns a transaction origin related to this update notification.
    pub fn origin(&self) -> Option<&Origin> {
        self.origin.as_ref()
    }

    /// Returns an enum informing if current update is result of undo or redo operation.
    pub fn kind(&self) -> EventKind {
        self.kind
    }

    /// Returns info about all changed shared collections.
    pub fn changed_parent_types(&self) -> &[BranchPtr] {
        &self.changed_parent_types
    }
}

/// Enum which informs if correlated [Event] was produced as a result of either undo or redo
/// operation over [UndoManager].
#[repr(u8)]
#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq)]
pub enum EventKind {
    /// Referenced event was result of [UndoManager::undo] operation.
    Undo,
    /// Referenced event was result of [UndoManager::redo] operation.
    Redo,
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use std::convert::TryInto;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crate::test_utils::exchange_updates;
    use crate::types::text::{Diff, YChange};
    use crate::types::{Attrs, ToJson};
    use crate::undo::Options;
    use crate::updates::decoder::Decode;
    use crate::{
        any, Any, Array, ArrayPrelim, Doc, GetString, Map, MapPrelim, MapRef, ReadTxn, StateVector,
        Text, TextPrelim, TextRef, Transact, UndoManager, Update, Xml, XmlElementPrelim,
        XmlElementRef, XmlFragment, XmlTextPrelim,
    };

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
        let frag = d1.get_or_insert_xml_fragment("xml");
        let xml1 = frag.insert(
            &mut d1.transact_mut(),
            0,
            XmlElementPrelim::empty("undefined"),
        );

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
        use crate::undo::UndoManager;
        type Metadata = HashMap<String, usize>;

        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        let mut mgr: UndoManager<Metadata> = UndoManager::new(&doc, &txt);

        let result = Arc::new(AtomicUsize::new(0));
        let counter = AtomicUsize::new(1);

        let txt_clone = txt.clone();
        let _sub1 = mgr.observe_item_added(move |_, e| {
            assert!(e.has_changed(&txt_clone));
            let c = counter.fetch_add(1, Ordering::SeqCst);
            let e = e.meta_mut().entry("test".to_string()).or_default();
            *e = c;
        });

        let txt_clone = txt.clone();
        let result_clone = result.clone();
        let _sub2 = mgr.observe_item_popped(move |_, e| {
            assert!(e.has_changed(&txt_clone));
            if let Some(&v) = e.meta_mut().get("test") {
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
        mgr.include_origin(ORIGIN);

        // create element
        {
            let mut txn = doc.transact_mut_with(ORIGIN);
            let e = f.insert(&mut txn, 0, XmlElementPrelim::empty("test-node"));
            e.insert_attribute(&mut txn, "a", "100");
            e.insert_attribute(&mut txn, "b", "0");
        }

        // change one attribute
        {
            let mut txn = doc.transact_mut_with(ORIGIN);
            let e: XmlElementRef = f.get(&txn, 0).unwrap().try_into().unwrap();
            e.insert_attribute(&mut txn, "a", "200");
        }

        // change both attributes
        {
            let mut txn = doc.transact_mut_with(ORIGIN);
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
        mgr.include_origin(ORIGIN);
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

    #[test]
    fn github_issue_345() {
        // https://github.com/y-crdt/y-crdt/issues/345
        let doc = Doc::new();
        let map = doc.get_or_insert_map("r");
        let mut mgr = UndoManager::with_options(&doc, &map, Options::default());
        mgr.include_origin(doc.client_id());

        let s1 = map.insert(
            &mut doc.transact_mut_with(doc.client_id()),
            "s1",
            MapPrelim::<i32>::new(),
        );
        mgr.reset();

        let mut txn = doc.transact_mut_with(doc.client_id());
        let a = s1.insert(&mut txn, "a", ArrayPrelim::default());
        a.insert(&mut txn, 0, "a1");
        drop(txn);
        mgr.reset();

        let mut txn = doc.transact_mut_with(doc.client_id());
        let b = s1.insert(&mut txn, "b", ArrayPrelim::default());
        b.insert(&mut txn, 0, "b1");
        drop(txn);
        mgr.reset();

        b.insert(&mut doc.transact_mut_with(doc.client_id()), 1, "b2");
        mgr.reset();

        a.insert(&mut doc.transact_mut_with(doc.client_id()), 1, "a3");
        mgr.reset();

        a.insert(&mut doc.transact_mut_with(doc.client_id()), 2, "a4");
        mgr.reset();

        a.insert(&mut doc.transact_mut_with(doc.client_id()), 3, "a5");
        mgr.reset();

        let actual = map.to_json(&doc.transact());
        assert_eq!(
            actual,
            any!({"s1": {"a": ["a1", "a3", "a4", "a5"], "b": ["b1", "b2"]}})
        );

        mgr.undo().unwrap(); // {"s1": {"a": ["a1", "a3", "a4"], "b": ["b1", "b2"]}}
        mgr.undo().unwrap(); // {"s1": {"a": ["a1", "a3"], "b": ["b1", "b2"]}}
        mgr.undo().unwrap(); // {"s1": {"a": ["a1"], "b": ["b1", "b2"]}}
        mgr.undo().unwrap(); // {"s1": {"a": ["a1"], "b": ["b1"]}}
        let actual = map.to_json(&doc.transact());
        assert_eq!(actual, any!({"s1": {"a": ["a1"], "b": ["b1"]}}));

        mgr.redo().unwrap();
        let actual = map.to_json(&doc.transact());
        assert_eq!(actual, any!({"s1": {"b": ["b1", "b2"], "a": ["a1"]}}));

        mgr.redo().unwrap();
        let actual = map.to_json(&doc.transact());
        assert_eq!(actual, any!({"s1": {"a": ["a1", "a3"], "b": ["b1", "b2"]}}));

        mgr.redo().unwrap();
        let actual = map.to_json(&doc.transact());
        assert_eq!(
            actual,
            any!({"s1": {"a": ["a1", "a3", "a4"], "b": ["b1", "b2"]}})
        );

        mgr.redo().unwrap();
        let actual = map.to_json(&doc.transact());
        assert_eq!(
            actual,
            any!({"s1": {"a": ["a1", "a3", "a4", "a5"], "b": ["b1", "b2"]}})
        );
    }

    #[test]
    fn github_issue_345_part_2() {
        // https://github.com/y-crdt/y-crdt/issues/345
        let d = Doc::new();
        let r = d.get_or_insert_map("r");

        let s1 = r.insert(&mut d.transact_mut(), "s1", MapPrelim::<i32>::new());

        let mut mgr = UndoManager::with_options(&d, &r, Options::default());
        {
            let mut txn = d.transact_mut();
            let b1 = s1.insert(&mut txn, "b1", MapPrelim::<i32>::new());
            b1.insert(&mut txn, "f1", 11);
        }
        mgr.reset();

        s1.remove(&mut d.transact_mut(), "b1");
        mgr.reset();

        {
            let mut txn = d.transact_mut();
            let b1 = s1.insert(&mut txn, "b1", MapPrelim::<i32>::new());
            b1.insert(&mut txn, "f1", 20);
        }
        mgr.reset();

        let actual = r.to_json(&d.transact());
        assert_eq!(actual, any!({"s1": {"b1": {"f1": 20}}}));

        mgr.undo().unwrap();
        let actual = r.to_json(&d.transact());
        assert_eq!(actual, any!({"s1": {}}));
        assert!(mgr.can_undo(), "should be able to undo");

        mgr.undo().unwrap();
        let actual = r.to_json(&d.transact());
        assert_eq!(actual, any!({"s1": {"b1": {"f1": 11}}}));
        assert!(mgr.can_undo(), "should be able to undo to the init state");
    }

    #[test]
    fn issue_371() {
        let doc = Doc::with_client_id(1);

        let r = doc.get_or_insert_map("r");
        let s1 = r.insert(&mut doc.transact_mut(), "s1", MapPrelim::<i32>::new()); // { s1: {} }
        let b1_arr = s1.insert(&mut doc.transact_mut(), "b1", ArrayPrelim::default()); // { s1: { b1: [] } }
        let el1 = b1_arr.insert(&mut doc.transact_mut(), 0, MapPrelim::<i32>::new()); // { s1: { b1: [{}] } }
        el1.insert(&mut doc.transact_mut(), "f1", 8); // { s1: { b1: [{ f1: 8 }] } }
        el1.insert(&mut doc.transact_mut(), "f2", true); // { s1: { b1: [{ f1: 8, f2: true }] } }

        let mut mgr = UndoManager::with_options(&doc, &r, Options::default());
        {
            let mut txn = doc.transact_mut();
            let el0 = b1_arr.insert(&mut txn, 0, MapPrelim::<i32>::new()); // { s1: { b1: [{}, { f1: 8, f2: true }] } }
            el0.insert(&mut txn, "f1", 8); // { s1: { b1: [{ f1: 8 }, { f1: 8, f2: true }] } }
            el0.insert(&mut txn, "f2", false); // { s1: { b1: [{ f1: 8, f2: false }, { f1: 8, f2: true }] } }

            el1.insert(&mut txn, "f1", 13); // { s1: { b1: [{ f1: 8, f2: false }, { f1: 13, f2: true }] } }
            el1.remove(&mut txn, "f2"); // { s1: { b1: [{ f1: 8, f2: false }, { f1: 13 }] } }
        }
        mgr.reset();

        el1.insert(&mut doc.transact_mut(), "f2", false); // { s1: { b1: [{ f1: 8, f2: false }, { f1: 13, f2: false }] } }
        mgr.reset();

        el1.insert(&mut doc.transact_mut(), "f2", true); // { s1: { b1: [{ f1: 8, f2: false }, { f1: 13, f2: true }] } }
        mgr.reset();

        mgr.undo().unwrap(); // { s1: { b1: [{ f1: 8, f2: false }, { f1: 13, f2: false }] } }
        assert_eq!(
            r.to_json(&doc.transact()),
            any!({ "s1": { "b1": [{ "f1": 8, "f2": false }, { "f1": 13, "f2": false }] } })
        );
        mgr.undo().unwrap(); // { s1: { b1: [{ f1: 8, f2: false }, { f1: 13 }] } }
        assert_eq!(
            r.to_json(&doc.transact()),
            any!({ "s1": { "b1": [{ "f1": 8, "f2": false }, { "f1": 13 }] } })
        );
        mgr.undo().unwrap(); // { s1: { b1: [{ f1: 8, f2: true }] } }
        assert_eq!(
            r.to_json(&doc.transact()),
            any!({ "s1": { "b1": [{ "f1": 8, "f2": true }] } })
        );
        assert!(!mgr.undo().unwrap()); // no more changes tracked by undo manager
    }

    #[test]
    fn issue_371_2() {
        let doc = Doc::with_client_id(1);
        let r = doc.get_or_insert_map("r");
        let s1 = r.insert(&mut doc.transact_mut(), "s1", MapPrelim::<i32>::new()); // { s1:{} }
        s1.insert(&mut doc.transact_mut(), "f2", "AAA"); // { s1: { f2: AAA } }
        s1.insert(&mut doc.transact_mut(), "f1", false); // { s1: { f1: false, f2: AAA } }

        let mut mgr = UndoManager::with_options(&doc, &r, Options::default());
        s1.remove(&mut doc.transact_mut(), "f2"); // { s1: { f1: false } }
        mgr.reset();

        s1.insert(&mut doc.transact_mut(), "f2", "C1"); // { s1: { f1: false, f2: C1 } }
        mgr.reset();

        s1.insert(&mut doc.transact_mut(), "f2", "C2"); // { s1: { f1: false, f2: C2 } }
        mgr.reset();

        mgr.undo().unwrap(); // { s1: { f1: false, f2: C1 } }
        assert_eq!(
            r.to_json(&doc.transact()),
            any!({ "s1": { "f1": false, "f2": "C1" } })
        );
        mgr.undo().unwrap(); // { s1: { f1: false } }
        assert_eq!(r.to_json(&doc.transact()), any!({ "s1": { "f1": false } }));
        mgr.undo().unwrap(); // { s1: { f1: false, f2: AAA } }
        assert_eq!(
            r.to_json(&doc.transact()),
            any!({ "s1": { "f1": false, "f2": "AAA" } })
        );
        assert!(!mgr.undo().unwrap()); // no more changes tracked by undo manager
    }

    #[test]
    fn issue_380() {
        let d = Doc::with_client_id(1);
        let r = d.get_or_insert_map("r"); // {r:{}}
        let s1 = r.insert(&mut d.transact_mut(), "s1", MapPrelim::<i32>::new()); // {r:{s1:{}}
        let b1_arr = s1.insert(&mut d.transact_mut(), "b1", ArrayPrelim::default()); // {r:{s1:{b1:[]}}

        let b1_el1 = b1_arr.insert(&mut d.transact_mut(), 0, MapPrelim::<i32>::new()); // {r:{s1:{b1:[{}]}}
        let b2_arr = b1_el1.insert(&mut d.transact_mut(), "b2", ArrayPrelim::default()); // {r:{s1:{b1:[{b2:[]}]}}
        let b2_arr_nest = b2_arr.insert(&mut d.transact_mut(), 0, ArrayPrelim::default()); // {r:{s1:{b1:[{b2:[[]]}]}}
        b2_arr_nest.insert(&mut d.transact_mut(), 0, 232291652); // {r:{s1:{b1:[{b2:[[232291652]]}]}}
        b2_arr_nest.insert(&mut d.transact_mut(), 1, -30); // {r:{s1:{b1:[{b2:[[232291652, -30]]}]}}

        let mut mgr = UndoManager::with_options(&d, &r, Options::default());

        let mut txn = d.transact_mut();
        b2_arr_nest.remove(&mut txn, 1); // {r:{s1:{b1:[{b2:[[232291652]]}]}}
        b2_arr_nest.insert(&mut txn, 1, -5); // {r:{s1:{b1:[{b2:[[232291652, -5]]}]}}

        drop(txn);
        mgr.reset();

        let mut txn = d.transact_mut();

        let b1_el0 = b1_arr.insert(&mut txn, 0, MapPrelim::<i32>::new()); // {r:{s1:{b1:[{},{b2:[[232291652, -5]]}]}}
        let b2_0_arr = b1_el0.insert(&mut txn, "b2", ArrayPrelim::default()); // {r:{s1:{b1:[{b2:[]},{b2:[[232291652, -5]]}]}}
        let b2_0_arr_nest = b2_0_arr.insert(&mut txn, 0, ArrayPrelim::default()); // {r:{s1:{b1:[{b2:[[]]},{b2:[[232291652, -5]]}]}}
        b2_0_arr_nest.insert(&mut txn, 0, 232291652); // {r:{s1:{b1:[{b2:[[232291652]]},{b2:[[232291652, -5]]}]}}
        b2_0_arr_nest.insert(&mut txn, 1, -6); // {r:{s1:{b1:[{b2:[[232291652, -6]]},{b2:[[232291652, -5]]}]}}
        b1_arr.remove(&mut txn, 1); // {r:{s1:{b1:[{b2:[[232291652, -6]]}]}}

        let b1_el1 = b1_arr.insert(&mut txn, 1, MapPrelim::<i32>::new()); // {r:{s1:{b1:[{b2:[[232291652, -6]]}, {}]}}
        b1_el1.insert(&mut txn, "f2", "C1"); // {r:{s1:{b1:[{b2:[[232291652, -6]]}, {f2:C1}]}}

        drop(txn);
        mgr.reset();
        assert_eq!(
            r.to_json(&d.transact()),
            any!({"s1":{"b1":[{"b2":[[232291652, -6]]}, {"f2":"C1"}]}})
        );

        mgr.undo().unwrap(); // {r:{s1:{b1:[{b2:[[232291652, -5]]}]}}
        assert_eq!(
            r.to_json(&d.transact()),
            any!({"s1":{"b1":[{"b2":[[232291652, -5]]}]}})
        );

        mgr.undo().unwrap(); // {r:{s1:{b1:[{b2:[[232291652, -30]]}]}}
        assert_eq!(
            r.to_json(&d.transact()),
            any!({"s1":{"b1":[{"b2":[[232291652, -30]]}]}})
        );
    }
}
