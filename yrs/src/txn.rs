use crate::block::{BlockPtr, ClientID, ItemContent};
use crate::block_store::BlockStore;
use crate::event::EventHandler;
use crate::txn::array::ArrayRef;
use crate::txn::map::MapRef;
use crate::txn::text::TextRef;
use crate::txn::xml::{XmlElementRef, XmlTextRef};
use crate::types::{
    Branch, BranchPtr, TypePtr, TypeRefs, TYPE_REFS_ARRAY, TYPE_REFS_MAP, TYPE_REFS_TEXT,
    TYPE_REFS_XML_ELEMENT, TYPE_REFS_XML_TEXT,
};
use crate::update::PendingUpdate;
use crate::{
    AfterTransactionEvent, DeleteSet, Options, StateVector, Subscription, SubscriptionId, Update,
    UpdateEvent, ID,
};
use lib0::any::Any;
use std::cell::{BorrowError, BorrowMutError, Ref, RefCell, RefMut};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

trait ReadTxn {
    fn store(&self) -> &Store;

    fn timestamp(&self) -> StateVector {
        self.store().blocks.get_state_vector()
    }
}

trait WriteTxn {
    fn store_mut(&mut self) -> &mut Store;
}

#[derive(Debug, Clone)]
pub struct Transaction<'doc> {
    store: Ref<'doc, Store>,
}

impl<'doc> Transaction<'doc> {
    fn new(store: Ref<'doc, Store>) -> Self {
        Transaction { store }
    }
}

impl<'doc> ReadTxn for Transaction<'doc> {
    #[inline]
    fn store(&self) -> &Store {
        self.store.deref()
    }
}

pub struct TransactionMut<'doc> {
    store: RefMut<'doc, Store>,
    /// State vector of a current transaction at the moment of its creation.
    before_timestamp: StateVector,
    /// ID's of the blocks to be merged.
    merge_blocks: Vec<ID>,
    /// Describes the set of deleted items by ids.
    delete_set: DeleteSet,
    /// We store the reference that last moved an item. This is needed to compute the delta
    /// when multiple ContentMove move the same item.
    prev_moved: HashMap<BlockPtr, BlockPtr>,
    /// All types that were directly modified (property added or child inserted/deleted).
    /// New types are not included in this Set.
    changed: HashMap<TypePtr, HashSet<Option<Rc<str>>>>,
}

impl<'doc> TransactionMut<'doc> {
    fn new(store: RefMut<'doc, Store>) -> Self {
        let before_timestamp = store.blocks.get_state_vector();
        TransactionMut {
            store,
            before_timestamp,
            merge_blocks: Vec::default(),
            delete_set: DeleteSet::default(),
            prev_moved: HashMap::default(),
            changed: HashMap::default(),
        }
    }

    pub fn apply(&mut self, update: Update) {
        todo!()
    }

    pub fn get_text<S: Into<Rc<str>>>(&mut self, name: S) -> TextRef<'doc> {
        let mut c = self
            .store_mut()
            .get_or_create_type(name, None, TYPE_REFS_TEXT);
        TextRef::from(c)
    }

    pub fn get_array<S: Into<String>>(&mut self, name: S) -> ArrayRef<'doc> {
        let mut c = self
            .store_mut()
            .get_or_create_type(name, None, TYPE_REFS_ARRAY);
        ArrayRef::from(c)
    }

    pub fn get_map<S: Into<String>>(&mut self, name: S) -> MapRef<'doc> {
        let mut c = self
            .store_mut()
            .get_or_create_type(name, None, TYPE_REFS_MAP);
        MapRef::from(c)
    }

    pub fn get_xml_text<S: Into<String>>(&mut self, name: S) -> XmlTextRef<'doc> {
        let mut c = self
            .store_mut()
            .get_or_create_type(name, None, TYPE_REFS_XML_TEXT);
        XmlTextRef::from(c)
    }

    pub fn get_xml_element<S: Into<String>>(&mut self, name: S) -> XmlElementRef<'doc> {
        let mut c = self
            .store_mut()
            .get_or_create_type(name, None, TYPE_REFS_XML_ELEMENT);
        XmlElementRef::from(c)
    }
}

impl<'doc> ReadTxn for TransactionMut<'doc> {
    #[inline]
    fn store(&self) -> &Store {
        self.store.deref()
    }
}

impl<'doc> WriteTxn for TransactionMut<'doc> {
    #[inline]
    fn store_mut(&mut self) -> &mut Store {
        self.store.deref_mut()
    }
}

