use crate::block::{ClientID, Item, ItemContent, ItemPtr, Prelim};
use crate::branch::BranchPtr;
use crate::cell::{Cell, CellMut, CellRef};
use crate::encoding::read::Error;
use crate::out::FromOut;
use crate::store::DocEvents;
use crate::transaction::{Origin, Subdocs};
use crate::types::{RootRef, ToJson};
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::{
    uuid_v4, uuid_v4_from, ArrayRef, MapRef, Out, StateVector, Store, TextRef, Transaction,
    TransactionMut, Uuid, XmlFragmentRef, ID,
};
use crate::{Any, SharedRef};
use std::collections::HashMap;
use std::fmt::Formatter;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::Arc;

macro_rules! define_observe {
    //TODO: reduce number of params once concat_idents! are stable
    ($event:ident, $observe:ident, $observe_with:ident, $unobserve:ident, $t:ty) => {
        #[cfg(not(feature = "sync"))]
        pub fn $observe<F>(&mut self, callback: F) -> crate::Subscription
        where
            F: Fn($t) + 'static,
        {
            self.events().$event.subscribe(Box::new(callback))
        }

        #[cfg(not(feature = "sync"))]
        pub fn $observe_with<K, F>(&mut self, key: K, callback: F)
        where
            K: Into<Origin>,
            F: Fn($t) + 'static,
        {
            self.events()
                .$event
                .subscribe_with(key.into(), Box::new(callback))
        }

        #[cfg(feature = "sync")]
        pub fn $observe<F>(&mut self, callback: F) -> crate::Subscription
        where
            F: Fn($t) + Send + Sync + 'static,
        {
            self.events().$event.subscribe(Box::new(callback))
        }

        #[cfg(feature = "sync")]
        pub fn $observe_with<K, F>(&mut self, key: K, callback: F)
        where
            K: Into<Origin>,
            F: Fn($t) + Send + Sync + 'static,
        {
            self.events()
                .$event
                .subscribe_with(key.into(), Box::new(callback))
        }

        pub fn $unobserve<K>(&mut self, key: K) -> bool
        where
            K: Into<Origin>,
        {
            let events = self.events();
            events.$event.unsubscribe(&key.into())
        }
    };
}

macro_rules! define_observe_tx {
    //TODO: reduce number of params once concat_idents! are stable
    ($event:ident, $observe:ident, $observe_with:ident, $unobserve:ident, $t:ty) => {
        #[cfg(not(feature = "sync"))]
        pub fn $observe<F>(&mut self, callback: F) -> crate::Subscription
        where
            F: Fn(&Transaction, &$t) + 'static,
        {
            self.events().$event.subscribe(Box::new(callback))
        }

        #[cfg(not(feature = "sync"))]
        pub fn $observe_with<K, F>(&mut self, key: K, callback: F)
        where
            K: Into<Origin>,
            F: Fn(&Transaction, &$t) + 'static,
        {
            self.events()
                .$event
                .subscribe_with(key.into(), Box::new(callback))
        }

        #[cfg(feature = "sync")]
        pub fn $observe<F>(&mut self, callback: F) -> crate::Subscription
        where
            F: Fn(&Transaction, &$t) + Send + Sync + 'static,
        {
            self.events().$event.subscribe(Box::new(callback))
        }

        #[cfg(feature = "sync")]
        pub fn $observe_with<K, F>(&mut self, key: K, callback: F)
        where
            K: Into<Origin>,
            F: Fn(&Transaction, &$t) + Send + Sync + 'static,
        {
            self.events()
                .$event
                .subscribe_with(key.into(), Box::new(callback))
        }

        pub fn $unobserve<K>(&mut self, key: K) -> bool
        where
            K: Into<Origin>,
        {
            let events = self.events();
            events.$event.unsubscribe(&key.into())
        }
    };
    ($event:ident, $observe:ident, $observe_with:ident, $unobserve:ident) => {
        #[cfg(not(feature = "sync"))]
        pub fn $observe<F>(&mut self, callback: F) -> crate::Subscription
        where
            F: Fn(&Transaction) + 'static,
        {
            self.events().$event.subscribe(Box::new(callback))
        }

        #[cfg(not(feature = "sync"))]
        pub fn $observe_with<K, F>(&mut self, key: K, callback: F)
        where
            K: Into<Origin>,
            F: Fn(&Transaction) + 'static,
        {
            self.events()
                .$event
                .subscribe_with(key.into(), Box::new(callback))
        }

        #[cfg(feature = "sync")]
        pub fn $observe<F>(&mut self, callback: F) -> crate::Subscription
        where
            F: Fn(&Transaction) + Send + Sync + 'static,
        {
            self.events().$event.subscribe(Box::new(callback))
        }

        #[cfg(feature = "sync")]
        pub fn $observe_with<K, F>(&mut self, key: K, callback: F)
        where
            K: Into<Origin>,
            F: Fn(&Transaction) + Send + Sync + 'static,
        {
            self.events()
                .$event
                .subscribe_with(key.into(), Box::new(callback))
        }

        pub fn $unobserve<K>(&mut self, key: K) -> bool
        where
            K: Into<Origin>,
        {
            let events = self.events();
            events.$event.unsubscribe(&key.into())
        }
    };
}

/// A Yrs document type. Documents are the most important units of collaborative resources management.
/// All shared collections live within a scope of their corresponding documents. All updates are
/// generated on per-document basis (rather than individual shared type). All operations on shared
/// collections happen via [Transaction](crate::Transaction), which lifetime is also bound to a document.
///
/// Document manages so-called root types, which are top-level shared types definitions (as opposed
/// to recursively nested types).
///
/// # Example
///
/// ```rust
/// use yrs::{Doc, StateVector, Text,  Update};
/// use yrs::updates::decoder::Decode;
/// use yrs::updates::encoder::Encode;
///
/// let mut doc = Doc::new();
/// let root = doc.get_or_insert_text("root-type-name");
/// let mut txn = doc.transact_mut(); // all Yrs operations happen in scope of a transaction
/// root.push(&mut txn, "hello world"); // append text to our collaborative document
///
/// // in order to exchange data with other documents we first need to create a state vector
/// let mut remote_doc = Doc::new();
/// let mut remote_txn = remote_doc.transact_mut();
/// let state_vector = remote_txn.state_vector().encode_v1();
///
/// // now compute a differential update based on remote document's state vector
/// let update = txn.encode_diff_v1(&StateVector::decode_v1(&state_vector).unwrap());
///
/// // both update and state vector are serializable, we can pass the over the wire
/// // now apply update to a remote document
/// remote_txn.apply_update(Update::decode_v1(update.as_slice()).unwrap()).unwrap();
/// ```
#[repr(transparent)]
#[derive(Debug)]
pub struct Doc {
    pub(crate) store: Pin<Box<Store>>,
}

impl Doc {
    /// Creates a new document with a randomized client identifier.
    pub fn new() -> Self {
        Self::with_options(Options::default())
    }

    /// Creates and returns a read-write capable transaction with an `origin` classifier attached.
    /// This transaction can be used to mutate the contents of underlying document store and upon
    /// dropping or committing it may subscription callbacks.
    ///
    /// An `origin` may be used to identify context of operations made (example updates performed
    /// locally vs. incoming from remote replicas) and it's used i.e. by [`UndoManager`][crate::undo::UndoManager].
    ///
    /// # Errors
    ///
    /// Only one read-write transaction can be active at the same time. If any other transaction -
    /// be it a read-write or read-only one - is active at the same time, this method will panic.
    pub fn transact_mut_with<T>(&mut self, origin: T) -> TransactionMut
    where
        T: Into<Origin>,
    {
        TransactionMut::new(self, Some(origin.into()))
    }

    /// Creates and returns a read-write capable transaction. This transaction can be used to
    /// mutate the contents of underlying document store and upon dropping or committing it may
    /// subscription callbacks.
    ///
    /// # Panics
    ///
    /// Only one read-write transaction can be active at the same time. If any other transaction -
    /// be it a read-write or read-only one - is active at the same time, this method will panic.
    pub fn transact_mut(&mut self) -> TransactionMut {
        TransactionMut::new(self, None)
    }

    /// Creates and returns a lightweight read-only transaction.
    ///
    /// # Panics
    ///
    /// While it's possible to have multiple read-only transactions active at the same time,
    /// this method will panic whenever called while a read-write transaction
    /// (see: [Self::transact_mut]) is active at the same time.
    pub fn transact(&self) -> Transaction {
        Transaction::new(self, None)
    }

    /// Creates a new document with a specified `client_id`. It's up to a caller to guarantee that
    /// this identifier is unique across all communicating replicas of that document.
    pub fn with_client_id(client_id: ClientID) -> Self {
        Self::with_options(Options::with_client_id(client_id))
    }

    /// Creates a new document with a configured set of [Options].
    pub fn with_options(options: Options) -> Self {
        Doc {
            store: Store::new(options),
        }
    }

    pub fn state_vector(&self) -> &StateVector {
        self.store.blocks.state_vector()
    }

    /// A unique client identifier, that's also a unique identifier of current document replica
    /// and it's subdocuments.
    ///
    /// Default: randomly generated.
    pub fn client_id(&self) -> ClientID {
        self.options.client_id
    }

    /// A globally unique identifier, that's also a unique identifier of current document replica,
    /// and unlike [Doc::client_id] it's not shared with its subdocuments.
    ///
    /// Default: randomly generated UUID v4.
    pub fn guid(&self) -> DocId {
        self.options.guid.clone()
    }

    /// Returns a unique collection identifier, if defined.
    ///
    /// Default: `None`.
    pub fn collection_id(&self) -> Option<Arc<str>> {
        self.options.collection_id.clone()
    }

    /// Informs if current document is skipping garbage collection on deleted collections
    /// on transaction commit.
    ///
    /// Default: `false`.
    pub fn skip_gc(&self) -> bool {
        self.options.skip_gc
    }

    /// If current document is subdocument, it will automatically for a document to load.
    ///
    /// Default: `false`.
    pub fn auto_load(&self) -> bool {
        self.options.auto_load
    }

    /// Whether the document should be synced by the provider now.
    /// This is toggled to true when you call [Doc::load]
    ///
    /// Default value: `true`.
    pub fn should_load(&self) -> bool {
        self.options.should_load
    }

    /// Returns encoding used to count offsets and lengths in text operations.
    pub fn offset_kind(&self) -> OffsetKind {
        self.options.offset_kind
    }

    pub fn get_or_insert<R, S>(&mut self, name: S) -> R
    where
        R: RootRef,
        S: Into<Arc<str>>,
    {
        R::root(name).get_or_create(&mut self.transact_mut())
    }

    pub fn get<R, S>(&self, name: S) -> Option<R>
    where
        R: SharedRef,
        S: AsRef<str>,
    {
        let branch = self.types.get(name.as_ref())?;
        Some(R::from(BranchPtr::from(branch)))
    }

