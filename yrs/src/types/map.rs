use crate::block::{EmbedPrelim, Item, ItemContent, ItemPosition, ItemPtr, Prelim};
use crate::encoding::read::Error;
use crate::encoding::serde::from_any;
use crate::lazy::{Lazy, Once};
use crate::out::FromOut;
use crate::transaction::{TransactionMut, TransactionState};
use crate::types::{
    event_keys, AsPrelim, Branch, BranchPtr, DefaultPrelim, Entries, EntryChange, In, Out, Path,
    RootRef, SharedRef, ToJson, TypeRef,
};
use crate::*;
use serde::de::DeserializeOwned;
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::convert::{TryFrom, TryInto};
use std::iter::FromIterator;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

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
/// use yrs::{any, Doc, Map, MapPrelim};
/// use yrs::types::ToJson;
///
/// let mut doc = Doc::new();
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
#[derive(Debug, Clone)]
pub struct MapRef(BranchPtr);

impl RootRef for MapRef {
    fn type_ref() -> TypeRef {
        TypeRef::Map
    }
}
impl SharedRef for MapRef {}
impl Map for MapRef {}

impl DeepObservable for MapRef {}
impl Observable for MapRef {
    type Event = MapEvent;
}

impl ToJson for MapRef {
    fn to_json<T: ReadTxn>(&self, txn: &T) -> Any {
        let inner = self.0;
        let mut res = HashMap::new();
        for (key, item) in inner.map.iter() {
            if !item.is_deleted() {
                let last = item.content.get_last().unwrap_or(Out::Any(Any::Null));
                res.insert(key.to_string(), last.to_json(txn));
            }
        }
        Any::from(res)
    }
}

impl AsRef<Branch> for MapRef {
    fn as_ref(&self) -> &Branch {
        self.0.deref()
    }
}

impl Eq for MapRef {}
impl PartialEq for MapRef {
    fn eq(&self, other: &Self) -> bool {
        self.0.id() == other.0.id()
    }
}

impl FromOut for MapRef {
    fn from_out<T: ReadTxn>(value: Out, txn: &T) -> Result<Self, Out>
    where
        Self: Sized,
    {
        match value {
            Out::Map(value) => Ok(value),
            other => Err(other),
        }
    }

    fn from_item<T: ReadTxn>(item: ItemPtr, txn: &T) -> Option<Self>
    where
        Self: Sized,
    {
        let branch = item.as_branch()?;
        Some(MapRef::from(branch))
    }
}

impl AsPrelim for MapRef {
    type Prelim = MapPrelim;

    fn as_prelim<T: ReadTxn>(&self, txn: &T) -> Self::Prelim {
        let mut prelim = HashMap::with_capacity(self.len(txn) as usize);
        for (key, &ptr) in self.0.map.iter() {
            if !ptr.is_deleted() {
                if let Ok(value) = Out::try_from(ptr) {
                    prelim.insert(key.clone(), value.as_prelim(txn));
                }
            }
        }
        MapPrelim(prelim)
    }
}

impl DefaultPrelim for MapRef {
    type Prelim = MapPrelim;

    #[inline]
    fn default_prelim() -> Self::Prelim {
        MapPrelim::default()
    }
}

