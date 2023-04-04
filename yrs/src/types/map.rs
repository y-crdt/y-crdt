use crate::block::{Block, BlockPtr, EmbedPrelim, ItemContent, ItemPosition, Prelim};
use crate::transaction::TransactionMut;
use crate::types::{
    event_keys, Branch, BranchPtr, Entries, EntryChange, EventHandler, Observers, Path, ToJson,
    Value, TYPE_REFS_MAP,
};
use crate::*;
use lib0::any::Any;
use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::collections::{HashMap, HashSet};
use std::convert::{TryFrom, TryInto};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

/// Collection used to store key-value entries in an unordered manner. Keys are always represented
/// as UTF-8 strings. Values can be any value type supported by Yrs: JSON-like primitives as well as
/// shared data types.
///
/// In terms of conflict resolution, [MapRef] uses logical last-write-wins principle, meaning the past
/// updates are automatically overridden and discarded by newer ones, while concurrent updates made
/// by different peers are resolved into a single value using document id seniority to establish
/// order.
///
/// # Example
///
/// ```rust
///
/// use lib0::any;
/// use yrs::{Doc, Map, MapPrelim, Transact};
/// use yrs::types::ToJson;
///
/// let doc = Doc::new();
/// let map = doc.get_or_insert_map("map");
/// let mut txn = doc.transact_mut();
///
/// // insert value
/// map.insert(&mut txn, "key1", "value1");
///
/// // insert nested shared type
/// let nested = map.insert(&mut txn, "key2", MapPrelim::from([("inner", "value2")]));
/// nested.insert(&mut txn, "inner2", 100);
///
/// assert_eq!(map.to_json(&txn), any!({
///   "key1": "value1",
///   "key2": {
///     "inner": "value2",
///     "inner2": 100
///   }
/// }));
///
/// // get value
/// assert_eq!(map.get(&txn, "key1"), Some("value1".into()));
///
/// // remove entry
/// map.remove(&mut txn, "key1");
/// assert_eq!(map.get(&txn, "key1"), None);
/// ```
#[repr(transparent)]
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MapRef(BranchPtr);

impl Map for MapRef {}

impl Observable for MapRef {
    type Event = MapEvent;

    fn try_observer(&self) -> Option<&EventHandler<Self::Event>> {
        if let Some(Observers::Map(eh)) = self.0.observers.as_ref() {
            Some(eh)
        } else {
            None
        }
    }

    fn try_observer_mut(&mut self) -> Option<&mut EventHandler<Self::Event>> {
        if let Observers::Map(eh) = self.0.observers.get_or_insert_with(Observers::map) {
            Some(eh)
        } else {
            None
        }
    }
}

impl ToJson for MapRef {
    fn to_json<T: ReadTxn>(&self, txn: &T) -> Any {
        let inner = self.0;
        let mut res = HashMap::new();
        for (key, ptr) in inner.map.iter() {
            if let Block::Item(item) = ptr.deref() {
                if !item.is_deleted() {
                    let last = item.content.get_last().unwrap_or(Value::Any(Any::Null));
                    res.insert(key.to_string(), last.to_json(txn));
                }
            }
        }
        Any::Map(Box::new(res))
    }
}

impl AsRef<Branch> for MapRef {
    fn as_ref(&self) -> &Branch {
        self.0.deref()
    }
}

impl AsMut<Branch> for MapRef {
    fn as_mut(&mut self) -> &mut Branch {
        self.0.deref_mut()
    }
}

impl TryFrom<BlockPtr> for MapRef {
    type Error = BlockPtr;

    fn try_from(value: BlockPtr) -> Result<Self, Self::Error> {
        if let Some(branch) = value.clone().as_branch() {
            Ok(MapRef::from(branch))
        } else {
            Err(value)
        }
    }
}

pub trait Map: AsRef<Branch> {
    /// Returns a number of entries stored within current map.
    fn len<T: ReadTxn>(&self, txn: &T) -> u32 {
        let mut len = 0;
        let inner = self.as_ref();
        for ptr in inner.map.values() {
            //TODO: maybe it would be better to just cache len in the map itself?
            if let Block::Item(item) = ptr.deref() {
                if !item.is_deleted() {
                    len += 1;
                }
            }
        }
        len
    }