impl<'doc> Drop for TransactionMut<'doc> {
    fn drop(&mut self) {
        todo!()
    }
}

pub struct Doc {
    client_id: ClientID,
    store: Rc<RefCell<Store>>,
}

impl Doc {
    pub fn new(client_id: ClientID) -> Self {
        let options = Options::with_client_id(client_id);
        let store = Rc::new(RefCell::new(Store::new(options)));
        Doc { client_id, store }
    }

    pub fn transact(&self) -> Transaction {
        self.try_transact().unwrap()
    }

    pub fn transact_mut(&self) -> TransactionMut {
        self.try_transact_mut().unwrap()
    }

    pub fn try_transact(&self) -> Result<Transaction, BorrowError> {
        let store = self.store.try_borrow()?;
        Ok(Transaction::new(store))
    }

    pub fn try_transact_mut(&self) -> Result<TransactionMut, BorrowMutError> {
        let store = self.store.try_borrow_mut()?;
        Ok(TransactionMut::new(store))
    }
}

pub struct Store {
    options: Options,

    /// Root types (a.k.a. top-level types). These types are defined by users at the document level,
    /// they have their own unique names and represent core shared types that expose operations
    /// which can be called concurrently by remote peers in a conflict-free manner.
    types: HashMap<Rc<str>, Box<Branch>>,

    /// A block store of a current document. It represent all blocks (inserted or tombstoned
    /// operations) integrated - and therefore visible - into a current document.
    blocks: BlockStore,

    /// A pending update. It contains blocks, which are not yet integrated into `blocks`, usually
    /// because due to issues in update exchange, there were some missing blocks that need to be
    /// integrated first before the data from `pending` can be applied safely.
    pending: Option<PendingUpdate>,

    /// A pending delete set. Just like `pending`, it contains deleted ranges of blocks that have
    /// not been yet applied due to missing blocks that prevent `pending` update to be integrated
    /// into `blocks`.
    pending_ds: Option<DeleteSet>,

    /// Handles subscriptions for the `afterTransactionCleanup` event. Events are called with the
    /// newest updates once they are committed and compacted.
    after_transaction_events: Option<EventHandler<AfterTransactionEvent>>,

    /// A subscription handler. It contains all callbacks with registered by user functions that
    /// are supposed to be called, once a new update arrives.
    update_v1_events: Option<EventHandler<UpdateEvent>>,

    /// A subscription handler. It contains all callbacks with registered by user functions that
    /// are supposed to be called, once a new update arrives.
    update_v2_events: Option<EventHandler<UpdateEvent>>,
}

impl Store {
    fn new(options: Options) -> Self {
        Store {
            options,
            types: HashMap::new(),
            blocks: BlockStore::new(),
            pending: None,
            pending_ds: None,
            after_transaction_events: None,
            update_v1_events: None,
            update_v2_events: None,
        }
    }

    pub fn client_id(&self) -> ClientID {
        self.options.client_id
    }

    /// Returns a branch reference to a complex type identified by its pointer. Returns `None` if
    /// no such type could be found or was ever defined.
    pub fn get_or_create_type<K: Into<Rc<str>>>(
        &mut self,
        key: K,
        node_name: Option<Rc<str>>,
        type_ref: TypeRefs,
    ) -> &Branch {
        let key = key.into();
        match self.types.entry(key.clone()) {
            Entry::Occupied(mut e) => {
                let branch = e.get_mut();
                branch.repair_type_ref(type_ref);
                branch
            }
            Entry::Vacant(e) => {
                let mut branch = Branch::new(type_ref, node_name);
                e.insert(branch)
            }
        }
    }
}

pub trait Observable {
    type Event;

    fn observe<F>(&mut self, f: F) -> Subscription<Self::Event>
    where
        F: Fn(&TransactionMut, &Self::Event) -> () + 'static;

    fn unobserve(&mut self, subscription_id: SubscriptionId);
}

pub trait PrelimOwned: for<'mat> Prelim<'mat> {}
impl<T> PrelimOwned for T where T: for<'mat> Prelim<'mat> {}

pub trait Prelim<'mat>: Sized {
    type Mat;

    fn into_content<'doc, T: WriteTxn>(self, txn: &mut T) -> (ItemContent, Option<Self>);

    fn integrate<'doc, T: WriteTxn>(self, txn: &mut T, inner_ref: BranchPtr) {}
}

pub trait ToAny {
    fn to_any<'doc, T: ReadTxn>(&self, txn: &T) -> Any;
}