    /// Returns a [TextRef] data structure stored under a given `name`. Text structures are used for
    /// collaborative text editing: they expose operations to append and remove chunks of text,
    /// which are free to execute concurrently by multiple peers over remote boundaries.
    ///
    /// If no structure under defined `name` existed before, it will be created and returned
    /// instead.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as a text (in such case a sequence component of complex data type will be
    /// interpreted as a list of text chunks).
    ///
    /// # Panics
    ///
    /// This method requires exclusive access to an underlying document store. If there
    /// is another transaction in process, it will panic. It's advised to define all root shared
    /// types during the document creation.
    pub fn get_or_insert_text<N: Into<Arc<str>>>(&mut self, name: N) -> TextRef {
        TextRef::root(name).get_or_create(&mut self.transact_mut())
    }

    /// Returns a [MapRef] data structure stored under a given `name`. Maps are used to store key-value
    /// pairs associated. These values can be primitive data (similar but not limited to
    /// a JavaScript Object Notation) as well as other shared types (Yrs maps, arrays, text
    /// structures etc.), enabling to construct a complex recursive tree structures.
    ///
    /// If no structure under defined `name` existed before, it will be created and returned
    /// instead.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as a map (in such case a map component of complex data type will be
    /// interpreted as native map).
    ///
    /// # Panics
    ///
    /// This method requires exclusive access to an underlying document store. If there
    /// is another transaction in process, it will panic. It's advised to define all root shared
    /// types during the document creation.
    pub fn get_or_insert_map<N: Into<Arc<str>>>(&mut self, name: N) -> MapRef {
        MapRef::root(name).get_or_create(&mut self.transact_mut())
    }

    /// Returns an [ArrayRef] data structure stored under a given `name`. Array structures are used for
    /// storing a sequences of elements in ordered manner, positioning given element accordingly
    /// to its index.
    ///
    /// If no structure under defined `name` existed before, it will be created and returned
    /// instead.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as an array (in such case a sequence component of complex data type will be
    /// interpreted as a list of inserted values).
    ///
    /// # Panics
    ///
    /// This method requires exclusive access to an underlying document store. If there
    /// is another transaction in process, it will panic. It's advised to define all root shared
    /// types during the document creation.
    pub fn get_or_insert_array<N: Into<Arc<str>>>(&mut self, name: N) -> ArrayRef {
        ArrayRef::root(name).get_or_create(&mut self.transact_mut())
    }

    /// Returns a [XmlFragmentRef] data structure stored under a given `name`. XML elements represent
    /// nodes of XML document. They can contain attributes (key-value pairs, both of string type)
    /// and other nested XML elements or text values, which are stored in their insertion
    /// order.
    ///
    /// If no structure under defined `name` existed before, it will be created and returned
    /// instead.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as a XML element (in such case a map component of complex data type will be
    /// interpreted as map of its attributes, while a sequence component - as a list of its child
    /// XML nodes).
    ///
    /// # Panics
    ///
    /// This method requires exclusive access to an underlying document store. If there
    /// is another transaction in process, it will panic. It's advised to define all root shared
    /// types during the document creation.
    pub fn get_or_insert_xml_fragment<N: Into<Arc<str>>>(&mut self, name: N) -> XmlFragmentRef {
        XmlFragmentRef::root(name).get_or_create(&mut self.transact_mut())
    }

    /// Returns a reference to an object used to manage observer callbacks for this document.
    pub fn events(&mut self) -> &mut DocEvents {
        self.store.events.get_or_insert_default()
    }

    pub fn ptr_eq(a: &Doc, b: &Doc) -> bool {
        std::ptr::eq(a as *const Doc, b as *const Doc)
    }

    /// Returns a collection of globally unique identifiers of sub documents linked within
    /// the structures of this document store.
    pub fn subdoc_guids(&self) -> SubdocGuids {
        SubdocGuids::new(&self.subdocs)
    }

    pub fn to_json(&self) -> Any {
        let mut m = HashMap::new();
        let txn = self.transact();
        for (key, value) in self.store.types.iter() {
            let out = Out::from(BranchPtr::from(value));
            m.insert(key.to_string(), out.to_json(&txn));
        }
        Any::from(m)
    }

    define_observe_tx!(
        update_v1,
        observe_update_v1,
        observe_update_v1_with,
        unobserve_update_v1,
        crate::UpdateEvent
    );
    define_observe_tx!(
        update_v2,
        observe_update_v2,
        observe_update_v2_with,
        unobserve_update_v2,
        crate::UpdateEvent
    );
    define_observe!(
        destroy,
        observe_destroy,
        observe_destroy_with,
        unobserve_destroy,
        &crate::Doc
    );
    define_observe!(
        subdocs,
        observe_subdocs,
        observe_subdocs_with,
        unobserve_subdocs,
        &mut crate::SubdocsEvent
    );
    define_observe_tx!(
        transaction_cleanup,
        observe_transaction_cleanup,
        observe_transaction_cleanup_with,
        unobserve_transaction_cleanup,
        crate::TransactionCleanupEvent
    );
    define_observe_tx!(
        after_transaction,
        observe_after_transaction,
        observe_after_transaction_with,
        unobserve_after_transaction
    );
}

impl Drop for Doc {
    fn drop(&mut self) {
        if !self.store.subdocs.is_empty() {
            let mut txn = self.transact_mut();
            txn.subdocs_mut(|subdoc| {
                subdoc.destroy();
            });
        }
        if let Some(e) = &self.events {
            e.destroy.trigger(|f| f(self));
        }
    }
}

impl Deref for Doc {
    type Target = Store;

    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

impl DerefMut for Doc {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.store
    }
}

impl PartialEq for Doc {
    fn eq(&self, other: &Self) -> bool {
        self.guid() == other.guid()
    }
}

impl std::fmt::Display for Doc {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Doc(id: {}, guid: {})", self.client_id(), self.guid())
    }
}

impl Default for Doc {
    fn default() -> Self {
        Doc::new()
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, PartialEq, PartialOrd, Ord, Eq, Hash)]
pub struct DocId(Uuid);

impl From<Uuid> for DocId {
    #[inline]
    fn from(guid: Uuid) -> Self {
        DocId(guid)
    }
}

impl From<DocId> for Uuid {
    #[inline]
    fn from(doc_id: DocId) -> Self {
        doc_id.0
    }
}

impl std::fmt::Display for DocId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub struct SubDoc<'tx> {
    parent_txn: &'tx Transaction<'tx>,
    subdoc: CellRef<'tx, Doc>,
}

impl<'tx> SubDoc<'tx> {
    pub(crate) fn new(parent_txn: &'tx Transaction<'tx>, subdoc: CellRef<'tx, Doc>) -> Self {
        SubDoc { parent_txn, subdoc }
    }

    pub fn parent_txn(&self) -> &'tx Transaction<'tx> {
        self.parent_txn
    }
}

impl<'tx> Deref for SubDoc<'tx> {
    type Target = Doc;

    fn deref(&self) -> &Self::Target {
        self.subdoc.deref()
    }
}

pub struct SubDocMut<'tx> {
    parent_scope: &'tx mut Option<Box<Subdocs>>,
    subdoc: CellMut<'tx, Doc>,
}

impl<'tx> SubDocMut<'tx> {
    pub(crate) fn new(
        parent_scope: &'tx mut Option<Box<Subdocs>>,
        subdoc: CellMut<'tx, Doc>,
    ) -> Self {
        SubDocMut {
            parent_scope,
            subdoc,
        }
    }

    pub fn load(&mut self) {
        if !self.should_load() {
            let scope = self.parent_scope.get_or_insert_default();
            if let Some(item) = self.subdoc.store.subdoc {
                if let ItemContent::Doc(doc) = &item.content {
                    scope.loaded.push(SubDocHook::new(doc.clone()));
                }
            } else {
                // subdoc should always have this field initialized
            }
        }
        self.options.should_load = true;
    }

    pub fn destroy(mut self) {
        if let Some(item) = self.subdoc.subdoc.take() {
            let is_deleted = item.is_deleted();
            let mut options = self.subdoc.options.clone();
            options.should_load = false;
            let mut new_doc = Doc::with_options(options);
            new_doc.store.subdoc = Some(item);
            let old_doc = std::mem::replace(self.subdoc.deref_mut(), new_doc);
            let old_doc = SubDocHook::new(Cell::new(old_doc));
            let scope = self.parent_scope.get_or_insert_default();
            if !is_deleted {
                scope.added.push(old_doc.clone());
            }
            scope.removed.push(old_doc);
        }
    }
}

impl<'tx> Deref for SubDocMut<'tx> {
    type Target = Doc;

    fn deref(&self) -> &Self::Target {
        self.subdoc.deref()
    }
}

impl<'tx> DerefMut for SubDocMut<'tx> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.subdoc.deref_mut()
    }
}

pub struct SubdocsIter<'tx> {
    txn: &'tx Transaction<'tx>,
    inner: std::collections::hash_set::Iter<'tx, (DocId, ID)>,
}

impl<'tx> SubdocsIter<'tx> {
    pub(crate) fn new(
        txn: &'tx Transaction,
        subdocs: &'tx std::collections::HashSet<(DocId, ID)>,
    ) -> Self {
        SubdocsIter {
            txn,
            inner: subdocs.iter(),
        }
    }
}

impl<'tx> Iterator for SubdocsIter<'tx> {
    type Item = SubDoc<'tx>;

    fn next(&mut self) -> Option<Self::Item> {
        let (_, id) = self.inner.next()?;
        let parent_doc = self.txn.doc();
        let block = parent_doc.blocks.get_block(id)?.as_item()?;
        let item: &'tx Item = unsafe { std::mem::transmute(block.deref()) };
        if let ItemContent::Doc(subdoc) = &item.content {
            Some(SubDoc::new(self.txn, subdoc.borrow()))
        } else {
            None
        }
    }
}

#[repr(transparent)]
pub struct SubdocGuids<'doc>(std::collections::hash_set::Iter<'doc, (DocId, ID)>);

impl<'doc> SubdocGuids<'doc> {
    pub(crate) fn new(subdocs: &'doc std::collections::HashSet<(DocId, ID)>) -> Self {
        SubdocGuids(subdocs.iter())
    }
}

impl<'doc> Iterator for SubdocGuids<'doc> {
    type Item = &'doc DocId;

    fn next(&mut self) -> Option<Self::Item> {
        let (id, _) = self.0.next()?;
        Some(id)
    }
}