pub trait Map: AsRef<Branch> + Sized {
    /// Returns a number of entries stored within current map.
    fn len<T: ReadTxn>(&self, _txn: &T) -> u32 {
        let mut len = 0;
        let inner = self.as_ref();
        for item in inner.map.values() {
            //TODO: maybe it would be better to just cache len in the map itself?
            if !item.is_deleted() {
                len += 1;
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

    fn into_iter<'a, T: ReadTxn + 'a>(self, txn: &'a T) -> MapIntoIter<'a, T> {
        let branch_ptr = BranchPtr::from(self.as_ref());
        MapIntoIter::new(branch_ptr, txn)
    }

    /// Inserts a new `value` under given `key` into current map. Returns an integrated value.
    fn insert<K, V>(&self, txn: &mut TransactionMut, key: K, value: V) -> V::Return
    where
        K: Into<Arc<str>>,
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

        let ptr = txn
            .create_item(&pos, value, Some(key))
            .expect("Cannot insert empty value");
        if let Some(integrated) = <V as Prelim>::Return::from_item(ptr, txn) {
            integrated
        } else {
            panic!("Defect: unexpected integrated type")
        }
    }

    /// Tries to update a value stored under a given `key` within current map, if it's different
    /// from the current one. Returns `true` if the value was updated, `false` otherwise.
    ///
    /// The main difference from [Map::insert] is that this method will not insert a new value if
    /// it's the same as the current one. It's important distinction when dealing with shared types,
    /// as inserting an element will force previous value to be tombstoned, causing minimal memory
    /// overhead.
    ///
    /// # Example
    ///
    /// ```rust
    /// use yrs::{Doc, Map};
    ///
    /// let mut doc = Doc::new();
    /// let mut txn = doc.transact_mut();
    /// let map = txn.get_or_insert_map("map");
    ///
    /// assert!(map.try_update(&mut txn, "key", 1)); // created a new entry
    /// assert!(!map.try_update(&mut txn, "key", 1)); // unchanged value doesn't trigger inserts...
    /// assert!(map.try_update(&mut txn, "key", 2)); // ... but changed one does
    /// ```
    fn try_update<K, V>(&self, txn: &mut TransactionMut, key: K, value: V) -> bool
    where
        K: Into<Arc<str>>,
        V: Into<Any>,
    {
        let key = key.into();
        let value = value.into();
        let branch = self.as_ref();
        if let Some(item) = branch.map.get(&key) {
            if !item.is_deleted() {
                if let ItemContent::Any(content) = &item.content {
                    if let Some(last) = content.last() {
                        if last == &value {
                            return false;
                        }
                    }
                }
            }
        }

        self.insert(txn, key, value);
        true
    }

    /// Returns an existing instance of a type stored under a given `key` within current map.
    /// If the given entry was not found, has been deleted or its type is different from expected,
    /// that entry will be reset to a given type and its reference will be returned.
    fn get_or_init<K, V>(&self, txn: &mut TransactionMut, key: K) -> V
    where
        K: Into<Arc<str>>,
        V: DefaultPrelim + FromOut,
    {
        let key = key.into();
        let branch = self.as_ref();
        if let Some(value) = branch.get(txn, &key) {
            if let Ok(value) = V::from_out(value, txn) {
                return value;
            }
        }
        let value = V::default_prelim();
        self.insert(txn, key, value)
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
    fn remove(&self, txn: &mut TransactionMut, key: &str) -> Option<Out> {
        let ptr = BranchPtr::from(self.as_ref());
        ptr.remove(txn, key)
    }

    /// Returns [WeakPrelim] to a given `key`, if it exists in a current map.
    #[cfg(feature = "weak")]
    fn link<T: ReadTxn>(&self, _txn: &T, key: &str) -> Option<crate::WeakPrelim<Self>> {
        let ptr = BranchPtr::from(self.as_ref());
        let block = ptr.map.get(key)?;
        let start = StickyIndex::from_id(block.id().clone(), Assoc::Before);
        let end = StickyIndex::from_id(block.id().clone(), Assoc::After);
        let link = crate::WeakPrelim::new(start, end);
        Some(link)
    }

    /// Returns a value stored under a given `key` within current map, or `None` if no entry
    /// with such `key` existed.
    fn get<T: ReadTxn, R: FromOut>(&self, txn: &T, key: &str) -> Option<R> {
        let ptr = BranchPtr::from(self.as_ref());
        let out = ptr.get(txn, key)?;
        R::from_out(out, txn).ok()
    }

    /// Returns a value stored under a given `key` within current map, deserializing it into expected
    /// type if found. If value was not found, the `Any::Null` will be substituted and deserialized
    /// instead (i.e. into instance of `Option` type, if so desired).
    ///
    /// # Example
    ///
    /// ```rust
    /// use yrs::{Doc, In, Map, MapPrelim};
    ///
    /// let mut doc = Doc::new();
    /// let mut txn = doc.transact_mut();
    /// let map = txn.get_or_insert_map("map");
    ///
    /// // insert a multi-nested shared refs
    /// let alice = map.insert(&mut txn, "Alice", MapPrelim::from([
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
    /// let alice: Person = map.get_as(&txn, "Alice").unwrap();
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
    /// let bob: Option<Person> = map.get_as(&txn, "Bob").unwrap();
    /// assert_eq!(bob, None);
    /// ```
    fn get_as<T, V>(&self, txn: &T, key: &str) -> Result<V, Error>
    where
        T: ReadTxn,
        V: DeserializeOwned,
    {
        let ptr = BranchPtr::from(self.as_ref());
        let out = ptr.get(txn, key).unwrap_or(Out::Any(Any::Null));
        //TODO: we could probably optimize this step by not serializing to intermediate Any value
        let any = out.to_json(txn);
        from_any(&any)
    }

    /// Checks if an entry with given `key` can be found within current map.
    fn contains_key<T: ReadTxn>(&self, _txn: &T, key: &str) -> bool {
        if let Some(item) = self.as_ref().map.get(key) {
            !item.is_deleted()
        } else {
            false
        }
    }

    /// Clears the contents of current map, effectively removing all of its entries.
    fn clear(&self, txn: &mut TransactionMut) {
        for (_, ptr) in self.as_ref().map.iter() {
            txn.delete(ptr.clone());
        }
    }
}

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
    type Item = (&'a str, Out);

    fn next(&mut self) -> Option<Self::Item> {
        let (key, item) = self.0.next()?;
        if let Some(content) = item.content.get_last() {
            Some((key, content))
        } else {
            self.next()
        }
    }
}

pub struct MapIntoIter<'a, T> {
    _txn: &'a T,
    entries: std::collections::hash_map::IntoIter<Arc<str>, ItemPtr>,
}

impl<'a, T: ReadTxn> MapIntoIter<'a, T> {
    fn new(map: BranchPtr, txn: &'a T) -> Self {
        let entries = map.map.clone().into_iter();
        MapIntoIter { _txn: txn, entries }
    }
}

impl<'a, T: ReadTxn> Iterator for MapIntoIter<'a, T> {
    type Item = (Arc<str>, Out);

    fn next(&mut self) -> Option<Self::Item> {
        let (key, item) = self.entries.next()?;
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
    type Item = Vec<Out>;

    fn next(&mut self) -> Option<Self::Item> {
        let (_, item) = self.0.next()?;
        let len = item.len() as usize;
        let mut values = vec![Out::default(); len];
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

/// A preliminary map. It can be used to early initialize the contents of a [MapRef], when it's about
/// to be inserted into another Yrs collection, such as [ArrayRef] or another [MapRef].
#[repr(transparent)]
#[derive(Debug, PartialEq, Default)]
pub struct MapPrelim(HashMap<Arc<str>, In>);

impl Deref for MapPrelim {
    type Target = HashMap<Arc<str>, In>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for MapPrelim {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<MapPrelim> for In {
    #[inline]
    fn from(value: MapPrelim) -> Self {
        In::Map(value)
    }
}

impl<S, T> FromIterator<(S, T)> for MapPrelim
where
    S: Into<Arc<str>>,
    T: Into<In>,
{
    fn from_iter<I: IntoIterator<Item = (S, T)>>(iter: I) -> Self {
        MapPrelim(
            iter.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}

impl<S, T, const C: usize> From<[(S, T); C]> for MapPrelim
where
    S: Into<Arc<str>>,
    T: Into<In>,
{
    fn from(map: [(S, T); C]) -> Self {
        let mut m = HashMap::with_capacity(C);
        for (key, value) in map {
            m.insert(key.into(), value.into());
        }
        MapPrelim(m)
    }
}

impl Prelim for MapPrelim {
    type Return = MapRef;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let inner = Branch::new(TypeRef::Map);
        (ItemContent::Type(inner), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: ItemPtr) {
        let map = MapRef::from(inner_ref.as_branch().unwrap());
        for (key, value) in self.0 {
            map.insert(txn, key, value);
        }
    }
}

impl Into<EmbedPrelim<MapPrelim>> for MapPrelim {
    #[inline]
    fn into(self) -> EmbedPrelim<MapPrelim> {
        EmbedPrelim::Shared(self)
    }
}

pub(crate) struct InitKeyChanges<'a> {
    target: BranchPtr,
    key_changes: HashSet<Option<Arc<str>>>,
    state: &'a TransactionState,
}

impl<'a> InitKeyChanges<'a> {
    pub fn new(
        target: BranchPtr,
        key_changes: HashSet<Option<Arc<str>>>,
        state: &'a TransactionState,
    ) -> Self {
        InitKeyChanges {
            target,
            key_changes,
            state,
        }
    }
}

impl<'a> Once for InitKeyChanges<'a> {
    type Output = HashMap<Arc<str>, EntryChange>;

    fn call(self) -> Self::Output {
        event_keys(self.state, self.target, &self.key_changes)
    }
}

/// Event generated by [Map::observe] method. Emitted during transaction commit phase.
pub struct MapEvent {
    pub(crate) current_target: BranchPtr,
    target: MapRef,
    keys: Lazy<HashMap<Arc<str>, EntryChange>, InitKeyChanges<'static>>,
}

impl MapEvent {
    pub(crate) fn new(
        branch_ref: BranchPtr,
        key_changes: HashSet<Option<Arc<str>>>,
        state: &TransactionState,
    ) -> Self {
        let current_target = branch_ref.clone();
        let state: &'static TransactionState = unsafe { std::mem::transmute(state) };
        MapEvent {
            target: MapRef::from(branch_ref),
            current_target,
            keys: Lazy::new(InitKeyChanges::new(current_target, key_changes, state)),
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
    pub fn keys(&self) -> &HashMap<Arc<str>, EntryChange> {
        &*self.keys
    }
}

#[cfg(test)]
mod test {
    use crate::test_utils::{exchange_updates, run_scenario, RngExt};
    use crate::transaction::ReadTxn;
    use crate::types::text::TextPrelim;
    use crate::types::{DeepObservable, EntryChange, Event, Out, Path, PathSegment};
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::{Encoder, EncoderV1};
    use crate::{
        any, Any, Array, ArrayPrelim, ArrayRef, Doc, GetString, In, Map, MapPrelim, MapRef,
        Observable, StateVector, Text, TextRef, Update, XmlFragment, XmlFragmentRef, XmlTextPrelim,
        XmlTextRef,
    };
    use arc_swap::ArcSwapOption;
    use fastrand::Rng;
    use serde::Deserialize;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn map_basic() {
        let mut d1 = Doc::with_client_id(1);
        let m1 = d1.get_or_insert_map("map");
        let mut t1 = d1.transact_mut();

        let mut d2 = Doc::with_client_id(2);
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
            assert_eq!(m.get(txn, &"number".to_owned()), Some(Out::from(1f64)));
            assert_eq!(m.get(txn, &"boolean0".to_owned()), Some(Out::from(false)));
            assert_eq!(m.get(txn, &"boolean1".to_owned()), Some(Out::from(true)));
            assert_eq!(m.get(txn, &"string".to_owned()), Some(Out::from("hello Y")));
            assert_eq!(
                m.get(txn, &"object".to_owned()),
                Some(Out::from(any!({
                    "key": {
                        "key2": "value"
                    }
                })))
            );
        }

        compare_all(&m1, &t1);

        let update = t1.encode_state_as_update_v1(&StateVector::default());
        t2.apply_update(Update::decode_v1(update.as_slice()).unwrap())
            .unwrap();

        compare_all(&m2, &t2);
    }

    #[test]
    fn map_get_set() {
        let mut d1 = Doc::with_client_id(1);
        let m1 = d1.get_or_insert_map("map");
        let mut t1 = d1.transact_mut();

        m1.insert(&mut t1, "stuff".to_owned(), "stuffy");
        m1.insert(&mut t1, "null".to_owned(), None as Option<String>);

        let update = t1.encode_state_as_update_v1(&StateVector::default());

        let mut d2 = Doc::with_client_id(2);
        let m2 = d2.get_or_insert_map("map");
        let mut t2 = d2.transact_mut();

        t2.apply_update(Update::decode_v1(update.as_slice()).unwrap())
            .unwrap();

        assert_eq!(m2.get(&t2, &"stuff".to_owned()), Some(Out::from("stuffy")));
        assert_eq!(m2.get(&t2, &"null".to_owned()), Some(Out::Any(Any::Null)));
    }

    #[test]
    fn map_get_set_sync_with_conflicts() {
        let mut d1 = Doc::with_client_id(1);
        let m1 = d1.get_or_insert_map("map");
        let mut t1 = d1.transact_mut();

        let mut d2 = Doc::with_client_id(2);
        let m2 = d2.get_or_insert_map("map");
        let mut t2 = d2.transact_mut();

        m1.insert(&mut t1, "stuff".to_owned(), "c0");
        m2.insert(&mut t2, "stuff".to_owned(), "c1");

        let u1 = t1.encode_state_as_update_v1(&StateVector::default());
        let u2 = t2.encode_state_as_update_v1(&StateVector::default());

        t1.apply_update(Update::decode_v1(u2.as_slice()).unwrap())
            .unwrap();
        t2.apply_update(Update::decode_v1(u1.as_slice()).unwrap())
            .unwrap();

        assert_eq!(m1.get(&t1, &"stuff".to_owned()), Some(Out::from("c1")));
        assert_eq!(m2.get(&t2, &"stuff".to_owned()), Some(Out::from("c1")));
    }

    #[test]
    fn map_len_remove() {
        let mut d1 = Doc::with_client_id(1);
        let m1 = d1.get_or_insert_map("map");
        let mut t1 = d1.transact_mut();

        let key1 = "stuff".to_owned();
        let key2 = "other-stuff".to_owned();

        m1.insert(&mut t1, key1.clone(), "c0");
        m1.insert(&mut t1, key2.clone(), "c1");
        assert_eq!(m1.len(&t1), 2);

        // remove 'stuff'
        assert_eq!(m1.remove(&mut t1, &key1), Some(Out::from("c0")));
        assert_eq!(m1.len(&t1), 1);

        // remove 'stuff' again - nothing should happen
        assert_eq!(m1.remove(&mut t1, &key1), None);
        assert_eq!(m1.len(&t1), 1);

        // remove 'other-stuff'
        assert_eq!(m1.remove(&mut t1, &key2), Some(Out::from("c1")));
        assert_eq!(m1.len(&t1), 0);
    }

    #[test]
    fn map_clear() {
        let mut d1 = Doc::with_client_id(1);
        let m1 = d1.get_or_insert_map("map");
        let mut t1 = d1.transact_mut();

        m1.insert(&mut t1, "key1".to_owned(), "c0");
        m1.insert(&mut t1, "key2".to_owned(), "c1");
        m1.clear(&mut t1);

        assert_eq!(m1.len(&t1), 0);
        assert_eq!(m1.get::<_, Out>(&t1, &"key1".to_owned()), None);
        assert_eq!(m1.get::<_, Out>(&t1, &"key2".to_owned()), None);

        let mut d2 = Doc::with_client_id(2);
        let m2 = d2.get_or_insert_map("map");
        let mut t2 = d2.transact_mut();

        let u1 = t1.encode_state_as_update_v1(&StateVector::default());
        t2.apply_update(Update::decode_v1(u1.as_slice()).unwrap())
            .unwrap();

        assert_eq!(m2.len(&t2), 0);
        assert_eq!(m2.get::<_, Out>(&t2, &"key1".to_owned()), None);
        assert_eq!(m2.get::<_, Out>(&t2, &"key2".to_owned()), None);
    }

    #[test]
    fn map_clear_sync() {
        let mut d1 = Doc::with_client_id(1);
        let mut d2 = Doc::with_client_id(2);
        let mut d3 = Doc::with_client_id(3);
        let mut d4 = Doc::with_client_id(4);

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

        exchange_updates([&mut d1, &mut d2, &mut d3, &mut d4]);

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

        exchange_updates([&mut d1, &mut d2, &mut d3, &mut d4]);

        for doc in [d1, d2, d3, d4] {
            let map: MapRef = doc.get("map").unwrap();

            assert_eq!(
                map.get::<_, Out>(&doc.transact(), &"key1".to_owned()),
                None,
                "'key1' entry for peer {} should be removed",
                doc.client_id()
            );
            assert_eq!(
                map.get::<_, Out>(&doc.transact(), &"key2".to_owned()),
                None,
                "'key2' entry for peer {} should be removed",
                doc.client_id()
            );
            assert_eq!(
                map.len(&doc.transact()),
                0,
                "all entries for peer {} should be removed",
                doc.client_id()
            );
        }
    }

    #[test]
    fn map_get_set_with_3_way_conflicts() {
        let mut d1 = Doc::with_client_id(1);
        let mut d2 = Doc::with_client_id(2);
        let mut d3 = Doc::with_client_id(3);

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

        exchange_updates([&mut d1, &mut d2, &mut d3]);

        for mut doc in [d1, d2, d3] {
            let map = doc.get_or_insert_map("map");

            assert_eq!(
                map.get(&doc.transact(), &"stuff".to_owned()),
                Some(Out::from("c3")),
                "peer {} - map entry resolved to unexpected value",
                doc.client_id()
            );
        }
    }

    #[test]
    fn map_get_set_remove_with_3_way_conflicts() {
        let mut d1 = Doc::with_client_id(1);
        let mut d2 = Doc::with_client_id(2);
        let mut d3 = Doc::with_client_id(3);
        let mut d4 = Doc::with_client_id(4);

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

        exchange_updates([&mut d1, &mut d2, &mut d3, &mut d4]);

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

        exchange_updates([&mut d1, &mut d2, &mut d3, &mut d4]);

        for doc in [d1, d2, d3, d4] {
            let map: MapRef = doc.get("map").unwrap();

            assert_eq!(
                map.get::<_, Out>(&doc.transact(), &"key1".to_owned()),
                None,
                "entry 'key1' on peer {} should be removed",
                doc.client_id()
            );
        }
    }

    #[test]
    fn insert_and_remove_events() {
        let mut d1 = Doc::with_client_id(1);
        let m1 = d1.get_or_insert_map("map");

        let entries = Arc::new(ArcSwapOption::default());
        let entries_c = entries.clone();
        let _sub = m1.observe(move |_, e| {
            let keys = e.keys();
            entries_c.store(Some(Arc::new(keys.clone())));
        });

        // insert new entry
        {
            let mut txn = d1.transact_mut();
            m1.insert(&mut txn, "a", 1);
            // txn is committed at the end of this scope
        }
        assert_eq!(
            entries.swap(None),
            Some(Arc::new(HashMap::from([(
                "a".into(),
                EntryChange::Inserted(Any::Number(1.0).into())
            )])))
        );

        // update existing entry once
        {
            let mut txn = d1.transact_mut();
            m1.insert(&mut txn, "a", 2);
        }
        assert_eq!(
            entries.swap(None),
            Some(Arc::new(HashMap::from([(
                "a".into(),
                EntryChange::Updated(Any::Number(1.0).into(), Any::Number(2.0).into())
            )])))
        );

        // update existing entry twice
        {
            let mut txn = d1.transact_mut();
            m1.insert(&mut txn, "a", 3);
            m1.insert(&mut txn, "a", 4);
        }
        assert_eq!(
            entries.swap(None),
            Some(Arc::new(HashMap::from([(
                "a".into(),
                EntryChange::Updated(Any::Number(2.0).into(), Any::Number(4.0).into())
            )])))
        );

        // remove existing entry
        {
            let mut txn = d1.transact_mut();
            m1.remove(&mut txn, "a");
        }
        assert_eq!(
            entries.swap(None),
            Some(Arc::new(HashMap::from([(
                "a".into(),
                EntryChange::Removed(Any::Number(4.0).into())
            )])))
        );

        // add another entry and update it
        {
            let mut txn = d1.transact_mut();
            m1.insert(&mut txn, "b", 1);
            m1.insert(&mut txn, "b", 2);
        }
        assert_eq!(
            entries.swap(None),
            Some(Arc::new(HashMap::from([(
                "b".into(),
                EntryChange::Inserted(Any::Number(2.0).into())
            )])))
        );

        // add and remove an entry
        {
            let mut txn = d1.transact_mut();
            m1.insert(&mut txn, "c", 1);
            m1.remove(&mut txn, "c");
        }
        assert_eq!(entries.swap(None), Some(HashMap::new().into()));

        // copy updates over
        let mut d2 = Doc::with_client_id(2);
        let m2 = d2.get_or_insert_map("map");

        let entries = Arc::new(ArcSwapOption::default());
        let entries_c = entries.clone();
        let _sub = m2.observe(move |_, e| {
            let keys = e.keys();
            entries_c.store(Some(Arc::new(keys.clone())));
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
            entries.swap(None),
            Some(Arc::new(HashMap::from([(
                "b".into(),
                EntryChange::Inserted(Any::Number(2.0).into())
            )])))
        );
    }

    fn map_transactions() -> [Box<dyn Fn(&mut Doc, &mut Rng)>; 3] {
        fn set(doc: &mut Doc, rng: &mut Rng) {
            let map = doc.get_or_insert_map("map");
            let mut txn = doc.transact_mut();
            let key = rng.choice(["one", "two"]).unwrap();
            let value: String = rng.random_string();
            map.insert(&mut txn, key.to_string(), value);
        }

        fn set_type(doc: &mut Doc, rng: &mut Rng) {
            let map = doc.get_or_insert_map("map");
            let mut txn = doc.transact_mut();
            let key = rng.choice(["one", "two", "three"]).unwrap();
            if rng.f32() <= 0.33 {
                map.insert(
                    &mut txn,
                    key.to_string(),
                    ArrayPrelim::from(vec![1, 2, 3, 4]),
                );
            } else if rng.f32() <= 0.33 {
                map.insert(&mut txn, key.to_string(), TextPrelim::new("deeptext"));
            } else {
                map.insert(
                    &mut txn,
                    key.to_string(),
                    MapPrelim::from([("deepkey".to_owned(), "deepvalue")]),
                );
            }
        }

        fn delete(doc: &mut Doc, rng: &mut Rng) {
            let map = doc.get_or_insert_map("map");
            let mut txn = doc.transact_mut();
            let key = rng.choice(["one", "two"]).unwrap();
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
        let mut doc = Doc::with_client_id(1);
        let map = doc.get_or_insert_map("map");

        let paths = Arc::new(Mutex::new(vec![]));
        let calls = Arc::new(AtomicU32::new(0));
        let paths_copy = paths.clone();
        let calls_copy = calls.clone();
        let _sub = map.observe_deep(move |_txn, e| {
            let path: Vec<Path> = e.iter().map(Event::path).collect();
            paths_copy.lock().unwrap().push(path);
            calls_copy.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });

        let nested = map.insert(&mut doc.transact_mut(), "map", MapPrelim::default());
        nested.insert(
            &mut doc.transact_mut(),
            "array",
            ArrayPrelim::from(Vec::<String>::default()),
        );
        let nested2: ArrayRef = nested.get(&doc.transact(), "array").unwrap();
        nested2.insert(&mut doc.transact_mut(), 0, "content");

        let nested_text = nested.insert(&mut doc.transact_mut(), "text", TextPrelim::new("text"));
        nested_text.push(&mut doc.transact_mut(), "!");

        assert_eq!(calls.load(Ordering::Relaxed), 5);
        let actual = paths.lock().unwrap();
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
    fn get_or_init() {
        let mut doc = Doc::with_client_id(1);
        let mut txn = doc.transact_mut();
        let map = txn.get_or_insert_map("map");

        let m: MapRef = map.get_or_init(&mut txn, "nested");
        m.insert(&mut txn, "key", 1);
        let m: MapRef = map.get_or_init(&mut txn, "nested");
        assert_eq!(m.get(&txn, "key"), Some(Out::from(1)));

        let m: ArrayRef = map.get_or_init(&mut txn, "nested");
        m.insert(&mut txn, 0, 1);
        let m: ArrayRef = map.get_or_init(&mut txn, "nested");
        assert_eq!(m.get(&txn, 0), Some(Out::from(1)));

        let m: TextRef = map.get_or_init(&mut txn, "nested");
        m.insert(&mut txn, 0, "a");
        let m: TextRef = map.get_or_init(&mut txn, "nested");
        assert_eq!(m.get_string(&txn), "a".to_string());

        let m: XmlFragmentRef = map.get_or_init(&mut txn, "nested");
        m.insert(&mut txn, 0, XmlTextPrelim::new("b"));
        let m: XmlFragmentRef = map.get_or_init(&mut txn, "nested");
        assert_eq!(m.get_string(&txn), "b".to_string());

        let m: XmlTextRef = map.get_or_init(&mut txn, "nested");
        m.insert(&mut txn, 0, "c");
        let m: XmlTextRef = map.get_or_init(&mut txn, "nested");
        assert_eq!(m.get_string(&txn), "c".to_string());
    }

    #[test]
    fn try_update() {
        let mut doc = Doc::new();
        let mut txn = doc.transact_mut();
        let map = txn.get_or_insert_map("map");

        assert!(map.try_update(&mut txn, "key", 1), "new entry");
        assert_eq!(map.get(&txn, "key"), Some(Out::from(1)));

        assert!(
            !map.try_update(&mut txn, "key", 1),
            "unchanged entry shouldn't trigger update"
        );
        assert_eq!(map.get(&txn, "key"), Some(Out::from(1)));

        assert!(map.try_update(&mut txn, "key", 2), "entry should change");
        assert_eq!(map.get(&txn, "key"), Some(Out::from(2)));

        map.remove(&mut txn, "key");
        assert!(
            map.try_update(&mut txn, "key", 2),
            "removed entry should trigger update"
        );
        assert_eq!(map.get(&txn, "key"), Some(Out::from(2)));
    }

    #[test]
    fn get_as() {
        #[derive(Debug, PartialEq, Deserialize)]
        struct Order {
            shipment_address: String,
            items: HashMap<String, OrderItem>,
            #[serde(default)]
            comment: Option<String>,
        }

        #[derive(Debug, PartialEq, Deserialize)]
        struct OrderItem {
            name: String,
            price: f64,
            quantity: u32,
        }

        let mut doc = Doc::new();
        let mut txn = doc.transact_mut();
        let map = txn.get_or_insert_map("map");

        map.insert(
            &mut txn,
            "orders",
            ArrayPrelim::from([In::from(MapPrelim::from([
                ("shipment_address", In::from("123 Main St")),
                (
                    "items",
                    In::from(MapPrelim::from([
                        (
                            "item1",
                            In::from(MapPrelim::from([
                                ("name", In::from("item1")),
                                ("price", In::from(1.99)),
                                ("quantity", In::from(2)),
                            ])),
                        ),
                        (
                            "item2",
                            In::from(MapPrelim::from([
                                ("name", In::from("item2")),
                                ("price", In::from(2.99)),
                                ("quantity", In::from(1)),
                            ])),
                        ),
                    ])),
                ),
            ]))]),
        );

        let expected = Order {
            comment: None,
            shipment_address: "123 Main St".to_string(),
            items: HashMap::from([
                (
                    "item1".to_string(),
                    OrderItem {
                        name: "item1".to_string(),
                        price: 1.99,
                        quantity: 2,
                    },
                ),
                (
                    "item2".to_string(),
                    OrderItem {
                        name: "item2".to_string(),
                        price: 2.99,
                        quantity: 1,
                    },
                ),
            ]),
        };

        let actual: Vec<Order> = map.get_as(&txn, "orders").unwrap();
        assert_eq!(actual, vec![expected]);
    }

    #[cfg(feature = "sync")]
    #[test]
    fn multi_threading() {
        use crate::types::ToJson;
        use std::sync::{Arc, RwLock};
        use std::thread::{sleep, spawn};
        use std::time::Duration;

        let doc = Arc::new(RwLock::new(Doc::with_client_id(1)));

        let d2 = doc.clone();
        let h2 = spawn(move || {
            for _ in 0..10 {
                let millis = fastrand::u64(1..20);
                sleep(Duration::from_millis(millis));

                let mut doc = d2.write().unwrap();
                let map = doc.get_or_insert_map("test");
                let mut txn = doc.transact_mut();
                map.insert(&mut txn, "key", 1);
            }
        });

        let d3 = doc.clone();
        let h3 = spawn(move || {
            for _ in 0..10 {
                let millis = fastrand::u64(1..20);
                sleep(Duration::from_millis(millis));

                let mut doc = d3.write().unwrap();
                let map = doc.get_or_insert_map("test");
                let mut txn = doc.transact_mut();
                map.insert(&mut txn, "key", 2);
            }
        });

        h3.join().unwrap();
        h2.join().unwrap();

        let doc = doc.read().unwrap();
        let map: MapRef = doc.get("test").unwrap();
        let txn = doc.transact();
        let value = map.get(&txn, "key").unwrap().to_json(&txn);

        assert!(value == 1.into() || value == 2.into())
    }
}
