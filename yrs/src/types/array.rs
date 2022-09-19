use crate::block::{ItemContent, Prelim};
use crate::block_iter::{BlockIter, SliceConcat};
use crate::event::Subscription;
use crate::moving::RelativePosition;
use crate::types::{
    event_change_set, Branch, BranchPtr, Change, ChangeSet, Observers, Path, Value, TYPE_REFS_ARRAY,
};
use crate::{SubscriptionId, Transaction, ID};
use lib0::any::Any;
use std::cell::UnsafeCell;
use std::collections::HashSet;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

/// A collection used to store data in an indexed sequence structure. This type is internally
/// implemented as a double linked list, which may squash values inserted directly one after another
/// into single list node upon transaction commit.
///
/// Reading a root-level type as an YArray means treating its sequence components as a list, where
/// every countable element becomes an individual entity:
///
/// - JSON-like primitives (booleans, numbers, strings, JSON maps, arrays etc.) are counted
///   individually.
/// - Text chunks inserted by [Text] data structure: each character becomes an element of an
///   array.
/// - Embedded and binary values: they count as a single element even though they correspond of
///   multiple bytes.
///
/// Like all Yrs shared data types, YArray is resistant to the problem of interleaving (situation
/// when elements inserted one after another may interleave with other peers concurrent inserts
/// after merging all updates together). In case of Yrs conflict resolution is solved by using
/// unique document id to determine correct and consistent ordering.
#[repr(transparent)]
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Array(BranchPtr);

impl Array {
    /// Returns a number of elements stored in current array.
    pub fn len(&self) -> u32 {
        self.0.len()
    }

    /// Inserts a `value` at the given `index`. Inserting at index `0` is equivalent to prepending
    /// current array with given `value`, while inserting at array length is equivalent to appending
    /// that value at the end of it.
    ///
    /// Using `index` value that's higher than current array length results in panic.
    pub fn insert<V: Prelim>(&self, txn: &mut Transaction, index: u32, value: V) {
        let mut walker = BlockIter::new(self.0);
        if walker.try_forward(txn, index) {
            walker.insert_contents(txn, value)
        } else {
            panic!("Index {} is outside of the range of an array", index);
        }
    }

    /// Inserts multiple `values` at the given `index`. Inserting at index `0` is equivalent to
    /// prepending current array with given `values`, while inserting at array length is equivalent
    /// to appending that value at the end of it.
    ///
    /// Using `index` value that's higher than current array length results in panic.
    pub fn insert_range<T, V>(&self, txn: &mut Transaction, index: u32, values: T)
    where
        T: IntoIterator<Item = V>,
        V: Into<Any>,
    {
        self.insert(txn, index, PrelimRange(values))
    }

    /// Inserts given `value` at the end of the current array.
    pub fn push_back<V: Prelim>(&self, txn: &mut Transaction, value: V) {
        let len = self.len();
        self.insert(txn, len, value)
    }

    /// Inserts given `value` at the beginning of the current array.
    pub fn push_front<V: Prelim>(&self, txn: &mut Transaction, content: V) {
        self.insert(txn, 0, content)
    }

    /// Removes a single element at provided `index`.
    pub fn remove(&self, txn: &mut Transaction, index: u32) {
        self.remove_range(txn, index, 1)
    }

    /// Removes a range of elements from current array, starting at given `index` up until
    /// a particular number described by `len` has been deleted. This method panics in case when
    /// not all expected elements were removed (due to insufficient number of elements in an array)
    /// or `index` is outside of the bounds of an array.
    pub fn remove_range(&self, txn: &mut Transaction, index: u32, len: u32) {
        let mut walker = BlockIter::new(self.0);
        if walker.try_forward(txn, index) {
            walker.delete(txn, len)
        } else {
            panic!("Index {} is outside of the range of an array", index);
        }
    }

    /// Retrieves a value stored at a given `index`. Returns `None` when provided index was out
    /// of the range of a current array.
    pub fn get(&self, index: u32) -> Option<Value> {
        let mut txn = self.0.try_transact().expect("Array is not integrated");
        let mut walker = BlockIter::new(self.0);
        if walker.try_forward(&mut txn, index) {
            walker.read_value(&mut txn)
        } else {
            None
        }
    }

    /// Moves element found at `source` index into `target` index position.
    pub fn move_to(&self, txn: &mut Transaction, source: u32, target: u32) {
        if source == target || source + 1 == target {
            // It doesn't make sense to move a range into the same range (it's basically a no-op).
            return;
        }
        let left = RelativePosition::from_type_index(txn, self.0, source, true)
            .expect("unbounded relative positions are not supported yet");
        let mut right = left.clone();
        right.assoc = false;
        let mut walker = BlockIter::new(self.0);
        if walker.try_forward(txn, target) {
            walker.insert_move(txn, left, right);
        } else {
            panic!("Index {} is outside of the range of an array", target);
        }
    }