mod text {
    use crate::block::ItemContent;
    use crate::txn::{Observable, Prelim, ReadTxn, ToAny, TransactionMut, WriteTxn};
    use crate::types::text::TextEvent;
    use crate::types::{Attrs, Branch};
    use crate::{Subscription, SubscriptionId};
    use lib0::any::Any;

    #[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
    pub struct TextPrelim<S: Into<String>>(pub S);
    impl<'doc, S: Into<String>> Prelim<'doc> for TextPrelim<S> {
        type Mat = TextRef<'doc>;

        fn into_content<T: WriteTxn>(self, txn: &mut T) -> (ItemContent, Option<Self>) {
            todo!()
        }
    }

    #[repr(transparent)]
    #[derive(Debug, Clone, Copy)]
    pub struct TextRef<'doc>(&'doc Branch);

    impl<'doc> Text for TextRef<'doc> {}

    impl<'doc> From<&'doc Branch> for TextRef<'doc> {
        fn from(branch: &'doc Branch) -> Self {
            Self(branch)
        }
    }

    impl<'doc> AsRef<Branch> for TextRef<'doc> {
        #[inline]
        fn as_ref(&self) -> &Branch {
            self.0
        }
    }

    impl<'doc> ToAny for TextRef<'doc> {
        fn to_any<T: ReadTxn>(&self, txn: &T) -> Any {
            let str = self.get_string(txn);
            Any::String(str.into_boxed_str())
        }
    }

    impl<'doc> Observable for TextRef<'doc> {
        type Event = TextEvent;

        fn observe<F>(&mut self, f: F) -> Subscription<Self::Event>
        where
            F: Fn(&TransactionMut, &Self::Event) -> () + 'static,
        {
            todo!()
        }

        fn unobserve(&mut self, subscription_id: SubscriptionId) {
            todo!()
        }
    }

    pub trait Text: AsRef<Branch> {
        fn get_string<'doc, T: ReadTxn>(&self, txn: &T) -> String {
            todo!()
        }

        fn len<'doc, T: ReadTxn>(&self, txn: &T) -> usize {
            todo!()
        }

        fn insert<'doc, T: WriteTxn, S: Into<String>>(&self, txn: &mut T, index: usize, chunk: S) {
            todo!()
        }

        fn push<'doc, T: WriteTxn, S: Into<String>>(&self, txn: &mut T, index: usize, chunk: S) {
            todo!()
        }

        fn format<'doc, T: WriteTxn>(&self, txn: &mut T, index: usize, len: usize, attrs: Attrs) {
            todo!()
        }

        fn insert_with_format<'doc, T: WriteTxn, S: Into<String>>(
            &self,
            txn: &mut T,
            index: usize,
            chunk: S,
            attrs: Attrs,
        ) {
            todo!()
        }

        fn insert_embed<'doc, T: WriteTxn, U: Into<Any>>(
            &self,
            txn: &mut T,
            index: usize,
            embed: U,
        ) {
            todo!()
        }

        fn insert_embed_with_format<'doc, T: WriteTxn, U: Into<Any>>(
            &self,
            txn: &mut T,
            index: usize,
            embed: U,
            attrs: Attrs,
        ) {
            todo!()
        }

        fn remove_range<'doc, T: WriteTxn>(&self, txn: &mut T, index: usize, len: usize) {
            todo!()
        }
    }
}

mod array {
    use crate::block::ItemContent;
    use crate::txn::{Observable, Prelim, PrelimOwned, ReadTxn, ToAny, TransactionMut, WriteTxn};
    use crate::types::array::ArrayEvent;
    use crate::types::{Branch, Value};
    use crate::{Subscription, SubscriptionId};
    use lib0::any::Any;

    #[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
    pub struct ArrayPrelim<I, U>(pub I)
    where
        I: IntoIterator<Item = U>,
        U: PrelimOwned;
    impl<'doc, I, U> Prelim<'doc> for ArrayPrelim<I, U>
    where
        I: IntoIterator<Item = U>,
        U: PrelimOwned,
    {
        type Mat = ArrayRef<'doc>;

        fn into_content<T: WriteTxn>(self, txn: &mut T) -> (ItemContent, Option<Self>) {
            todo!()
        }
    }

