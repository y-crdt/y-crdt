use serde::{Deserialize, Serialize};

use crate::block::ItemPtr;
use crate::branch::{Branch, BranchPtr};
use crate::iter::TxnIterator;
use crate::slice::BlockSlice;
use crate::sync::Clock;
use crate::transaction::Origin;
use crate::{DeleteSet, Doc, Observer, ReadTxn, TransactionMut, ID};

use std::collections::HashSet;
use std::fmt::Formatter;
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::Arc;

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
pub struct UndoManager<M> {
    state: Wrap<State<M>>,
    self_origin: Origin,
    on_after_transaction: crate::Subscription,
}

#[cfg(feature = "sync")]
type UndoFn<M> = Box<dyn Fn(&TransactionMut, &mut Event<M>) + Send + Sync + 'static>;

#[cfg(not(feature = "sync"))]
type UndoFn<M> = Box<dyn Fn(&TransactionMut, &mut Event<M>) + 'static>;

#[cfg(feature = "sync")]
pub trait Meta: Default + Send + Sync {}
#[cfg(feature = "sync")]
impl<M> Meta for M where M: Default + Send + Sync {}

#[cfg(not(feature = "sync"))]
pub trait Meta: Default {}
#[cfg(not(feature = "sync"))]
impl<M> Meta for M where M: Default {}

pub(crate) trait UndoStackExt<M> {
    fn is_deleted(&self, id: &ID) -> bool;
}

impl<M> UndoStackExt<M> for Vec<StackItem<M>> {
    fn is_deleted(&self, id: &ID) -> bool {
        self.iter().any(|i| i.deletions.is_deleted(id))
    }
}

struct State<M> {
    scope: HashSet<BranchPtr>,
    options: Options<M>,
    last_change: u64,
    undoing: bool,
    redoing: bool,
    undo: Vec<StackItem<M>>,
    redo: Vec<StackItem<M>>,
    observer_added: Observer<UndoFn<M>>,
    observer_updated: Observer<UndoFn<M>>,
    observer_popped: Observer<UndoFn<M>>,
}

impl<M> State<M>
where
    M: Meta + 'static,
{
    fn is_undoing(&self) -> bool {
        self.undoing
    }

    fn is_redoing(&self) -> bool {
        self.redoing
    }

    fn should_skip(&self, txn: &TransactionMut) -> bool {
        if let Some(capture_transaction) = &self.options.capture_transaction {
            if !capture_transaction(txn) {
                return true;
            }
        }
        !self
            .scope
            .iter()
            .any(|parent| txn.changed_parent_types().contains(parent))
            || !txn
                .origin()
                .map(|o| self.options.tracked_origins.contains(o))
                .unwrap_or(self.options.tracked_origins.len() == 1) // tracked origins contain only undo manager itself
    }

    fn on_after_transaction(&mut self, txn: &TransactionMut) {
        if self.should_skip(txn) {
            return;
        }
        let undoing = self.is_undoing();
        let redoing = self.is_redoing();
        if undoing {
            self.last_change = 0; // next undo should not be appended to last stack item
        } else if !redoing {
            // neither undoing nor redoing: delete redoStack
            for item in self.redo.drain(..) {
                clear_item(&self.scope, txn, item);
            }
        }

        let mut insertions = DeleteSet::new();
        for (client, &end_clock) in txn.after_state().iter() {
            let start_clock = txn.before_state().get(client);
            let diff = end_clock - start_clock;
            if diff != 0 {
                insertions.insert(ID::new(*client, start_clock), diff);
            }
        }
        let now = self.options.timestamp.now();
        let stack = if undoing {
            &mut self.redo
        } else {
            &mut self.undo
        };
        let last_change = self.last_change;
        let extend = !undoing
            && !redoing
            && !stack.is_empty()
            && last_change > 0
            && now - last_change < self.options.capture_timeout_millis;

        if extend {
            // append change to last stack op
            if let Some(mut op) = stack.last_mut() {
                // always true - we checked if stack is empty above
                op.deletions.merge(txn.delete_set().clone());
                op.insertions.merge(insertions);
            }
        } else {
            // create a new stack op
            let item = StackItem::new(txn.delete_set().clone(), insertions);
            stack.push(item);
        }

        if !undoing && !redoing {
            self.last_change = now;
        }

        // make sure that deleted structs are not gc'd
        let ds = txn.delete_set().clone();
        let mut deleted = ds.deleted_blocks();
        while let Some(slice) = deleted.next(txn) {
            if let Some(item) = slice.as_item() {
                if self.scope.iter().any(|b| b.is_parent_of(Some(item))) {
                    item.keep(true);
                }
            }
        }

        let last_op = stack.last_mut().unwrap();
        let meta = std::mem::take(&mut last_op.meta);
        let mut event = if undoing {
            Event::undo(
                meta,
                txn.origin().cloned(),
                txn.changed_parent_types().into(),
            )
        } else {
            Event::redo(
                meta,
                txn.origin().cloned(),
                txn.changed_parent_types().into(),
            )
        };
        if !extend {
            if self.observer_added.has_subscribers() {
                self.observer_added.trigger(|fun| fun(txn, &mut event));
            }
        } else {
            if self.observer_updated.has_subscribers() {
                self.observer_updated.trigger(|fun| fun(txn, &mut event));
            }
        }
        last_op.meta = event.meta;
    }
}