    /// Returns an iterator that enables to traverse over all keys of entries stored within
    /// current map. These keys are not ordered.
    fn keys<'a, T: ReadTxn + 'a>(&'a self, txn: &'a T) -> Keys<'a, &'a T, T> {
        Keys::new(self.as_ref(), txn)
    }

    /// Returns an iterator that enables to traverse over all values stored within current map.
    fn values<'a, T: ReadTxn + 'a>(&'a self, txn: &'a T) -> Values<'a, &'a T, T> {
        Values::new(self.as_ref(), txn)
    }

    /// Returns an iterator that enables to traverse over all entries - tuple of key-value pairs -
    /// stored within current map.
    fn iter<'a, T: ReadTxn + 'a>(&'a self, txn: &'a T) -> MapIter<'a, &'a T, T> {
        MapIter::new(self.as_ref(), txn)
    }

    /// Inserts a new `value` under given `key` into current map. Returns an integrated value.
    fn insert<K, V>(&self, txn: &mut TransactionMut, key: K, value: V) -> V::Return
    where
        K: Into<Rc<str>>,
        V: Prelim,
    {
        let key = key.into();
        let pos = {
            let inner = self.as_ref();
            let left = inner.map.get(&key);
            ItemPosition {
                parent: BranchPtr::from(inner).into(),
                left: left.cloned(),
                right: None,
                index: 0,
                current_attrs: None,
            }
        };

        let ptr = txn.create_item(&pos, value, Some(key));
        if let Ok(integrated) = ptr.try_into() {
            integrated
        } else {
            panic!("Defect: unexpected integrated type")
        }
    }

    /// Removes a stored within current map under a given `key`. Returns that value or `None` if
    /// no entry with a given `key` was present in current map.
    ///
    /// ### Removing nested shared types
    ///
    /// In case when a nested shared type (eg. [MapRef], [ArrayRef], [TextRef]) is being removed,
    /// all of its contents will also be deleted recursively. A returned value will contain a
    /// reference to a current removed shared type (which will be empty due to all of its elements
    /// being deleted), **not** the content prior the removal.
    fn remove(&self, txn: &mut TransactionMut, key: &str) -> Option<Value> {
        let ptr = BranchPtr::from(self.as_ref());
        ptr.remove(txn, key)
    }

    /// Returns a value stored under a given `key` within current map, or `None` if no entry
    /// with such `key` existed.
    fn get<T: ReadTxn>(&self, txn: &T, key: &str) -> Option<Value> {
        let ptr = BranchPtr::from(self.as_ref());
        ptr.get(txn, key)
    }

    /// Checks if an entry with given `key` can be found within current map.
    fn contains_key<T: ReadTxn>(&self, txn: &T, key: &str) -> bool {
        if let Some(ptr) = self.as_ref().map.get(key) {
            if let Block::Item(item) = ptr.deref() {
                return !item.is_deleted();
            }
        }
        false
    }

    /// Clears the contents of current map, effectively removing all of its entries.
    fn clear(&self, txn: &mut TransactionMut) {
        for (_, ptr) in self.as_ref().map.iter() {
            txn.delete(ptr.clone());
        }
    }
}

#[derive(Debug)]
pub struct MapIter<'a, B, T>(Entries<'a, B, T>);

impl<'a, B, T> MapIter<'a, B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
    pub fn new(branch: &'a Branch, txn: B) -> Self {
        let entries = Entries::new(&branch.map, txn);
        MapIter(entries)
    }
}

impl<'a, B, T> Iterator for MapIter<'a, B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
    type Item = (&'a str, Value);

    fn next(&mut self) -> Option<Self::Item> {
        let (key, item) = self.0.next()?;
        if let Some(content) = item.content.get_last() {
            Some((key, content))
        } else {
            self.next()
        }
    }
}

/// An unordered iterator over the keys of a [Map].
#[derive(Debug)]
pub struct Keys<'a, B, T>(Entries<'a, B, T>);