    #[repr(transparent)]
    #[derive(Debug, Clone, Copy)]
    pub struct ArrayRef<'doc>(&'doc Branch);

    impl<'doc> Array for ArrayRef<'doc> {}

    impl<'doc> From<&'doc Branch> for ArrayRef<'doc> {
        fn from(branch: &'doc Branch) -> Self {
            Self(branch)
        }
    }

    impl<'doc> AsRef<Branch> for ArrayRef<'doc> {
        #[inline]
        fn as_ref(&self) -> &Branch {
            self.0
        }
    }

    impl<'doc> ToAny for ArrayRef<'doc> {
        fn to_any<T: ReadTxn>(&self, txn: &T) -> Any {
            todo!()
        }
    }

    impl<'doc> Observable for ArrayRef<'doc> {
        type Event = ArrayEvent;

        fn observe<F>(&mut self, f: F) -> Subscription<Self::Event>
        where
            F: Fn(&TransactionMut, &Self::Event) -> () + 'static,
        {
            todo!()
        }

        fn unobserve(&mut self, subscription_id: SubscriptionId) {
            todo!()
        }
    }

    pub trait Array: AsRef<Branch> {
        fn len<'doc, T: ReadTxn>(&self, txn: &T) -> usize {
            todo!()
        }

        fn iter<'doc, T: ReadTxn>(&self, txn: &T) -> Iter<'doc> {
            todo!()
        }

        fn get<'doc, T: ReadTxn>(&self, txn: &T, index: usize) -> Option<Value> {
            todo!()
        }

        fn insert<'doc, T: WriteTxn, U: Prelim<'doc>>(
            &self,
            txn: &mut T,
            index: usize,
            value: U,
        ) -> &mut U::Mat {
            todo!()
        }

        fn push_front<'doc, T: WriteTxn, U: Prelim<'doc>>(
            &self,
            txn: &mut T,
            value: U,
        ) -> &mut U::Mat {
            todo!()
        }

        fn push_back<'doc, T: WriteTxn, U: Prelim<'doc>>(
            &self,
            txn: &mut T,
            value: U,
        ) -> &mut U::Mat {
            todo!()
        }

        fn insert_range<'doc, T, I, V>(&self, txn: &mut T, index: u32, values: I)
        where
            T: WriteTxn,
            I: IntoIterator<Item = V>,
            V: Into<Any>,
        {
            todo!()
        }

        fn remove<'doc, T: WriteTxn>(&self, txn: &mut T, index: usize) {
            self.remove_range(txn, index, 1)
        }

        fn remove_range<'doc, T: WriteTxn>(&self, txn: &mut T, index: usize, len: usize) {
            todo!()
        }

        fn move_to<'doc, T: WriteTxn>(&self, txn: &mut T, source: usize, target: usize) {
            todo!()
        }
    }

    pub struct Iter<'a, 'doc> {}

    impl<'a, 'doc> Iterator for Iter<'a, 'doc> {
        type Item = &'doc Value;

        fn next(&mut self) -> Option<Self::Item> {
            todo!()
        }
    }
}

mod map {
    use crate::block::ItemContent;
    use crate::txn::{Observable, Prelim, PrelimOwned, ReadTxn, ToAny, TransactionMut, WriteTxn};
    use crate::types::map::MapEvent;
    use crate::types::{Branch, Value};
    use crate::{Subscription, SubscriptionId};
    use lib0::any::Any;
    use std::rc::Rc;

    #[derive(Debug, Clone, Eq, PartialEq)]
    pub struct MapPrelim<I, K, V>(pub I)
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<Rc<str>>,
        V: PrelimOwned;
    impl<'doc, I, K, V> Prelim<'doc> for MapPrelim<I, K, V>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<Rc<str>>,
        V: PrelimOwned,
    {
        type Mat = MapRef<'doc>;

        fn into_content<T: WriteTxn>(self, txn: &mut T) -> (ItemContent, Option<Self>) {
            todo!()
        }
    }

