use crate::block::{Block, BlockPtr};
use crate::doc::{AfterTransactionSubscription, TransactionAcqError};
use crate::transaction::Origin;
use crate::types::{Branch, BranchPtr};
use crate::{
    DeleteSet, DestroySubscription, Doc, Observer, Store, Subscription, SubscriptionId, Transact,
    TransactionMut, ID,
};
use std::cell::Cell;
use std::collections::HashSet;
use std::fmt::Formatter;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[repr(transparent)]
pub struct UndoManager(Box<Inner>);

struct Inner {
    doc: Doc,
    scope: HashSet<BranchPtr>,
    options: Options,
    undo_stack: Vec<StackItem>,
    redo_stack: Vec<StackItem>,
    undoing: Cell<bool>,
    redoing: Cell<bool>,
    last_change: SystemTime,
    on_after_transaction: Option<AfterTransactionSubscription>,
    on_destroy: Option<DestroySubscription>,
    observer_added: Observer<Arc<dyn Fn(&TransactionMut, &Event) -> ()>>,
    observer_updated: Observer<Arc<dyn Fn(&TransactionMut, &Event) -> ()>>,
    observer_popped: Observer<Arc<dyn Fn(&TransactionMut, &Event) -> ()>>,
}

impl UndoManager {
    pub fn new<T>(doc: &Doc, scope: &T) -> Self
    where
        T: AsRef<Branch>,
    {
        Self::with_options(doc, scope, Options::default())
    }

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
            last_change: SystemTime::UNIX_EPOCH,
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
            .insert(Origin::from(unsafe { inner_ptr as usize }));
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
                .unwrap_or(true)
    }

    fn handle_after_transaction(inner: &mut Inner, txn: &mut TransactionMut) {
        if Self::should_skip(inner, txn) {
            return;
        }
        let undoing = inner.undoing.get();
        let redoing = inner.redoing.get();
        if undoing {
            inner.last_change = UNIX_EPOCH; // next undo should not be appended to last stack item
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
        let now = SystemTime::now();
        let stack = if undoing {
            &mut inner.redo_stack
        } else {
            &mut inner.undo_stack
        };
        let extend = !undoing
            && !redoing
            && !stack.is_empty()
            && inner.last_change > UNIX_EPOCH
            && now.duration_since(inner.last_change).unwrap() < inner.options.capture_timeout;

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

    pub fn observe_item_added<F>(&self, f: F) -> UndoEventSubscription
    where
        F: Fn(&TransactionMut, &Event) -> () + 'static,
    {
        self.0.observer_added.subscribe(Arc::new(f))
    }

    pub fn unobserve_item_added(&self, subscription_id: SubscriptionId) {
        self.0.observer_added.unsubscribe(subscription_id)
    }

    pub fn observe_item_popped<F>(&self, f: F) -> UndoEventSubscription
    where
        F: Fn(&TransactionMut, &Event) -> () + 'static,
    {
        self.0.observer_popped.subscribe(Arc::new(f))
    }

    pub fn unobserve_item_popped(&self, subscription_id: SubscriptionId) {
        self.0.observer_popped.unsubscribe(subscription_id)
    }

    pub fn expand_scope<T>(&mut self, scope: &T)
    where
        T: AsRef<Branch>,
    {
        let ptr = BranchPtr::from(scope.as_ref());
        self.0.scope.insert(ptr);
    }

    pub fn include_origin<O>(&mut self, origin: O)
    where
        O: Into<Origin>,
    {
        self.0.options.tracked_origins.insert(origin.into());
    }

    pub fn exclude_origin<O>(&mut self, origin: O)
    where
        O: Into<Origin>,
    {
        self.0.options.tracked_origins.remove(&origin.into());
    }

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
    /// [Options::capture_timeout]. You can call this method so that the next stack item won't be
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
    /// mgr.stop();
    /// txt.insert(&mut doc.transact_mut(), 1, "b");
    /// mgr.undo().unwrap();
    /// txt.get_string(&doc.transact()); // => "a" (note that only 'b' was removed)
    /// ```
    pub fn stop(&mut self) {
        self.0.last_change = UNIX_EPOCH;
    }

    /// Are there any undo steps available?
    pub fn can_undo(&self) -> bool {
        !self.0.undo_stack.is_empty()
    }

    /// Undo last change on type.
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

    /// Redo last undo operation.
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
                        let (block, diff) = follow_redone(txn.store(), &id);
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

    fn fmt_with(&self, txn: &mut TransactionMut) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        s.push_str("UndoManager(");
        write!(&mut s, "\n\tscope: {:?}", &self.0.scope).unwrap();
        write!(&mut s, "\n\torigins: {:?}", &self.0.options.tracked_origins).unwrap();
        if !self.0.undo_stack.is_empty() {
            s.push_str("\n\tundo: [");
            for stack_item in self.0.undo_stack.iter() {
                s.push_str("\n\t\t");
                s.push_str(&stack_item.fmt_with(txn));
            }
            s.push_str("\n\t]");
        }
        if !self.0.redo_stack.is_empty() {
            s.push_str("\n\tredo: [");
            for stack_item in self.0.redo_stack.iter() {
                s.push_str("\n\t\t");
                s.push_str(&stack_item.fmt_with(txn));
            }
            s.push_str("\n\t]");
        }
        s
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

fn follow_redone(store: &Store, id: &ID) -> (BlockPtr, u32) {
    let mut next_id = Some(*id);
    let mut ptr = None;
    let mut diff = 0;
    while {
        if let Some(mut next) = next_id {
            if diff > 0 {
                next.clock += diff;
                next_id = Some(next.clone());
            }
            ptr = store.blocks.get_block(&next);
            if let Some(Block::Item(item)) = ptr.as_deref() {
                diff = next.clock - item.id.clock;
                next_id = item.redone;
                true
            } else {
                false
            }
        } else {
            false
        }
    } {}
    (ptr.unwrap(), diff)
}

pub type UndoEventSubscription = Subscription<Arc<dyn Fn(&TransactionMut, &Event) -> ()>>;

#[derive(Clone)]
pub struct Options {
    pub capture_timeout: Duration,
    pub tracked_origins: HashSet<Origin>,
    pub capture_transaction: Rc<dyn Fn(&TransactionMut) -> bool>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            capture_timeout: Duration::from_millis(500),
            tracked_origins: HashSet::new(),
            capture_transaction: Rc::new(|_txn| true),
        }
    }
}

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

    pub fn deletions(&self) -> &DeleteSet {
        &self.deletions
    }

    pub fn insertions(&self) -> &DeleteSet {
        &self.insertions
    }

    fn fmt_with(&self, txn: &mut TransactionMut) -> String {
        let i: Vec<_> = self
            .insertions
            .deleted_blocks(txn)
            .map(|ptr| ptr.deref().to_string())
            .collect();
        let d: Vec<_> = self
            .deletions
            .deleted_blocks(txn)
            .map(|ptr| ptr.deref().to_string())
            .collect();
        format!("StackItem({:#?}, {:#?})", i, d)
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
    use crate::{
        Array, Doc, GetString, Map, MapPrelim, ReadTxn, StateVector, Text, Transact, Update, Xml,
        XmlElementPrelim, XmlElementRef, XmlFragment, XmlTextPrelim,
    };
    use lib0::any::Any;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::convert::TryInto;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    //fn print_undo_manager(prefix: &str, mgr: &UndoManager) {
    //    println!("{}", prefix);
    //    if !mgr.0.undo_stack.is_empty() {
    //        println!("undo stack:");
    //        for i in mgr.0.undo_stack.iter() {
    //            println!("\t{}", i);
    //        }
    //    }
    //    if !mgr.0.redo_stack.is_empty() {
    //        println!("redo stack:");
    //        for i in mgr.0.redo_stack.iter() {
    //            println!("\t{}", i);
    //        }
    //    }
    //}

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
        mgr.stop();
        txt1.remove_range(&mut d1.transact_mut(), 0, 1);
        mgr.stop();
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
        let sub_type = MapPrelim::from([("x".to_owned(), 42)]);
        map1.insert(&mut d1.transact_mut(), "a", MapPrelim::<u32>::new());
        let sub_type = map1.get(&d1.transact(), "a").unwrap().to_ymap().unwrap();
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
        mgr.stop();
        map1.insert(&mut d1.transact_mut(), "b", "val1");
        map1.insert(&mut d1.transact_mut(), "b", "val2");
        mgr.stop();
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
        array1.insert(&mut d1.transact_mut(), 0, MapPrelim::<u32>::new());
        let map = array1.get(&d1.transact(), 0).unwrap().to_ymap().unwrap();
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{}]"#).unwrap();
        assert_eq!(actual, expected);
        mgr.stop();
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

        let map2 = array2.get(&d2.transact(), 0).unwrap().to_ymap().unwrap();
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
        let text_child = child.insert(&mut d1.transact_mut(), 0, XmlTextPrelim("content"));

        assert_eq!(
            xml1.get_string(&d1.transact()),
            "<undefined><p>content</p></undefined>"
        );
        // format textchild and revert that change
        mgr.stop();
        text_child.format(
            &mut d1.transact_mut(),
            3,
            4,
            Attrs::from([("bold".into(), Any::Map(Box::new(HashMap::new())))]),
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
        let _sub1 = mgr.observe_item_added(move |txn, e| {
            assert!(e.has_changed(&txt_clone));
            let c = counter.fetch_add(1, Ordering::SeqCst);
            let mut data = data_clone.borrow_mut();
            data.insert(e.item.clone(), c);
        });

        let txt_clone = txt.clone();
        let data_clone = data.clone();
        let result_clone = result.clone();
        let _sub2 = mgr.observe_item_popped(move |txn, e| {
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
        let doc1 = Doc::with_client_id(1);
        let doc2 = Doc::with_client_id(2);
        let d1 = doc1.clone();
        let d2 = doc2.clone();
        let _sub1 = doc1.observe_update_v1(move |txn, e| {
            d2.transact_mut()
                .apply_update(Update::decode_v1(&e.update).unwrap());
        });
        let _sub2 = doc2.observe_update_v1(move |txn, e| {
            d1.transact_mut()
                .apply_update(Update::decode_v1(&e.update).unwrap());
        });

        let arr1 = doc1.get_or_insert_array("array");
        let arr2 = doc2.get_or_insert_array("array");
        arr1.push_back(
            &mut doc1.transact_mut(),
            MapPrelim::from([("hello".to_owned(), "world".to_owned())]),
        );
        let map1 = arr1.get(&doc1.transact(), 0).unwrap().to_ymap().unwrap();
        arr2.push_back(
            &mut doc2.transact_mut(),
            MapPrelim::from([("key".to_owned(), "value".to_owned())]),
        );
        let map2 = arr1.get(&doc2.transact(), 0).unwrap().to_ymap().unwrap();

        let mut mgr1 = UndoManager::new(&doc1, &arr1);
        mgr1.include_origin(doc1.client_id());
        let mut mgr2 = UndoManager::new(&doc2, &arr2);
        mgr2.include_origin(doc2.client_id());

        map2.insert(
            &mut doc2.transact_mut_with(doc1.client_id()),
            "key",
            "value modified",
        );
        mgr1.stop();

        map1.insert(
            &mut doc2.transact_mut_with(doc1.client_id()),
            "hello",
            "world modified",
        );
        arr1.remove_range(&mut doc1.transact_mut_with(doc2.client_id()), 0, 1);
        mgr2.undo().unwrap();
        mgr1.undo().unwrap();
        assert_eq!(map2.get(&doc2.transact(), "key"), Some("value".into()));
    }

    #[test]
    fn nested_undo() {
        // This issue has been reported in https://github.com/yjs/yjs/issues/317
        let doc = Doc::with_options({
            let mut o = crate::doc::Options::default();
            o.skip_gc = true;
            o.client_id = 1;
            o
        });
        let design = doc.get_or_insert_map("map");
        let mut mgr = UndoManager::with_options(&doc, &design, {
            let mut o = Options::default();
            o.capture_timeout = Duration::default();
            o
        });
        {
            let mut txn = doc.transact_mut();
            design.insert(&mut txn, "text", MapPrelim::<u32>::new());
            let a = design.get(&txn, "text").unwrap().to_ymap().unwrap();
            a.insert(
                &mut txn,
                "blocks",
                MapPrelim::from([("text".to_owned(), "Type something".to_owned())]),
            );
        }

        {
            let mut txn = doc.transact_mut();
            design.insert(&mut txn, "blocks", MapPrelim::<u32>::new());
            let a = design.get(&txn, "text").unwrap().to_ymap().unwrap();
            a.insert(
                &mut txn,
                "blocks",
                MapPrelim::from([("text".to_owned(), "Something".to_owned())]),
            );
        }

        {
            let mut txn = doc.transact_mut();
            design.insert(&mut txn, "blocks", MapPrelim::<u32>::new());
            let a = design.get(&txn, "text").unwrap().to_ymap().unwrap();
            a.insert(
                &mut txn,
                "blocks",
                MapPrelim::from([("text".to_owned(), "Something else".to_owned())]),
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
        let point = root.get(&doc.transact(), "a").unwrap().to_ymap().unwrap();
        mgr.stop();

        point.insert(&mut doc.transact_mut(), "x", 100);
        point.insert(&mut doc.transact_mut(), "y", 100);
        mgr.stop();

        point.insert(&mut doc.transact_mut(), "x", 200);
        point.insert(&mut doc.transact_mut(), "y", 200);
        mgr.stop();

        point.insert(&mut doc.transact_mut(), "x", 300);
        point.insert(&mut doc.transact_mut(), "y", 300);
        mgr.stop();

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
        let point = root.get(&doc.transact(), "a").unwrap().to_ymap().unwrap();

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
            o.capture_timeout = Duration::default();
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
        assert_eq!(str, r#"<test-node a="180" b="50"></test-node>"#);
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
            o.capture_timeout = Duration::default();
            o
        });
        let text = {
            let mut txn = doc.transact_mut();
            design.insert(
                &mut txn,
                "text",
                MapPrelim::from([(
                    "blocks".into(),
                    MapPrelim::from([("text".to_owned(), "1".to_owned())]),
                )]),
            );
            design.get(&txn, "text").unwrap().to_ymap().unwrap()
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
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        txt.insert(
            &mut doc.transact_mut(),
            0,
            "Attack ships on fire off the shoulder of Orion.",
        );
        let doc2 = Doc::with_client_id(2);
        let txt2 = doc2.get_or_insert_text("test");

        doc2.transact_mut().apply_update(
            Update::decode_v1(
                &doc.transact()
                    .encode_state_as_update_v1(&StateVector::default()),
            )
            .unwrap(),
        );
        let mut mgr = UndoManager::new(&doc, &txt);
        let attrs = Attrs::from([("bold".into(), true.into())]);
        txt.format(&mut doc.transact_mut(), 13, 7, attrs.clone());
        mgr.stop();
        doc2.transact_mut().apply_update(
            Update::decode_v1(
                &doc.transact()
                    .encode_state_as_update_v1(&StateVector::default()),
            )
            .unwrap(),
        );
        let attrs2 = Attrs::from([("bold".into(), Any::Null)]);
        txt.format(&mut doc.transact_mut(), 16, 4, attrs.clone());

        mgr.stop();
        doc2.transact_mut().apply_update(
            Update::decode_v1(
                &doc.transact()
                    .encode_state_as_update_v1(&StateVector::default()),
            )
            .unwrap(),
        );

        mgr.undo().unwrap();
        doc2.transact_mut().apply_update(
            Update::decode_v1(
                &doc.transact()
                    .encode_state_as_update_v1(&StateVector::default()),
            )
            .unwrap(),
        );
        let expected = vec![
            Diff::new("Attack ships ".into(), None),
            Diff::new("on fire".into(), Some(Box::new(attrs))),
            Diff::new(" off the shoulder of Orion.".into(), None),
        ];
        assert_eq!(txt.diff(&doc.transact(), YChange::identity), expected);
        assert_eq!(txt2.diff(&doc2.transact(), YChange::identity), expected);
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
        assert_eq!(
            f.get_string(&doc.transact()),
            r#"<test a="1" b="2"></test>"#
        );
        {
            // change attribute "b" and delete test-node
            let mut txn = doc.transact_mut_with(ORIGIN);
            let e: XmlElementRef = f.get(&txn, 0).unwrap().try_into().unwrap();
            e.insert_attribute(&mut txn, "b", "3");
            f.remove_range(&mut txn, 0, 1);
        }
        assert_eq!(f.get_string(&doc.transact()), "");
        mgr.undo().unwrap();
        assert_eq!(
            f.get_string(&doc.transact()),
            r#"<test a="1" b="2"></test>"#
        );
    }
}