impl<M> UndoManager<M>
where
    M: Meta + 'static,
{
    /// Creates a new instance of the [UndoManager] working in a `scope` of a particular shared
    /// type and document. While it's possible for undo manager to observe multiple shared types
    /// (see: [UndoManager::expand_scope]), it can only work with a single document at the same time.
    #[cfg(not(target_family = "wasm"))]
    pub fn new<T>(doc: &mut Doc, scope: &T) -> Self
    where
        T: AsRef<Branch>,
    {
        Self::with_scope_and_options(doc, scope, Options::default())
    }

    /// Creates a new instance of the [UndoManager] working in a context of a given document, but
    /// without any pre-initialize scope. While it's possible for undo manager to observe multiple
    /// shared types (see: [UndoManager::expand_scope]), it can only work with a single document
    /// at the same time.
    pub fn with_options(doc: &mut Doc, options: Options<M>) -> Self {
        let mut state = Wrap::new(State {
            scope: HashSet::new(),
            options,
            last_change: Default::default(),
            undoing: Default::default(),
            redoing: Default::default(),
            undo: Default::default(),
            redo: Default::default(),
            observer_added: Observer::new(),
            observer_updated: Observer::new(),
            observer_popped: Observer::new(),
        });
        let state_ptr = std::ptr::from_mut::<State<M>>(state.borrow_mut().deref_mut()) as usize;
        let self_origin = Origin::from(state_ptr);
        state
            .borrow_mut()
            .options
            .tracked_origins
            .insert(self_origin.clone());
        let on_after_transaction = doc
            .events()
            .after_transaction
            .subscribe(Box::new(move |txn| {
                // SAFETY: we are guaranteed that `state_ptr` points to a valid `State<M>` instance
                // because its lifetime and execution are strictly bound to the lifetime of
                // document and executing transaction (which requires exclusive access in order
                // to trigger after_transaction event).
                let state_ref = unsafe { (state_ptr as *mut State<M>).as_mut().unwrap() };
                state_ref.on_after_transaction(txn);
            }));

        UndoManager {
            state,
            self_origin,
            on_after_transaction,
        }
    }

    /// Creates a new instance of the [UndoManager] working in a `scope` of a particular shared
    /// type and document. While it's possible for undo manager to observe multiple shared types
    /// (see: [UndoManager::expand_scope]), it can only work with a single document at the same time.
    pub fn with_scope_and_options<T>(doc: &mut Doc, scope: &T, options: Options<M>) -> Self
    where
        T: AsRef<Branch>,
    {
        let mut mgr = Self::with_options(doc, options);
        mgr.expand_scope(scope);
        mgr
    }

    /// Registers a callback function to be called every time a new [StackItem] is created. This
    /// usually happens when a new update over an tracked shared type happened after capture timeout
    /// threshold from the previous stack item occurence has been reached or [UndoManager::reset]
    /// has been called.
    ///
    /// Returns a subscription object which - when dropped - will unregister provided callback.
    #[cfg(feature = "sync")]
    pub fn observe_item_added<F>(&self, f: F) -> crate::Subscription
    where
        F: Fn(&TransactionMut, &mut Event<M>) + Send + Sync + 'static,
    {
        self.state.borrow().observer_added.subscribe(Box::new(f))
    }

    /// Registers a callback function to be called every time a new [StackItem] is created. This
    /// usually happens when a new update over an tracked shared type happened after capture timeout
    /// threshold from the previous stack item occurence has been reached or [UndoManager::reset]
    /// has been called.
    ///
    /// Returns a subscription object which - when dropped - will unregister provided callback.
    #[cfg(not(feature = "sync"))]
    pub fn observe_item_added<F>(&self, f: F) -> crate::Subscription
    where
        F: Fn(&TransactionMut, &mut Event<M>) + 'static,
    {
        self.state.borrow().observer_added.subscribe(Box::new(f))
    }

    /// Registers a callback function to be called every time a new [StackItem] is created. This
    /// usually happens when a new update over an tracked shared type happened after capture timeout
    /// threshold from the previous stack item occurence has been reached or [UndoManager::reset]
    /// has been called.
    ///
    /// Provided `key` is used to identify the origin of the callback. It can be used to unregister
    /// the callback later on.
    #[cfg(feature = "sync")]
    pub fn observe_item_added_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &mut Event<M>) + Send + Sync + 'static,
    {
        self.state
            .borrow()
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
    #[cfg(not(feature = "sync"))]
    pub fn observe_item_added_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &mut Event<M>) + 'static,
    {
        self.state
            .borrow()
            .observer_added
            .subscribe_with(key.into(), Box::new(f))
    }

    pub fn unobserve_item_added<K>(&self, key: K) -> bool
    where
        K: Into<Origin>,
    {
        self.state.borrow().observer_added.unsubscribe(&key.into())
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// extended as a result of updates from tracked types which happened before a capture timeout
    /// has passed.
    ///
    /// Returns a subscription object which - when dropped - will unregister provided callback.
    #[cfg(feature = "sync")]
    pub fn observe_item_updated<F>(&self, f: F) -> crate::Subscription
    where
        F: Fn(&TransactionMut, &mut Event<M>) + Send + Sync + 'static,
    {
        self.state.borrow().observer_updated.subscribe(Box::new(f))
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// extended as a result of updates from tracked types which happened before a capture timeout
    /// has passed.
    ///
    /// Returns a subscription object which - when dropped - will unregister provided callback.
    #[cfg(not(feature = "sync"))]
    pub fn observe_item_updated<F>(&self, f: F) -> crate::Subscription
    where
        F: Fn(&TransactionMut, &mut Event<M>) + 'static,
    {
        self.state.borrow().observer_updated.subscribe(Box::new(f))
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// extended as a result of updates from tracked types which happened before a capture timeout
    /// has passed.
    ///
    /// Provided `key` is used to identify the origin of the callback. It can be used to unregister
    /// the callback later on.
    #[cfg(feature = "sync")]
    pub fn observe_item_updated_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &mut Event<M>) + Send + Sync + 'static,
    {
        self.state
            .borrow()
            .observer_updated
            .subscribe_with(key.into(), Box::new(f))
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// extended as a result of updates from tracked types which happened before a capture timeout
    /// has passed.
    ///
    /// Provided `key` is used to identify the origin of the callback. It can be used to unregister
    /// the callback later on.
    #[cfg(not(feature = "sync"))]
    pub fn observe_item_updated_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &mut Event<M>) + 'static,
    {
        self.state
            .borrow()
            .observer_updated
            .subscribe_with(key.into(), Box::new(f))
    }

    pub fn unobserve_item_updated<K>(&self, key: K) -> bool
    where
        K: Into<Origin>,
    {
        self.state
            .borrow()
            .observer_updated
            .unsubscribe(&key.into())
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// removed as a result of [UndoManager::undo] or [UndoManager::redo] method.
    ///
    /// Returns a subscription object which - when dropped - will unregister provided callback.
    #[cfg(feature = "sync")]
    pub fn observe_item_popped<F>(&self, f: F) -> crate::Subscription
    where
        F: Fn(&TransactionMut, &mut Event<M>) + Send + Sync + 'static,
    {
        self.state.borrow().observer_popped.subscribe(Box::new(f))
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// removed as a result of [UndoManager::undo] or [UndoManager::redo] method.
    ///
    /// Returns a subscription object which - when dropped - will unregister provided callback.
    #[cfg(not(feature = "sync"))]
    pub fn observe_item_popped<F>(&self, f: F) -> crate::Subscription
    where
        F: Fn(&TransactionMut, &mut Event<M>) + 'static,
    {
        self.state.borrow().observer_popped.subscribe(Box::new(f))
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// removed as a result of [UndoManager::undo] or [UndoManager::redo] method.
    ///
    /// Provided `key` is used to identify the origin of the callback. It can be used to unregister
    /// the callback later on.
    #[cfg(feature = "sync")]
    pub fn observe_item_popped_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &mut Event<M>) + Send + Sync + 'static,
    {
        self.state
            .borrow()
            .observer_popped
            .subscribe_with(key.into(), Box::new(f))
    }

    /// Registers a callback function to be called every time an existing [StackItem] has been
    /// removed as a result of [UndoManager::undo] or [UndoManager::redo] method.
    ///
    /// Provided `key` is used to identify the origin of the callback. It can be used to unregister
    /// the callback later on.
    #[cfg(not(feature = "sync"))]
    pub fn observe_item_popped_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &mut Event<M>) + 'static,
    {
        self.state
            .borrow()
            .observer_popped
            .subscribe_with(key.into(), Box::new(f))
    }

    pub fn unobserve_item_popped<K>(&self, key: K) -> bool
    where
        K: Into<Origin>,
    {
        self.state.borrow().observer_popped.unsubscribe(&key.into())
    }

    /// Extends a list of shared types tracked by current undo manager by a given `scope`.
    pub fn expand_scope<T>(&mut self, scope: &T)
    where
        T: AsRef<Branch>,
    {
        let ptr = BranchPtr::from(scope.as_ref());
        let mut inner = self.state.borrow_mut();
        inner.scope.insert(ptr);
    }

    /// Extends a list of origins tracked by current undo manager by given `origin`. Origin markers
    /// can be assigned to updates executing in a scope of a particular transaction
    /// (see: [Doc::transact_mut_with]).
    pub fn include_origin<O>(&mut self, origin: O)
    where
        O: Into<Origin>,
    {
        let mut inner = self.state.borrow_mut();
        inner.options.tracked_origins.insert(origin.into());
    }

    /// Removes an `origin` from the list of origins tracked by a current undo manager.
    pub fn exclude_origin<O>(&mut self, origin: O)
    where
        O: Into<Origin>,
    {
        let mut inner = self.state.borrow_mut();
        inner.options.tracked_origins.remove(&origin.into());
    }

    /// Clears all [StackItem]s stored within current UndoManager, effectively resetting its state.
    ///
    /// # Deadlocks
    ///
    /// In order to perform its function, this method must guarantee that underlying document store
    /// is not being modified by another running `TransactionMut`. It does so by acquiring
    /// a read-only transaction itself. If transaction couldn't be acquired (because another
    /// read-write transaction is in progress), it will hold current thread until, acquisition is
    /// available.
    pub fn clear(&mut self, doc: &Doc) {
        let txn = doc.transact();
        let mut state = self.state.borrow_mut();
        let state = &mut *state;
        for item in state.undo.drain(..) {
            clear_item(&state.scope, &txn, item);
        }
        for item in state.redo.drain(..) {
            clear_item(&state.scope, &txn, item);
        }
    }

    pub fn as_origin(&self) -> Origin {
        self.self_origin.clone()
    }

    /// [UndoManager] merges undo stack items if they were created withing the time gap smaller than
    /// [Options::capture_timeout_millis]. You can call this method so that the next stack item won't be
    /// merged.
    ///
    /// Example:
    /// ```rust
    /// use yrs::{Doc, GetString, Text, UndoManager};
    /// let mut doc = Doc::new();
    ///
    /// // without UndoManager::stop
    /// let txt = doc.get_or_insert_text("no-stop");
    /// let mut mgr = UndoManager::new(&mut doc, &txt);
    /// txt.insert(&mut doc.transact_mut(), 0, "a");
    /// txt.insert(&mut doc.transact_mut(), 1, "b");
    /// mgr.undo(&mut doc);
    /// txt.get_string(&doc.transact()); // => "" (note that 'ab' was removed)
    ///
    /// // with UndoManager::stop
    /// let txt = doc.get_or_insert_text("with-stop");
    /// let mut mgr = UndoManager::new(&mut doc, &txt);
    /// txt.insert(&mut doc.transact_mut(), 0, "a");
    /// mgr.reset();
    /// txt.insert(&mut doc.transact_mut(), 1, "b");
    /// mgr.undo(&mut doc);
    /// txt.get_string(&doc.transact()); // => "a" (note that only 'b' was removed)
    /// ```
    pub fn reset(&mut self) {
        let mut state = self.state.borrow_mut();
        state.last_change = 0;
    }

    /// Are there any undo steps available?
    pub fn can_undo(&self) -> bool {
        let state = self.state.borrow();
        !state.undo.is_empty()
    }

    /// Are there any redo steps available?
    pub fn can_redo(&self) -> bool {
        let state = self.state.borrow();
        !state.redo.is_empty()
    }

    /// Returns a number of [StackItem]s stored within current undo manager responsible for performing
    /// potential undo operations.
    pub fn undo_len(&self) -> usize {
        let state = self.state.borrow();
        state.undo.len()
    }

    /// Returns a number of [StackItem]s stored within current undo manager responsible for performing
    /// potential redo operations.
    pub fn redo_len(&self) -> usize {
        let state = self.state.borrow();
        state.redo.len()
    }

    /// Undo last action tracked by current undo manager. Actions (a.k.a. [StackItem]s) are groups
    /// of updates performed in a given time range - they also can be separated explicitly by
    /// calling [UndoManager::reset].
    ///
    /// Successful execution returns a boolean value telling if an undo call has performed any changes.
    ///
    /// # Deadlocks
    ///
    /// This method requires exclusive access to underlying document store. This means that
    /// no other transaction on that same document can be active while calling this method.
    /// Otherwise, it may cause a deadlock.
    ///
    /// See also: [UndoManager::try_undo] and [UndoManager::undo_blocking].
    pub fn undo(&mut self, doc: &mut Doc) -> bool {
        let origin = self.as_origin();
        let mut state = self.state.borrow_mut();
        let state = &mut *state;
        let mut txn = doc.transact_mut_with(origin.clone());
        state.undoing = true;
        let result = Self::pop(&mut state.undo, &state.redo, &mut txn, &state.scope);
        txn.commit();
        let changed = if let Some(item) = result {
            let mut e = Event::undo(item.meta, Some(origin), txn.changed_parent_types().into());
            if state.observer_popped.has_subscribers() {
                state.observer_popped.trigger(|fun| fun(&txn, &mut e));
            }
            true
        } else {
            false
        };
        state.undoing = false;
        changed
    }

    /// Redo'es last action previously undo'ed by current undo manager. Actions
    /// (a.k.a. [StackItem]s) are groups of updates performed in a given time range - they also can
    /// be separated explicitly by calling [UndoManager::reset].
    ///
    /// Successful execution returns a boolean value telling if an undo call has performed any changes.
    ///
    /// # Deadlocks
    ///
    /// This method requires exclusive access to underlying document store. This means that
    /// no other transaction on that same document can be active while calling this method.
    /// Otherwise, it may cause a deadlock.
    ///
    /// See also: [UndoManager::try_redo] and [UndoManager::redo_blocking].
    pub fn redo(&mut self, doc: &mut Doc) -> bool {
        let origin = self.as_origin();
        let mut state = self.state.borrow_mut();
        let state = &mut *state;
        let mut txn = doc.transact_mut_with(origin.clone());
        state.redoing = true;
        let result = Self::pop(&mut state.redo, &state.undo, &mut txn, &state.scope);
        txn.commit();
        let changed = if let Some(item) = result {
            let mut e = Event::redo(item.meta, Some(origin), txn.changed_parent_types().into());
            if state.observer_popped.has_subscribers() {
                state.observer_popped.trigger(|fun| fun(&txn, &mut e));
            }
            true
        } else {
            false
        };
        state.redoing = false;
        changed
    }

    fn pop(
        stack: &mut Vec<StackItem<M>>,
        other: &Vec<StackItem<M>>,
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
                    let mut item = txn.doc_mut().materialize(slice);
                    if item.redone.is_some() {
                        let slice = txn.doc_mut().follow_redone(item.id())?;
                        item = txn.doc_mut().materialize(slice);
                    }

                    if !item.is_deleted() && scope.iter().any(|b| b.is_parent_of(Some(item))) {
                        to_delete.push(item);
                    }
                }
            }

            let mut deleted = item.deletions.deleted_blocks();
            while let Some(slice) = deleted.next(txn) {
                if let BlockSlice::Item(slice) = slice {
                    let ptr = txn.doc_mut().materialize(slice);
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
        let state = self.state.borrow();
        s.field("scope", &state.scope);
        s.field("tracked_origins", &state.options.tracked_origins);
        s.field("undo", &state.undo.len());
        s.field("redo", &state.redo.len());
        s.finish()
    }
}

fn clear_item<M, T: ReadTxn>(scope: &HashSet<BranchPtr>, txn: &T, stack_item: StackItem<M>) {
    let mut deleted = stack_item.deletions.deleted_blocks();
    while let Some(slice) = deleted.next(txn) {
        if let Some(item) = slice.as_item() {
            if scope.iter().any(|b| b.is_parent_of(Some(item))) {
                item.keep(false);
            }
        }
    }
}

#[cfg(feature = "sync")]
#[repr(transparent)]
struct Wrap<S> {
    inner: Pin<Box<parking_lot::Mutex<S>>>,
}

#[cfg(feature = "sync")]
type WeakWrap<S> = std::sync::Weak<parking_lot::Mutex<S>>;

#[cfg(feature = "sync")]
impl<S> Wrap<S> {
    fn new(inner: S) -> Self {
        Wrap {
            inner: Box::pin(parking_lot::Mutex::new(inner)),
        }
    }

    fn borrow(&self) -> parking_lot::MutexGuard<'_, S> {
        self.inner.try_lock().unwrap()
    }

    fn borrow_mut(&mut self) -> parking_lot::MutexGuard<'_, S> {
        self.inner.try_lock().unwrap()
    }
}