    #[repr(transparent)]
    #[derive(Debug, Clone, Copy)]
    pub struct MapRef<'doc>(&'doc Branch);

    impl<'doc> Map for MapRef<'doc> {}

    impl<'doc> From<&'doc Branch> for MapRef<'doc> {
        fn from(branch: &'doc Branch) -> Self {
            Self(branch)
        }
    }

    impl<'doc> AsRef<Branch> for MapRef<'doc> {
        #[inline]
        fn as_ref(&self) -> &Branch {
            self.0
        }
    }

    impl<'doc> ToAny for MapRef<'doc> {
        fn to_any<T: ReadTxn>(&self, txn: &T) -> Any {
            todo!()
        }
    }

    impl<'doc> Observable for MapRef<'doc> {
        type Event = MapEvent;

        fn observe<F>(&mut self, f: F) -> Subscription<Self::Event>
        where
            F: Fn(&TransactionMut, &Self::Event) -> () + 'static,
        {
            todo!()
        }

        fn unobserve(&mut self, subscription_id: SubscriptionId) {
            todo!()
        }
    }

    pub trait Map: AsRef<Branch> {
        fn len<'doc, T: ReadTxn>(&self, txn: &T) -> usize {
            todo!()
        }

        fn get<'doc, T: ReadTxn, S: AsRef<str>>(&self, txn: &T, key: S) -> Option<Value> {
            todo!()
        }

        fn contains_key<'doc, T: ReadTxn, S: AsRef<str>>(&self, txn: &T, key: S) -> bool {
            todo!()
        }

        fn remove<'doc, T: WriteTxn, S: AsRef<str>>(&self, txn: &T, key: S) -> Option<Value> {
            todo!()
        }

        fn clear<'doc, T: WriteTxn>(&self, txn: &T) {
            todo!()
        }

        fn insert<'doc, T: WriteTxn, K, V>(&self, txn: &T, key: K, value: V) -> Option<Value>
        where
            K: Into<Rc<str>>,
            V: PrelimOwned,
        {
            todo!()
        }

        fn iter<'doc, T: ReadTxn>(&self, txn: &T) -> Iter<'doc> {
            todo!()
        }

        fn keys<'doc, T: ReadTxn>(&self, txn: &T) -> Keys<'doc> {
            todo!()
        }

        fn values<'doc, T: ReadTxn>(&self, txn: &T) -> Values<'doc> {
            todo!()
        }
    }

    pub struct Iter<'doc> {}

    impl<'doc> Iterator for Iter<'doc> {
        type Item = (&'doc str, Value);

        fn next(&mut self) -> Option<Self::Item> {
            todo!()
        }
    }

    pub struct Keys<'doc> {}

    impl<'doc> Iterator for Keys<'doc> {
        type Item = &'doc str;

        fn next(&mut self) -> Option<Self::Item> {
            todo!()
        }
    }

    pub struct Values<'doc> {}

    impl<'doc> Iterator for Values<'doc> {
        type Item = Value;

        fn next(&mut self) -> Option<Self::Item> {
            todo!()
        }
    }
}

mod xml {
    use crate::block::ItemContent;
    use crate::txn::array::Array;
    use crate::txn::text::Text;
    use crate::txn::{Observable, Prelim, ReadTxn, ToAny, TransactionMut, WriteTxn};
    use crate::types::xml::{XmlEvent, XmlTextEvent};
    use crate::types::Branch;
    use crate::{Subscription, SubscriptionId};
    use lib0::any::Any;
    use std::rc::Rc;

    pub trait XmlPrelim<'doc>: Prelim<'doc> {}

    #[derive(Debug, Clone)]
    pub struct XmlTextPrelim<S>(pub S);
    impl<'doc, S> XmlPrelim<'doc> for XmlTextPrelim<S> where S: Into<String> {}
    impl<'doc, S> Prelim<'doc> for XmlTextPrelim<S>
    where
        S: Into<String>,
    {
        type Mat = XmlTextRef<'doc>;

        fn into_content<T: WriteTxn>(self, txn: &mut T) -> (ItemContent, Option<Self>) {
            todo!()
        }
    }