impl<'a, B, T> Keys<'a, B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
    pub fn new(branch: &'a Branch, txn: B) -> Self {
        let entries = Entries::new(&branch.map, txn);
        Keys(entries)
    }
}

impl<'a, B, T> Iterator for Keys<'a, B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        let (key, _) = self.0.next()?;
        Some(key)
    }
}

/// Iterator over the values of a [Map].
#[derive(Debug)]
pub struct Values<'a, B, T>(Entries<'a, B, T>);

impl<'a, B, T> Values<'a, B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
    pub fn new(branch: &'a Branch, txn: B) -> Self {
        let entries = Entries::new(&branch.map, txn);
        Values(entries)
    }
}

impl<'a, B, T> Iterator for Values<'a, B, T>
where
    B: Borrow<T>,
    T: ReadTxn,
{
    type Item = Vec<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        let (_, item) = self.0.next()?;
        let len = item.len() as usize;
        let mut values = vec![Value::default(); len];
        if item.content.read(0, &mut values) == len {
            Some(values)
        } else {
            panic!("Defect: iterator didn't read all elements")
        }
    }
}

impl From<BranchPtr> for MapRef {
    fn from(inner: BranchPtr) -> Self {
        MapRef(inner)
    }
}

/// A preliminary map. It can be used to early initialize the contents of a [Map], when it's about
/// to be inserted into another Yrs collection, such as [Array] or another [Map].
pub struct MapPrelim<T>(HashMap<String, T>);

impl<T> MapPrelim<T> {
    pub fn new() -> Self {
        MapPrelim(HashMap::default())
    }
}

impl<T> From<HashMap<String, T>> for MapPrelim<T> {
    fn from(map: HashMap<String, T>) -> Self {
        MapPrelim(map)
    }
}

impl<K, V, const N: usize> From<[(K, V); N]> for MapPrelim<V>
where
    K: Into<String>,
    V: Prelim,
{
    fn from(arr: [(K, V); N]) -> Self {
        let mut m = HashMap::with_capacity(N);
        for (k, v) in arr {
            m.insert(k.into(), v);
        }
        MapPrelim::from(m)
    }
}

impl<T: Prelim> Prelim for MapPrelim<T> {
    type Return = MapRef;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let inner = Branch::new(TYPE_REFS_MAP, None);
        (ItemContent::Type(inner), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        let map = MapRef::from(inner_ref);
        for (key, value) in self.0 {
            map.insert(txn, key, value);
        }
    }
}

impl<T: Prelim> Into<EmbedPrelim<MapPrelim<T>>> for MapPrelim<T> {
    #[inline]
    fn into(self) -> EmbedPrelim<MapPrelim<T>> {
        EmbedPrelim::Shared(self)
    }
}

/// Event generated by [Map::observe] method. Emitted during transaction commit phase.
pub struct MapEvent {
    pub(crate) current_target: BranchPtr,
    target: MapRef,
    keys: UnsafeCell<Result<HashMap<Rc<str>, EntryChange>, HashSet<Option<Rc<str>>>>>,
}

impl MapEvent {
    pub(crate) fn new(branch_ref: BranchPtr, key_changes: HashSet<Option<Rc<str>>>) -> Self {
        let current_target = branch_ref.clone();
        MapEvent {
            target: MapRef::from(branch_ref),
            current_target,
            keys: UnsafeCell::new(Err(key_changes)),
        }
    }

    /// Returns a [Map] instance which emitted this event.
    pub fn target(&self) -> &MapRef {
        &self.target
    }

    /// Returns a path from root type down to [Map] instance which emitted this event.
    pub fn path(&self) -> Path {
        Branch::path(self.current_target, self.target.0)
    }