    /// Moves all elements found within `start`..`end` indexes range (both side inclusive) into
    /// new position pointed by `target` index. All elements inserted concurrently by other peers
    /// inside of moved range will be moved as well after synchronization (although it make take
    /// more than one sync roundtrip to achieve convergence).
    ///
    /// `assoc_start`/`assoc_end` flags are used to mark if ranges should include elements that
    /// might have been inserted concurrently at the edges of the range definition.
    ///
    /// Example:
    /// ```
    /// use yrs::Doc;
    /// let doc = Doc::new();
    /// let array = doc.transact().get_array("array");
    /// array.insert_range(&mut doc.transact(), 0, [1,2,3,4]);
    /// // move elements 2 and 3 after the 4
    /// array.move_range_to(&mut doc.transact(), 1, true, 2, false, 4);
    /// ```
    pub fn move_range_to(
        &self,
        txn: &mut Transaction,
        start: u32,
        assoc_start: bool,
        end: u32,
        assoc_end: bool,
        target: u32,
    ) {
        if start <= target && target <= end {
            // It doesn't make sense to move a range into the same range (it's basically a no-op).
            return;
        }
        let left = RelativePosition::from_type_index(txn, self.0, start, assoc_start)
            .expect("unbounded relative positions are not supported yet");
        let right = RelativePosition::from_type_index(txn, self.0, end + 1, assoc_end)
            .expect("unbounded relative positions are not supported yet");
        let mut walker = BlockIter::new(self.0);
        if walker.try_forward(txn, target) {
            walker.insert_move(txn, left, right);
        } else {
            panic!("Index {} is outside of the range of an array", target);
        }
    }

    /// Returns an iterator, that can be used to lazely traverse over all values stored in a current
    /// array.
    pub fn iter(&self) -> ArrayIter {
        ArrayIter::new(self)
    }

    /// Converts all contents of current array into a JSON-like representation.
    pub fn to_json(&self) -> Any {
        let len = self.0.len();
        let mut walker = BlockIter::new(self.0);
        let mut txn = self.0.try_transact().unwrap();
        let values = walker
            .slice::<ArraySliceConcat>(&mut txn, len, Vec::default())
            .unwrap();
        let res = values.into_iter().map(Value::to_json).collect();
        Any::Array(res)
    }

    /// Subscribes a given callback to be triggered whenever current array is changed.
    /// A callback is triggered whenever a transaction gets committed. This function does not
    /// trigger if changes have been observed by nested shared collections.
    ///
    /// All array changes can be tracked by using [Event::delta] method.
    ///
    /// Returns an [Observer] which, when dropped, will unsubscribe current callback.
    pub fn observe<F>(&mut self, f: F) -> Subscription<ArrayEvent>
    where
        F: Fn(&Transaction, &ArrayEvent) -> () + 'static,
    {
        if let Observers::Array(eh) = self.0.observers.get_or_insert_with(Observers::array) {
            eh.subscribe(f)
        } else {
            panic!("Observed collection is of different type") //TODO: this should be Result::Err
        }
    }

    /// Unsubscribes a previously subscribed event callback identified by given `subscription_id`.
    pub fn unobserve(&mut self, subscription_id: SubscriptionId) {
        if let Some(Observers::Array(eh)) = self.0.observers.as_mut() {
            eh.unsubscribe(subscription_id);
        }
    }
}

impl AsRef<Branch> for Array {
    fn as_ref(&self) -> &Branch {
        self.0.deref()
    }
}

impl AsMut<Branch> for Array {
    fn as_mut(&mut self) -> &mut Branch {
        self.0.deref_mut()
    }
}

pub struct ArrayIter<'a> {
    inner: BlockIter,
    txn: Transaction,
    _marker: PhantomData<&'a Array>,
}

impl<'a> ArrayIter<'a> {
    fn new(array: &'a Array) -> Self {
        ArrayIter {
            inner: BlockIter::new(array.0),
            txn: array.0.try_transact().unwrap(),
            _marker: PhantomData,
        }
    }
}

impl<'b> Iterator for ArrayIter<'b> {
    type Item = Value;

    fn next(&mut self) -> Option<Self::Item> {
        if self.inner.finished() {
            None
        } else {
            let mut res = self
                .inner
                .slice::<ArraySliceConcat>(&mut self.txn, 1, Vec::default())?;
            res.pop()
        }
    }
}

impl From<BranchPtr> for Array {
    fn from(inner: BranchPtr) -> Self {
        Array(inner)
    }
}

/// A preliminary array. It's can be used to initialize an YArray, when it's about to be nested
/// into another Yrs data collection, such as [Map] or another YArray.
pub struct PrelimArray<T, V>(T)
where
    T: IntoIterator<Item = V>;

impl<T, V> From<T> for PrelimArray<T, V>
where
    T: IntoIterator<Item = V>,
{
    fn from(iter: T) -> Self {
        PrelimArray(iter)
    }
}

/// Prelim range defines a way to insert multiple elements effectively at once one after another
/// in an efficient way, provided that these elements correspond to a primitive JSON-like types.
struct PrelimRange<T, V>(T)
where
    T: IntoIterator<Item = V>,
    V: Into<Any>;

impl<T, V> Prelim for PrelimRange<T, V>
where
    T: IntoIterator<Item = V>,
    V: Into<Any>,
{
    fn into_content(self, _txn: &mut Transaction) -> (ItemContent, Option<Self>) {
        let vec: Vec<Any> = self.0.into_iter().map(|v| v.into()).collect();
        (ItemContent::Any(vec), None)
    }

    fn integrate(self, _txn: &mut Transaction, _inner_ref: BranchPtr) {}
}