#[cfg(not(feature = "sync"))]
#[repr(transparent)]
struct Wrap<S> {
    inner: Pin<Box<std::cell::RefCell<S>>>,
}

#[cfg(not(feature = "sync"))]
type WeakWrap<S> = std::rc::Weak<std::cell::RefCell<S>>;

#[cfg(not(feature = "sync"))]
impl<S> Wrap<S> {
    fn new(inner: S) -> Self {
        Wrap {
            inner: Box::pin(std::cell::RefCell::new(inner)),
        }
    }

    fn borrow(&self) -> std::cell::Ref<'_, S> {
        self.inner.borrow()
    }

    fn borrow_mut(&mut self) -> std::cell::RefMut<'_, S> {
        self.inner.borrow_mut()
    }
}

/// Set of options used to configure [UndoManager].
pub struct Options<M> {
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

    /// Initial undo stack that can be pre-filled with some operations that can be undone.
    pub init_undo_stack: Vec<StackItem<M>>,

    /// Initial redo stack that can be pre-filled with some operations that can be redone.
    pub init_redo_stack: Vec<StackItem<M>>,
}

pub type CaptureTransactionFn = Arc<dyn Fn(&TransactionMut) -> bool + Send + Sync + 'static>;

#[cfg(not(target_family = "wasm"))]
impl<M> Default for Options<M> {
    fn default() -> Self {
        Options {
            capture_timeout_millis: 500,
            tracked_origins: HashSet::new(),
            capture_transaction: None,
            timestamp: Arc::new(crate::sync::time::SystemClock),
            init_undo_stack: Vec::new(),
            init_redo_stack: Vec::new(),
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
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct StackItem<T> {
    deletions: DeleteSet,
    insertions: DeleteSet,

    /// A custom user metadata that can be attached to a particular [StackItem]. It can be used
    /// to carry over the additional information (such as ie. user cursor position) between
    /// undo/redo operations.
    pub meta: T,
}

impl<M> StackItem<M> {
    pub fn with_meta(deletions: DeleteSet, insertions: DeleteSet, meta: M) -> Self {
        StackItem {
            deletions,
            insertions,
            meta,
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

    /// Returns metaobject reference associated with this stack item.
    pub fn meta(&self) -> &M {
        &self.meta
    }

    /// Merged another [StackItem] into current one. `merge_meta` function is used to merge user's
    /// custom metadata structures together.
    pub fn merge<F>(&mut self, other: Self, merge_meta: F)
    where
        F: FnOnce(&mut M, M),
    {
        self.insertions.merge(other.insertions);
        self.deletions.merge(other.deletions);
        merge_meta(&mut self.meta, other.meta);
    }
}

impl<M: Default> StackItem<M> {
    pub fn new(deletions: DeleteSet, insertions: DeleteSet) -> Self {
        Self::with_meta(deletions, insertions, M::default())
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
    use crate::undo::{Options, StackItem};
    use crate::updates::decoder::Decode;
    use crate::{
        any, Any, Array, ArrayPrelim, Doc, GetString, Map, MapPrelim, MapRef, ReadTxn, StateVector,
        Text, TextPrelim, TextRef, UndoManager, Update, Xml, XmlElementPrelim, XmlElementRef,
        XmlFragment, XmlTextPrelim,
    };

    #[test]
    fn undo_text() {
        let mut d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("test");
        let mut mgr = UndoManager::new(&mut d1, &txt1);

        let mut d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("test");

        // items that are added & deleted in the same transaction won't be undo
        txt1.insert(&mut d1.transact_mut(), 0, "test");
        txt1.remove_range(&mut d1.transact_mut(), 0, 4);
        mgr.undo(&mut d1);
        assert_eq!(txt1.get_string(&d1.transact()), "");

        // follow redone items
        txt1.insert(&mut d1.transact_mut(), 0, "a");
        mgr.reset();
        txt1.remove_range(&mut d1.transact_mut(), 0, 1);
        mgr.reset();
        mgr.undo(&mut d1);
        assert_eq!(txt1.get_string(&d1.transact()), "a");
        mgr.undo(&mut d1);
        assert_eq!(txt1.get_string(&d1.transact()), "");

        txt1.insert(&mut d1.transact_mut(), 0, "abc");
        txt2.insert(&mut d2.transact_mut(), 0, "xyz");

        exchange_updates([&mut d1, &mut d2]);
        mgr.undo(&mut d1);
        assert_eq!(txt1.get_string(&d1.transact()), "xyz");
        mgr.redo(&mut d1);
        assert_eq!(txt1.get_string(&d1.transact()), "abcxyz");

        exchange_updates([&mut d1, &mut d2]);

        txt2.remove_range(&mut d2.transact_mut(), 0, 1);

        exchange_updates([&mut d1, &mut d2]);

        mgr.undo(&mut d1);
        assert_eq!(txt1.get_string(&d1.transact()), "xyz");
        mgr.redo(&mut d1);
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

        mgr.undo(&mut d1);
        let diff = txt1.diff(&d1.transact(), YChange::identity);
        assert_eq!(diff, vec![Diff::new("bcxyz".into(), None)]);

        mgr.redo(&mut d1);
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
        let mut doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        txt.insert(&mut doc.transact_mut(), 0, "1221");

        let mut mgr = UndoManager::new(&mut doc, &txt);
        txt.insert(&mut doc.transact_mut(), 2, "3");
        txt.insert(&mut doc.transact_mut(), 3, "3");

        mgr.undo(&mut doc);
        mgr.undo(&mut doc);

        txt.insert(&mut doc.transact_mut(), 2, "3");
        assert_eq!(txt.get_string(&doc.transact()), "12321");
    }

    #[test]
    fn undo_map() {
        let mut d1 = Doc::with_client_id(1);
        let map1 = d1.get_or_insert_map("test");

        let mut d2 = Doc::with_client_id(2);
        let map2 = d2.get_or_insert_map("test");

        map1.insert(&mut d1.transact_mut(), "a", 0);
        let mut mgr = UndoManager::new(&mut d1, &map1);
        map1.insert(&mut d1.transact_mut(), "a", 1);
        mgr.undo(&mut d1);
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 0.into());
        mgr.redo(&mut d1);
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 1.into());

        // testing sub-types and if it can restore a whole type
        let sub_type = map1.insert(&mut d1.transact_mut(), "a", MapPrelim::default());
        sub_type.insert(&mut d1.transact_mut(), "x", 42);
        let actual = map1.to_json(&d1.transact());
        let expected = Any::from_json(r#"{ "a": { "x": 42 } }"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo(&mut d1);
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 1.into());
        mgr.redo(&mut d1);
        let actual = map1.to_json(&d1.transact());
        let expected = Any::from_json(r#"{ "a": { "x": 42 } }"#).unwrap();
        assert_eq!(actual, expected);

        exchange_updates([&mut d1, &mut d2]);

        // if content is overwritten by another user, undo operations should be skipped
        map2.insert(&mut d2.transact_mut(), "a", 44);

        exchange_updates([&mut d1, &mut d2]);
        exchange_updates([&mut d1, &mut d2]);

        mgr.undo(&mut d1);
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 44.into());
        mgr.redo(&mut d1);
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 44.into());

        // test setting value multiple times
        map1.insert(&mut d1.transact_mut(), "b", "initial");
        mgr.reset();
        map1.insert(&mut d1.transact_mut(), "b", "val1");
        map1.insert(&mut d1.transact_mut(), "b", "val2");
        mgr.reset();
        mgr.undo(&mut d1);
        assert_eq!(map1.get(&d1.transact(), "b").unwrap(), "initial".into());
    }

    #[test]
    fn undo_array() {
        let mut d1 = Doc::with_client_id(1);
        let array1 = d1.get_or_insert_array("test");

        let mut d2 = Doc::with_client_id(2);
        let array2 = d2.get_or_insert_array("test");

        let mut mgr = UndoManager::new(&mut d1, &array1);
        array1.insert_range(&mut d1.transact_mut(), 0, [1, 2, 3]);
        array2.insert_range(&mut d2.transact_mut(), 0, [4, 5, 6]);

        exchange_updates([&mut d1, &mut d2]);

        assert_eq!(
            array1.to_json(&d1.transact()),
            vec![1, 2, 3, 4, 5, 6].into()
        );
        mgr.undo(&mut d1);
        assert_eq!(array1.to_json(&d1.transact()), vec![4, 5, 6].into());
        mgr.redo(&mut d1);
        assert_eq!(
            array1.to_json(&d1.transact()),
            vec![1, 2, 3, 4, 5, 6].into()
        );

        exchange_updates([&mut d1, &mut d2]);

        array2.remove_range(&mut d2.transact_mut(), 0, 1); // user2 deletes [1]

        exchange_updates([&mut d1, &mut d2]);

        mgr.undo(&mut d1);
        assert_eq!(array1.to_json(&d1.transact()), vec![4, 5, 6].into());
        mgr.redo(&mut d1);
        assert_eq!(array1.to_json(&d1.transact()), vec![2, 3, 4, 5, 6].into());
        array1.remove_range(&mut d1.transact_mut(), 0, 5);

        // test nested structure
        let map = array1.insert(&mut d1.transact_mut(), 0, MapPrelim::default());
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{}]"#).unwrap();
        assert_eq!(actual, expected);
        mgr.reset();
        map.insert(&mut d1.transact_mut(), "a", 1);
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"a":1}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo(&mut d1);
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo(&mut d1);
        assert_eq!(array1.to_json(&d1.transact()), vec![2, 3, 4, 5, 6].into());

        mgr.redo(&mut d1);
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.redo(&mut d1);
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"a":1}]"#).unwrap();
        assert_eq!(actual, expected);