/// Configuration options of [Doc] instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Globally unique client identifier. This value must be unique across all active collaborating
    /// peers, otherwise a update collisions will happen, causing document store state to be corrupted.
    ///
    /// Default value: randomly generated.
    pub client_id: ClientID,
    /// A globally unique identifier for this document.
    ///
    /// Default value: randomly generated UUID v4.
    pub guid: DocId,
    /// Associate this document with a collection. This only plays a role if your provider has
    /// a concept of collection.
    ///
    /// Default value: `None`.
    pub collection_id: Option<Arc<str>>,
    /// How to we count offsets and lengths used in text operations.
    ///
    /// Default value: [OffsetKind::Bytes].
    pub offset_kind: OffsetKind,
    /// Determines if transactions commits should try to perform GC-ing of deleted items.
    ///
    /// Default value: `false`.
    pub skip_gc: bool,
    /// If a subdocument, automatically load document. If this is a subdocument, remote peers will
    /// load the document as well automatically.
    ///
    /// Default value: `false`.
    pub auto_load: bool,
    /// Whether the document should be synced by the provider now.
    /// This is toggled to true when you call ydoc.load().
    ///
    /// Default value: `true`.
    pub should_load: bool,
}

impl Options {
    pub fn with_client_id(client_id: ClientID) -> Self {
        Options {
            client_id,
            guid: uuid_v4().into(),
            collection_id: None,
            offset_kind: OffsetKind::Bytes,
            skip_gc: false,
            auto_load: false,
            should_load: true,
        }
    }

    pub fn with_guid_and_client_id<U: Into<DocId>>(guid: U, client_id: ClientID) -> Self {
        Options {
            client_id,
            guid: guid.into(),
            collection_id: None,
            offset_kind: OffsetKind::Bytes,
            skip_gc: false,
            auto_load: false,
            should_load: true,
        }
    }

    fn as_any(&self) -> Any {
        let mut m = HashMap::new();
        m.insert("gc".to_owned(), (!self.skip_gc).into());
        if let Some(collection_id) = self.collection_id.as_ref() {
            m.insert("collectionId".to_owned(), collection_id.clone().into());
        }
        let encoding = match self.offset_kind {
            OffsetKind::Bytes => 1,
            OffsetKind::Utf16 => 0, // 0 for compatibility with Yjs, which doesn't have this option
        };
        m.insert("encoding".to_owned(), Any::BigInt(encoding));
        m.insert("autoLoad".to_owned(), self.auto_load.into());
        m.insert("shouldLoad".to_owned(), self.should_load.into());
        Any::from(m)
    }
}

impl Default for Options {
    fn default() -> Self {
        let mut rng = fastrand::Rng::new();
        let client_id: u32 = rng.u32(0..u32::MAX);
        let uuid = uuid_v4_from(rng.u128(..));
        Self::with_guid_and_client_id(uuid, client_id as ClientID)
    }
}

impl Encode for Options {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        let guid = self.guid.to_string();
        encoder.write_string(&guid);
        encoder.write_any(&self.as_any())
    }
}

impl Decode for Options {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let mut options = Options::default();
        options.should_load = false; // for decoding shouldLoad is false by default
        let guid = decoder.read_string()?;
        options.guid = Uuid::from(guid).into();

        if let Any::Map(opts) = decoder.read_any()? {
            for (k, v) in opts.iter() {
                match (k.as_str(), v) {
                    ("gc", Any::Bool(gc)) => options.skip_gc = !*gc,
                    ("autoLoad", Any::Bool(auto_load)) => options.auto_load = *auto_load,
                    ("collectionId", Any::String(cid)) => options.collection_id = Some(cid.clone()),
                    ("encoding", Any::BigInt(1)) => options.offset_kind = OffsetKind::Bytes,
                    ("encoding", _) => options.offset_kind = OffsetKind::Utf16,
                    _ => { /* do nothing */ }
                }
            }
        }

        Ok(options)
    }
}

/// Determines how string length and offsets of [Text]/[XmlText] are being determined.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetKind {
    /// Compute editable strings length and offset using UTF-8 byte count.
    Bytes,
    /// Compute editable strings length and offset using UTF-16 chars count.
    Utf16,
}

impl FromOut for Cell<Doc> {
    fn from_out(value: Out, _txn: &Transaction) -> Result<Self, Out>
    where
        Self: Sized,
    {
        match value {
            Out::SubDoc(value) => Ok(value.inner),
            other => Err(other),
        }
    }

    fn from_item(item: ItemPtr, _txn: &Transaction) -> Option<Self>
    where
        Self: Sized,
    {
        match &item.content {
            ItemContent::Doc(doc) => Some(doc.clone()),
            _ => None,
        }
    }
}

impl Prelim for Doc {
    type Return = SubDocHook;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        (ItemContent::Doc(Cell::new(self)), None)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubDocHook {
    pub(crate) inner: Cell<Doc>,
}

impl SubDocHook {
    pub fn new(inner: Cell<Doc>) -> Self {
        SubDocHook { inner }
    }

    pub fn guid(&self) -> DocId {
        self.inner.borrow().guid()
    }

    pub fn as_ref<'tx>(&'tx self, parent_txn: &'tx Transaction) -> SubDoc<'tx> {
        SubDoc::new(parent_txn, self.inner.borrow())
    }

    pub fn as_mut<'tx>(&'tx mut self, parent_tx: &'tx mut TransactionMut) -> SubDocMut<'tx> {
        let (_, state) = parent_tx.split_mut();
        SubDocMut::new(&mut state.subdocs, self.inner.borrow_mut())
    }

    #[inline]
    pub fn borrow(&self) -> CellRef<Doc> {
        self.inner.borrow()
    }

    pub fn borrow_mut(&mut self) -> CellMut<Doc> {
        self.inner.borrow_mut()
    }
}

impl FromOut for SubDocHook {
    fn from_out(value: Out, _txn: &Transaction) -> Result<Self, Out>
    where
        Self: Sized,
    {
        match value {
            Out::SubDoc(subdoc) => Ok(subdoc),
            other => Err(other),
        }
    }

    fn from_item(item: ItemPtr, _txn: &Transaction) -> Option<Self>
    where
        Self: Sized,
    {
        match &item.content {
            ItemContent::Doc(doc) => Some(SubDocHook::new(doc.clone())),
            _ => None,
        }
    }
}

#[cfg(test)]
mod test {
    use crate::block::{BlockCell, ClientID, ItemContent, GC};
    use crate::error::Error;
    use crate::doc::SubDocHook;
    use crate::test_utils::exchange_updates;
    use crate::types::ToJson;
    use crate::update::Update;
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::{Encode, Encoder, EncoderV1};
    use crate::{
        any, uuid_v4, Any, Array, ArrayPrelim, ArrayRef, DeleteSet, Doc, DocId, GetString, Map,
        MapRef, OffsetKind, Options, StateVector, Subscription, Text, TextPrelim, TextRef,
        Transaction, Uuid, XmlElementPrelim, XmlFragment, XmlFragmentRef, XmlTextPrelim,
        XmlTextRef, ID,
    };
    use arc_swap::ArcSwapOption;
    use assert_matches2::assert_matches;
    use std::collections::BTreeSet;
    use std::iter::FromIterator;
    use std::ops::Deref;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn apply_update_basic_v1() {
        /* Result of calling following code:
        ```javascript
        const doc = new Y.Doc()
        const ytext = doc.getText('type')
        doc.transact(function () {
            for (let i = 0; i < 3; i++) {
                ytext.insert(0, (i % 10).toString())
            }
        })
        const update = Y.encodeStateAsUpdate(doc)
        ```
         */
        let update = &[
            1, 3, 227, 214, 245, 198, 5, 0, 4, 1, 4, 116, 121, 112, 101, 1, 48, 68, 227, 214, 245,
            198, 5, 0, 1, 49, 68, 227, 214, 245, 198, 5, 1, 1, 50, 0,
        ];
        let mut doc = Doc::new();
        let txt = doc.get_or_insert_text("type");
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v1(update).unwrap())
            .unwrap();

        let actual = txt.get_string(&txn);
        assert_eq!(actual, "210".to_owned());
    }

    #[test]
    fn apply_update_basic_v2() {
        /* Result of calling following code:
        ```javascript
        const doc = new Y.Doc()
        const ytext = doc.getText('type')
        doc.transact(function () {
            for (let i = 0; i < 3; i++) {
                ytext.insert(0, (i % 10).toString())
            }
        })
        const update = Y.encodeStateAsUpdateV2(doc)
        ```
         */
        let update = &[
            0, 0, 6, 195, 187, 207, 162, 7, 1, 0, 2, 0, 2, 3, 4, 0, 68, 11, 7, 116, 121, 112, 101,
            48, 49, 50, 4, 65, 1, 1, 1, 0, 0, 1, 3, 0, 0,
        ];
        let mut doc = Doc::new();
        let txt = doc.get_or_insert_text("type");
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v2(update).unwrap())
            .unwrap();