impl<T, V> Prelim for PrelimArray<T, V>
where
    V: Prelim,
    T: IntoIterator<Item = V>,
{
    fn into_content(self, _txn: &mut Transaction) -> (ItemContent, Option<Self>) {
        let inner = Branch::new(TYPE_REFS_ARRAY, None);
        (ItemContent::Type(inner), Some(self))
    }

    fn integrate(self, txn: &mut Transaction, inner_ref: BranchPtr) {
        let array = Array::from(inner_ref);
        for value in self.0 {
            array.push_back(txn, value);
        }
    }
}

/// Event generated by [Array::observe] method. Emitted during transaction commit phase.
pub struct ArrayEvent {
    pub(crate) current_target: BranchPtr,
    target: Array,
    change_set: UnsafeCell<Option<Box<ChangeSet<Change>>>>,
}

impl ArrayEvent {
    pub(crate) fn new(branch_ref: BranchPtr) -> Self {
        let current_target = branch_ref.clone();
        ArrayEvent {
            target: Array::from(branch_ref),
            current_target,
            change_set: UnsafeCell::new(None),
        }
    }

    /// Returns an [Array] instance which emitted this event.
    pub fn target(&self) -> &Array {
        &self.target
    }

    /// Returns a path from root type down to [Text] instance which emitted this event.
    pub fn path(&self) -> Path {
        Branch::path(self.current_target, self.target.0)
    }

    /// Returns summary of changes made over corresponding [Array] collection within
    /// a bounds of current transaction.
    pub fn delta(&self, txn: &Transaction) -> &[Change] {
        self.changes(txn).delta.as_slice()
    }

    /// Returns a collection of block identifiers that have been added within a bounds of
    /// current transaction.
    pub fn inserts(&self, txn: &Transaction) -> &HashSet<ID> {
        &self.changes(txn).added
    }

    /// Returns a collection of block identifiers that have been removed within a bounds of
    /// current transaction.
    pub fn removes(&self, txn: &Transaction) -> &HashSet<ID> {
        &self.changes(txn).deleted
    }

    fn changes(&self, txn: &Transaction) -> &ChangeSet<Change> {
        let change_set = unsafe { self.change_set.get().as_mut().unwrap() };
        change_set.get_or_insert_with(|| Box::new(event_change_set(txn, self.target.0.start)))
    }
}

pub(crate) struct ArraySliceConcat;

impl SliceConcat for ArraySliceConcat {
    fn slice(content: &mut ItemContent, offset: usize, len: usize) -> Vec<Value> {
        let mut content = content.get_content();
        if content.len() <= len && offset == 0 {
            content
        } else {
            if offset != 0 {
                for _ in content.drain(0..offset) { /* do nothing */ }
            }
            for _ in content.drain(len..) { /* do nothing */ }
            content
        }
    }

    #[inline]
    fn concat(mut a: Vec<Value>, b: Vec<Value>) -> Vec<Value> {
        a.extend(b);
        a
    }
}