    /// Returns a summary of key-value changes made over corresponding [Map] collection within
    /// bounds of current transaction.
    pub fn keys(&self, txn: &TransactionMut) -> &HashMap<Rc<str>, EntryChange> {
        let keys = unsafe { self.keys.get().as_mut().unwrap() };

        match keys {
            Ok(keys) => {
                return keys;
            }
            Err(subs) => {
                let subs = event_keys(txn, self.target.0, subs);
                *keys = Ok(subs);
                if let Ok(keys) = keys {
                    keys
                } else {
                    panic!("Defect: should not happen");
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::test_utils::{exchange_updates, run_scenario};
    use crate::transaction::ReadTxn;
    use crate::types::text::TextPrelim;
    use crate::types::{DeepObservable, EntryChange, Event, Path, PathSegment, ToJson, Value};
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::{Encoder, EncoderV1};
    use crate::{
        Array, ArrayPrelim, Doc, Map, MapPrelim, MapRef, Observable, StateVector, Text, Transact,
        Update,
    };
    use lib0::any;
    use lib0::any::Any;
    use rand::distributions::Alphanumeric;
    use rand::prelude::{SliceRandom, StdRng};
    use rand::Rng;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ops::{Deref, DerefMut};
    use std::rc::Rc;
    use std::time::Duration;

    #[test]
    fn map_basic() {
        let d1 = Doc::with_client_id(1);
        let m1 = d1.get_or_insert_map("map");
        let mut t1 = d1.transact_mut();

        let d2 = Doc::with_client_id(2);
        let m2 = d2.get_or_insert_map("map");
        let mut t2 = d2.transact_mut();

        m1.insert(&mut t1, "number".to_owned(), 1);
        m1.insert(&mut t1, "string".to_owned(), "hello Y");
        m1.insert(&mut t1, "object".to_owned(), {
            let mut v = HashMap::new();
            v.insert("key2".to_owned(), "value");

            let mut map = HashMap::new();
            map.insert("key".to_owned(), v);
            map // { key: { key2: 'value' } }
        });
        m1.insert(&mut t1, "boolean1".to_owned(), true);
        m1.insert(&mut t1, "boolean0".to_owned(), false);

        //let m1m = t1.get_map("y-map");
        //let m1a = t1.get_text("y-text");
        //m1a.insert(&mut t1, 0, "a");
        //m1a.insert(&mut t1, 0, "b");
        //m1m.insert(&mut t1, "y-text".to_owned(), m1a);

        //TODO: YArray within YMap
        fn compare_all<T: ReadTxn>(m: &MapRef, txn: &T) {
            assert_eq!(m.len(txn), 5);
            assert_eq!(m.get(txn, &"number".to_owned()), Some(Value::from(1f64)));
            assert_eq!(m.get(txn, &"boolean0".to_owned()), Some(Value::from(false)));
            assert_eq!(m.get(txn, &"boolean1".to_owned()), Some(Value::from(true)));
            assert_eq!(
                m.get(txn, &"string".to_owned()),
                Some(Value::from("hello Y"))
            );
            assert_eq!(
                m.get(txn, &"object".to_owned()),
                Some(Value::from(any!({
                    "key": {
                        "key2": "value"
                    }
                })))
            );
        }

        compare_all(&m1, &t1);

        let update = t1
            .encode_state_as_update_v1(&StateVector::default())
            .unwrap();
        t2.apply_update(Update::decode_v1(update.as_slice()).unwrap());

        compare_all(&m2, &t2);
    }

    #[test]
    fn map_get_set() {
        let d1 = Doc::with_client_id(1);
        let m1 = d1.get_or_insert_map("map");
        let mut t1 = d1.transact_mut();

        m1.insert(&mut t1, "stuff".to_owned(), "stuffy");
        m1.insert(&mut t1, "null".to_owned(), None as Option<String>);

        let update = t1
            .encode_state_as_update_v1(&StateVector::default())
            .unwrap();

        let d2 = Doc::with_client_id(2);
        let m2 = d2.get_or_insert_map("map");
        let mut t2 = d2.transact_mut();

        t2.apply_update(Update::decode_v1(update.as_slice()).unwrap());

        assert_eq!(
            m2.get(&t2, &"stuff".to_owned()),
            Some(Value::from("stuffy"))
        );
        assert_eq!(m2.get(&t2, &"null".to_owned()), Some(Value::Any(Any::Null)));
    }

    #[test]
    fn map_get_set_sync_with_conflicts() {
        let d1 = Doc::with_client_id(1);
        let m1 = d1.get_or_insert_map("map");
        let mut t1 = d1.transact_mut();

        let d2 = Doc::with_client_id(2);
        let m2 = d2.get_or_insert_map("map");
        let mut t2 = d2.transact_mut();

        m1.insert(&mut t1, "stuff".to_owned(), "c0");
        m2.insert(&mut t2, "stuff".to_owned(), "c1");

        let u1 = t1
            .encode_state_as_update_v1(&StateVector::default())
            .unwrap();
        let u2 = t2
            .encode_state_as_update_v1(&StateVector::default())
            .unwrap();

        t1.apply_update(Update::decode_v1(u2.as_slice()).unwrap());
        t2.apply_update(Update::decode_v1(u1.as_slice()).unwrap());

        assert_eq!(m1.get(&t1, &"stuff".to_owned()), Some(Value::from("c1")));
        assert_eq!(m2.get(&t2, &"stuff".to_owned()), Some(Value::from("c1")));
    }

    #[test]
    fn map_len_remove() {
        let d1 = Doc::with_client_id(1);
        let m1 = d1.get_or_insert_map("map");
        let mut t1 = d1.transact_mut();

        let key1 = "stuff".to_owned();
        let key2 = "other-stuff".to_owned();

        m1.insert(&mut t1, key1.clone(), "c0");
        m1.insert(&mut t1, key2.clone(), "c1");
        assert_eq!(m1.len(&t1), 2);

        // remove 'stuff'
        assert_eq!(m1.remove(&mut t1, &key1), Some(Value::from("c0")));
        assert_eq!(m1.len(&t1), 1);

        // remove 'stuff' again - nothing should happen
        assert_eq!(m1.remove(&mut t1, &key1), None);
        assert_eq!(m1.len(&t1), 1);

        // remove 'other-stuff'
        assert_eq!(m1.remove(&mut t1, &key2), Some(Value::from("c1")));
        assert_eq!(m1.len(&t1), 0);
    }

    #[test]
    fn map_clear() {
        let d1 = Doc::with_client_id(1);
        let m1 = d1.get_or_insert_map("map");
        let mut t1 = d1.transact_mut();

        m1.insert(&mut t1, "key1".to_owned(), "c0");
        m1.insert(&mut t1, "key2".to_owned(), "c1");
        m1.clear(&mut t1);

        assert_eq!(m1.len(&t1), 0);
        assert_eq!(m1.get(&t1, &"key1".to_owned()), None);
        assert_eq!(m1.get(&t1, &"key2".to_owned()), None);

        let d2 = Doc::with_client_id(2);
        let m2 = d2.get_or_insert_map("map");
        let mut t2 = d2.transact_mut();

        let u1 = t1
            .encode_state_as_update_v1(&StateVector::default())
            .unwrap();
        t2.apply_update(Update::decode_v1(u1.as_slice()).unwrap());

        assert_eq!(m2.len(&t2), 0);
        assert_eq!(m2.get(&t2, &"key1".to_owned()), None);
        assert_eq!(m2.get(&t2, &"key2".to_owned()), None);
    }

    #[test]
    fn map_clear_sync() {
        let d1 = Doc::with_client_id(1);
        let d2 = Doc::with_client_id(2);
        let d3 = Doc::with_client_id(3);
        let d4 = Doc::with_client_id(4);

        {
            let m1 = d1.get_or_insert_map("map");
            let m2 = d2.get_or_insert_map("map");
            let m3 = d3.get_or_insert_map("map");

            let mut t1 = d1.transact_mut();
            let mut t2 = d2.transact_mut();
            let mut t3 = d3.transact_mut();

            m1.insert(&mut t1, "key1".to_owned(), "c0");
            m2.insert(&mut t2, "key1".to_owned(), "c1");
            m2.insert(&mut t2, "key1".to_owned(), "c2");
            m3.insert(&mut t3, "key1".to_owned(), "c3");
        }

        exchange_updates(&[&d1, &d2, &d3, &d4]);

        {
            let m1 = d1.get_or_insert_map("map");
            let m2 = d2.get_or_insert_map("map");
            let m3 = d3.get_or_insert_map("map");

            let mut t1 = d1.transact_mut();
            let mut t2 = d2.transact_mut();
            let mut t3 = d3.transact_mut();

            m1.insert(&mut t1, "key2".to_owned(), "c0");
            m2.insert(&mut t2, "key2".to_owned(), "c1");
            m2.insert(&mut t2, "key2".to_owned(), "c2");
            m3.insert(&mut t3, "key2".to_owned(), "c3");
            m3.clear(&mut t3);
        }

        exchange_updates(&[&d1, &d2, &d3, &d4]);

        for doc in [d1, d2, d3, d4] {
            let map = doc.get_or_insert_map("map");

            assert_eq!(
                map.get(&map.transact(), &"key1".to_owned()),
                None,
                "'key1' entry for peer {} should be removed",
                doc.client_id()
            );
            assert_eq!(
                map.get(&map.transact(), &"key2".to_owned()),
                None,
                "'key2' entry for peer {} should be removed",
                doc.client_id()
            );
            assert_eq!(
                map.len(&map.transact()),
                0,
                "all entries for peer {} should be removed",
                doc.client_id()
            );
        }
    }

    #[test]
    fn map_get_set_with_3_way_conflicts() {
        let d1 = Doc::with_client_id(1);
        let d2 = Doc::with_client_id(2);
        let d3 = Doc::with_client_id(3);

        {
            let m1 = d1.get_or_insert_map("map");
            let m2 = d2.get_or_insert_map("map");
            let m3 = d3.get_or_insert_map("map");

            let mut t1 = d1.transact_mut();
            let mut t2 = d2.transact_mut();
            let mut t3 = d3.transact_mut();

            m1.insert(&mut t1, "stuff".to_owned(), "c0");
            m2.insert(&mut t2, "stuff".to_owned(), "c1");
            m2.insert(&mut t2, "stuff".to_owned(), "c2");
            m3.insert(&mut t3, "stuff".to_owned(), "c3");
        }

        exchange_updates(&[&d1, &d2, &d3]);

        for doc in [d1, d2, d3] {
            let map = doc.get_or_insert_map("map");

            assert_eq!(
                map.get(&map.transact(), &"stuff".to_owned()),
                Some(Value::from("c3")),
                "peer {} - map entry resolved to unexpected value",
                doc.client_id()
            );
        }
    }

    #[test]
    fn map_get_set_remove_with_3_way_conflicts() {
        let d1 = Doc::with_client_id(1);
        let d2 = Doc::with_client_id(2);
        let d3 = Doc::with_client_id(3);
        let d4 = Doc::with_client_id(4);

        {
            let m1 = d1.get_or_insert_map("map");
            let m2 = d2.get_or_insert_map("map");
            let m3 = d3.get_or_insert_map("map");

            let mut t1 = d1.transact_mut();
            let mut t2 = d2.transact_mut();
            let mut t3 = d3.transact_mut();

            m1.insert(&mut t1, "key1".to_owned(), "c0");
            m2.insert(&mut t2, "key1".to_owned(), "c1");
            m2.insert(&mut t2, "key1".to_owned(), "c2");
            m3.insert(&mut t3, "key1".to_owned(), "c3");
        }

        exchange_updates(&[&d1, &d2, &d3, &d4]);

        {
            let m1 = d1.get_or_insert_map("map");
            let m2 = d2.get_or_insert_map("map");
            let m3 = d3.get_or_insert_map("map");
            let m4 = d4.get_or_insert_map("map");

            let mut t1 = d1.transact_mut();
            let mut t2 = d2.transact_mut();
            let mut t3 = d3.transact_mut();
            let mut t4 = d4.transact_mut();

            m1.insert(&mut t1, "key1".to_owned(), "deleteme");
            m2.insert(&mut t2, "key1".to_owned(), "c1");
            m3.insert(&mut t3, "key1".to_owned(), "c2");
            m4.insert(&mut t4, "key1".to_owned(), "c3");
            m4.remove(&mut t4, &"key1".to_owned());
        }

        exchange_updates(&[&d1, &d2, &d3, &d4]);

        for doc in [d1, d2, d3, d4] {
            let map = doc.get_or_insert_map("map");

            assert_eq!(
                map.get(&map.transact(), &"key1".to_owned()),
                None,
                "entry 'key1' on peer {} should be removed",
                doc.client_id()
            );
        }
    }

    #[test]
    fn insert_and_remove_events() {
        let d1 = Doc::with_client_id(1);
        let mut m1 = d1.get_or_insert_map("map");

        let entries = Rc::new(RefCell::new(None));
        let entries_c = entries.clone();
        let _sub = m1.observe(move |txn, e| {
            let keys = e.keys(txn);
            *entries_c.borrow_mut() = Some(keys.clone());
        });

        // insert new entry
        {
            let mut txn = d1.transact_mut();
            m1.insert(&mut txn, "a", 1);
            // txn is committed at the end of this scope
        }
        assert_eq!(
            entries.take(),
            Some(HashMap::from([(
                "a".into(),
                EntryChange::Inserted(Any::Number(1.0).into())
            )]))
        );

        // update existing entry once
        {
            let mut txn = d1.transact_mut();
            m1.insert(&mut txn, "a", 2);
        }
        assert_eq!(
            entries.take(),
            Some(HashMap::from([(
                "a".into(),
                EntryChange::Updated(Any::Number(1.0).into(), Any::Number(2.0).into())
            )]))
        );

        // update existing entry twice
        {
            let mut txn = d1.transact_mut();
            m1.insert(&mut txn, "a", 3);
            m1.insert(&mut txn, "a", 4);
        }
        assert_eq!(
            entries.take(),
            Some(HashMap::from([(
                "a".into(),
                EntryChange::Updated(Any::Number(2.0).into(), Any::Number(4.0).into())
            )]))
        );

        // remove existing entry
        {
            let mut txn = d1.transact_mut();
            m1.remove(&mut txn, "a");
        }
        assert_eq!(
            entries.take(),
            Some(HashMap::from([(
                "a".into(),
                EntryChange::Removed(Any::Number(4.0).into())
            )]))
        );

        // add another entry and update it
        {
            let mut txn = d1.transact_mut();
            m1.insert(&mut txn, "b", 1);
            m1.insert(&mut txn, "b", 2);
        }
        assert_eq!(
            entries.take(),
            Some(HashMap::from([(
                "b".into(),
                EntryChange::Inserted(Any::Number(2.0).into())
            )]))
        );

        // add and remove an entry
        {
            let mut txn = d1.transact_mut();
            m1.insert(&mut txn, "c", 1);
            m1.remove(&mut txn, "c");
        }
        assert_eq!(entries.take(), Some(HashMap::new()));

        // copy updates over
        let d2 = Doc::with_client_id(2);
        let mut m2 = d2.get_or_insert_map("map");

        let entries = Rc::new(RefCell::new(None));
        let entries_c = entries.clone();
        let _sub = m2.observe(move |txn, e| {
            let keys = e.keys(txn);
            *entries_c.borrow_mut() = Some(keys.clone());
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
            entries.take(),
            Some(HashMap::from([(
                "b".into(),
                EntryChange::Inserted(Any::Number(2.0).into())
            )]))
        );
    }

    fn random_string(rng: &mut StdRng) -> String {
        let len = rng.gen_range(1, 10);
        rng.sample_iter(&Alphanumeric)
            .take(len)
            .map(char::from)
            .collect()
    }

    fn map_transactions() -> [Box<dyn Fn(&mut Doc, &mut StdRng)>; 3] {
        fn set(doc: &mut Doc, rng: &mut StdRng) {
            let map = doc.get_or_insert_map("map");
            let mut txn = doc.transact_mut();
            let key = ["one", "two"].choose(rng).unwrap();
            let value: String = random_string(rng);
            map.insert(&mut txn, key.to_string(), value);
        }

        fn set_type(doc: &mut Doc, rng: &mut StdRng) {
            let map = doc.get_or_insert_map("map");
            let mut txn = doc.transact_mut();
            let key = ["one", "two", "three"].choose(rng).unwrap();
            if rng.gen_bool(0.33) {
                map.insert(
                    &mut txn,
                    key.to_string(),
                    ArrayPrelim::from(vec![1, 2, 3, 4]),
                );
            } else if rng.gen_bool(0.33) {
                map.insert(&mut txn, key.to_string(), TextPrelim::new("deeptext"));
            } else {
                map.insert(
                    &mut txn,
                    key.to_string(),
                    MapPrelim::from({
                        let mut map = HashMap::default();
                        map.insert("deepkey".to_owned(), "deepvalue");
                        map
                    }),
                );
            }
        }

        fn delete(doc: &mut Doc, rng: &mut StdRng) {
            let map = doc.get_or_insert_map("map");
            let mut txn = doc.transact_mut();
            let key = ["one", "two"].choose(rng).unwrap();
            map.remove(&mut txn, key);
        }
        [Box::new(set), Box::new(set_type), Box::new(delete)]
    }

    fn fuzzy(iterations: usize) {
        run_scenario(0, &map_transactions(), 5, iterations)
    }

    #[test]
    fn fuzzy_test_6() {
        fuzzy(6)
    }

    #[test]
    fn observe_deep() {
        let doc = Doc::with_client_id(1);
        let mut map = doc.get_or_insert_map("map");

        let paths = Rc::new(RefCell::new(vec![]));
        let calls = Rc::new(RefCell::new(0));
        let paths_copy = paths.clone();
        let calls_copy = calls.clone();
        let _sub = map.observe_deep(move |_txn, e| {
            let path: Vec<Path> = e.iter().map(Event::path).collect();
            paths_copy.borrow_mut().push(path);
            let mut count = calls_copy.borrow_mut();
            let count = count.deref_mut();
            *count += 1;
        });

        let nested = map.insert(&mut doc.transact_mut(), "map", MapPrelim::<String>::new());
        nested.insert(
            &mut doc.transact_mut(),
            "array",
            ArrayPrelim::from(Vec::<String>::default()),
        );
        let nested2 = nested
            .get(&nested.transact(), "array")
            .unwrap()
            .to_yarray()
            .unwrap();
        nested2.insert(&mut doc.transact_mut(), 0, "content");

        let nested_text = nested.insert(&mut doc.transact_mut(), "text", TextPrelim::new("text"));
        nested_text.push(&mut doc.transact_mut(), "!");

        assert_eq!(*calls.borrow().deref(), 5);
        let actual = paths.borrow();
        assert_eq!(
            actual.as_slice(),
            &[
                vec![Path::from(vec![])],
                vec![Path::from(vec![PathSegment::Key("map".into())])],
                vec![Path::from(vec![
                    PathSegment::Key("map".into()),
                    PathSegment::Key("array".into())
                ])],
                vec![Path::from(vec![PathSegment::Key("map".into()),])],
                vec![Path::from(vec![
                    PathSegment::Key("map".into()),
                    PathSegment::Key("text".into()),
                ])],
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
                let map = doc.get_or_insert_map("test");
                let mut txn = doc.transact_mut();
                map.insert(&mut txn, "key", 1);
            }
        });

        let d3 = doc.clone();
        let h3 = spawn(move || {
            for _ in 0..10 {
                let millis = thread_rng().gen_range(1, 20);
                sleep(Duration::from_millis(millis));

                let doc = d3.write().unwrap();
                let map = doc.get_or_insert_map("test");
                let mut txn = doc.transact_mut();
                map.insert(&mut txn, "key", 2);
            }
        });

        h3.join().unwrap();
        h2.join().unwrap();

        let doc = doc.read().unwrap();
        let map = doc.get_or_insert_map("test");
        let txn = doc.transact();
        let value = map.get(&txn, "key").unwrap().to_json(&txn);

        assert!(value == 1.into() || value == 2.into())
    }
}
