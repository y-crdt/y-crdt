use crate::block::{EmbedPrelim, ItemContent, ItemPtr, Prelim, Unused};
use crate::block_iter::BlockIter;
use crate::encoding::read::Error;
use crate::encoding::serde::from_any;
use crate::moving::StickyIndex;
use crate::transaction::TransactionMut;
use crate::types::{
    event_change_set, AsPrelim, Branch, BranchPtr, Change, ChangeSet, DefaultPrelim, In, Out, Path,
    RootRef, SharedRef, ToJson, TypeRef,
};
use crate::{Any, Assoc, DeepObservable, IndexedSequence, Observable, ReadTxn, ID};
use serde::de::DeserializeOwned;
use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::collections::HashSet;
use std::convert::{TryFrom, TryInto};
use std::iter::FromIterator;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

/// A collection used to store data in an indexed sequence structure. This type is internally
/// implemented as a double linked list, which may squash values inserted directly one after another
/// into single list node upon transaction commit.
///
/// Reading a root-level type as an [ArrayRef] means treating its sequence components as a list, where
/// every countable element becomes an individual entity:
///
/// - JSON-like primitives (booleans, numbers, strings, JSON maps, arrays etc.) are counted
///   individually.
/// - Text chunks inserted by [Text] data structure: each character becomes an element of an
///   array.
/// - Embedded and binary values: they count as a single element even though they correspond of
///   multiple bytes.
///
/// Like all Yrs shared data types, [ArrayRef] is resistant to the problem of interleaving (situation
/// when elements inserted one after another may interleave with other peers concurrent inserts
/// after merging all updates together). In case of Yrs conflict resolution is solved by using
/// unique document id to determine correct and consistent ordering.
///
/// # Example
///
/// ```rust
/// use yrs::{Array, Doc, Map, MapPrelim, Transact, Any, any};
/// use yrs::types::ToJson;
///
/// let doc = Doc::new();
/// let array = doc.get_or_insert_array("array");
/// let mut txn = doc.transact_mut();
///
/// // insert single scalar value
/// array.insert(&mut txn, 0, "value");
/// array.remove_range(&mut txn, 0, 1);
///
/// assert_eq!(array.len(&txn), 0);
///
/// // insert multiple values at once
/// array.insert_range(&mut txn, 0, ["a", "b", "c"]);
/// assert_eq!(array.len(&txn), 3);
///
/// // get value
/// let value = array.get(&txn, 1);
/// assert_eq!(value, Some("b".into()));
///
/// // insert nested shared types
/// let map = array.insert(&mut txn, 1, MapPrelim::from([("key1", "value1")]));
/// map.insert(&mut txn, "key2", "value2");
///
/// assert_eq!(array.to_json(&txn), any!([
///   "a",
///   { "key1": "value1", "key2": "value2" },
///   "b",
///   "c"
/// ]));
/// ```
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct ArrayRef(BranchPtr);

impl RootRef for ArrayRef {
    fn type_ref() -> TypeRef {
        TypeRef::Array
    }
}
impl SharedRef for ArrayRef {}
impl Array for ArrayRef {}
impl IndexedSequence for ArrayRef {}

#[cfg(feature = "weak")]
impl crate::Quotable for ArrayRef {}

impl ToJson for ArrayRef {
    fn to_json<T: ReadTxn>(&self, txn: &T) -> Any {
        let mut walker = BlockIter::new(self.0);
        let len = self.0.len();
        let mut buf = vec![Out::default(); len as usize];
        let read = walker.slice(txn, &mut buf);
        if read == len {
            let res = buf.into_iter().map(|v| v.to_json(txn)).collect();
            Any::Array(res)
        } else {
            panic!(
                "Defect: Array::to_json didn't read all elements ({}/{})",
                read, len
            )
        }
    }
}

impl Eq for ArrayRef {}
impl PartialEq for ArrayRef {
    fn eq(&self, other: &Self) -> bool {
        self.0.id() == other.0.id()
    }
}

impl AsRef<Branch> for ArrayRef {
    fn as_ref(&self) -> &Branch {
        self.0.deref()
    }
}

impl DeepObservable for ArrayRef {}
impl Observable for ArrayRef {
    type Event = ArrayEvent;
}

impl TryFrom<ItemPtr> for ArrayRef {
    type Error = ItemPtr;

    fn try_from(value: ItemPtr) -> Result<Self, Self::Error> {
        if let Some(branch) = value.clone().as_branch() {
            Ok(ArrayRef::from(branch))
        } else {
            Err(value)
        }
    }
}

impl TryFrom<Out> for ArrayRef {
    type Error = Out;

    fn try_from(value: Out) -> Result<Self, Self::Error> {
        match value {
            Out::YArray(value) => Ok(value),
            other => Err(other),
        }
    }
}

impl AsPrelim for ArrayRef {
    type Prelim = ArrayPrelim;

    fn as_prelim<T: ReadTxn>(&self, txn: &T) -> Self::Prelim {
        let mut prelim = Vec::with_capacity(self.len(txn) as usize);
        for value in self.iter(txn) {
            prelim.push(value.as_prelim(txn));
        }
        ArrayPrelim(prelim)
    }
}

impl DefaultPrelim for ArrayRef {
    type Prelim = ArrayPrelim;

    #[inline]
    fn default_prelim() -> Self::Prelim {
        ArrayPrelim::default()
    }
}

pub trait Array: AsRef<Branch> + Sized {
    /// Returns a number of elements stored in current array.
    fn len<T: ReadTxn>(&self, _txn: &T) -> u32 {
        self.as_ref().len()
    }

    /// Inserts a `value` at the given `index`. Inserting at index `0` is equivalent to prepending
    /// current array with given `value`, while inserting at array length is equivalent to appending
    /// that value at the end of it.
    ///
    /// Returns a reference to an integrated preliminary input.
    ///
    /// # Panics
    ///
    /// This method will panic if provided `index` is greater than the current length of an [ArrayRef].
    fn insert<V>(&self, txn: &mut TransactionMut, index: u32, value: V) -> V::Return
    where
        V: Prelim,
    {
        let mut walker = BlockIter::new(BranchPtr::from(self.as_ref()));
        if walker.try_forward(txn, index) {
            let ptr = walker
                .insert_contents(txn, value)
                .expect("cannot insert empty value");
            if let Ok(integrated) = ptr.try_into() {
                integrated
            } else {
                panic!("Defect: unexpected integrated type")
            }
        } else {
            panic!("Index {} is outside of the range of an array", index);
        }
    }