        let actual = txt.get_string(&txn);
        assert_eq!(actual, "210".to_owned());
    }

    #[test]
    fn encode_basic() {
        let mut doc = Doc::with_client_id(1490905955);
        let txt = doc.get_or_insert_text("type");
        let mut t = doc.transact_mut();
        txt.insert(&mut t, 0, "0");
        txt.insert(&mut t, 0, "1");
        txt.insert(&mut t, 0, "2");

        let encoded = t.encode_state_as_update_v1(&StateVector::default());
        let expected = &[
            1, 3, 227, 214, 245, 198, 5, 0, 4, 1, 4, 116, 121, 112, 101, 1, 48, 68, 227, 214, 245,
            198, 5, 0, 1, 49, 68, 227, 214, 245, 198, 5, 1, 1, 50, 0,
        ];
        assert_eq!(encoded.as_slice(), expected);
    }

    #[test]
    fn integrate() {
        // create new document at A and add some initial text to it
        let mut d1 = Doc::new();
        let txt = d1.get_or_insert_text("test");
        let mut t1 = d1.transact_mut();
        // Question: why YText.insert uses positions of blocks instead of actual cursor positions
        // in text as seen by user?
        txt.insert(&mut t1, 0, "hello");
        txt.insert(&mut t1, 5, " ");
        txt.insert(&mut t1, 6, "world");

        assert_eq!(txt.get_string(&t1), "hello world".to_string());

        // create document at B
        let mut d2 = Doc::new();
        let txt = d2.get_or_insert_text("test");
        let mut t2 = d2.transact_mut();
        let sv = t2.state_vector().encode_v1();

        // create an update A->B based on B's state vector
        let mut encoder = EncoderV1::new();
        t1.encode_diff(
            &StateVector::decode_v1(sv.as_slice()).unwrap(),
            &mut encoder,
        );
        let binary = encoder.to_vec();

        // decode an update incoming from A and integrate it at B
        let update = Update::decode_v1(binary.as_slice()).unwrap();
        let pending = update.integrate(&mut t2).unwrap();

        assert!(pending.0.is_none());
        assert!(pending.1.is_none());

        // check if B sees the same thing that A does
        assert_eq!(txt.get_string(&t1), "hello world".to_string());
    }

    #[test]
    fn on_update() {
        let counter = Arc::new(AtomicU32::new(0));
        let mut doc = Doc::new();
        let mut doc2 = Doc::new();
        let c = counter.clone();
        let sub = doc2.observe_update_v1(move |_, e| {
            let u = Update::decode_v1(&e.update).unwrap();
            for block in u.blocks.blocks() {
                c.fetch_add(block.len(), Ordering::SeqCst);
            }
        });
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();
        {
            txt.insert(&mut txn, 0, "abc");
            let mut txn2 = doc2.transact_mut();
            let sv = txn2.state_vector().encode_v1();
            let u = txn.encode_diff_v1(&StateVector::decode_v1(sv.as_slice()).unwrap());
            txn2.apply_update(Update::decode_v1(u.as_slice()).unwrap())
                .unwrap();
        }
        assert_eq!(counter.load(Ordering::SeqCst), 3); // update has been propagated

        drop(sub);

        {
            txt.insert(&mut txn, 3, "de");
            let mut txn2 = doc2.transact_mut();
            let sv = txn2.state_vector().encode_v1();
            let u = txn.encode_diff_v1(&StateVector::decode_v1(sv.as_slice()).unwrap());
            txn2.apply_update(Update::decode_v1(u.as_slice()).unwrap())
                .unwrap();
        }
        assert_eq!(counter.load(Ordering::SeqCst), 3); // since subscription has been dropped, update was not propagated
    }

    #[test]
    fn pending_update_integration() {
        let mut doc = Doc::new();
        let txt = doc.get_or_insert_text("source");

        let updates = [
            vec![
                1, 2, 242, 196, 218, 129, 3, 0, 40, 1, 5, 115, 116, 97, 116, 101, 5, 100, 105, 114,
                116, 121, 1, 121, 40, 1, 7, 99, 111, 110, 116, 101, 120, 116, 4, 112, 97, 116, 104,
                1, 119, 13, 117, 110, 116, 105, 116, 108, 101, 100, 52, 46, 116, 120, 116, 0,
            ],
            vec![
                1, 1, 242, 196, 218, 129, 3, 2, 40, 1, 7, 99, 111, 110, 116, 101, 120, 116, 13,
                108, 97, 115, 116, 95, 109, 111, 100, 105, 102, 105, 101, 100, 1, 119, 27, 50, 48,
                50, 50, 45, 48, 52, 45, 49, 51, 84, 49, 48, 58, 49, 48, 58, 53, 55, 46, 48, 55, 51,
                54, 50, 51, 90, 0,
            ],
            vec![
                1, 2, 242, 196, 218, 129, 3, 3, 4, 1, 6, 115, 111, 117, 114, 99, 101, 1, 97, 168,
                242, 196, 218, 129, 3, 0, 1, 120, 0,
            ],
            vec![
                1, 1, 242, 196, 218, 129, 3, 4, 168, 242, 196, 218, 129, 3, 0, 1, 120, 1, 242, 196,
                218, 129, 3, 1, 0, 1,
            ],
            vec![
                1, 1, 152, 182, 129, 244, 193, 193, 227, 4, 0, 168, 242, 196, 218, 129, 3, 4, 1,
                121, 1, 242, 196, 218, 129, 3, 2, 0, 1, 4, 1,
            ],
            vec![
                1, 2, 242, 196, 218, 129, 3, 5, 132, 242, 196, 218, 129, 3, 3, 1, 98, 168, 152,
                190, 167, 244, 1, 0, 1, 120, 0,
            ],
            vec![
                1, 1, 242, 196, 218, 129, 3, 6, 168, 152, 190, 167, 244, 1, 0, 1, 120, 1, 152, 190,
                167, 244, 1, 1, 0, 1,
            ],
            vec![
                1, 1, 242, 196, 218, 129, 3, 7, 132, 242, 196, 218, 129, 3, 5, 1, 99, 0,
            ],
            vec![
                1, 1, 242, 196, 218, 129, 3, 8, 132, 242, 196, 218, 129, 3, 7, 1, 100, 0,
            ],
        ];

        for u in updates {
            let mut txn = doc.transact_mut();
            let u = Update::decode_v1(u.as_slice()).unwrap();
            txn.apply_update(u).unwrap();
        }
        assert_eq!(txt.get_string(&doc.transact()), "abcd".to_string());
    }

    #[test]
    fn ypy_issue_32() {
        let mut d1 = Doc::with_client_id(1971027812);
        let source_1 = d1.get_or_insert_text("source");
        source_1.push(&mut d1.transact_mut(), "a");

        let updates = [
            vec![
                1, 2, 201, 210, 153, 56, 0, 40, 1, 5, 115, 116, 97, 116, 101, 5, 100, 105, 114,
                116, 121, 1, 121, 40, 1, 7, 99, 111, 110, 116, 101, 120, 116, 4, 112, 97, 116, 104,
                1, 119, 13, 117, 110, 116, 105, 116, 108, 101, 100, 52, 46, 116, 120, 116, 0,
            ],
            vec![
                1, 1, 201, 210, 153, 56, 2, 168, 201, 210, 153, 56, 0, 1, 120, 1, 201, 210, 153,
                56, 1, 0, 1,
            ],
            vec![
                1, 1, 201, 210, 153, 56, 3, 40, 1, 7, 99, 111, 110, 116, 101, 120, 116, 13, 108,
                97, 115, 116, 95, 109, 111, 100, 105, 102, 105, 101, 100, 1, 119, 27, 50, 48, 50,
                50, 45, 48, 52, 45, 49, 54, 84, 49, 52, 58, 48, 51, 58, 53, 51, 46, 57, 51, 48, 52,
                54, 56, 90, 0,
            ],
            vec![
                1, 1, 201, 210, 153, 56, 4, 168, 201, 210, 153, 56, 2, 1, 121, 1, 201, 210, 153,
                56, 1, 2, 1,
            ],
        ];
        for u in updates {
            let u = Update::decode_v1(&u).unwrap();
            d1.transact_mut().apply_update(u).unwrap();
        }

        assert_eq!("a", source_1.get_string(&d1.transact()));

        let mut d2 = Doc::new();
        let source_2 = d2.get_or_insert_text("source");
        let state_2 = d2.transact().state_vector().encode_v1();
        let update = d1
            .transact()
            .encode_state_as_update_v1(&StateVector::decode_v1(&state_2).unwrap());
        let update = Update::decode_v1(&update).unwrap();
        d2.transact_mut().apply_update(update).unwrap();

        assert_eq!("a", source_2.get_string(&d2.transact()));

        let update = Update::decode_v1(&[
            1, 2, 201, 210, 153, 56, 5, 132, 228, 254, 237, 171, 7, 0, 1, 98, 168, 201, 210, 153,
            56, 4, 1, 120, 0,
        ])
        .unwrap();
        d1.transact_mut().apply_update(update).unwrap();
        assert_eq!("ab", source_1.get_string(&d1.transact()));

        let mut d3 = Doc::new();
        let source_3 = d3.get_or_insert_text("source");
        let state_3 = d3.transact().state_vector().encode_v1();
        let state_3 = StateVector::decode_v1(&state_3).unwrap();
        let update = d1.transact().encode_state_as_update_v1(&state_3);
        let update = Update::decode_v1(&update).unwrap();
        d3.transact_mut().apply_update(update).unwrap();

        assert_eq!("ab", source_3.get_string(&d3.transact()));
    }

    #[test]
    fn observe_transaction_cleanup() {
        // Setup
        let mut doc = Doc::new();
        let text = doc.get_or_insert_text("test");
        let before_state = Arc::new(ArcSwapOption::default());
        let after_state = Arc::new(ArcSwapOption::default());
        let delete_set = Arc::new(ArcSwapOption::default());
        // Create interior mutable references for the callback.
        let before_ref = before_state.clone();
        let after_ref = after_state.clone();
        let delete_ref = delete_set.clone();
        // Subscribe callback

        let sub: Subscription = doc.observe_transaction_cleanup(move |_: &Transaction, event| {
            before_ref.store(Some(event.before_state.clone().into()));
            after_ref.store(Some(event.after_state.clone().into()));
            delete_ref.store(Some(event.delete_set.clone().into()));
        });

        {
            let mut txn = doc.transact_mut();

            // Update the document
            text.insert(&mut txn, 0, "abc");
            text.remove_range(&mut txn, 1, 2);
            let tx_state = txn.commit().unwrap();

            // Compare values
            assert_eq!(
                before_state.swap(None),
                Some(Arc::new(tx_state.before_state.clone()))
            );
            assert_eq!(
                after_state.swap(None),
                Some(Arc::new(tx_state.after_state.clone()))
            );
            assert_eq!(delete_set.swap(None).as_deref(), Some(&tx_state.delete_set));
        }

        // Ensure that the subscription is successfully dropped.
        drop(sub);
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "should not update");
        txn.commit();
        assert_ne!(
            after_state.swap(None),
            Some(Arc::new(txn.after_state().clone()))
        );
    }

    #[test]
    fn partially_duplicated_update() {
        let mut d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("text");
        txt1.insert(&mut d1.transact_mut(), 0, "hello");
        let u = d1
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        let mut d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("text");
        d2.transact_mut()
            .apply_update(Update::decode_v1(&u).unwrap())
            .unwrap();

        txt1.insert(&mut d1.transact_mut(), 5, "world");
        let u = d1
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        d2.transact_mut()
            .apply_update(Update::decode_v1(&u).unwrap())
            .unwrap();

        assert_eq!(
            txt1.get_string(&d1.transact()),
            txt2.get_string(&d2.transact())
        );
    }

    #[test]
    fn incremental_observe_update() {
        const INPUT: &'static str = "hello";

        let mut d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("text");
        let acc = Arc::new(Mutex::new(String::new()));

        let a = acc.clone();
        let _sub = d1.observe_update_v1(move |_: &Transaction, e| {
            let u = Update::decode_v1(&e.update).unwrap();
            for mut block in u.blocks.into_blocks(false) {
                match block.as_item_ptr().as_deref() {
                    Some(item) => {
                        if let ItemContent::String(s) = &item.content {
                            // each character is appended in individual transaction 1-by-1,
                            // therefore each update should contain a single string with only
                            // one element
                            let mut aref = a.lock().unwrap();
                            aref.push_str(s.as_str());
                        } else {
                            panic!("unexpected content type")
                        }
                    }
                    _ => {}
                }
            }
        });

        for c in INPUT.chars() {
            // append characters 1-by-1 (1 transactions per character)
            txt1.push(&mut d1.transact_mut(), &c.to_string());
        }

        assert_eq!(acc.lock().unwrap().as_str(), INPUT);

        // test incremental deletes
        let acc = Arc::new(Mutex::new(vec![]));
        let a = acc.clone();
        let _sub = d1.observe_update_v1(move |_: &Transaction, e| {
            let u = Update::decode_v1(&e.update).unwrap();
            for (&client_id, range) in u.delete_set.iter() {
                if client_id == 1 {
                    let mut aref = a.lock().unwrap();
                    for r in range.iter() {
                        aref.push(r.clone());
                    }
                }
            }
        });

        for _ in 0..INPUT.len() as u32 {
            txt1.remove_range(&mut d1.transact_mut(), 0, 1);
        }

        let expected = vec![(0..1), (1..2), (2..3), (3..4), (4..5)];
        assert_eq!(&*acc.lock().unwrap(), &expected);
    }

    #[test]
    fn ycrdt_issue_174() {
        let mut doc = Doc::new();
        let bin = &[
            0, 0, 11, 176, 133, 128, 149, 31, 205, 190, 199, 196, 21, 7, 3, 0, 3, 5, 0, 17, 168, 1,
            8, 0, 40, 0, 8, 0, 40, 0, 8, 0, 40, 0, 33, 1, 39, 110, 91, 49, 49, 49, 114, 111, 111,
            116, 105, 51, 50, 114, 111, 111, 116, 115, 116, 114, 105, 110, 103, 114, 111, 111, 116,
            97, 95, 108, 105, 115, 116, 114, 111, 111, 116, 97, 95, 109, 97, 112, 114, 111, 111,
            116, 105, 51, 50, 95, 108, 105, 115, 116, 114, 111, 111, 116, 105, 51, 50, 95, 109, 97,
            112, 114, 111, 111, 116, 115, 116, 114, 105, 110, 103, 95, 108, 105, 115, 116, 114,
            111, 111, 116, 115, 116, 114, 105, 110, 103, 95, 109, 97, 112, 65, 1, 4, 3, 4, 6, 4, 6,
            4, 5, 4, 8, 4, 7, 4, 11, 4, 10, 3, 0, 5, 1, 6, 0, 1, 0, 1, 0, 1, 2, 65, 8, 2, 8, 0,
            125, 2, 119, 5, 119, 111, 114, 108, 100, 118, 2, 1, 98, 119, 1, 97, 1, 97, 125, 1, 118,
            2, 1, 98, 119, 1, 98, 1, 97, 125, 2, 125, 1, 125, 2, 119, 1, 97, 119, 1, 98, 8, 0, 1,
            141, 223, 163, 226, 10, 1, 0, 1,
        ];
        let update = Update::decode_v2(bin).unwrap();
        doc.transact_mut().apply_update(update).unwrap();

        let root = doc.get_or_insert_map("root");
        let actual = root.to_json(&doc.transact());
        let expected = Any::from_json(
            r#"{
              "string": "world",
              "a_list": [{"b": "a", "a": 1}],
              "i32_map": {"1": 2},
              "a_map": {
                "1": {"a": 2, "b": "b"}
              },
              "string_list": ["a"],
              "i32": 2,
              "string_map": {"1": "b"},
              "i32_list": [1]
            }"#,
        )
        .unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn snapshots_splitting_text() {
        let mut options = Options::with_client_id(1);
        options.skip_gc = true;

        let mut d1 = Doc::with_options(options);
        let txt1 = d1.get_or_insert_text("text");
        txt1.insert(&mut d1.transact_mut(), 0, "hello");
        let snapshot = d1.transact_mut().snapshot();
        txt1.insert(&mut d1.transact_mut(), 5, "_world");

        let mut encoder = EncoderV1::new();
        d1.transact_mut()
            .encode_state_from_snapshot(&snapshot, &mut encoder)
            .unwrap();
        let update = Update::decode_v1(&encoder.to_vec()).unwrap();

        let mut d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("text");
        d2.transact_mut().apply_update(update).unwrap();

        assert_eq!(txt2.get_string(&d2.transact()), "hello".to_string());
    }

    #[test]
    fn snapshot_non_splitting_text() {
        let mut options = Options::default();
        options.skip_gc = true;

        let mut doc = Doc::with_options(options.clone().into());
        let txt = doc.get_or_insert_text("name");

        let mut txn = doc.transact_mut();
        txt.insert(&mut txn, 0, "Lucas");
        drop(txn);

        let txn = doc.transact();
        let snapshot = txn.snapshot();

        let mut encoder = EncoderV1::new();
        txn.encode_state_from_snapshot(&snapshot, &mut encoder)
            .unwrap();
        let state_diff = encoder.to_vec();

        let mut remote_doc = Doc::with_options(options);
        let remote_txt = remote_doc.get_or_insert_text("name");
        let mut txn = remote_doc.transact_mut();
        let update = Update::decode_v1(&state_diff).unwrap();
        txn.apply_update(update).unwrap();

        let actual = remote_txt.get_string(&txn);

        assert_eq!(actual, "Lucas");
    }

    #[test]
    fn yrb_issue_45() {
        let diffs: Vec<Vec<u8>> = vec![
            vec![
                1, 3, 197, 134, 244, 186, 10, 0, 7, 1, 7, 100, 101, 102, 97, 117, 108, 116, 3, 9,
                112, 97, 114, 97, 103, 114, 97, 112, 104, 7, 0, 197, 134, 244, 186, 10, 0, 6, 4, 0,
                197, 134, 244, 186, 10, 1, 1, 115, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 3, 132, 197, 134, 244, 186, 10, 2, 3, 227, 129, 149,
                1, 197, 134, 244, 186, 10, 1, 2, 1,
            ],
            vec![
                1, 4, 197, 134, 244, 186, 10, 0, 7, 1, 7, 100, 101, 102, 97, 117, 108, 116, 3, 9,
                112, 97, 114, 97, 103, 114, 97, 112, 104, 7, 0, 197, 134, 244, 186, 10, 0, 6, 1, 0,
                197, 134, 244, 186, 10, 1, 1, 132, 197, 134, 244, 186, 10, 2, 3, 227, 129, 149, 1,
                197, 134, 244, 186, 10, 1, 2, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 4, 132, 197, 134, 244, 186, 10, 3, 1, 120, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 5, 132, 197, 134, 244, 186, 10, 4, 3, 227, 129, 129,
                1, 197, 134, 244, 186, 10, 1, 4, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 6, 132, 197, 134, 244, 186, 10, 5, 1, 107, 0,
            ],
            vec![
                1, 2, 197, 134, 244, 186, 10, 4, 129, 197, 134, 244, 186, 10, 3, 1, 132, 197, 134,
                244, 186, 10, 4, 3, 227, 129, 129, 1, 197, 134, 244, 186, 10, 1, 4, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 7, 132, 197, 134, 244, 186, 10, 6, 3, 227, 129, 147,
                1, 197, 134, 244, 186, 10, 1, 6, 1,
            ],
            vec![
                1, 2, 197, 134, 244, 186, 10, 6, 129, 197, 134, 244, 186, 10, 5, 1, 132, 197, 134,
                244, 186, 10, 6, 3, 227, 129, 147, 1, 197, 134, 244, 186, 10, 1, 6, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 8, 132, 197, 134, 244, 186, 10, 7, 1, 114, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 9, 132, 197, 134, 244, 186, 10, 8, 3, 227, 130, 140,
                1, 197, 134, 244, 186, 10, 1, 8, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 8, 132, 197, 134, 244, 186, 10, 7, 1, 114, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 10, 132, 197, 134, 244, 186, 10, 9, 1, 107, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 11, 132, 197, 134, 244, 186, 10, 10, 3, 227, 129,
                139, 1, 197, 134, 244, 186, 10, 1, 10, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 12, 132, 197, 134, 244, 186, 10, 11, 1, 114, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 13, 132, 197, 134, 244, 186, 10, 12, 3, 227, 130,
                137, 1, 197, 134, 244, 186, 10, 1, 12, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 9, 132, 197, 134, 244, 186, 10, 8, 3, 227, 130, 140,
                1, 197, 134, 244, 186, 10, 1, 8, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 10, 132, 197, 134, 244, 186, 10, 9, 1, 107, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 11, 132, 197, 134, 244, 186, 10, 10, 3, 227, 129,
                139, 1, 197, 134, 244, 186, 10, 1, 10, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 12, 132, 197, 134, 244, 186, 10, 11, 1, 114, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 14, 132, 197, 134, 244, 186, 10, 13, 1, 98, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 16, 132, 197, 134, 244, 186, 10, 15, 1, 103, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 15, 132, 197, 134, 244, 186, 10, 14, 3, 227, 129,
                176, 1, 197, 134, 244, 186, 10, 1, 14, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 17, 132, 197, 134, 244, 186, 10, 16, 3, 227, 129,
                144, 1, 197, 134, 244, 186, 10, 1, 16, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 17, 132, 197, 134, 244, 186, 10, 16, 3, 227, 129,
                144, 1, 197, 134, 244, 186, 10, 1, 16, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 18, 132, 197, 134, 244, 186, 10, 17, 6, 227, 131,
                144, 227, 130, 176, 1, 197, 134, 244, 186, 10, 2, 15, 1, 17, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 20, 132, 197, 134, 244, 186, 10, 19, 1, 103, 0,
            ],
            vec![
                1, 3, 197, 134, 244, 186, 10, 13, 132, 197, 134, 244, 186, 10, 12, 3, 227, 130,
                137, 129, 197, 134, 244, 186, 10, 13, 1, 132, 197, 134, 244, 186, 10, 14, 4, 227,
                129, 176, 103, 1, 197, 134, 244, 186, 10, 2, 12, 1, 14, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 21, 132, 197, 134, 244, 186, 10, 20, 3, 227, 129,
                140, 1, 197, 134, 244, 186, 10, 1, 20, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 23, 132, 197, 134, 244, 186, 10, 22, 3, 227, 129,
                170, 1, 197, 134, 244, 186, 10, 1, 22, 1,
            ],
            vec![
                1, 3, 197, 134, 244, 186, 10, 18, 132, 197, 134, 244, 186, 10, 17, 6, 227, 131,
                144, 227, 130, 176, 129, 197, 134, 244, 186, 10, 19, 1, 132, 197, 134, 244, 186,
                10, 20, 3, 227, 129, 140, 1, 197, 134, 244, 186, 10, 3, 15, 1, 17, 1, 20, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 24, 132, 197, 134, 244, 186, 10, 23, 3, 227, 129,
                132, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 22, 132, 197, 134, 244, 186, 10, 21, 1, 110, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 26, 132, 197, 134, 244, 186, 10, 25, 3, 227, 129,
                139, 1, 197, 134, 244, 186, 10, 1, 25, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 25, 132, 197, 134, 244, 186, 10, 24, 1, 107, 0,
            ],
            vec![
                1, 4, 197, 134, 244, 186, 10, 22, 129, 197, 134, 244, 186, 10, 21, 1, 132, 197,
                134, 244, 186, 10, 22, 6, 227, 129, 170, 227, 129, 132, 129, 197, 134, 244, 186,
                10, 24, 1, 132, 197, 134, 244, 186, 10, 25, 3, 227, 129, 139, 1, 197, 134, 244,
                186, 10, 2, 22, 1, 25, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 27, 132, 197, 134, 244, 186, 10, 26, 1, 100, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 28, 132, 197, 134, 244, 186, 10, 27, 3, 227, 129,
                169, 1, 197, 134, 244, 186, 10, 1, 27, 1,
            ],
            vec![
                1, 2, 197, 134, 244, 186, 10, 27, 129, 197, 134, 244, 186, 10, 26, 1, 132, 197,
                134, 244, 186, 10, 27, 3, 227, 129, 169, 1, 197, 134, 244, 186, 10, 1, 27, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 29, 132, 197, 134, 244, 186, 10, 28, 3, 227, 129,
                134, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 30, 132, 197, 134, 244, 186, 10, 29, 1, 107, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 29, 132, 197, 134, 244, 186, 10, 28, 3, 227, 129,
                134, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 31, 132, 197, 134, 244, 186, 10, 30, 3, 227, 129,
                139, 1, 197, 134, 244, 186, 10, 1, 30, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 30, 132, 197, 134, 244, 186, 10, 29, 1, 107, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 31, 132, 197, 134, 244, 186, 10, 30, 3, 227, 129,
                139, 1, 197, 134, 244, 186, 10, 1, 30, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 32, 135, 197, 134, 244, 186, 10, 0, 3, 9, 112, 97,
                114, 97, 103, 114, 97, 112, 104, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 32, 135, 197, 134, 244, 186, 10, 0, 3, 9, 112, 97,
                114, 97, 103, 114, 97, 112, 104, 0,
            ],
            vec![
                1, 2, 197, 134, 244, 186, 10, 33, 7, 0, 197, 134, 244, 186, 10, 32, 6, 4, 0, 197,
                134, 244, 186, 10, 33, 1, 107, 0,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 35, 132, 197, 134, 244, 186, 10, 34, 3, 227, 129,
                139, 1, 197, 134, 244, 186, 10, 1, 34, 1,
            ],
            vec![
                1, 1, 197, 134, 244, 186, 10, 36, 132, 197, 134, 244, 186, 10, 35, 1, 107, 0,
            ],
        ];

        let mut doc = Doc::new();
        let mut txn = doc.transact_mut();
        for diff in diffs {
            let u = Update::decode_v1(diff.as_slice()).unwrap();
            txn.apply_update(u).unwrap();
        }
    }

    #[test]
    fn root_refs() {
        let mut doc = Doc::new();
        {
            let _txt = doc.get_or_insert_text("text");
            let _array = doc.get_or_insert_array("array");
            let _map = doc.get_or_insert_map("map");
            let _xml_elem = doc.get_or_insert_xml_fragment("xml_elem");
        }

        let txn = doc.transact();
        for (key, value) in txn.root_refs() {
            match key {
                "text" => assert!(value.cast::<TextRef>(&txn).is_ok()),
                "array" => assert!(value.cast::<ArrayRef>(&txn).is_ok()),
                "map" => assert!(value.cast::<MapRef>(&txn).is_ok()),
                "xml_elem" => assert!(value.cast::<XmlFragmentRef>(&txn).is_ok()),
                "xml_text" => assert!(value.cast::<XmlTextRef>(&txn).is_ok()),
                other => panic!("unrecognized root type: '{}'", other),
            }
        }
    }

    #[test]
    fn integrate_block_with_parent_gc() {
        let mut d1 = Doc::with_client_id(1);
        let mut d2 = Doc::with_client_id(2);
        let mut d3 = Doc::with_client_id(3);

        {
            let root = d1.get_or_insert_array("array");
            let mut txn = d1.transact_mut();
            root.push_back(&mut txn, ArrayPrelim::from(["A"]));
        }

        exchange_updates([&mut d1, &mut d2, &mut d3]);

        {
            let root = d2.get_or_insert_array("array");
            let mut t2 = d2.transact_mut();
            root.remove(&mut t2, 0);
            d1.transact_mut()
                .apply_update(Update::decode_v1(&t2.encode_update_v1()).unwrap())
                .unwrap();
        }

        {
            let root = d3.get_or_insert_array("array");
            let mut t3 = d3.transact_mut();
            let a3: ArrayRef = root.get(&t3, 0).unwrap();
            a3.push_back(&mut t3, "B");
            // D1 got update which already removed a3, but this must not cause panic
            d1.transact_mut()
                .apply_update(Update::decode_v1(&t3.encode_update_v1()).unwrap())
                .unwrap();
        }

        exchange_updates([&mut d1, &mut d2, &mut d3]);

        let r1 = d1.get_or_insert_array("array").to_json(&d1.transact());
        let r2 = d2.get_or_insert_array("array").to_json(&d2.transact());
        let r3 = d3.get_or_insert_array("array").to_json(&d3.transact());

        assert_eq!(r1, r2);
        assert_eq!(r2, r3);
        assert_eq!(r3, r1);
    }

    #[test]
    fn subdoc() {
        let mut doc = Doc::with_client_id(1);
        let event = Arc::new(ArcSwapOption::default());
        let event_c = event.clone();
        let _sub = doc.observe_subdocs(move |e| {
            let added = e.added().iter().map(SubDocHook::guid).collect();
            let removed = e.removed().iter().map(|d| d.guid()).collect();
            let loaded = e.loaded().iter().map(SubDocHook::guid).collect();
            event_c.store(Some(Arc::new((added, removed, loaded))));
        });
        let subdocs = doc.get_or_insert_map("mysubdocs");
        let uuid_a: DocId = Uuid::from("A").into();
        let doc_a = Doc::with_options({
            let mut o = Options::default();
            o.guid = uuid_a.clone();
            o
        });
        {
            let mut txn = doc.transact_mut();
            let mut hook = subdocs.insert(&mut txn, "a", doc_a);
            let mut doc_a_ref = hook.as_mut(&mut txn);
            doc_a_ref.load();
        }

        let actual = event.swap(None);
        assert_eq!(
            actual,
            Some((vec![uuid_a.clone()], vec![], vec![uuid_a.clone()]).into())
        );

        {
            let mut txn = doc.transact_mut();
            let mut hook: SubDocHook = subdocs.get(&txn, "a").unwrap();
            let mut doc_a_ref = hook.as_mut(&mut txn);
            doc_a_ref.load();
        }
        let actual = event.swap(None);
        assert_eq!(actual, None);

        {
            let mut txn = doc.transact_mut();
            let mut doc_a_ref: SubDocHook = subdocs.get(&txn, "a").unwrap();
            let doc_a_ref = doc_a_ref.as_mut(&mut txn);
            doc_a_ref.destroy();
        }
        let actual = event.swap(None);
        assert_eq!(
            actual,
            Some(Arc::new((
                vec![uuid_a.clone()],
                vec![uuid_a.clone()],
                vec![]
            )))
        );

        {
            let mut txn = doc.transact_mut();
            let mut doc_a_ref: SubDocHook = subdocs.get(&txn, "a").unwrap();
            let mut doc_a_ref = doc_a_ref.as_mut(&mut txn);
            doc_a_ref.load();
        }
        let actual = event.swap(None);
        assert_eq!(
            actual,
            Some(Arc::new((vec![], vec![], vec![uuid_a.clone()])))
        );

        let doc_b = Doc::with_options({
            let mut o = Options::default();
            o.guid = uuid_a.clone();
            o.should_load = false;
            o
        });
        subdocs.insert(&mut doc.transact_mut(), "b", doc_b);
        let actual = event.swap(None);
        assert_eq!(
            actual,
            Some(Arc::new((vec![uuid_a.clone()], vec![], vec![])))
        );

        {
            let mut txn = doc.transact_mut();
            let mut doc_b_ref: SubDocHook = subdocs.get(&txn, "b").unwrap();
            let mut doc_b_ref = doc_b_ref.as_mut(&mut txn);
            doc_b_ref.load();
        }
        let actual = event.swap(None);
        assert_eq!(
            actual,
            Some(Arc::new((vec![], vec![], vec![uuid_a.clone()])))
        );

        let uuid_c: DocId = Uuid::from("C").into();
        let doc_c = Doc::with_options({
            let mut o = Options::default();
            o.guid = uuid_c.clone();
            o
        });
        {
            let mut txn = doc.transact_mut();
            let mut doc_c_ref = subdocs.insert(&mut txn, "c", doc_c);
            let mut doc_c_ref = doc_c_ref.as_mut(&mut txn);
            doc_c_ref.load();
        }
        let actual = event.swap(None);
        assert_eq!(
            actual,
            Some(Arc::new((
                vec![uuid_c.clone()],
                vec![],
                vec![uuid_c.clone()]
            )))
        );

        let guids: BTreeSet<_> = doc.subdoc_guids().collect();
        assert_eq!(guids, BTreeSet::from([&uuid_a, &uuid_c]));

        let data = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        let mut doc2 = Doc::new();
        let event = Arc::new(ArcSwapOption::default());
        let event_c = event.clone();
        let _sub = doc2.observe_subdocs(move |e| {
            let added: Vec<_> = e.added().iter().map(SubDocHook::guid).collect();
            let removed: Vec<_> = e.removed().iter().map(SubDocHook::guid).collect();
            let loaded: Vec<_> = e.loaded().iter().map(SubDocHook::guid).collect();
            event_c.store(Some(Arc::new((added, removed, loaded))));
        });
        let update = Update::decode_v1(&data).unwrap();
        doc2.transact_mut().apply_update(update).unwrap();
        let mut actual = event.swap(None).unwrap();
        Arc::get_mut(&mut actual).unwrap().0.sort();
        assert_eq!(
            actual,
            Arc::new((
                vec![uuid_a.clone(), uuid_a.clone(), uuid_c.clone()],
                vec![],
                vec![]
            ))
        );

        let subdocs = doc2.transact().get_map("mysubdocs").unwrap();
        {
            let mut txn = doc2.transact_mut();
            let mut doc_ref: SubDocHook = subdocs.get(&mut txn, "a").unwrap();
            let mut doc_ref = doc_ref.as_mut(&mut txn);
            doc_ref.load();
        }
        let actual = event.swap(None);
        assert_eq!(
            actual,
            Some(Arc::new((vec![], vec![], vec![uuid_a.clone()])))
        );

        let guids: BTreeSet<_> = doc2.subdoc_guids().collect();
        assert_eq!(guids, BTreeSet::from([&uuid_a, &uuid_c]));
        {
            let mut txn = doc2.transact_mut();
            subdocs.remove(&mut txn, "a");
        }

        let actual = event.swap(None);
        assert_eq!(
            actual,
            Some(Arc::new((vec![], vec![uuid_a.clone()], vec![])))
        );

        let guids: BTreeSet<_> = doc2.subdoc_guids().collect();
        assert_eq!(guids, BTreeSet::from([&uuid_a, &uuid_c]));
    }

    #[test]
    fn subdoc_load_edge_cases() {
        fn pointer(doc: &Doc) -> usize {
            let store = doc.store.deref();
            std::ptr::from_ref(store) as usize
        }
        let mut doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("test");
        let subdoc_1 = Doc::new();

        let event = Arc::new(ArcSwapOption::default());
        let event_c = event.clone();
        let _sub = doc.observe_subdocs(move |e| {
            let added = e.added().iter().map(SubDocHook::guid).collect();
            let removed = e.removed().iter().map(SubDocHook::guid).collect();
            let loaded = e.loaded().iter().map(SubDocHook::guid).collect();

            event_c.store(Some(Arc::new((added, removed, loaded))));
        });
        let (doc_ptr, mut uuid_1) = {
            let mut txn = doc.transact_mut();
            let hook = array.insert(&mut txn, 0, subdoc_1);
            let ptr = {
                let doc_ref = hook.as_ref(&txn);
                assert!(doc_ref.should_load());
                assert!(!doc_ref.auto_load());
                pointer(doc_ref.deref())
            };
            (ptr, hook)
        };
        let e = event.swap(None);
        assert_eq!(
            e,
            Some((vec![uuid_1.guid()], vec![], vec![uuid_1.guid()]).into())
        );

        let uuid_2 = {
            // destroy and check whether lastEvent adds it again to added (it shouldn't)
            uuid_1.as_mut(&mut doc.transact_mut()).destroy();
            let tx = doc.transact();
            let uuid_2: SubDocHook = array.get(&tx, 0).unwrap();
            let doc_ref = uuid_2.as_ref(&tx);
            assert_ne!(doc_ptr, pointer(doc_ref.deref()));
            uuid_2.guid()
        };

        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some((vec![uuid_2.clone()], vec![uuid_2.clone()], vec![]).into())
        );

        // load

        {
            let mut txn = doc.transact_mut();
            let mut sd: SubDocHook = array.get(&txn, 0).unwrap();
            let mut doc_ref_2 = sd.as_mut(&mut txn);
            doc_ref_2.load();
        }
        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some(Arc::new((vec![], vec![], vec![uuid_2.clone()])))
        );

        // apply from remote
        let mut doc2 = Doc::with_client_id(2);
        let event_c = event.clone();
        let _sub = doc2.observe_subdocs(move |e| {
            let added = e.added().iter().map(SubDocHook::guid).collect();
            let removed = e.removed().iter().map(SubDocHook::guid).collect();
            let loaded = e.loaded().iter().map(SubDocHook::guid).collect();

            event_c.store(Some(Arc::new((added, removed, loaded))));
        });
        let u = Update::decode_v1(
            &doc.transact()
                .encode_state_as_update_v1(&StateVector::default()),
        );
        doc2.transact_mut().apply_update(u.unwrap()).unwrap();
        let mut uuid_3 = {
            let array = doc2.get_or_insert_array("test");
            let tx = doc2.transact();
            let sd_3: SubDocHook = array.get(&tx, 0).unwrap();
            {
                let doc_ref_3 = sd_3.as_ref(&tx);
                assert!(!doc_ref_3.should_load());
                assert!(!doc_ref_3.auto_load());
            }
            sd_3
        };
        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some(Arc::new((vec![uuid_3.guid()], vec![], vec![])))
        );

        // load
        {
            let mut txn = doc2.transact_mut();
            let mut doc_ref_3 = uuid_3.as_mut(&mut txn);
            doc_ref_3.load();
            assert!(doc_ref_3.should_load());
        }
        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some(Arc::new((vec![], vec![], vec![uuid_3.guid()])))
        );
    }

    #[test]
    fn subdoc_auto_load_edge_cases() {
        fn pointer(doc: &Doc) -> usize {
            std::ptr::from_ref(doc) as usize
        }
        let mut doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("test");
        let subdoc_1 = Doc::with_options({
            let mut o = Options::default();
            o.auto_load = true;
            o
        });

        let event = Arc::new(ArcSwapOption::default());
        let event_c = event.clone();
        let _sub = doc.observe_subdocs(move |e| {
            let added = e.added().iter().map(SubDocHook::guid).collect();
            let removed = e.removed().iter().map(SubDocHook::guid).collect();
            let loaded = e.loaded().iter().map(SubDocHook::guid).collect();

            event_c.store(Some(Arc::new((added, removed, loaded))));
        });

        let subdoc_ptr_1 = pointer(&subdoc_1);
        let subdoc_uuid_1 = subdoc_1.guid();
        let mut uuid_1 = {
            let mut txn = doc.transact_mut();
            array.insert(&mut txn, 0, subdoc_1)
        };

        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some(Arc::new((vec![uuid_1.guid()], vec![], vec![uuid_1.guid()])))
        );

        // destroy and check whether lastEvent adds it again to added (it shouldn't)
        {
            let mut txn = doc.transact_mut();
            let subdoc_1 = uuid_1.as_mut(&mut txn);
            assert!(subdoc_1.should_load());
            assert!(subdoc_1.auto_load());
            subdoc_1.destroy();
        }

        let mut uuid_2 = {
            let txn = doc.transact();
            let s: SubDocHook = array.get(&txn, 0).unwrap();
            {
                let subdoc_2 = s.as_ref(&txn);
                assert_ne!(subdoc_ptr_1, pointer(subdoc_2.deref()));
                assert_eq!(subdoc_uuid_1, subdoc_2.guid());
            }
            s
        };

        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some(Arc::new((vec![uuid_2.guid()], vec![uuid_2.guid()], vec![])))
        );

        uuid_2.as_mut(&mut doc.transact_mut()).load();
        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some(Arc::new((vec![], vec![], vec![uuid_2.guid()])))
        );

        // apply from remote
        let mut doc2 = Doc::with_client_id(2);
        let event_c = event.clone();
        let _sub = doc2.observe_subdocs(move |e| {
            let added = e.added().iter().map(SubDocHook::guid).collect();
            let removed = e.removed().iter().map(SubDocHook::guid).collect();
            let loaded = e.loaded().iter().map(SubDocHook::guid).collect();

            event_c.store(Some(Arc::new((added, removed, loaded))));
        });
        let u = Update::decode_v1(
            &doc.transact()
                .encode_state_as_update_v1(&StateVector::default()),
        );
        doc2.transact_mut().apply_update(u.unwrap()).unwrap();
        let uuid_3 = {
            let array = doc2.get_or_insert_array("test");
            let txn = doc2.transact();
            let subdoc_3: SubDocHook = array.get(&txn, 0).unwrap();
            {
                let subdoc = subdoc_3.as_ref(&txn);
                assert!(subdoc.should_load());
                assert!(subdoc.auto_load());
            }
            subdoc_3
        };
        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some(Arc::new((vec![uuid_3.guid()], vec![], vec![uuid_3.guid()])))
        );
    }

    #[test]
    fn to_json() {
        let mut doc = Doc::new();
        let mut txn = doc.transact_mut();
        let text = txn.get_or_insert_text("text");
        let array = txn.get_or_insert_array("array");
        let map = txn.get_or_insert_map("map");
        let xml_fragment = txn.get_or_insert_xml_fragment("xml-fragment");
        let xml_element = xml_fragment.insert(&mut txn, 0, XmlElementPrelim::empty("xml-element"));
        let xml_text = xml_fragment.insert(&mut txn, 0, XmlTextPrelim::new(""));

        text.push(&mut txn, "hello");
        xml_text.push(&mut txn, "world");
        xml_fragment.insert(&mut txn, 0, XmlElementPrelim::empty("div"));
        xml_element.insert(&mut txn, 0, XmlElementPrelim::empty("body"));
        array.insert_range(&mut txn, 0, [1, 2, 3]);
        map.insert(&mut txn, "key1", "value1");

        // sub documents cannot use their parent's transaction
        let mut sub_doc = Doc::new();
        let sub_text = sub_doc.get_or_insert_text("sub-text");
        let mut sub_doc = map.insert(&mut txn, "sub-doc", sub_doc);

        let mut sub_doc = sub_doc.as_mut(&mut txn);
        sub_text.push(&mut sub_doc.transact_mut(), "sample");

        let actual = txn.doc().to_json();
        let expected = any!({
            "text": "hello",
            "array": [1,2,3],
            "map": {
                "key1": "value1",
                "sub-doc": {
                    "sub-text": "sample"
                }
            },
            "xml-fragment": "<div></div>world<xml-element><body></body></xml-element>",
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn apply_snapshot_updates() {
        let update = {
            let mut doc = Doc::with_options(Options {
                client_id: 1,
                skip_gc: true,
                offset_kind: OffsetKind::Utf16,
                ..Options::default()
            });
            let txt = doc.get_or_insert_text("test");
            let mut txn = doc.transact_mut();
            txt.insert(&mut txn, 0, "hello");

            let snap = txn.snapshot();

            txt.insert(&mut txn, 5, " world");

            let mut encoder = EncoderV1::new();
            txn.encode_state_from_snapshot(&snap, &mut encoder).unwrap();
            encoder.to_vec()
        };

        let mut doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
        let str = txt.get_string(&txn);
        assert_eq!(&str, "hello");
    }

    #[test]
    fn out_of_order_updates() {
        let updates = Arc::new(Mutex::new(vec![]));

        let mut d1 = Doc::new();
        let _sub = {
            let updates = updates.clone();
            d1.observe_update_v1(move |_, e| {
                let mut u = updates.lock().unwrap();
                u.push(Update::decode_v1(&e.update).unwrap());
            })
        };

        let map = d1.get_or_insert_map("map");
        map.insert(&mut d1.transact_mut(), "a", 1);
        map.insert(&mut d1.transact_mut(), "a", 1.1);
        map.insert(&mut d1.transact_mut(), "b", 2);

        assert_eq!(map.to_json(&d1.transact()), any!({"a": 1.1, "b": 2}));

        let mut d2 = Doc::new();

        {
            let mut updates = updates.lock().unwrap();
            let u3 = updates.pop().unwrap();
            let u2 = updates.pop().unwrap();
            let u1 = updates.pop().unwrap();
            let mut txn = d2.transact_mut();
            txn.apply_update(u1).unwrap();
            assert!(txn.doc().pending.is_none()); // applied
            txn.apply_update(u3).unwrap();
            assert!(txn.doc().pending.is_some()); // pending update waiting for u2
            txn.apply_update(u2).unwrap();
            assert!(txn.doc().pending.is_none()); // applied after fixing the missing update
        }

        let map = d2.get_or_insert_map("map");
        assert_eq!(map.to_json(&d2.transact()), any!({"a": 1.1, "b": 2}));
    }

    #[test]
    fn encoding_buffer_overflow_errors() {
        assert_matches!(
            Update::decode_v1(&vec![
                0xe4, 0x9c, 0x10, 0x00, 0x05, 0xff, 0xff, 0x05, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
                0x01, 0x00, 0x00, 0x00, 0xed, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0xfe, 0xb8, 0xc2,
                0xe9, 0xad, 0x87, 0xd9, 0x12, 0x00, 0x00, 0x01, 0x01, 0xff, 0xed, 0xf6,
            ]),
            Err(crate::encoding::read::Error::EndOfBuffer(_))
        );

        assert_matches!(
            Update::decode_v2(&vec![
                0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x02, 0x00, 0x00,
                0x16, 0x02, 0x00, 0x00, 0x01, 0xfd, 0xff, 0xff, 0xff, 0xff, 0x7f, 0x00, 0x00,
            ]),
            Err(crate::encoding::read::Error::EndOfBuffer(_))
        );
        assert_matches!(
            Update::decode_v2(&vec![
                0xe4, 0x95, 0x00, 0x00, 0x01, 0x18, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01,
                0x00, 0x00, 0xed, 0x01, 0xbe, 0x82, 0xe3, 0xc3, 0x1c, 0x01, 0x02, 0xe4, 0x95, 0x00,
                0x00, 0x01, 0x18, 0x00, 0x00, 0x01, 0x18, 0x00, 0x00, 0x01, 0x00, 0x01, 0xed, 0x00,
            ]),
            Err(crate::encoding::read::Error::InvalidVarInt)
        );
        assert_matches!(
            Update::decode_v2(&vec![
                0x8f, 0x01, 0x80, 0x00, 0x00, 0x00, 0x01, 0xaa, 0x01, 0x00, 0x01, 0x02, 0x00, 0x00,
                0x16, 0x02, 0x00, 0xe5, 0xc4, 0x43, 0x14, 0xe7, 0xa6, 0x8b, 0x93, 0xae, 0xb5, 0xfd,
                0x5d, 0xe8, 0x26, 0x9a, 0x8a, 0x59, 0x00, 0x31, 0xd5, 0x0f, 0x12, 0x01, 0x30, 0x00,
                0x00, 0x00,
            ]),
            Err(crate::encoding::read::Error::EndOfBuffer(_))
        );
        assert_matches!(
            Update::decode_v2(&vec![
                0x00, 0x01, 0x23, 0x00, 0x00, 0x00, 0x01, 0x02, 0x81, 0x00, 0x00, 0x10, 0x00, 0xc7,
                0xdc, 0x00, 0xc4, 0x7a, 0x80, 0x00, 0x41, 0xab, 0xea, 0xd6, 0x00, 0x01, 0x00, 0x00,
                0x01, 0x00, 0x00, 0x84, 0x00, 0x00, 0x10, 0xff, 0xc7, 0xdc, 0xff, 0x00, 0x00, 0x00,
            ]),
            Err(crate::encoding::read::Error::EndOfBuffer(_))
        );
    }

    #[test]
    fn observe_after_transaction() {
        let mut d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("text");

        let e = Arc::new(ArcSwapOption::default());
        let e_copy = e.clone();
        d1.observe_after_transaction_with("key", move |txn| {
            e_copy.swap(Some(Arc::new((
                txn.before_state().clone(),
                txn.after_state().clone(),
                txn.delete_set().cloned().unwrap_or_default(),
            ))));
        });

        txt1.insert(&mut d1.transact_mut(), 0, "hello world");
        let actual = e.swap(None);
        assert_eq!(
            actual,
            Some(Arc::new((
                StateVector::default(),
                StateVector::from_iter([(1, 11)]),
                DeleteSet::default()
            )))
        );

        txt1.remove_range(&mut d1.transact_mut(), 2, 7);
        let actual = e.swap(None);
        assert_eq!(
            actual,
            Some(Arc::new((
                StateVector::from_iter([(1, 11)]),
                StateVector::from_iter([(1, 11)]),
                {
                    let mut ds = DeleteSet::new();
                    ds.insert(ID::new(1, 2), 7);
                    ds
                }
            )))
        );

        d1.unobserve_after_transaction("key");

        txt1.insert(&mut d1.transact_mut(), 4, " the door");
        let actual = e.swap(None);
        assert!(actual.is_none());
    }

    fn init_test_data<const N: usize>(txn: &mut TransactionMut, data: [&str; N]) -> TextRef {
        let map = txn.get_or_insert_map("map");
        let txt = map.insert(txn, "text", TextPrelim::default());
        for ch in data {
            txt.insert(txn, 0, ch);
        }
        txt
    }

    #[test]
    fn force_gc() {
        let mut doc = Doc::with_options(Options {
            client_id: 1,
            skip_gc: true,
            ..Default::default()
        });
        let map = doc.get_or_insert_map("map");

        {
            // create some initial data
            let mut txn = doc.transact_mut();
            init_test_data(&mut txn, ["c", "b", "a"]);

            // drop nested type
            map.remove(&mut txn, "text");
        }

        // verify that skip_gc works and we have access to an original text content
        {
            let txn = doc.transact();
            let mut i = 1;
            for c in ["c", "b", "a"] {
                let block = txn
                    .doc()
                    .blocks
                    .get_block(&ID::new(1, i))
                    .unwrap()
                    .as_item()
                    .unwrap();
                assert!(block.is_deleted(), "`abc` should be marked as deleted");
                assert_eq!(&block.content, &ItemContent::String(c.into()));
                i += 1;
            }
        }

        // force GC and check if original content is hard deleted
        doc.transact_mut().gc(None);

        let txn = doc.transact();
        let block = txn.doc().blocks.get_block(&ID::new(1, 1)).unwrap();
        assert_eq!(block.len(), 3, "GCed blocks should be squashed");
        assert!(block.is_deleted(), "`abc` should be deleted");
        assert_matches!(&block, &BlockCell::GC(_));
    }

    #[test]
    fn force_gc_with_delete_set() {
        let doc = Doc::with_options(Options {
            client_id: 1,
            skip_gc: true,
            ..Default::default()
        });
        let m0 = doc.get_or_insert_map("map");
        let s1 = {
            let mut tx = doc.transact_mut();
            let t1 = init_test_data(&mut tx, ["c", "b", "a"]); // <1#1..3>
            assert_eq!(t1.get_string(&tx), "abc");
            tx.snapshot()
        };

        let s2 = {
            let mut tx = doc.transact_mut();
            let t2 = init_test_data(&mut tx, ["f", "e", "d"]); // <1#5..7>
            assert_eq!(t2.get_string(&tx), "def");
            tx.snapshot()
        };

        let s3 = {
            let mut tx = doc.transact_mut();
            let t3 = init_test_data(&mut tx, ["i", "h", "g"]); // <1#9..11>
            assert_eq!(t3.get_string(&tx), "ghi");
            tx.snapshot()
        };

        // restore data to s1
        {
            let doc_restored = restore_from_snapshot(&doc, &s1).unwrap();
            let txn = doc_restored.transact();
            let m0_restored = txn.get_map("map").unwrap();
            let txt = m0_restored
                .get(&txn, "text")
                .unwrap()
                .cast::<TextRef>()
                .unwrap();
            assert_eq!(txt.get_string(&txn), "abc");
        }

        // verify that blocks 'abc' are not GCed and available
        {
            let txn = doc.transact();
            let mut i = 1;
            for c in ["c", "b", "a"] {
                let block = txn
                    .store()
                    .blocks
                    .get_block(&ID::new(1, i))
                    .unwrap()
                    .as_item()
                    .unwrap();
                assert!(block.is_deleted(), "`abc` should be marked as deleted");
                assert_eq!(&block.content, &ItemContent::String(c.into()));
                i += 1;
            }
        }

        // garbage collect anything below s2
        doc.transact_mut().gc(Some(&s2.delete_set));

        // verify that we GC 'abc' blocks and compressed them
        let txn = doc.transact();
        let block = txn.store().blocks.get_block(&ID::new(1, 1)).unwrap();
        assert_eq!(
            block,
            &BlockCell::GC(GC::new(1, 3)),
            "block should be GCed & compressed"
        );

        // try to restore data to s1 again
        let doc_restored = restore_from_snapshot(&doc, &s1).unwrap();
        let txn = doc_restored.transact();
        let m0_restored = txn.get_map("map").unwrap();
        let txt = m0_restored.get(&txn, "text");
        assert!(
            txt.is_none(),
            "we restored snapshot s1, but it's content should be already GCed"
        );

        // verify that blocks from s2 are still accessible
        {
            let doc_restored = restore_from_snapshot(&doc, &s2).unwrap();
            let txn = doc_restored.transact();
            let m0_restored = txn.get_map("map").unwrap();
            let txt = m0_restored
                .get(&txn, "text")
                .unwrap()
                .cast::<TextRef>()
                .unwrap();
            assert_eq!(txt.get_string(&txn), "def");
        }
    }

    fn restore_from_snapshot(doc: &Doc, snapshot: &crate::Snapshot) -> Result<Doc, Error> {
        let mut encoder = EncoderV1::new();
        doc.transact()
            .encode_state_from_snapshot(&snapshot, &mut encoder)?;
        let doc = Doc::new();
        doc.transact_mut()
            .apply_update(Update::decode_v1(&encoder.to_vec()).unwrap())
            .unwrap();
        Ok(doc)
    }

    #[test]
    fn uuid_generation() {
        let guid = uuid_v4();
        let uuid = uuid::Uuid::parse_str(&guid).unwrap();
        assert_eq!(&*uuid.to_string(), &*guid);
    }

    #[test]
    fn pending_delete_out_of_order() {
        // Test for bug fix: pending deletes should be recorded when the target client
        // doesn't exist in the block store yet
        let mut doc = Doc::new();

        let (upd1, upd2) = {
            let mut doc2 = Doc::new();
            let mut tx = doc2.transact_mut();
            let text = tx.get_or_insert_text("example");
            text.insert(&mut tx, 0, "foo");
            let upd1 = tx.encode_update_v2();
            drop(tx);

            let mut tx = doc2.transact_mut();
            let text = tx.get_or_insert_text("example");
            text.remove_range(&mut tx, 0, 1);
            let upd2 = tx.encode_update_v2();
            assert_eq!(text.get_string(&tx), "oo");
            drop(tx);

            (upd1, upd2)
        };

        // Apply delete BEFORE insert (out of order)
        let mut tx = doc.transact_mut();
        tx.apply_update(Update::decode_v2(&upd2).unwrap()).unwrap();

        // Delete should be pending since the blocks don't exist yet
        assert!(tx.has_missing_updates(), "Delete should be pending");

        // Apply insert
        tx.apply_update(Update::decode_v2(&upd1).unwrap()).unwrap();

        // After insert arrives, pending delete should be auto-applied
        let text = tx.get_or_insert_text("example");
        assert_eq!(
            text.get_string(&tx),
            "oo",
            "Pending delete should have been applied"
        );
    }
}