#[cfg(test)]
mod test {
    use crate::test_utils::{exchange_updates, run_scenario, RngExt};
    use crate::types::map::PrelimMap;
    use crate::types::{Change, DeepObservable, Event, Path, PathSegment, Value};
    use crate::{Doc, PrelimArray, StateVector, Update, ID};
    use lib0::any::Any;
    use rand::prelude::StdRng;
    use rand::Rng;
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, HashSet};
    use std::fs::File;
    use std::ops::Deref;
    use std::rc::Rc;

    #[test]
    fn push_back() {
        let doc = Doc::with_client_id(1);
        let mut txn = doc.transact();
        let a = txn.get_array("array");

        a.push_back(&mut txn, "a");
        a.push_back(&mut txn, "b");
        a.push_back(&mut txn, "c");

        let actual: Vec<_> = a.iter().collect();
        assert_eq!(actual, vec!["a".into(), "b".into(), "c".into()]);
    }

    #[test]
    fn push_front() {
        let doc = Doc::with_client_id(1);
        let mut txn = doc.transact();
        let a = txn.get_array("array");

        a.push_front(&mut txn, "c");
        a.push_front(&mut txn, "b");
        a.push_front(&mut txn, "a");

        let actual: Vec<_> = a.iter().collect();
        assert_eq!(actual, vec!["a".into(), "b".into(), "c".into()]);
    }

    #[test]
    fn insert() {
        let doc = Doc::with_client_id(1);
        let mut txn = doc.transact();
        let a = txn.get_array("array");

        a.insert(&mut txn, 0, "a");
        a.insert(&mut txn, 1, "c");
        a.insert(&mut txn, 1, "b");

        let actual: Vec<_> = a.iter().collect();
        assert_eq!(actual, vec!["a".into(), "b".into(), "c".into()]);
    }

    #[test]
    fn basic() {
        let d1 = Doc::with_client_id(1);
        let d2 = Doc::with_client_id(2);

        let mut t1 = d1.transact();
        let a1 = t1.get_array("array");

        a1.insert(&mut t1, 0, "Hi");
        let update = d1.encode_state_as_update_v1(&StateVector::default());

        let mut t2 = d2.transact();
        t2.apply_update(Update::decode_v1(update.as_slice()).unwrap());
        let a2 = t2.get_array("array");
        let actual: Vec<_> = a2.iter().collect();

        assert_eq!(actual, vec!["Hi".into()]);
    }

    #[test]
    fn len() {
        let d = Doc::with_client_id(1);

        {
            let mut txn = d.transact();
            let a = txn.get_array("array");

            a.push_back(&mut txn, 0); // len: 1
            a.push_back(&mut txn, 1); // len: 2
            a.push_back(&mut txn, 2); // len: 3
            a.push_back(&mut txn, 3); // len: 4

            a.remove_range(&mut txn, 0, 1); // len: 3
            a.insert(&mut txn, 0, 0); // len: 4

            assert_eq!(a.len(), 4);
        }
        {
            let mut txn = d.transact();
            let a = txn.get_array("array");
            a.remove_range(&mut txn, 1, 1); // len: 3
            assert_eq!(a.len(), 3);

            a.insert(&mut txn, 1, 1); // len: 4
            assert_eq!(a.len(), 4);

            a.remove_range(&mut txn, 2, 1); // len: 3
            assert_eq!(a.len(), 3);

            a.insert(&mut txn, 2, 2); // len: 4
            assert_eq!(a.len(), 4);
        }

        let mut txn = d.transact();
        let a = txn.get_array("array");
        assert_eq!(a.len(), 4);

        a.remove_range(&mut txn, 1, 1);
        assert_eq!(a.len(), 3);

        a.insert(&mut txn, 1, 1);
        assert_eq!(a.len(), 4);
    }

    #[test]
    fn remove_insert() {
        let d1 = Doc::with_client_id(1);

        let mut t1 = d1.transact();
        let a1 = t1.get_array("array");
        a1.insert(&mut t1, 0, "A");
        a1.remove_range(&mut t1, 1, 0);
    }

    #[test]
    fn insert_3_elements_try_re_get() {
        let d1 = Doc::with_client_id(1);
        let d2 = Doc::with_client_id(2);
        {
            let mut t1 = d1.transact();
            let a1 = t1.get_array("array");

            a1.push_back(&mut t1, 1);
            a1.push_back(&mut t1, true);
            a1.push_back(&mut t1, false);
            let actual: Vec<_> = a1.iter().collect();
            assert_eq!(
                actual,
                vec![Value::from(1.0), Value::from(true), Value::from(false)]
            );
        }

        exchange_updates(&[&d1, &d2]);

        let mut t2 = d2.transact();
        let a2 = t2.get_array("array");
        let actual: Vec<_> = a2.iter().collect();
        assert_eq!(
            actual,
            vec![Value::from(1.0), Value::from(true), Value::from(false)]
        );
    }

    #[test]
    fn concurrent_insert_with_3_conflicts() {
        let d1 = Doc::with_client_id(1);
        {
            let mut txn = d1.transact();
            let a = txn.get_array("array");
            a.insert(&mut txn, 0, 0);
        }

        let d2 = Doc::with_client_id(2);
        {
            let mut txn = d1.transact();
            let a = txn.get_array("array");
            a.insert(&mut txn, 0, 1);
        }

        let d3 = Doc::with_client_id(3);
        {
            let mut txn = d1.transact();
            let a = txn.get_array("array");
            a.insert(&mut txn, 0, 2);
        }

        exchange_updates(&[&d1, &d2, &d3]);

        let a1 = to_array(&d1);
        let a2 = to_array(&d2);
        let a3 = to_array(&d3);

        assert_eq!(a1, a2, "Peer 1 and peer 2 states are different");
        assert_eq!(a2, a3, "Peer 2 and peer 3 states are different");
    }

    fn to_array(d: &Doc) -> Vec<Value> {
        let mut txn = d.transact();
        let a = txn.get_array("array");
        a.iter().collect()
    }

    #[test]
    fn concurrent_insert_remove_with_3_conflicts() {
        let d1 = Doc::with_client_id(1);
        {
            let mut txn = d1.transact();
            let a = txn.get_array("array");
            a.insert_range(&mut txn, 0, ["x", "y", "z"]);
        }
        let d2 = Doc::with_client_id(2);
        let d3 = Doc::with_client_id(3);

        exchange_updates(&[&d1, &d2, &d3]);

        {
            // start state: [x,y,z]
            let mut t1 = d1.transact();
            let mut t2 = d2.transact();
            let mut t3 = d3.transact();
            let a1 = t1.get_array("array");
            let a2 = t2.get_array("array");
            let a3 = t3.get_array("array");

            a1.insert(&mut t1, 1, 0); // [x,0,y,z]
            a2.remove_range(&mut t2, 0, 1); // [y,z]
            a2.remove_range(&mut t2, 1, 1); // [y]
            a3.insert(&mut t3, 1, 2); // [x,2,y,z]
        }

        exchange_updates(&[&d1, &d2, &d3]);
        // after exchange expected: [0,2,y]

        let a1 = to_array(&d1);
        let a2 = to_array(&d2);
        let a3 = to_array(&d3);

        assert_eq!(a1, a2, "Peer 1 and peer 2 states are different");
        assert_eq!(a2, a3, "Peer 2 and peer 3 states are different");
    }

    #[test]
    fn insertions_in_late_sync() {
        let d1 = Doc::with_client_id(1);
        {
            let mut txn = d1.transact();
            let a = txn.get_array("array");
            a.push_back(&mut txn, "x");
            a.push_back(&mut txn, "y");
        }
        let d2 = Doc::with_client_id(2);
        let d3 = Doc::with_client_id(3);

        exchange_updates(&[&d1, &d2, &d3]);

        {
            let mut t1 = d1.transact();
            let mut t2 = d2.transact();
            let mut t3 = d3.transact();
            let a1 = t1.get_array("array");
            let a2 = t2.get_array("array");
            let a3 = t3.get_array("array");

            a1.insert(&mut t1, 1, "user0");
            a2.insert(&mut t2, 1, "user1");
            a3.insert(&mut t3, 1, "user2");
        }

        exchange_updates(&[&d1, &d2, &d3]);

        let a1 = to_array(&d1);
        let a2 = to_array(&d2);
        let a3 = to_array(&d3);

        assert_eq!(a1, a2, "Peer 1 and peer 2 states are different");
        assert_eq!(a2, a3, "Peer 2 and peer 3 states are different");
    }

    #[test]
    fn removals_in_late_sync() {
        let d1 = Doc::with_client_id(1);
        {
            let mut txn = d1.transact();
            let a = txn.get_array("array");
            a.push_back(&mut txn, "x");
            a.push_back(&mut txn, "y");
        }
        let d2 = Doc::with_client_id(2);

        exchange_updates(&[&d1, &d2]);

        {
            let mut t1 = d1.transact();
            let mut t2 = d2.transact();
            let a1 = t1.get_array("array");
            let a2 = t2.get_array("array");

            a2.remove_range(&mut t2, 1, 1);
            a1.remove_range(&mut t1, 0, 2);
        }

        exchange_updates(&[&d1, &d2]);

        let a1 = to_array(&d1);
        let a2 = to_array(&d2);

        assert_eq!(a1, a2, "Peer 1 and peer 2 states are different");
    }

    #[test]
    fn insert_then_merge_delete_on_sync() {
        let d1 = Doc::with_client_id(1);
        {
            let mut txn = d1.transact();
            let a = txn.get_array("array");
            a.push_back(&mut txn, "x");
            a.push_back(&mut txn, "y");
            a.push_back(&mut txn, "z");
        }
        let d2 = Doc::with_client_id(2);

        exchange_updates(&[&d1, &d2]);

        {
            let mut t2 = d2.transact();
            let a2 = t2.get_array("array");

            a2.remove_range(&mut t2, 0, 3);
        }

        exchange_updates(&[&d1, &d2]);

        let a1 = to_array(&d1);
        let a2 = to_array(&d2);

        assert_eq!(a1, a2, "Peer 1 and peer 2 states are different");
    }

    #[test]
    fn iter_array_containing_types() {
        let d = Doc::with_client_id(1);
        let mut txn = d.transact();
        let a = txn.get_array("arr");
        for i in 0..10 {
            let mut m = HashMap::new();
            m.insert("value".to_owned(), i);
            a.push_back(&mut txn, PrelimMap::from(m));
        }

        for (i, value) in a.iter().enumerate() {
            let mut expected = HashMap::new();
            expected.insert("value".to_owned(), Any::Number(i as f64));
            match value {
                Value::YMap(_) => {
                    assert_eq!(value.to_json(), Any::Map(Box::new(expected)))
                }
                _ => panic!("Value of array at index {} was no YMap", i),
            }
        }
    }

    #[test]
    fn insert_and_remove_events() {
        let d = Doc::with_client_id(1);
        let mut array = {
            let mut txn = d.transact();
            txn.get_array("array")
        };
        let happened = Rc::new(Cell::new(false));
        let happened_clone = happened.clone();
        let _sub = array.observe(move |_, _| {
            happened_clone.set(true);
        });

        {
            let mut txn = d.transact();
            array.insert_range(&mut txn, 0, [0, 1, 2]);
            // txn is committed at the end of this scope
        }
        assert!(
            happened.replace(false),
            "insert of [0,1,2] should trigger event"
        );

        {
            let mut txn = d.transact();
            array.remove_range(&mut txn, 0, 1);
            // txn is committed at the end of this scope
        }
        assert!(
            happened.replace(false),
            "removal of [0] should trigger event"
        );

        {
            let mut txn = d.transact();
            array.remove_range(&mut txn, 0, 2);
            // txn is committed at the end of this scope
        }
        assert!(
            happened.replace(false),
            "removal of [1,2] should trigger event"
        );
    }

    #[test]
    fn insert_and_remove_event_changes() {
        let d1 = Doc::with_client_id(1);
        let mut array = {
            let mut txn = d1.transact();
            txn.get_array("array")
        };
        let added = Rc::new(RefCell::new(None));
        let removed = Rc::new(RefCell::new(None));
        let delta = Rc::new(RefCell::new(None));

        let (added_c, removed_c, delta_c) = (added.clone(), removed.clone(), delta.clone());
        let _sub = array.observe(move |txn, e| {
            *added_c.borrow_mut() = Some(e.inserts(txn).clone());
            *removed_c.borrow_mut() = Some(e.removes(txn).clone());
            *delta_c.borrow_mut() = Some(e.delta(txn).to_vec());
        });

        {
            let mut txn = d1.transact();
            array.push_back(&mut txn, 4);
            array.push_back(&mut txn, "dtrn");
            // txn is committed at the end of this scope
        }
        assert_eq!(
            added.borrow_mut().take(),
            Some(HashSet::from([ID::new(1, 0), ID::new(1, 1)]))
        );
        assert_eq!(removed.borrow_mut().take(), Some(HashSet::new()));
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![Change::Added(vec![
                Any::Number(4.0).into(),
                Any::String("dtrn".into()).into()
            ])])
        );

        {
            let mut txn = d1.transact();
            array.remove_range(&mut txn, 0, 1);
        }
        assert_eq!(added.borrow_mut().take(), Some(HashSet::new()));
        assert_eq!(
            removed.borrow_mut().take(),
            Some(HashSet::from([ID::new(1, 0)]))
        );
        assert_eq!(delta.borrow_mut().take(), Some(vec![Change::Removed(1)]));

        {
            let mut txn = d1.transact();
            array.insert(&mut txn, 1, 0.5);
        }
        assert_eq!(
            added.borrow_mut().take(),
            Some(HashSet::from([ID::new(1, 2)]))
        );
        assert_eq!(removed.borrow_mut().take(), Some(HashSet::new()));
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![
                Change::Retain(1),
                Change::Added(vec![Any::Number(0.5).into()])
            ])
        );

        let d2 = Doc::with_client_id(2);
        let mut array2 = {
            let mut txn = d2.transact();
            txn.get_array("array")
        };
        let (added_c, removed_c, delta_c) = (added.clone(), removed.clone(), delta.clone());
        let _sub = array2.observe(move |txn, e| {
            *added_c.borrow_mut() = Some(e.inserts(txn).clone());
            *removed_c.borrow_mut() = Some(e.removes(txn).clone());
            *delta_c.borrow_mut() = Some(e.delta(txn).to_vec());
        });

        {
            let t1 = d1.transact();
            let mut t2 = d2.transact();

            let sv = t2.state_vector();
            let mut encoder = EncoderV1::new();
            t1.encode_diff(&sv, &mut encoder);
            t2.apply_update(Update::decode_v1(encoder.to_vec().as_slice()).unwrap());
        }

        assert_eq!(
            added.borrow_mut().take(),
            Some(HashSet::from([ID::new(1, 1)]))
        );
        assert_eq!(removed.borrow_mut().take(), Some(HashSet::new()));
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![Change::Added(vec![
                Any::String("dtrn".into()).into(),
                Any::Number(0.5).into(),
            ])])
        );
    }

    #[test]
    fn target_on_local_and_remote() {
        let d1 = Doc::with_client_id(1);
        let d2 = Doc::with_client_id(2);
        let mut a1 = {
            let mut txn = d1.transact();
            txn.get_array("array")
        };
        let mut a2 = {
            let mut txn = d2.transact();
            txn.get_array("array")
        };

        let c1 = Rc::new(RefCell::new(None));
        let c1c = c1.clone();
        let _s1 = a1.observe(move |_, e| {
            *c1c.borrow_mut() = Some(e.target().clone());
        });
        let c2 = Rc::new(RefCell::new(None));
        let c2c = c2.clone();
        let _s2 = a2.observe(move |_, e| {
            *c2c.borrow_mut() = Some(e.target().clone());
        });

        {
            let mut t1 = d1.transact();
            a1.insert_range(&mut t1, 0, [1, 2]);
        }
        exchange_updates(&[&d1, &d2]);

        assert_eq!(c1.borrow_mut().take(), Some(a1));
        assert_eq!(c2.borrow_mut().take(), Some(a2));
    }

    use crate::updates::decoder::{Decode, Decoder, DecoderV1};
    use crate::updates::encoder::{Encoder, EncoderV1};
    use lib0::decoding::{Cursor, Read};
    use std::sync::atomic::{AtomicI64, Ordering};

    static UNIQUE_NUMBER: AtomicI64 = AtomicI64::new(0);

    fn get_unique_number() -> i64 {
        UNIQUE_NUMBER.fetch_add(1, Ordering::SeqCst)
    }

    fn array_transactions() -> [Box<dyn Fn(&mut Doc, &mut StdRng)>; 5] {
        fn move_one(doc: &mut Doc, rng: &mut StdRng) {
            let mut txn = doc.transact();
            let yarray = txn.get_array("array");
            if yarray.len() != 0 {
                let pos = rng.between(0, yarray.len() - 1);
                let len = 1;
                let new_pos_adjusted = rng.between(0, yarray.len() - 1);
                let new_pos = new_pos_adjusted + if new_pos_adjusted > pos { len } else { 0 };
                if let Any::Array(expected) = yarray.to_json() {
                    let mut expected = Vec::from(expected);
                    let moved = expected.remove(pos as usize);
                    let insert_pos = if pos < new_pos {
                        new_pos - len
                    } else {
                        new_pos
                    } as usize;
                    expected.insert(insert_pos, moved);

                    yarray.move_to(&mut txn, pos, new_pos);

                    let actual = yarray.to_json();
                    assert_eq!(actual, Any::Array(expected.into_boxed_slice()))
                } else {
                    panic!("should not happen")
                }
            }
        }
        fn insert(doc: &mut Doc, rng: &mut StdRng) {
            let mut txn = doc.transact();
            let yarray = txn.get_array("array");
            let unique_number = get_unique_number();
            let len = rng.between(1, 4);
            let content: Vec<_> = (0..len)
                .into_iter()
                .map(|_| Any::BigInt(unique_number))
                .collect();
            let mut pos = rng.between(0, yarray.len()) as usize;
            if let Any::Array(expected) = yarray.to_json() {
                let mut expected = Vec::from(expected);
                yarray.insert_range(&mut txn, pos as u32, content.clone());

                for any in content {
                    expected.insert(pos, any);
                    pos += 1;
                }
                let actual = yarray.to_json();
                assert_eq!(actual, Any::Array(expected.into_boxed_slice()))
            } else {
                panic!("should not happen")
            }
        }

        fn insert_type_array(doc: &mut Doc, rng: &mut StdRng) {
            let mut txn = doc.transact();
            let yarray = txn.get_array("array");
            let pos = rng.between(0, yarray.len());
            yarray.insert(&mut txn, pos, PrelimArray::from([1, 2, 3, 4]));
            if let Value::YArray(array2) = yarray.get(pos).unwrap() {
                let expected: Box<[Any]> = (1..=4).map(|i| Any::Number(i as f64)).collect();
                assert_eq!(array2.to_json(), Any::Array(expected));
            } else {
                panic!("should not happen")
            }
        }

        fn insert_type_map(doc: &mut Doc, rng: &mut StdRng) {
            let mut txn = doc.transact();
            let yarray = txn.get_array("array");
            let pos = rng.between(0, yarray.len());
            yarray.insert(&mut txn, pos, PrelimMap::<i32>::from(HashMap::default()));
            if let Value::YMap(map) = yarray.get(pos).unwrap() {
                map.insert(&mut txn, "someprop".to_string(), 42);
                map.insert(&mut txn, "someprop".to_string(), 43);
                map.insert(&mut txn, "someprop".to_string(), 44);
            } else {
                panic!("should not happen")
            }
        }

        fn delete(doc: &mut Doc, rng: &mut StdRng) {
            let mut txn = doc.transact();
            let yarray = txn.get_array("array");
            let len = yarray.len();
            if len > 0 {
                let pos = rng.between(0, len - 1);
                let del_len = rng.between(1, 2.min(len - pos));
                if rng.gen_bool(0.5) {
                    if let Value::YArray(array2) = yarray.get(pos).unwrap() {
                        let pos = rng.between(0, array2.len() - 1);
                        let del_len = rng.between(0, 2.min(array2.len() - pos));
                        array2.remove_range(&mut txn, pos, del_len);
                    }
                } else {
                    if let Any::Array(old_content) = yarray.to_json() {
                        let mut old_content = Vec::from(old_content);
                        yarray.remove_range(&mut txn, pos, del_len);
                        old_content.drain(pos as usize..(pos + del_len) as usize);
                        assert_eq!(yarray.to_json(), Any::Array(old_content.into_boxed_slice()));
                    } else {
                        panic!("should not happen")
                    }
                }
            }
        }

        [
            Box::new(insert),
            Box::new(insert_type_array),
            Box::new(insert_type_map),
            Box::new(delete),
            Box::new(move_one),
        ]
    }

    fn fuzzy(iterations: usize) {
        run_scenario(0, &array_transactions(), 5, iterations)
    }

    #[test]
    fn fuzzy_test_6() {
        fuzzy(6)
    }

    #[test]
    fn fuzzy_test_300() {
        fuzzy(300)
    }

    #[test]
    fn get_at_removed_index() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();

        let a1 = t1.get_array("array");
        a1.insert_range(&mut t1, 0, ["A"]);
        a1.remove(&mut t1, 0);

        let actual = t1.get_array("array").get(0);
        assert_eq!(actual, None);
    }

    #[test]
    fn observe_deep_event_order() {
        let doc = Doc::with_client_id(1);
        let mut array = {
            let mut txn = doc.transact();
            txn.get_array("array")
        };

        let paths = Rc::new(RefCell::new(Vec::new()));
        let paths_copy = paths.clone();

        let _sub = array.observe_deep(move |_txn, e| {
            let path: Vec<Path> = e.iter().map(Event::path).collect();
            paths_copy.borrow_mut().push(path);
        });

        array.insert(&mut doc.transact(), 0, PrelimMap::<String>::new());

        {
            let mut txn = doc.transact();
            let map = array.get(0).unwrap().to_ymap().unwrap();
            map.insert(&mut txn, "a", "a");
            array.insert(&mut txn, 0, 0);
        }

        let expected = &[
            vec![Path::default()],
            vec![Path::default(), Path::from([PathSegment::Index(1)])],
        ];
        let actual = RefCell::borrow(&paths);
        assert_eq!(actual.as_slice(), expected);
    }

    #[test]
    fn move_1() {
        let d1 = Doc::with_client_id(1);
        let mut a1 = d1.transact().get_array("array");

        let d2 = Doc::with_client_id(2);
        let mut a2 = d2.transact().get_array("array");

        let e1: Rc<RefCell<Vec<Change>>> = Rc::new(RefCell::new(Vec::default()));
        let inner = e1.clone();
        let _s1 = a1.observe(move |txn, e| {
            let mut x = inner.as_ref().borrow_mut();
            *x = e.delta(txn).to_vec();
        });

        let e2: Rc<RefCell<Vec<Change>>> = Rc::new(RefCell::new(Vec::default()));
        let inner = e2.clone();
        let _s2 = a2.observe(move |txn, e| {
            let mut x = inner.borrow_mut();
            *x = e.delta(txn).to_vec();
        });

        {
            let mut txn = d1.transact();
            a1.insert_range(&mut txn, 0, [1, 2, 3]);
            a1.move_to(&mut txn, 1, 0);
        }
        assert_eq!(a1.to_json(), vec![2, 1, 3].into());

        exchange_updates(&[&d1, &d2]);

        assert_eq!(a2.to_json(), vec![2, 1, 3].into());
        let actual = e2.as_ref().borrow();
        assert_eq!(
            actual.deref(),
            &vec![Change::Added(vec![2.into(), 1.into(), 3.into()])]
        );

        a1.move_to(&mut d1.transact(), 0, 2);

        assert_eq!(a1.to_json(), vec![1, 2, 3].into());
        let actual = e1.as_ref().borrow();
        assert_eq!(
            actual.deref(),
            &vec![
                Change::Removed(1),
                Change::Retain(1),
                Change::Added(vec![2.into()])
            ]
        )
    }

    #[test]
    fn move_2() {
        let d1 = Doc::with_client_id(1);
        let mut a1 = { d1.transact().get_array("array") };

        let d2 = Doc::with_client_id(2);
        let mut a2 = { d2.transact().get_array("array") };

        let e1: Rc<RefCell<Vec<Change>>> = Rc::new(RefCell::new(Vec::default()));
        let inner = e1.clone();
        let _s1 = a1.observe(move |txn, e| {
            let mut x = inner.as_ref().borrow_mut();
            *x = e.delta(txn).to_vec();
        });

        let e2: Rc<RefCell<Vec<Change>>> = Rc::new(RefCell::new(Vec::default()));
        let inner = e2.clone();
        let _s2 = a2.observe(move |txn, e| {
            let mut x = inner.borrow_mut();
            *x = e.delta(txn).to_vec();
        });

        a1.insert_range(&mut d1.transact(), 0, [1, 2]);
        a1.move_to(&mut d1.transact(), 1, 0);
        assert_eq!(a1.to_json(), vec![2, 1].into());
        {
            let actual = e1.as_ref().borrow();
            assert_eq!(
                actual.deref(),
                &vec![
                    Change::Added(vec![2.into()]),
                    Change::Retain(1),
                    Change::Removed(1)
                ]
            );
        }

        exchange_updates(&[&d1, &d2]);

        assert_eq!(a2.to_json(), vec![2, 1].into());
        {
            let actual = e2.as_ref().borrow();
            assert_eq!(
                actual.deref(),
                &vec![Change::Added(vec![2.into(), 1.into()])]
            );
        }

        a1.move_to(&mut d1.transact(), 0, 2);
        assert_eq!(a1.to_json(), vec![1, 2].into());
        {
            let actual = e1.as_ref().borrow();
            assert_eq!(
                actual.deref(),
                &vec![
                    Change::Removed(1),
                    Change::Retain(1),
                    Change::Added(vec![2.into()])
                ]
            );
        }
    }

    #[test]
    fn move_cycles() {
        let d1 = Doc::with_client_id(1);
        let a1 = d1.transact().get_array("array");

        let d2 = Doc::with_client_id(2);
        let a2 = d2.transact().get_array("array");

        a1.insert_range(&mut d1.transact(), 0, [1, 2, 3, 4]);
        exchange_updates(&[&d1, &d2]);

        a1.move_range_to(&mut d1.transact(), 0, true, 1, false, 3);
        assert_eq!(a1.to_json(), vec![3, 1, 2, 4].into());

        a2.move_range_to(&mut d2.transact(), 2, true, 3, false, 1);
        assert_eq!(a2.to_json(), vec![1, 3, 4, 2].into());

        exchange_updates(&[&d1, &d2]);
        exchange_updates(&[&d1, &d2]); // move cycles may not be detected within a single update exchange

        assert_eq!(a1.len(), 4);
        assert_eq!(a1.to_json(), a2.to_json());
    }

    fn move_tests<P: AsRef<std::path::Path>>(path: P) {
        let mut file = File::open(path).unwrap();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut buf).unwrap();
        let mut decoder = DecoderV1::new(Cursor::new(&buf));

        let test_case_count: u32 = decoder.read_var().unwrap();
        for i in 0..test_case_count {
            let doc = Doc::new();
            let array = doc.transact().get_array("array");

            let update_count: u32 = decoder.read_var().unwrap();
            for j in 0..update_count {
                let data = decoder.read_buf().unwrap();
                let update = Update::decode_v1(data).unwrap();
                doc.transact().apply_update(update);
            }
            let expected = decoder.read_any().unwrap();
            let actual = array.to_json();
            assert_eq!(actual, expected, "failed at test case nr {}", i);
        }
    }
}