        exchange_updates([&mut d1, &mut d2]);

        let map2 = array2
            .get(&d2.transact(), 0)
            .unwrap()
            .cast::<MapRef>()
            .unwrap();
        map2.insert(&mut d2.transact_mut(), "b", 2);
        exchange_updates([&mut d1, &mut d2]);

        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"a":1,"b":2}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo(&mut d1);
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"b":2}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo(&mut d1);
        assert_eq!(array1.to_json(&d1.transact()), vec![2, 3, 4, 5, 6].into());

        mgr.redo(&mut d1);
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"b":2}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.redo(&mut d1);
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"a":1,"b":2}]"#).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn undo_xml() {
        let mut d1 = Doc::with_client_id(1);
        let frag = d1.get_or_insert_xml_fragment("xml");
        let xml1 = frag.insert(
            &mut d1.transact_mut(),
            0,
            XmlElementPrelim::empty("undefined"),
        );

        let mut mgr = UndoManager::new(&mut d1, &xml1);
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
        mgr.undo(&mut d1);
        assert_eq!(
            xml1.get_string(&d1.transact()),
            "<undefined><p>content</p></undefined>"
        );
        mgr.redo(&mut d1);
        assert_eq!(
            xml1.get_string(&d1.transact()),
            "<undefined><p>con<bold>tent</bold></p></undefined>"
        );
        xml1.remove_range(&mut d1.transact_mut(), 0, 1);
        assert_eq!(xml1.get_string(&d1.transact()), "<undefined></undefined>");
        mgr.undo(&mut d1);
        assert_eq!(
            xml1.get_string(&d1.transact()),
            "<undefined><p>con<bold>tent</bold></p></undefined>"
        );
    }

    #[test]
    fn undo_events() {
        use crate::undo::UndoManager;
        type Metadata = HashMap<String, usize>;

        let mut doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        let mut mgr: UndoManager<Metadata> = UndoManager::new(&mut doc, &txt);

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
        mgr.undo(&mut doc);
        assert_eq!(result.load(Ordering::SeqCst), 1);
        mgr.redo(&mut doc);
        assert_eq!(result.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn undo_stack_serialization() {
        use crate::undo::UndoManager;
        type Metadata = HashMap<String, usize>;
        let (undo_stack_json, redo_stack_json, state) = {
            let mut d1 = Doc::with_client_id(1);
            let txt = d1.get_or_insert_text("test");
            let mut m1: UndoManager<Metadata> = UndoManager::new(&mut d1, &txt);

            let txt_clone = txt.clone();
            let _sub1 = m1.observe_item_added(move |_, e| {
                let e = e.meta_mut().entry("test".to_string()).or_default();
            });

            txt.insert(&mut d1.transact_mut(), 0, "c");
            m1.reset();
            txt.insert(&mut d1.transact_mut(), 0, "b");
            m1.reset();
            txt.insert(&mut d1.transact_mut(), 0, "a");

            m1.undo(&mut d1);
            assert_eq!(txt.get_string(&d1.transact()), "bc");
            m1.redo(&mut d1);
            assert_eq!(txt.get_string(&d1.transact()), "abc");

            let undo_stack_json = serde_json::to_string(m1.undo_stack()).unwrap();
            let redo_stack_json = serde_json::to_string(m1.redo_stack()).unwrap();
            let doc_state = d1.transact().encode_diff_v1(&StateVector::default());
            (undo_stack_json, redo_stack_json, doc_state)
        };

        let undo_stack: Vec<StackItem<Metadata>> = serde_json::from_str(&undo_stack_json).unwrap();
        let redo_stack: Vec<StackItem<Metadata>> = serde_json::from_str(&undo_stack_json).unwrap();

        // try to recreate the stack
        let mut d2 = Doc::with_client_id(1);
        let txt = d2.get_or_insert_text("test");
        let undo_options = Options {
            init_undo_stack: undo_stack,
            init_redo_stack: redo_stack,
            ..Default::default()
        };
        d2.transact_mut()
            .apply_update(Update::decode_v1(&state).unwrap())
            .unwrap();
        let mut m2: UndoManager<Metadata> = UndoManager::with_options(&mut d2, undo_options);
        m2.expand_scope(&txt);

        m2.undo(&mut d2);
        assert_eq!(txt.get_string(&d2.transact()), "bc");
        m2.redo(&mut d2);
        assert_eq!(txt.get_string(&d2.transact()), "abc");
    }

    #[test]
    fn undo_until_change_performed() {
        let mut d1 = Doc::with_client_id(1);
        let arr1 = d1.get_or_insert_array("array");

        let mut d2 = Doc::with_client_id(2);
        let arr2 = d2.get_or_insert_array("array");

        let map1a = arr1.push_back(
            &mut d1.transact_mut(),
            MapPrelim::from([("hello".to_owned(), "world".to_owned())]),
        );

        let map1b = arr1.push_back(
            &mut d1.transact_mut(),
            MapPrelim::from([("key".to_owned(), "value".to_owned())]),
        );

        exchange_updates([&mut d1, &mut d2]);

        let mut mgr1 = UndoManager::new(&mut d1, &arr1);
        mgr1.include_origin(d1.client_id());

        let mut mgr2 = UndoManager::new(&mut d2, &arr2);
        mgr2.include_origin(d2.client_id());

        map1b.insert(
            &mut d1.transact_mut_with(d1.client_id()),
            "key",
            "value modified",
        );

        exchange_updates([&mut d1, &mut d2]);
        mgr1.reset();

        map1a.insert(
            &mut d1.transact_mut_with(d1.client_id()),
            "hello",
            "world modified",
        );

        exchange_updates([&mut d1, &mut d2]);

        arr2.remove_range(&mut d2.transact_mut_with(d2.client_id()), 0, 1);

        exchange_updates([&mut d1, &mut d2]);
        mgr2.undo(&mut d2);

        exchange_updates([&mut d1, &mut d2]);
        mgr1.undo(&mut d1);
        exchange_updates([&mut d1, &mut d2]);

        assert_eq!(map1b.get(&d1.transact(), "key"), Some("value".into()));
    }

    #[test]
    fn nested_undo() {
        // This issue has been reported in https://github.com/yjs/yjs/issues/317
        let mut doc = Doc::with_options(crate::doc::Options {
            skip_gc: true,
            client_id: 1,
            ..crate::doc::Options::default()
        });
        let design = doc.get_or_insert_map("map");
        let mut mgr = UndoManager::with_scope_and_options(&mut doc, &design, {
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
        mgr.undo(&mut doc);
        assert_eq!(
            design.to_json(&doc.transact()),
            Any::from_json(r#"{ "text": { "blocks": { "text": "Something" } } }"#).unwrap()
        );
        mgr.undo(&mut doc);
        assert_eq!(
            design.to_json(&doc.transact()),
            Any::from_json(r#"{ "text": { "blocks": { "text": "Type something" } } }"#).unwrap()
        );
        mgr.undo(&mut doc);
        assert_eq!(
            design.to_json(&doc.transact()),
            Any::from_json(r#"{}"#).unwrap()
        );
        mgr.redo(&mut doc);
        assert_eq!(
            design.to_json(&doc.transact()),
            Any::from_json(r#"{ "text": { "blocks": { "text": "Type something" } } }"#).unwrap()
        );
        mgr.redo(&mut doc);
        assert_eq!(
            design.to_json(&doc.transact()),
            Any::from_json(r#"{ "text": { "blocks": { "text": "Something" } } }"#).unwrap()
        );
        mgr.redo(&mut doc);
        assert_eq!(
            design.to_json(&doc.transact()),
            Any::from_json(r#"{ "text": { "blocks": { "text": "Something else" } } }"#).unwrap()
        );
    }

    #[test]
    fn consecutive_redo_bug() {
        // https://github.com/yjs/yjs/issues/355
        let mut doc = Doc::with_client_id(1);
        let root = doc.get_or_insert_map("root");
        let mut mgr = UndoManager::new(&mut doc, &root);

        root.insert(
            &mut doc.transact_mut(),
            "a",
            MapPrelim::from([("x".to_owned(), Any::from(0)), ("y".to_owned(), 0.into())]),
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

        mgr.undo(&mut doc); // x=200, y=200
        let actual = point.to_json(&doc.transact());
        assert_eq!(actual, Any::from_json(r#"{"x":200,"y":200}"#).unwrap());

        mgr.undo(&mut doc); // x=100, y=100
        let actual = point.to_json(&doc.transact());
        assert_eq!(actual, Any::from_json(r#"{"x":100,"y":100}"#).unwrap());

        mgr.undo(&mut doc); // x=0, y=0
        let actual = point.to_json(&doc.transact());
        assert_eq!(actual, Any::from_json(r#"{"x":0,"y":0}"#).unwrap());

        mgr.undo(&mut doc); // null
        assert_eq!(root.get(&doc.transact(), "a"), None);

        mgr.redo(&mut doc); // x=0, y=0
        let point = root
            .get(&doc.transact(), "a")
            .unwrap()
            .cast::<MapRef>()
            .unwrap();

        assert_eq!(actual, Any::from_json(r#"{"x":0,"y":0}"#).unwrap());

        mgr.redo(&mut doc); // x=100, y=100
        let actual = point.to_json(&doc.transact());
        assert_eq!(actual, Any::from_json(r#"{"x":100,"y":100}"#).unwrap());

        mgr.redo(&mut doc); // x=200, y=200
        let actual = point.to_json(&doc.transact());
        assert_eq!(actual, Any::from_json(r#"{"x":200,"y":200}"#).unwrap());

        mgr.redo(&mut doc); // x=300, y=300
        let actual = point.to_json(&doc.transact());
        assert_eq!(actual, Any::from_json(r#"{"x":300,"y":300}"#).unwrap());
    }

    #[test]
    fn undo_xml_bug() {
        // https://github.com/yjs/yjs/issues/304
        const ORIGIN: &str = "origin";
        let mut doc = Doc::with_client_id(1);
        let f = doc.get_or_insert_xml_fragment("t");
        let mut mgr = UndoManager::with_scope_and_options(&mut doc, &f, {
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

        mgr.undo(&mut doc);
        mgr.undo(&mut doc);
        mgr.undo(&mut doc);

        mgr.redo(&mut doc);
        mgr.redo(&mut doc);
        mgr.redo(&mut doc);

        let str = f.get_string(&doc.transact());
        assert!(
            str == r#"<test-node a="180" b="50"></test-node>"#
                || str == r#"<test-node b="50" a="180"></test-node>"#
        );
    }

    #[test]
    fn undo_block_bug() {
        // https://github.com/yjs/yjs/issues/343
        let mut doc = Doc::with_options({
            let mut o = crate::doc::Options::default();
            o.client_id = 1;
            o.skip_gc = true;
            o
        });
        let design = doc.get_or_insert_map("map");
        let mut mgr = UndoManager::with_scope_and_options(&mut doc, &design, {
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
        mgr.undo(&mut doc); // {"text":{"blocks":{"3"}}}
        mgr.undo(&mut doc); // {"text":{"blocks":{"text":"2"}}}
        mgr.undo(&mut doc); // {"text":{"blocks":{"text":"1"}}}
        mgr.undo(&mut doc); // {}
        mgr.redo(&mut doc); // {"text":{"blocks":{"text":"1"}}}
        mgr.redo(&mut doc); // {"text":{"blocks":{"text":"2"}}}
        mgr.redo(&mut doc); // {"text":{"blocks":{"text":"3"}}}
        mgr.redo(&mut doc); // {"text":{}}
        let actual = design.to_json(&doc.transact());
        assert_eq!(
            actual,
            Any::from_json(r#"{"text":{"blocks":{"text":"4"}}}"#).unwrap()
        );
    }

    #[test]
    fn undo_delete_text_format() {
        // https://github.com/yjs/yjs/issues/392
        fn send(src: &Doc, dst: &mut Doc) {
            let update = Update::decode_v1(
                &src.transact()
                    .encode_state_as_update_v1(&StateVector::default()),
            )
            .unwrap();
            dst.transact_mut().apply_update(update).unwrap();
        }

        let mut doc1 = Doc::with_client_id(1);
        let txt = doc1.get_or_insert_text("test");
        txt.insert(
            &mut doc1.transact_mut(),
            0,
            "Attack ships on fire off the shoulder of Orion.",
        ); // D1: 'Attack ships on fire off the shoulder of Orion.'
        let mut doc2 = Doc::with_client_id(2);
        let txt2 = doc2.get_or_insert_text("test");

        send(&doc1, &mut doc2); // D2: 'Attack ships on fire off the shoulder of Orion.'
        let mut mgr = UndoManager::new(&mut doc1, &txt);

        let attrs = Attrs::from([("bold".into(), true.into())]);
        txt.format(&mut doc1.transact_mut(), 13, 7, attrs.clone()); // D1: 'Attack ships <b>on fire</b> off the shoulder of Orion.'

        mgr.reset();

        send(&doc1, &mut doc2); // D2: 'Attack ships <b>on fire</b> off the shoulder of Orion.'

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
        send(&doc1, &mut doc2); // D2: 'Attack ships <b>on </b>fire off the shoulder of Orion.'

        mgr.undo(&mut doc1); // D1: 'Attack ships <b>on fire</b> off the shoulder of Orion.'
        send(&doc1, &mut doc2); // D2: 'Attack ships <b>on fire</b> off the shoulder of Orion.'

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
        let mut doc = Doc::with_client_id(1);
        let f = doc.get_or_insert_xml_fragment("test");
        let mut mgr = UndoManager::new(&mut doc, &f);
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

        mgr.undo(&mut doc);
        let s = f.get_string(&doc.transact());
        assert!(s == r#"<test a="1" b="2"></test>"# || s == r#"<test b="2" a="1"></test>"#);
    }

    #[test]
    fn undo_in_embed() {
        let mut d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("test");
        let mut mgr = UndoManager::new(&mut d1, &txt1);

        let mut d2 = Doc::with_client_id(2);
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
        mgr.undo(&mut d1);
        assert_eq!(
            nested.get_string(&d1.transact()),
            "initial text".to_string()
        );

        exchange_updates([&mut d1, &mut d2]);
        let diff = txt2.diff(&d1.transact(), YChange::identity);
        let nested2 = diff[0].insert.clone().cast::<TextRef>().unwrap();
        assert_eq!(
            nested2.get_string(&d2.transact()),
            "initial text".to_string()
        );

        mgr.undo(&mut d1);
        assert_eq!(nested.len(&d1.transact()), 0);
    }

    #[test]
    fn github_issue_345() {
        // https://github.com/y-crdt/y-crdt/issues/345
        let mut doc = Doc::new();
        let map = doc.get_or_insert_map("r");
        let mut mgr = UndoManager::with_scope_and_options(&mut doc, &map, Options::default());
        mgr.include_origin(doc.client_id());

        let s1 = map.insert(
            &mut doc.transact_mut_with(doc.client_id()),
            "s1",
            MapPrelim::default(),
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

        mgr.undo(&mut doc); // {"s1": {"a": ["a1", "a3", "a4"], "b": ["b1", "b2"]}}
        mgr.undo(&mut doc); // {"s1": {"a": ["a1", "a3"], "b": ["b1", "b2"]}}
        mgr.undo(&mut doc); // {"s1": {"a": ["a1"], "b": ["b1", "b2"]}}
        mgr.undo(&mut doc); // {"s1": {"a": ["a1"], "b": ["b1"]}}
        let actual = map.to_json(&doc.transact());
        assert_eq!(actual, any!({"s1": {"a": ["a1"], "b": ["b1"]}}));

        mgr.redo(&mut doc);
        let actual = map.to_json(&doc.transact());
        assert_eq!(actual, any!({"s1": {"b": ["b1", "b2"], "a": ["a1"]}}));

        mgr.redo(&mut doc);
        let actual = map.to_json(&doc.transact());
        assert_eq!(actual, any!({"s1": {"a": ["a1", "a3"], "b": ["b1", "b2"]}}));

        mgr.redo(&mut doc);
        let actual = map.to_json(&doc.transact());
        assert_eq!(
            actual,
            any!({"s1": {"a": ["a1", "a3", "a4"], "b": ["b1", "b2"]}})
        );

        mgr.redo(&mut doc);
        let actual = map.to_json(&doc.transact());
        assert_eq!(
            actual,
            any!({"s1": {"a": ["a1", "a3", "a4", "a5"], "b": ["b1", "b2"]}})
        );
    }

    #[test]
    fn github_issue_345_part_2() {
        // https://github.com/y-crdt/y-crdt/issues/345
        let mut doc = Doc::new();
        let r = doc.get_or_insert_map("r");

        let s1 = r.insert(&mut doc.transact_mut(), "s1", MapPrelim::default());

        let mut mgr = UndoManager::with_scope_and_options(&mut doc, &r, Options::default());
        {
            let mut txn = doc.transact_mut();
            let b1 = s1.insert(&mut txn, "b1", MapPrelim::default());
            b1.insert(&mut txn, "f1", 11);
        }
        mgr.reset();

        s1.remove(&mut doc.transact_mut(), "b1");
        mgr.reset();

        {
            let mut txn = doc.transact_mut();
            let b1 = s1.insert(&mut txn, "b1", MapPrelim::default());
            b1.insert(&mut txn, "f1", 20);
        }
        mgr.reset();

        let actual = r.to_json(&doc.transact());
        assert_eq!(actual, any!({"s1": {"b1": {"f1": 20}}}));

        mgr.undo(&mut doc);
        let actual = r.to_json(&doc.transact());
        assert_eq!(actual, any!({"s1": {}}));
        assert!(mgr.can_undo(), "should be able to undo");

        mgr.undo(&mut doc);
        let actual = r.to_json(&doc.transact());
        assert_eq!(actual, any!({"s1": {"b1": {"f1": 11}}}));
        assert!(mgr.can_undo(), "should be able to undo to the init state");
    }

    #[test]
    fn issue_371() {
        let mut doc = Doc::with_client_id(1);

        let r = doc.get_or_insert_map("r");
        let s1 = r.insert(&mut doc.transact_mut(), "s1", MapPrelim::default()); // { s1: {} }
        let b1_arr = s1.insert(&mut doc.transact_mut(), "b1", ArrayPrelim::default()); // { s1: { b1: [] } }
        let el1 = b1_arr.insert(&mut doc.transact_mut(), 0, MapPrelim::default()); // { s1: { b1: [{}] } }
        el1.insert(&mut doc.transact_mut(), "f1", 8); // { s1: { b1: [{ f1: 8 }] } }
        el1.insert(&mut doc.transact_mut(), "f2", true); // { s1: { b1: [{ f1: 8, f2: true }] } }

        let mut mgr = UndoManager::with_scope_and_options(&mut doc, &r, Options::default());
        {
            let mut txn = doc.transact_mut();
            let el0 = b1_arr.insert(&mut txn, 0, MapPrelim::default()); // { s1: { b1: [{}, { f1: 8, f2: true }] } }
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

        mgr.undo(&mut doc); // { s1: { b1: [{ f1: 8, f2: false }, { f1: 13, f2: false }] } }
        assert_eq!(
            r.to_json(&doc.transact()),
            any!({ "s1": { "b1": [{ "f1": 8, "f2": false }, { "f1": 13, "f2": false }] } })
        );
        mgr.undo(&mut doc); // { s1: { b1: [{ f1: 8, f2: false }, { f1: 13 }] } }
        assert_eq!(
            r.to_json(&doc.transact()),
            any!({ "s1": { "b1": [{ "f1": 8, "f2": false }, { "f1": 13 }] } })
        );
        mgr.undo(&mut doc); // { s1: { b1: [{ f1: 8, f2: true }] } }
        assert_eq!(
            r.to_json(&doc.transact()),
            any!({ "s1": { "b1": [{ "f1": 8, "f2": true }] } })
        );
        assert!(!mgr.undo(&mut doc)); // no more changes tracked by undo manager
    }

    #[test]
    fn issue_371_2() {
        let mut doc = Doc::with_client_id(1);
        let r = doc.get_or_insert_map("r");
        let s1 = r.insert(&mut doc.transact_mut(), "s1", MapPrelim::default()); // { s1:{} }
        s1.insert(&mut doc.transact_mut(), "f2", "AAA"); // { s1: { f2: AAA } }
        s1.insert(&mut doc.transact_mut(), "f1", false); // { s1: { f1: false, f2: AAA } }

        let mut mgr = UndoManager::with_scope_and_options(&mut doc, &r, Options::default());
        s1.remove(&mut doc.transact_mut(), "f2"); // { s1: { f1: false } }
        mgr.reset();

        s1.insert(&mut doc.transact_mut(), "f2", "C1"); // { s1: { f1: false, f2: C1 } }
        mgr.reset();

        s1.insert(&mut doc.transact_mut(), "f2", "C2"); // { s1: { f1: false, f2: C2 } }
        mgr.reset();

        mgr.undo(&mut doc); // { s1: { f1: false, f2: C1 } }
        assert_eq!(
            r.to_json(&doc.transact()),
            any!({ "s1": { "f1": false, "f2": "C1" } })
        );
        mgr.undo(&mut doc); // { s1: { f1: false } }
        assert_eq!(r.to_json(&doc.transact()), any!({ "s1": { "f1": false } }));
        mgr.undo(&mut doc); // { s1: { f1: false, f2: AAA } }
        assert_eq!(
            r.to_json(&doc.transact()),
            any!({ "s1": { "f1": false, "f2": "AAA" } })
        );
        assert!(!mgr.undo(&mut doc)); // no more changes tracked by undo manager
    }

    #[test]
    fn issue_380() {
        let mut doc = Doc::with_client_id(1);
        let r = doc.get_or_insert_map("r"); // {r:{}}
        let s1 = r.insert(&mut doc.transact_mut(), "s1", MapPrelim::default()); // {r:{s1:{}}
        let b1_arr = s1.insert(&mut doc.transact_mut(), "b1", ArrayPrelim::default()); // {r:{s1:{b1:[]}}

        let b1_el1 = b1_arr.insert(&mut doc.transact_mut(), 0, MapPrelim::default()); // {r:{s1:{b1:[{}]}}
        let b2_arr = b1_el1.insert(&mut doc.transact_mut(), "b2", ArrayPrelim::default()); // {r:{s1:{b1:[{b2:[]}]}}
        let b2_arr_nest = b2_arr.insert(&mut doc.transact_mut(), 0, ArrayPrelim::default()); // {r:{s1:{b1:[{b2:[[]]}]}}
        b2_arr_nest.insert(&mut doc.transact_mut(), 0, 232291652); // {r:{s1:{b1:[{b2:[[232291652]]}]}}
        b2_arr_nest.insert(&mut doc.transact_mut(), 1, -30); // {r:{s1:{b1:[{b2:[[232291652, -30]]}]}}

        let mut mgr = UndoManager::with_scope_and_options(&mut doc, &r, Options::default());

        let mut txn = doc.transact_mut();
        b2_arr_nest.remove(&mut txn, 1); // {r:{s1:{b1:[{b2:[[232291652]]}]}}
        b2_arr_nest.insert(&mut txn, 1, -5); // {r:{s1:{b1:[{b2:[[232291652, -5]]}]}}

        drop(txn);
        mgr.reset();

        let mut txn = doc.transact_mut();

        let b1_el0 = b1_arr.insert(&mut txn, 0, MapPrelim::default()); // {r:{s1:{b1:[{},{b2:[[232291652, -5]]}]}}
        let b2_0_arr = b1_el0.insert(&mut txn, "b2", ArrayPrelim::default()); // {r:{s1:{b1:[{b2:[]},{b2:[[232291652, -5]]}]}}
        let b2_0_arr_nest = b2_0_arr.insert(&mut txn, 0, ArrayPrelim::default()); // {r:{s1:{b1:[{b2:[[]]},{b2:[[232291652, -5]]}]}}
        b2_0_arr_nest.insert(&mut txn, 0, 232291652); // {r:{s1:{b1:[{b2:[[232291652]]},{b2:[[232291652, -5]]}]}}
        b2_0_arr_nest.insert(&mut txn, 1, -6); // {r:{s1:{b1:[{b2:[[232291652, -6]]},{b2:[[232291652, -5]]}]}}
        b1_arr.remove(&mut txn, 1); // {r:{s1:{b1:[{b2:[[232291652, -6]]}]}}

        let b1_el1 = b1_arr.insert(&mut txn, 1, MapPrelim::default()); // {r:{s1:{b1:[{b2:[[232291652, -6]]}, {}]}}
        b1_el1.insert(&mut txn, "f2", "C1"); // {r:{s1:{b1:[{b2:[[232291652, -6]]}, {f2:C1}]}}

        drop(txn);
        mgr.reset();
        assert_eq!(
            r.to_json(&doc.transact()),
            any!({"s1":{"b1":[{"b2":[[232291652, -6]]}, {"f2":"C1"}]}})
        );

        mgr.undo(&mut doc); // {r:{s1:{b1:[{b2:[[232291652, -5]]}]}}
        assert_eq!(
            r.to_json(&doc.transact()),
            any!({"s1":{"b1":[{"b2":[[232291652, -5]]}]}})
        );

        mgr.undo(&mut doc); // {r:{s1:{b1:[{b2:[[232291652, -30]]}]}}
        assert_eq!(
            r.to_json(&doc.transact()),
            any!({"s1":{"b1":[{"b2":[[232291652, -30]]}]}})
        );
    }
}