    /// Inserts multiple `values` at the given `index`. Inserting at index `0` is equivalent to
    /// prepending current array with given `values`, while inserting at array length is equivalent
    /// to appending that value at the end of it.
    ///
    /// # Panics
    ///
    /// This method will panic if provided `index` is greater than the current length of an [ArrayRef].
    fn insert_range<T, V>(&self, txn: &mut TransactionMut, index: u32, values: T)
    where
        T: IntoIterator<Item = V>,
        V: Into<Any>,
    {
        let prelim = RangePrelim::new(values);
        if !prelim.is_empty() {
            self.insert(txn, index, prelim);
        }
    }

    /// Inserts given `value` at the end of the current array.
    ///
    /// Returns a reference to an integrated preliminary input.
    fn push_back<V>(&self, txn: &mut TransactionMut, value: V) -> V::Return
    where
        V: Prelim,
    {
        let len = self.len(txn);
        self.insert(txn, len, value)
    }

    /// Inserts given `value` at the beginning of the current array.
    ///
    /// Returns a reference to an integrated preliminary input.
    fn push_front<V>(&self, txn: &mut TransactionMut, content: V) -> V::Return
    where
        V: Prelim,
    {
        self.insert(txn, 0, content)
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
    fn get<T: ReadTxn>(&self, txn: &T, index: u32) -> Option<Out> {
        let mut walker = BlockIter::new(BranchPtr::from(self.as_ref()));
        if walker.try_forward(txn, index) {
            walker.read_value(txn)
        } else {
            None
        }
    }

    /// Returns a value stored under a given `index` within current map, deserializing it into
    /// expected type if found. If value was not found, the `Any::Null` will be substituted and
    /// deserialized instead (i.e. into instance of `Option` type, if so desired).
    ///
    /// # Example
    ///
    /// ```rust
    /// use yrs::{Doc, In, Array, MapPrelim, Transact, WriteTxn};
    ///
    /// let doc = Doc::new();
    /// let mut txn = doc.transact_mut();
    /// let array = txn.get_or_insert_array("array");
    ///
    /// // insert a multi-nested shared refs
    /// let alice = array.insert(&mut txn, 0, MapPrelim::from([
    ///   ("name", In::from("Alice")),
    ///   ("age", In::from(30)),
    ///   ("address", MapPrelim::from([
    ///     ("city", In::from("London")),
    ///     ("street", In::from("Baker st.")),
    ///   ]).into())
    /// ]));
    ///
    /// // define Rust types to map from the shared refs
    ///
    /// #[derive(Debug, PartialEq, serde::Deserialize)]
    /// struct Person {
    ///   name: String,
    ///   age: u32,
    ///   address: Option<Address>,
    /// }
    ///
    /// #[derive(Debug, PartialEq, serde::Deserialize)]
    /// struct Address {
    ///   city: String,
    ///   street: String,
    /// }
    ///
    /// // retrieve and deserialize the value across multiple shared refs
    /// let alice: Person = array.get_as(&txn, 0).unwrap();
    /// assert_eq!(alice, Person {
    ///   name: "Alice".to_string(),
    ///   age: 30,
    ///   address: Some(Address {
    ///     city: "London".to_string(),
    ///     street: "Baker st.".to_string(),
    ///   })
    /// });
    ///
    /// // try to retrieve value that doesn't exist
    /// let bob: Option<Person> = array.get_as(&txn, 1).unwrap();
    /// assert_eq!(bob, None);
    /// ```
    fn get_as<T, V>(&self, txn: &T, index: u32) -> Result<V, Error>
    where
        T: ReadTxn,
        V: DeserializeOwned,
    {
        let out = self.get(txn, index).unwrap_or(Out::Any(Any::Null));
        //TODO: we could probably optimize this step by not serializing to intermediate Any value
        let any = out.to_json(txn);
        from_any(&any)
    }

    /// Moves element found at `source` index into `target` index position. Both indexes refer to a
    /// current state of the document.
    ///
    /// # Panics
    ///
    /// This method panics if either `source` or `target` indexes are greater than current array's
    /// length.
    fn move_to(&self, txn: &mut TransactionMut, source: u32, target: u32) {
        if source == target || source + 1 == target {
            // It doesn't make sense to move a range into the same range (it's basically a no-op).
            return;
        }
        let this = BranchPtr::from(self.as_ref());
        let left = StickyIndex::at(txn, this, source, Assoc::After)
            .expect("`source` index parameter is beyond the range of an y-array");
        let mut right = left.clone();
        right.assoc = Assoc::Before;
        let mut walker = BlockIter::new(this);
        if walker.try_forward(txn, target) {
            walker.insert_move(txn, left, right);
        } else {
            panic!(
                "`target` index parameter {} is outside of the range of an array",
                target
            );
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
    /// use yrs::{Doc, Transact, Array, Assoc};
    /// let doc = Doc::new();
    /// let array = doc.get_or_insert_array("array");
    /// array.insert_range(&mut doc.transact_mut(), 0, [1,2,3,4]);
    /// // move elements 2 and 3 after the 4
    /// array.move_range_to(&mut doc.transact_mut(), 1, Assoc::After, 2, Assoc::Before, 4);
    /// let values: Vec<_> = array.iter(&doc.transact()).collect();
    /// assert_eq!(values, vec![1.into(), 4.into(), 2.into(), 3.into()]);
    /// ```
    /// # Panics
    ///
    /// This method panics if either `start`, `end` or `target` indexes are greater than current
    /// array's length.
    fn move_range_to(
        &self,
        txn: &mut TransactionMut,
        start: u32,
        assoc_start: Assoc,
        end: u32,
        assoc_end: Assoc,
        target: u32,
    ) {
        if start <= target && target <= end {
            // It doesn't make sense to move a range into the same range (it's basically a no-op).
            return;
        }
        let this = BranchPtr::from(self.as_ref());
        let left = StickyIndex::at(txn, this, start, assoc_start)
            .expect("`start` index parameter is beyond the range of an y-array");
        let right = StickyIndex::at(txn, this, end + 1, assoc_end)
            .expect("`end` index parameter is beyond the range of an y-array");
        let mut walker = BlockIter::new(this);
        if walker.try_forward(txn, target) {
            walker.insert_move(txn, left, right);
        } else {
            panic!(
                "`target` index parameter {} is outside of the range of an array",
                target
            );
        }
    }

    /// Returns an iterator, that can be used to lazely traverse over all values stored in a current
    /// array.
    fn iter<'a, T: ReadTxn + 'a>(&self, txn: &'a T) -> ArrayIter<&'a T, T> {
        ArrayIter::from_ref(self.as_ref(), txn)
    }
}

pub struct ArrayIter<B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
    inner: BlockIter,
    txn: B,
    _marker: PhantomData<T>,
}

impl<T> ArrayIter<T, T>
where
    T: Borrow<T> + ReadTxn,
{
    pub fn from(array: &ArrayRef, txn: T) -> Self {
        ArrayIter {
            inner: BlockIter::new(array.0),
            txn,
            _marker: PhantomData::default(),
        }
    }
}

impl<'a, T> ArrayIter<&'a T, T>
where
    T: Borrow<T> + ReadTxn,
{
    pub fn from_ref(array: &Branch, txn: &'a T) -> Self {
        ArrayIter {
            inner: BlockIter::new(BranchPtr::from(array)),
            txn,
            _marker: PhantomData::default(),
        }
    }
}

impl<B, T> Iterator for ArrayIter<B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
    type Item = Out;

    fn next(&mut self) -> Option<Self::Item> {
        if self.inner.finished() {
            None
        } else {
            let mut buf = [Out::default(); 1];
            let txn = self.txn.borrow();
            if self.inner.slice(txn, &mut buf) != 0 {
                Some(std::mem::replace(&mut buf[0], Out::default()))
            } else {
                None
            }
        }
    }
}

impl From<BranchPtr> for ArrayRef {
    fn from(inner: BranchPtr) -> Self {
        ArrayRef(inner)
    }
}

/// A preliminary array. It can be used to initialize an [ArrayRef], when it's about to be nested
/// into another Yrs data collection, such as [Map] or another [ArrayRef].
#[repr(transparent)]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArrayPrelim(Vec<In>);

impl Deref for ArrayPrelim {
    type Target = Vec<In>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ArrayPrelim {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<ArrayPrelim> for In {
    #[inline]
    fn from(value: ArrayPrelim) -> Self {
        In::Array(value)
    }
}

impl<T> FromIterator<T> for ArrayPrelim
where
    T: Into<In>,
{
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        ArrayPrelim(iter.into_iter().map(|v| v.into()).collect())
    }
}

impl<I, T> From<I> for ArrayPrelim
where
    I: IntoIterator<Item = T>,
    T: Into<In>,
{
    fn from(iter: I) -> Self {
        ArrayPrelim(iter.into_iter().map(|v| v.into()).collect())
    }
}

impl Prelim for ArrayPrelim {
    type Return = ArrayRef;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let inner = Branch::new(TypeRef::Array);
        (ItemContent::Type(inner), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        let array = ArrayRef::from(inner_ref);
        for value in self.0 {
            array.push_back(txn, value);
        }
    }
}

impl Into<EmbedPrelim<ArrayPrelim>> for ArrayPrelim {
    #[inline]
    fn into(self) -> EmbedPrelim<ArrayPrelim> {
        EmbedPrelim::Shared(self)
    }
}

/// Prelim range defines a way to insert multiple elements effectively at once one after another
/// in an efficient way, provided that these elements correspond to a primitive JSON-like types.
#[repr(transparent)]
struct RangePrelim(Vec<Any>);

impl RangePrelim {
    fn new<I, T>(iter: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Any>,
    {
        RangePrelim(iter.into_iter().map(|v| v.into()).collect())
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Prelim for RangePrelim {
    type Return = Unused;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        (ItemContent::Any(self.0), None)
    }

    fn integrate(self, _txn: &mut TransactionMut, _inner_ref: BranchPtr) {}
}

/// Event generated by [ArrayRef::observe] method. Emitted during transaction commit phase.
#[derive(Debug)]
pub struct ArrayEvent {
    pub(crate) current_target: BranchPtr,
    target: ArrayRef,
    change_set: UnsafeCell<Option<Box<ChangeSet<Change>>>>,
}

impl ArrayEvent {
    pub(crate) fn new(branch_ref: BranchPtr) -> Self {
        let current_target = branch_ref.clone();
        ArrayEvent {
            target: ArrayRef::from(branch_ref),
            current_target,
            change_set: UnsafeCell::new(None),
        }
    }

    /// Returns an [ArrayRef] instance which emitted this event.
    pub fn target(&self) -> &ArrayRef {
        &self.target
    }

    /// Returns a path from root type down to [ArrayRef] instance which emitted this event.
    pub fn path(&self) -> Path {
        Branch::path(self.current_target, self.target.0)
    }

    /// Returns summary of changes made over corresponding [ArrayRef] collection within
    /// a bounds of current transaction.
    pub fn delta(&self, txn: &TransactionMut) -> &[Change] {
        self.changes(txn).delta.as_slice()
    }

    /// Returns a collection of block identifiers that have been added within a bounds of
    /// current transaction.
    pub fn inserts(&self, txn: &TransactionMut) -> &HashSet<ID> {
        &self.changes(txn).added
    }

    /// Returns a collection of block identifiers that have been removed within a bounds of
    /// current transaction.
    pub fn removes(&self, txn: &TransactionMut) -> &HashSet<ID> {
        &self.changes(txn).deleted
    }

    fn changes(&self, txn: &TransactionMut) -> &ChangeSet<Change> {
        let change_set = unsafe { self.change_set.get().as_mut().unwrap() };
        change_set.get_or_insert_with(|| Box::new(event_change_set(txn, self.target.0.start)))
    }
}

#[cfg(test)]
mod test {
    use crate::test_utils::{exchange_updates, run_scenario, RngExt};
    use crate::types::map::MapPrelim;
    use crate::types::{Change, DeepObservable, Event, Out, Path, PathSegment, ToJson};
    use crate::{
        any, Any, Array, ArrayPrelim, Assoc, Doc, Map, MapRef, Observable, SharedRef, StateVector,
        Transact, Update, WriteTxn, ID,
    };
    use std::collections::{HashMap, HashSet};
    use std::iter::FromIterator;
    use std::sync::{Arc, Mutex};

    #[test]
    fn push_back() {
        let doc = Doc::with_client_id(1);
        let a = doc.get_or_insert_array("array");
        let mut txn = doc.transact_mut();

        a.push_back(&mut txn, "a");
        a.push_back(&mut txn, "b");
        a.push_back(&mut txn, "c");

        let actual: Vec<_> = a.iter(&txn).collect();
        assert_eq!(actual, vec!["a".into(), "b".into(), "c".into()]);
    }

    #[test]
    fn push_front() {
        let doc = Doc::with_client_id(1);
        let a = doc.get_or_insert_array("array");
        let mut txn = doc.transact_mut();

        a.push_front(&mut txn, "c");
        a.push_front(&mut txn, "b");
        a.push_front(&mut txn, "a");

        let actual: Vec<_> = a.iter(&txn).collect();
        assert_eq!(actual, vec!["a".into(), "b".into(), "c".into()]);
    }

    #[test]
    fn insert() {
        let doc = Doc::with_client_id(1);
        let a = doc.get_or_insert_array("array");
        let mut txn = doc.transact_mut();

        a.insert(&mut txn, 0, "a");
        a.insert(&mut txn, 1, "c");
        a.insert(&mut txn, 1, "b");

        let actual: Vec<_> = a.iter(&txn).collect();
        assert_eq!(actual, vec!["a".into(), "b".into(), "c".into()]);
    }

    #[test]
    fn basic() {
        let d1 = Doc::with_client_id(1);
        let d2 = Doc::with_client_id(2);

        let a1 = d1.get_or_insert_array("array");

        a1.insert(&mut d1.transact_mut(), 0, "Hi");
        let update = d1
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        let a2 = d2.get_or_insert_array("array");
        let mut t2 = d2.transact_mut();
        t2.apply_update(Update::decode_v1(update.as_slice()).unwrap())
            .unwrap();
        let actual: Vec<_> = a2.iter(&t2).collect();

        assert_eq!(actual, vec!["Hi".into()]);
    }

    #[test]
    fn len() {
        let d = Doc::with_client_id(1);
        let a = d.get_or_insert_array("array");

        {
            let mut txn = d.transact_mut();

            a.push_back(&mut txn, 0); // len: 1
            a.push_back(&mut txn, 1); // len: 2
            a.push_back(&mut txn, 2); // len: 3
            a.push_back(&mut txn, 3); // len: 4

            a.remove_range(&mut txn, 0, 1); // len: 3
            a.insert(&mut txn, 0, 0); // len: 4

            assert_eq!(a.len(&txn), 4);
        }
        {
            let mut txn = d.transact_mut();
            a.remove_range(&mut txn, 1, 1); // len: 3
            assert_eq!(a.len(&txn), 3);

            a.insert(&mut txn, 1, 1); // len: 4
            assert_eq!(a.len(&txn), 4);

            a.remove_range(&mut txn, 2, 1); // len: 3
            assert_eq!(a.len(&txn), 3);

            a.insert(&mut txn, 2, 2); // len: 4
            assert_eq!(a.len(&txn), 4);
        }

        let mut txn = d.transact_mut();
        assert_eq!(a.len(&txn), 4);

        a.remove_range(&mut txn, 1, 1);
        assert_eq!(a.len(&txn), 3);

        a.insert(&mut txn, 1, 1);
        assert_eq!(a.len(&txn), 4);
    }

    #[test]
    fn remove_insert() {
        let d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");

        let mut t1 = d1.transact_mut();
        a1.insert(&mut t1, 0, "A");
        a1.remove_range(&mut t1, 1, 0);
    }

    #[test]
    fn insert_3_elements_try_re_get() {
        let d1 = Doc::with_client_id(1);
        let d2 = Doc::with_client_id(2);
        let a1 = d1.get_or_insert_array("array");
        {
            let mut t1 = d1.transact_mut();

            a1.push_back(&mut t1, 1);
            a1.push_back(&mut t1, true);
            a1.push_back(&mut t1, false);
            let actual: Vec<_> = a1.iter(&t1).collect();
            assert_eq!(
                actual,
                vec![Out::from(1.0), Out::from(true), Out::from(false)]
            );
        }

        exchange_updates(&[&d1, &d2]);

        let a2 = d2.get_or_insert_array("array");
        let t2 = d2.transact();
        let actual: Vec<_> = a2.iter(&t2).collect();
        assert_eq!(
            actual,
            vec![Out::from(1.0), Out::from(true), Out::from(false)]
        );
    }

    #[test]
    fn concurrent_insert_with_3_conflicts() {
        let d1 = Doc::with_client_id(1);
        let a = d1.get_or_insert_array("array");
        {
            let mut txn = d1.transact_mut();
            a.insert(&mut txn, 0, 0);
        }

        let d2 = Doc::with_client_id(2);
        {
            let mut txn = d1.transact_mut();
            a.insert(&mut txn, 0, 1);
        }

        let d3 = Doc::with_client_id(3);
        {
            let mut txn = d1.transact_mut();
            a.insert(&mut txn, 0, 2);
        }

        exchange_updates(&[&d1, &d2, &d3]);

        let a1 = to_array(&d1);
        let a2 = to_array(&d2);
        let a3 = to_array(&d3);

        assert_eq!(a1, a2, "Peer 1 and peer 2 states are different");
        assert_eq!(a2, a3, "Peer 2 and peer 3 states are different");
    }

    fn to_array(d: &Doc) -> Vec<Out> {
        let a = d.get_or_insert_array("array");
        a.iter(&d.transact()).collect()
    }

    #[test]
    fn concurrent_insert_remove_with_3_conflicts() {
        let d1 = Doc::with_client_id(1);
        {
            let a = d1.get_or_insert_array("array");
            let mut txn = d1.transact_mut();
            a.insert_range(&mut txn, 0, ["x", "y", "z"]);
        }
        let d2 = Doc::with_client_id(2);
        let d3 = Doc::with_client_id(3);

        exchange_updates(&[&d1, &d2, &d3]);

        {
            // start state: [x,y,z]
            let a1 = d1.get_or_insert_array("array");
            let a2 = d2.get_or_insert_array("array");
            let a3 = d3.get_or_insert_array("array");
            let mut t1 = d1.transact_mut();
            let mut t2 = d2.transact_mut();
            let mut t3 = d3.transact_mut();

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
            let a = d1.get_or_insert_array("array");
            let mut txn = d1.transact_mut();
            a.push_back(&mut txn, "x");
            a.push_back(&mut txn, "y");
        }
        let d2 = Doc::with_client_id(2);
        let d3 = Doc::with_client_id(3);

        exchange_updates(&[&d1, &d2, &d3]);

        {
            let a1 = d1.get_or_insert_array("array");
            let a2 = d2.get_or_insert_array("array");
            let a3 = d3.get_or_insert_array("array");
            let mut t1 = d1.transact_mut();
            let mut t2 = d2.transact_mut();
            let mut t3 = d3.transact_mut();

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
            let a = d1.get_or_insert_array("array");
            let mut txn = d1.transact_mut();
            a.push_back(&mut txn, "x");
            a.push_back(&mut txn, "y");
        }
        let d2 = Doc::with_client_id(2);

        exchange_updates(&[&d1, &d2]);

        {
            let a1 = d1.get_or_insert_array("array");
            let a2 = d2.get_or_insert_array("array");
            let mut t1 = d1.transact_mut();
            let mut t2 = d2.transact_mut();

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
            let a = d1.get_or_insert_array("array");
            let mut txn = d1.transact_mut();
            a.push_back(&mut txn, "x");
            a.push_back(&mut txn, "y");
            a.push_back(&mut txn, "z");
        }
        let d2 = Doc::with_client_id(2);

        exchange_updates(&[&d1, &d2]);

        {
            let a2 = d2.get_or_insert_array("array");
            let mut t2 = d2.transact_mut();

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
        let a = d.get_or_insert_array("arr");
        let mut txn = d.transact_mut();
        for i in 0..10 {
            let mut m = HashMap::new();
            m.insert("value".to_owned(), i);
            a.push_back(&mut txn, MapPrelim::from_iter(m));
        }

        for (i, value) in a.iter(&txn).enumerate() {
            match value {
                Out::YMap(_) => {
                    assert_eq!(value.to_json(&txn), any!({"value": (i as f64) }))
                }
                _ => panic!("Value of array at index {} was no YMap", i),
            }
        }
    }

    #[test]
    fn insert_and_remove_events() {
        let d = Doc::with_client_id(1);
        let array = d.get_or_insert_array("array");
        let happened = Arc::new(AtomicBool::new(false));
        let happened_clone = happened.clone();
        let _sub = array.observe(move |_, _| {
            happened_clone.store(true, Ordering::Relaxed);
        });

        {
            let mut txn = d.transact_mut();
            array.insert_range(&mut txn, 0, [0, 1, 2]);
            // txn is committed at the end of this scope
        }
        assert!(
            happened.swap(false, Ordering::Relaxed),
            "insert of [0,1,2] should trigger event"
        );

        {
            let mut txn = d.transact_mut();
            array.remove_range(&mut txn, 0, 1);
            // txn is committed at the end of this scope
        }
        assert!(
            happened.swap(false, Ordering::Relaxed),
            "removal of [0] should trigger event"
        );

        {
            let mut txn = d.transact_mut();
            array.remove_range(&mut txn, 0, 2);
            // txn is committed at the end of this scope
        }
        assert!(
            happened.swap(false, Ordering::Relaxed),
            "removal of [1,2] should trigger event"
        );
    }

    #[test]
    fn insert_and_remove_event_changes() {
        let d1 = Doc::with_client_id(1);
        let array = d1.get_or_insert_array("array");
        let added = Arc::new(ArcSwapOption::default());
        let removed = Arc::new(ArcSwapOption::default());
        let delta = Arc::new(ArcSwapOption::default());

        let (added_c, removed_c, delta_c) = (added.clone(), removed.clone(), delta.clone());
        let _sub = array.observe(move |txn, e| {
            added_c.store(Some(Arc::new(e.inserts(txn).clone())));
            removed_c.store(Some(Arc::new(e.removes(txn).clone())));
            delta_c.store(Some(Arc::new(e.delta(txn).to_vec())));
        });

        {
            let mut txn = d1.transact_mut();
            array.push_back(&mut txn, 4);
            array.push_back(&mut txn, "dtrn");
            // txn is committed at the end of this scope
        }
        assert_eq!(
            added.swap(None),
            Some(HashSet::from([ID::new(1, 0), ID::new(1, 1)]).into())
        );
        assert_eq!(removed.swap(None), Some(HashSet::new().into()));
        assert_eq!(
            delta.swap(None),
            Some(
                vec![Change::Added(vec![
                    Any::Number(4.0).into(),
                    Any::String("dtrn".into()).into()
                ])]
                .into()
            )
        );

        {
            let mut txn = d1.transact_mut();
            array.remove_range(&mut txn, 0, 1);
        }
        assert_eq!(added.swap(None), Some(HashSet::new().into()));
        assert_eq!(
            removed.swap(None),
            Some(HashSet::from([ID::new(1, 0)]).into())
        );
        assert_eq!(delta.swap(None), Some(vec![Change::Removed(1)].into()));

        {
            let mut txn = d1.transact_mut();
            array.insert(&mut txn, 1, 0.5);
        }
        assert_eq!(
            added.swap(None),
            Some(HashSet::from([ID::new(1, 2)]).into())
        );
        assert_eq!(removed.swap(None), Some(HashSet::new().into()));
        assert_eq!(
            delta.swap(None),
            Some(
                vec![
                    Change::Retain(1),
                    Change::Added(vec![Any::Number(0.5).into()])
                ]
                .into()
            )
        );

        let d2 = Doc::with_client_id(2);
        let array2 = d2.get_or_insert_array("array");
        let (added_c, removed_c, delta_c) = (added.clone(), removed.clone(), delta.clone());
        let _sub = array2.observe(move |txn, e| {
            added_c.store(Some(e.inserts(txn).clone().into()));
            removed_c.store(Some(e.removes(txn).clone().into()));
            delta_c.store(Some(e.delta(txn).to_vec().into()));
        });

        {
            let t1 = d1.transact_mut();
            let mut t2 = d2.transact_mut();

            let sv = t2.state_vector();
            let mut encoder = EncoderV1::new();
            t1.encode_diff(&sv, &mut encoder);
            t2.apply_update(Update::decode_v1(encoder.to_vec().as_slice()).unwrap())
                .unwrap();
        }

        assert_eq!(
            added.swap(None),
            Some(HashSet::from([ID::new(1, 1)]).into())
        );
        assert_eq!(removed.swap(None), Some(HashSet::new().into()));
        assert_eq!(
            delta.swap(None),
            Some(
                vec![Change::Added(vec![
                    Any::String("dtrn".into()).into(),
                    Any::Number(0.5).into(),
                ])]
                .into()
            )
        );
    }

    #[test]
    fn target_on_local_and_remote() {
        let d1 = Doc::with_client_id(1);
        let d2 = Doc::with_client_id(2);
        let a1 = d1.get_or_insert_array("array");
        let a2 = d2.get_or_insert_array("array");

        let c1 = Arc::new(ArcSwapOption::default());
        let c1c = c1.clone();
        let _s1 = a1.observe(move |_, e| {
            c1c.store(Some(e.target().hook().into()));
        });
        let c2 = Arc::new(ArcSwapOption::default());
        let c2c = c2.clone();
        let _s2 = a2.observe(move |_, e| {
            c2c.store(Some(e.target().hook().into()));
        });

        {
            let mut t1 = d1.transact_mut();
            a1.insert_range(&mut t1, 0, [1, 2]);
        }
        exchange_updates(&[&d1, &d2]);

        assert_eq!(c1.swap(None), Some(Arc::new(a1.hook())));
        assert_eq!(c2.swap(None), Some(Arc::new(a2.hook())));
    }

    use crate::transaction::ReadTxn;
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::{Encoder, EncoderV1};
    use arc_swap::ArcSwapOption;
    use fastrand::Rng;
    use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
    use std::time::Duration;

    static UNIQUE_NUMBER: AtomicI64 = AtomicI64::new(0);

    fn get_unique_number() -> i64 {
        UNIQUE_NUMBER.fetch_add(1, Ordering::SeqCst)
    }

    fn array_transactions() -> [Box<dyn Fn(&mut Doc, &mut Rng)>; 5] {
        fn move_one(doc: &mut Doc, rng: &mut Rng) {
            let yarray = doc.get_or_insert_array("array");
            let mut txn = doc.transact_mut();
            if yarray.len(&txn) != 0 {
                let pos = rng.between(0, yarray.len(&txn) - 1);
                let len = 1;
                let new_pos_adjusted = rng.between(0, yarray.len(&txn) - 1);
                let new_pos = new_pos_adjusted + if new_pos_adjusted > pos { len } else { 0 };
                if let Any::Array(expected) = yarray.to_json(&txn) {
                    let mut expected = Vec::from(expected.as_ref());
                    let moved = expected.remove(pos as usize);
                    let insert_pos = if pos < new_pos {
                        new_pos - len
                    } else {
                        new_pos
                    } as usize;
                    expected.insert(insert_pos, moved);

                    yarray.move_to(&mut txn, pos, new_pos);

                    let actual = yarray.to_json(&txn);
                    assert_eq!(actual, Any::from(expected))
                } else {
                    panic!("should not happen")
                }
            }
        }
        fn insert(doc: &mut Doc, rng: &mut Rng) {
            let yarray = doc.get_or_insert_array("array");
            let mut txn = doc.transact_mut();
            let unique_number = get_unique_number();
            let len = rng.between(1, 4);
            let content: Vec<_> = (0..len)
                .into_iter()
                .map(|_| Any::BigInt(unique_number))
                .collect();
            let mut pos = rng.between(0, yarray.len(&txn)) as usize;
            if let Any::Array(expected) = yarray.to_json(&txn) {
                let mut expected = Vec::from(expected.as_ref());
                yarray.insert_range(&mut txn, pos as u32, content.clone());

                for any in content {
                    expected.insert(pos, any);
                    pos += 1;
                }
                let actual = yarray.to_json(&txn);
                assert_eq!(actual, Any::from(expected))
            } else {
                panic!("should not happen")
            }
        }

        fn insert_type_array(doc: &mut Doc, rng: &mut Rng) {
            let yarray = doc.get_or_insert_array("array");
            let mut txn = doc.transact_mut();
            let pos = rng.between(0, yarray.len(&txn));
            let array2 = yarray.insert(&mut txn, pos, ArrayPrelim::from([1, 2, 3, 4]));
            let expected: Arc<[Any]> = (1..=4).map(|i| Any::Number(i as f64)).collect();
            assert_eq!(array2.to_json(&txn), Any::Array(expected));
        }

        fn insert_type_map(doc: &mut Doc, rng: &mut Rng) {
            let yarray = doc.get_or_insert_array("array");
            let mut txn = doc.transact_mut();
            let pos = rng.between(0, yarray.len(&txn));
            let map = yarray.insert(&mut txn, pos, MapPrelim::default());
            map.insert(&mut txn, "someprop".to_string(), 42);
            map.insert(&mut txn, "someprop".to_string(), 43);
            map.insert(&mut txn, "someprop".to_string(), 44);
        }

        fn delete(doc: &mut Doc, rng: &mut Rng) {
            let yarray = doc.get_or_insert_array("array");
            let mut txn = doc.transact_mut();
            let len = yarray.len(&txn);
            if len > 0 {
                let pos = rng.between(0, len - 1);
                let del_len = rng.between(1, 2.min(len - pos));
                if rng.bool() {
                    if let Out::YArray(array2) = yarray.get(&txn, pos).unwrap() {
                        let pos = rng.between(0, array2.len(&txn) - 1);
                        let del_len = rng.between(0, 2.min(array2.len(&txn) - pos));
                        array2.remove_range(&mut txn, pos, del_len);
                    }
                } else {
                    if let Any::Array(old_content) = yarray.to_json(&txn) {
                        let mut old_content = Vec::from(old_content.as_ref());
                        yarray.remove_range(&mut txn, pos, del_len);
                        old_content.drain(pos as usize..(pos + del_len) as usize);
                        assert_eq!(yarray.to_json(&txn), Any::from(old_content));
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
        let a1 = d1.get_or_insert_array("array");
        let mut t1 = d1.transact_mut();

        a1.insert_range(&mut t1, 0, ["A"]);
        a1.remove(&mut t1, 0);

        let actual = a1.get(&t1, 0);
        assert_eq!(actual, None);
    }

    #[test]
    fn observe_deep_event_order() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");

        let paths = Arc::new(Mutex::new(vec![]));
        let paths_copy = paths.clone();

        let _sub = array.observe_deep(move |_txn, e| {
            let path: Vec<Path> = e.iter().map(Event::path).collect();
            paths_copy.lock().unwrap().push(path);
        });

        array.insert(&mut doc.transact_mut(), 0, MapPrelim::default());

        {
            let mut txn = doc.transact_mut();
            let map = array.get(&txn, 0).unwrap().cast::<MapRef>().unwrap();
            map.insert(&mut txn, "a", "a");
            array.insert(&mut txn, 0, 0);
        }

        let expected = &[
            vec![Path::default()],
            vec![Path::default(), Path::from([PathSegment::Index(1)])],
        ];
        let actual = paths.lock().unwrap();
        assert_eq!(actual.as_slice(), expected);
    }

    #[test]
    fn move_1() {
        let d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");

        let d2 = Doc::with_client_id(2);
        let a2 = d2.get_or_insert_array("array");

        let e1 = Arc::new(ArcSwapOption::default());
        let inner = e1.clone();
        let _s1 = a1.observe(move |txn, e| {
            inner.store(Some(Arc::new(e.delta(txn).to_vec())));
        });

        let e2 = Arc::new(ArcSwapOption::default());
        let inner = e2.clone();
        let _s2 = a2.observe(move |txn, e| {
            inner.store(Some(Arc::new(e.delta(txn).to_vec())));
        });

        {
            let mut txn = d1.transact_mut();
            a1.insert_range(&mut txn, 0, [1, 2, 3]);
            a1.move_to(&mut txn, 1, 0);
        }
        assert_eq!(a1.to_json(&d1.transact()), vec![2, 1, 3].into());

        exchange_updates(&[&d1, &d2]);

        assert_eq!(a2.to_json(&d2.transact()), vec![2, 1, 3].into());
        let actual = e2.load_full();
        assert_eq!(
            actual,
            Some(Arc::new(vec![Change::Added(vec![
                2.into(),
                1.into(),
                3.into()
            ])]))
        );

        a1.move_to(&mut d1.transact_mut(), 0, 2);

        assert_eq!(a1.to_json(&d1.transact()), vec![1, 2, 3].into());
        let actual = e1.load_full();
        assert_eq!(
            actual,
            Some(Arc::new(vec![
                Change::Removed(1),
                Change::Retain(1),
                Change::Added(vec![2.into()])
            ]))
        )
    }

    #[test]
    fn move_2() {
        let d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");

        let d2 = Doc::with_client_id(2);
        let a2 = d2.get_or_insert_array("array");

        let e1 = Arc::new(ArcSwapOption::default());
        let inner = e1.clone();
        let _s1 = a1.observe(move |txn, e| {
            inner.store(Some(Arc::new(e.delta(txn).to_vec())));
        });

        let e2 = Arc::new(ArcSwapOption::default());
        let inner = e2.clone();
        let _s2 = a2.observe(move |txn, e| {
            inner.store(Some(Arc::new(e.delta(txn).to_vec())));
        });

        a1.insert_range(&mut d1.transact_mut(), 0, [1, 2]);
        a1.move_to(&mut d1.transact_mut(), 1, 0);
        assert_eq!(a1.to_json(&d1.transact()), vec![2, 1].into());
        {
            let actual = e1.load_full();
            assert_eq!(
                actual,
                Some(Arc::new(vec![
                    Change::Added(vec![2.into()]),
                    Change::Retain(1),
                    Change::Removed(1)
                ]))
            );
        }

        exchange_updates(&[&d1, &d2]);

        assert_eq!(a2.to_json(&d2.transact()), vec![2, 1].into());
        {
            let actual = e2.load_full();
            assert_eq!(
                actual,
                Some(Arc::new(vec![Change::Added(vec![2.into(), 1.into()])]))
            );
        }

        a1.move_to(&mut d1.transact_mut(), 0, 2);
        assert_eq!(a1.to_json(&d1.transact()), vec![1, 2].into());
        {
            let actual = e1.load_full();
            assert_eq!(
                actual,
                Some(Arc::new(vec![
                    Change::Removed(1),
                    Change::Retain(1),
                    Change::Added(vec![2.into()])
                ]))
            );
        }
    }

    #[test]
    fn move_cycles() {
        let d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");

        let d2 = Doc::with_client_id(2);
        let a2 = d2.get_or_insert_array("array");

        a1.insert_range(&mut d1.transact_mut(), 0, [1, 2, 3, 4]);
        exchange_updates(&[&d1, &d2]);

        a1.move_range_to(&mut d1.transact_mut(), 0, Assoc::After, 1, Assoc::Before, 3);
        assert_eq!(a1.to_json(&d1.transact()), vec![3, 1, 2, 4].into());

        a2.move_range_to(&mut d2.transact_mut(), 2, Assoc::After, 3, Assoc::Before, 1);
        assert_eq!(a2.to_json(&d2.transact()), vec![1, 3, 4, 2].into());

        exchange_updates(&[&d1, &d2]);
        exchange_updates(&[&d1, &d2]); // move cycles may not be detected within a single update exchange

        assert_eq!(a1.len(&d1.transact()), 4);
        assert_eq!(a1.to_json(&d1.transact()), a2.to_json(&d2.transact()));
    }

    #[test]
    #[ignore] //TODO: investigate (see: https://github.com/y-crdt/y-crdt/pull/266)
    fn move_range_to() {
        let doc = Doc::with_client_id(1);
        let arr = doc.get_or_insert_array("array");
        // Move 1-2 to 4
        {
            let mut txn = doc.transact_mut();
            let arr_len = arr.len(&txn);
            arr.remove_range(&mut txn, 0, arr_len);
            let arr_len = arr.len(&txn);
            assert_eq!(arr_len, 0);
            arr.insert_range(&mut txn, arr_len, [0, 1, 2, 3]);
        }
        arr.move_range_to(
            &mut doc.transact_mut(),
            1,
            Assoc::After,
            2,
            Assoc::Before,
            4,
        );
        assert_eq!(arr.to_json(&doc.transact()), vec![0, 3, 1, 2].into());

        // Move 0-0 to 10
        {
            let mut txn = doc.transact_mut();
            let arr_len = arr.len(&txn);
            arr.remove_range(&mut txn, 0, arr_len);
            let arr_len = arr.len(&txn);
            assert_eq!(arr_len, 0);
            arr.insert_range(&mut txn, arr_len, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        }
        arr.move_range_to(
            &mut doc.transact_mut(),
            0,
            Assoc::After,
            0,
            Assoc::Before,
            10,
        );
        assert_eq!(
            arr.to_json(&doc.transact()),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 0].into()
        );

        // Move 0-1 to 10
        {
            let mut txn = doc.transact_mut();
            let arr_len = arr.len(&txn);
            arr.remove_range(&mut txn, 0, arr_len);
            let arr_len = arr.len(&txn);
            assert_eq!(arr_len, 0);
            arr.insert_range(&mut txn, arr_len, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        }
        arr.move_range_to(
            &mut doc.transact_mut(),
            0,
            Assoc::After,
            1,
            Assoc::Before,
            10,
        );
        assert_eq!(
            arr.to_json(&doc.transact()),
            vec![2, 3, 4, 5, 6, 7, 8, 9, 0, 1].into()
        );

        // Move 3-5 to 7
        {
            let mut txn = doc.transact_mut();
            let arr_len = arr.len(&txn);
            arr.remove_range(&mut txn, 0, arr_len);
            let arr_len = arr.len(&txn);
            assert_eq!(arr_len, 0);
            arr.insert_range(&mut txn, arr_len, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        }
        arr.move_range_to(
            &mut doc.transact_mut(),
            3,
            Assoc::After,
            5,
            Assoc::Before,
            7,
        );
        assert_eq!(
            arr.to_json(&doc.transact()),
            vec![0, 1, 2, 6, 3, 4, 5, 7, 8, 9].into()
        );

        // Move 1-0 to 10
        {
            let mut txn = doc.transact_mut();
            let arr_len = arr.len(&txn);
            arr.remove_range(&mut txn, 0, arr_len);
            let arr_len = arr.len(&txn);
            assert_eq!(arr_len, 0);
            arr.insert_range(&mut txn, arr_len, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        }
        arr.move_range_to(
            &mut doc.transact_mut(),
            1,
            Assoc::After,
            0,
            Assoc::Before,
            10,
        );
        assert_eq!(
            arr.to_json(&doc.transact()),
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9].into()
        );

        // Move 3-5 to 5
        {
            let mut txn = doc.transact_mut();
            let arr_len = arr.len(&txn);
            arr.remove_range(&mut txn, 0, arr_len);
            let arr_len = arr.len(&txn);
            assert_eq!(arr_len, 0);
            arr.insert_range(&mut txn, arr_len, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        }
        arr.move_range_to(
            &mut doc.transact_mut(),
            3,
            Assoc::After,
            5,
            Assoc::Before,
            5,
        );
        assert_eq!(
            arr.to_json(&doc.transact()),
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9].into()
        );

        // Move 9-9 to 0
        {
            let mut txn = doc.transact_mut();
            let arr_len = arr.len(&txn);
            arr.remove_range(&mut txn, 0, arr_len);
            let arr_len = arr.len(&txn);
            assert_eq!(arr_len, 0);
            arr.insert_range(&mut txn, arr_len, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        }
        arr.move_range_to(
            &mut doc.transact_mut(),
            9,
            Assoc::After,
            9,
            Assoc::Before,
            0,
        );
        assert_eq!(
            arr.to_json(&doc.transact()),
            vec![9, 0, 1, 2, 3, 4, 5, 6, 7, 8].into()
        );

        // Move 8-9 to 0
        {
            let mut txn = doc.transact_mut();
            let arr_len = arr.len(&txn);
            arr.remove_range(&mut txn, 0, arr_len);
            let arr_len = arr.len(&txn);
            assert_eq!(arr_len, 0);
            arr.insert_range(&mut txn, arr_len, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        }
        arr.move_range_to(
            &mut doc.transact_mut(),
            8,
            Assoc::After,
            9,
            Assoc::Before,
            0,
        );
        assert_eq!(
            arr.to_json(&doc.transact()),
            vec![8, 9, 0, 1, 2, 3, 4, 5, 6, 7].into()
        );

        // Move 4-6 to 3
        {
            let mut txn = doc.transact_mut();
            let arr_len = arr.len(&txn);
            arr.remove_range(&mut txn, 0, arr_len);
            let arr_len = arr.len(&txn);
            assert_eq!(arr_len, 0);
            arr.insert_range(&mut txn, arr_len, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        }
        arr.move_range_to(
            &mut doc.transact_mut(),
            4,
            Assoc::After,
            6,
            Assoc::Before,
            3,
        );
        assert_eq!(
            arr.to_json(&doc.transact()),
            vec![0, 1, 2, 4, 5, 6, 3, 7, 8, 9].into()
        );

        // Move 3-5 to 3
        {
            let mut txn = doc.transact_mut();
            let arr_len = arr.len(&txn);
            arr.remove_range(&mut txn, 0, arr_len);
            let arr_len = arr.len(&txn);
            assert_eq!(arr_len, 0);
            arr.insert_range(&mut txn, arr_len, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        }
        arr.move_range_to(
            &mut doc.transact_mut(),
            3,
            Assoc::After,
            5,
            Assoc::Before,
            3,
        );
        assert_eq!(
            arr.to_json(&doc.transact()),
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9].into()
        );
    }

    #[test]
    fn multi_threading() {
        use std::sync::{Arc, RwLock};
        use std::thread::{sleep, spawn};

        let doc = Arc::new(RwLock::new(Doc::with_client_id(1)));

        let d2 = doc.clone();
        let h2 = spawn(move || {
            for _ in 0..10 {
                let millis = fastrand::u64(1..20);
                sleep(Duration::from_millis(millis));

                let doc = d2.write().unwrap();
                let array = doc.get_or_insert_array("test");
                let mut txn = doc.transact_mut();
                array.push_back(&mut txn, "a");
            }
        });

        let d3 = doc.clone();
        let h3 = spawn(move || {
            for _ in 0..10 {
                let millis = fastrand::u64(1..20);
                sleep(Duration::from_millis(millis));

                let doc = d3.write().unwrap();
                let array = doc.get_or_insert_array("test");
                let mut txn = doc.transact_mut();
                array.push_back(&mut txn, "b");
            }
        });

        h3.join().unwrap();
        h2.join().unwrap();

        let doc = doc.read().unwrap();
        let array = doc.get_or_insert_array("test");
        let len = array.len(&doc.transact());
        assert_eq!(len, 20);
    }

    #[test]
    fn move_last_elem_iter() {
        // https://github.com/y-crdt/y-crdt/issues/186

        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");
        let mut txn = doc.transact_mut();
        array.insert_range(&mut txn, 0, [1, 2, 3]);
        drop(txn);

        let mut txn = doc.transact_mut();
        array.move_to(&mut txn, 2, 0);

        let mut iter = array.iter(&txn);
        let v = iter.next();
        assert_eq!(v, Some(3.into()));
        let v = iter.next();
        assert_eq!(v, Some(1.into()));
        let v = iter.next();
        assert_eq!(v, Some(2.into()));
        let v = iter.next();
        assert_eq!(v, None);
    }

    #[test]
    fn insert_empty_range() {
        let doc = Doc::with_client_id(1);
        let mut txn = doc.transact_mut();
        let array = txn.get_or_insert_array("array");

        array.insert(&mut txn, 0, 1);
        array.insert_range::<_, Any>(&mut txn, 1, []);
        array.push_back(&mut txn, 2);

        assert_eq!(
            array.iter(&txn).collect::<Vec<_>>(),
            vec![1.into(), 2.into()]
        );

        let data = txn.encode_state_as_update_v1(&StateVector::default());

        let doc2 = Doc::with_client_id(2);
        let mut txn = doc2.transact_mut();
        let array = txn.get_or_insert_array("array");
        txn.apply_update(Update::decode_v1(&data).unwrap()).unwrap();

        assert_eq!(
            array.iter(&txn).collect::<Vec<_>>(),
            vec![1.into(), 2.into()]
        );
    }
}