    #[repr(transparent)]
    #[derive(Debug, Clone, Copy)]
    pub struct XmlTextRef<'doc>(&'doc Branch);

    impl<'doc> Text for XmlTextRef<'doc> {}
    impl<'doc> Xml for XmlTextRef<'doc> {}

    impl<'doc> From<&'doc Branch> for XmlTextRef<'doc> {
        fn from(branch: &'doc Branch) -> Self {
            Self(branch)
        }
    }

    impl<'doc> AsRef<Branch> for XmlTextRef<'doc> {
        #[inline]
        fn as_ref(&self) -> &Branch {
            self.0
        }
    }

    impl<'doc> ToAny for XmlTextRef<'doc> {
        fn to_any<T: ReadTxn>(&self, txn: &T) -> Any {
            let str = self.get_string(txn);
            Any::String(str.into_boxed_str())
        }
    }

    impl<'doc> Observable for XmlTextRef<'doc> {
        type Event = XmlTextEvent;

        fn observe<F>(&mut self, f: F) -> Subscription<Self::Event>
        where
            F: Fn(&TransactionMut, &Self::Event) -> () + 'static,
        {
            todo!()
        }

        fn unobserve(&mut self, subscription_id: SubscriptionId) {
            todo!()
        }
    }

    #[derive(Debug, Clone)]
    pub struct XmlElementPrelim<S>(pub S)
    where
        S: Into<Rc<str>>;
    impl<'doc, S> Prelim<'doc> for XmlElementPrelim<S>
    where
        S: Into<Rc<str>>,
    {
        type Mat = XmlElementRef<'doc>;

        fn into_content<T: WriteTxn>(self, txn: &mut T) -> (ItemContent, Option<Self>) {
            todo!()
        }
    }
    impl<'doc, S> XmlPrelim<'doc> for XmlElementPrelim<S> where S: Into<Rc<str>> {}

    #[repr(transparent)]
    #[derive(Debug, Clone, Copy)]
    pub struct XmlElementRef<'doc>(&'doc Branch);

    impl<'doc> Xml for XmlElementRef<'doc> {}
    impl<'doc> Array for XmlElementRef<'doc> {}

    impl<'doc> From<&'doc Branch> for XmlElementRef<'doc> {
        fn from(branch: &'doc Branch) -> Self {
            Self(branch)
        }
    }

    impl<'doc> XmlElementRef<'doc> {
        pub fn tag(&self) -> &str {
            todo!()
        }
    }

    impl<'doc> AsRef<Branch> for XmlElementRef<'doc> {
        #[inline]
        fn as_ref(&self) -> &Branch {
            self.0
        }
    }

    impl<'doc> ToAny for XmlElementRef<'doc> {
        fn to_any<T: ReadTxn>(&self, txn: &T) -> Any {
            todo!()
        }
    }

    impl<'doc> Observable for XmlElementRef<'doc> {
        type Event = XmlEvent;

        fn observe<F>(&mut self, f: F) -> Subscription<Self::Event>
        where
            F: Fn(&TransactionMut, &Self::Event) -> () + 'static,
        {
            todo!()
        }

        fn unobserve(&mut self, subscription_id: SubscriptionId) {
            todo!()
        }
    }

    pub trait Xml: AsRef<Branch> {
        fn get_attr<'doc, T: ReadTxn, K: AsRef<str>>(&self, txn: &T, name: K) {
            todo!()
        }

        fn insert_attr<'doc, T: WriteTxn, K: Into<Rc<str>>, V: AsRef<str>>(
            &self,
            txn: &mut T,
            attr_name: K,
            attr_value: V,
        ) {
            todo!()
        }

        fn remove_attr<'doc, T: WriteTxn, K: AsRef<str>>(&self, txn: &mut T, attr_name: &K) {
            todo!()
        }

        fn attrs<'doc, T: ReadTxn>(&self, txn: &T) -> Attributes<'doc> {
            todo!()
        }

        fn parent<'doc, T: ReadTxn>(&self, txn: &T) -> Option<XmlElementRef<'doc>> {
            todo!()
        }

        fn siblings<'doc, T: ReadTxn>(&self, txn: &T) -> Siblings<'doc> {
            todo!()
        }
    }

    pub struct Siblings<'doc> {}

    impl<'doc> Iterator for Siblings<'doc> {
        type Item = XmlRef<'doc>;

        fn next(&mut self) -> Option<Self::Item> {
            todo!()
        }
    }

    impl<'doc> DoubleEndedIterator for Siblings<'doc> {
        fn next_back(&mut self) -> Option<Self::Item> {
            todo!()
        }
    }

    pub struct TreeWalker<'doc> {}

    impl<'doc> Iterator for TreeWalker<'doc> {
        type Item = XmlRef<'doc>;

        fn next(&mut self) -> Option<Self::Item> {
            todo!()
        }
    }

    pub struct Attributes<'doc> {}

    impl<'doc> Iterator for Attributes<'doc> {
        type Item = (&'doc str, &'doc str);

        fn next(&mut self) -> Option<Self::Item> {
            todo!()
        }
    }

    pub enum XmlRef<'doc> {
        Text(XmlTextRef<'doc>),
        Element(XmlElementRef<'doc>),
    }

    impl<'doc> From<XmlTextRef<'doc>> for XmlRef<'doc> {
        fn from(xml: XmlTextRef<'doc>) -> Self {
            XmlRef::Text(xml)
        }
    }

    impl<'doc> From<XmlElementRef<'doc>> for XmlRef<'doc> {
        fn from(xml: XmlElementRef<'doc>) -> Self {
            XmlRef::Element(xml)
        }
    }
}
