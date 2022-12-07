use crate::block::{Block, BlockPtr, ClientID, ItemContent, Prelim};
use crate::event::{AfterTransactionEvent, SubdocsEvent, UpdateEvent};
use crate::store::{Store, StoreRef, WeakStoreRef};
use crate::transaction::{Transaction, TransactionMut};
use crate::types::{
    Branch, BranchPtr, ToJson, TYPE_REFS_ARRAY, TYPE_REFS_MAP, TYPE_REFS_TEXT,
    TYPE_REFS_XML_ELEMENT, TYPE_REFS_XML_FRAGMENT, TYPE_REFS_XML_TEXT,
};
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::utils::OptionExt;
use crate::{
    uuid_v4, ArrayRef, MapRef, ReadTxn, SubscriptionId, TextRef, Uuid, WriteTxn, XmlElementRef,
    XmlFragmentRef, XmlTextRef,
};
use atomic_refcell::{AtomicRef, AtomicRefMut, BorrowError, BorrowMutError};
use lib0::any::Any;
use lib0::error::Error;
use rand::Rng;
use std::collections::HashMap;
use std::fmt::Formatter;
use std::ops::DerefMut;
use std::sync::Arc;
use thiserror::Error;

/// A Yrs document type. Documents are most important units of collaborative resources management.
/// All shared collections live within a scope of their corresponding documents. All updates are
/// generated on per document basis (rather than individual shared type). All operations on shared
/// collections happen via [Transaction], which lifetime is also bound to a document.
///
/// Document manages so called root types, which are top-level shared types definitions (as opposed
/// to recursively nested types).
///
/// A basic workflow sample:
///
/// ```
/// use yrs::{Doc, ReadTxn, StateVector, Text, Transact, Update};
/// use yrs::updates::decoder::Decode;
/// use yrs::updates::encoder::Encode;
///
/// let doc = Doc::new();
/// let root = doc.get_or_insert_text("root-type-name");
/// let mut txn = doc.transact_mut(); // all Yrs operations happen in scope of a transaction
/// root.push(&mut txn, "hello world"); // append text to our collaborative document
///
/// // in order to exchange data with other documents we first need to create a state vector
/// let remote_doc = Doc::new();
/// let mut remote_txn = remote_doc.transact_mut();
/// let state_vector = remote_txn.state_vector().encode_v1();
///
/// // now compute a differential update based on remote document's state vector
/// let update = txn.encode_diff_v1(&StateVector::decode_v1(&state_vector).unwrap());
///
/// // both update and state vector are serializable, we can pass the over the wire
/// // now apply update to a remote document
/// remote_txn.apply_update(Update::decode_v1(update.as_slice()).unwrap());
/// ```
#[derive(Debug, Clone)]
pub struct Doc {
    store: StoreRef,
}

unsafe impl Send for Doc {}
unsafe impl Sync for Doc {}

impl Doc {
    /// Creates a new document with a randomized client identifier.
    pub fn new() -> Self {
        Self::with_options(Options::default())
    }

    /// Creates a new document with a specified `client_id`. It's up to a caller to guarantee that
    /// this identifier is unique across all communicating replicas of that document.
    pub fn with_client_id(client_id: ClientID) -> Self {
        Self::with_options(Options::with_client_id(client_id))
    }

    pub fn with_options(options: Options) -> Self {
        Doc {
            store: Store::new(options).into(),
        }
    }

    pub(crate) fn subdoc(parent: BlockPtr, options: Options) -> Self {
        let mut store = Store::new(options);
        store.parent = Some(parent);
        Doc {
            store: store.into(),
        }
    }

    /// A unique client identifier, that's also a unique identifier of current document replica
    /// and it's subdocuments.
    pub fn client_id(&self) -> ClientID {
        self.options().client_id
    }

    /// A globally unique identifier, that's also a unique identifier of current document replica,
    /// and unlike [Doc::client_id] it's not shared with its subdocuments.
    pub fn guid(&self) -> &Uuid {
        &self.options().guid
    }

    /// Returns config options of this [Doc] instance.
    pub fn options(&self) -> &Options {
        self.store.options()
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
    pub fn get_or_insert_text(&self, name: &str) -> TextRef {
        let mut r = self.store.try_borrow_mut().expect(
            "tried to get a root level type while another transaction on the document is open",
        );
        let mut c = r.get_or_create_type(name, None, TYPE_REFS_TEXT);
        c.store = Some(self.store.weak_ref());
        TextRef::from(c)
    }

    /// Returns a [MapRef] data structure stored under a given `name`. Maps are used to store key-value
    /// pairs associated together. These values can be primitive data (similar but not limited to
    /// a JavaScript Object Notation) as well as other shared types (Yrs maps, arrays, text
    /// structures etc.), enabling to construct a complex recursive tree structures.
    ///
    /// If no structure under defined `name` existed before, it will be created and returned
    /// instead.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as a map (in such case a map component of complex data type will be
    /// interpreted as native map).
    pub fn get_or_insert_map(&self, name: &str) -> MapRef {
        let mut r = self.store.try_borrow_mut().expect(
            "tried to get a root level type while another transaction on the document is open",
        );
        let mut c = r.get_or_create_type(name, None, TYPE_REFS_MAP);
        c.store = Some(self.store.weak_ref());
        MapRef::from(c)
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
    pub fn get_or_insert_array(&self, name: &str) -> ArrayRef {
        let mut r = self.store.try_borrow_mut().expect(
            "tried to get a root level type while another transaction on the document is open",
        );
        let mut c = r.get_or_create_type(name, None, TYPE_REFS_ARRAY);
        c.store = Some(self.store.weak_ref());
        ArrayRef::from(c)
    }

    /// Returns a [XmlFragmentRef] data structure stored under a given `name`. XML elements represent
    /// nodes of XML document. They can contain attributes (key-value pairs, both of string type)
    /// as well as other nested XML elements or text values, which are stored in their insertion
    /// order.
    ///
    /// If no structure under defined `name` existed before, it will be created and returned
    /// instead.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as a XML element (in such case a map component of complex data type will be
    /// interpreted as map of its attributes, while a sequence component - as a list of its child
    /// XML nodes).
    pub fn get_or_insert_xml_fragment(&self, name: &str) -> XmlFragmentRef {
        let mut r = self.store.try_borrow_mut().expect(
            "tried to get a root level type while another transaction on the document is open",
        );
        let mut c = r.get_or_create_type(name, None, TYPE_REFS_XML_FRAGMENT);
        c.store = Some(self.store.weak_ref());
        XmlFragmentRef::from(c)
    }

    /// Returns a [XmlElementRef] data structure stored under a given `name`. XML elements represent
    /// nodes of XML document. They can contain attributes (key-value pairs, both of string type)
    /// as well as other nested XML elements or text values, which are stored in their insertion
    /// order.
    ///
    /// If no structure under defined `name` existed before, it will be created and returned
    /// instead.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as a XML element (in such case a map component of complex data type will be
    /// interpreted as map of its attributes, while a sequence component - as a list of its child
    /// XML nodes).
    pub fn get_or_insert_xml_element(&self, name: &str) -> XmlElementRef {
        let mut r = self.store.try_borrow_mut().expect(
            "tried to get a root level type while another transaction on the document is open",
        );
        let mut c = r.get_or_create_type(name, Some(name.into()), TYPE_REFS_XML_ELEMENT);
        c.store = Some(self.store.weak_ref());
        XmlElementRef::from(c)
    }

    /// Returns a [XmlTextRef] data structure stored under a given `name`. Text structures are used
    /// for collaborative text editing: they expose operations to append and remove chunks of text,
    /// which are free to execute concurrently by multiple peers over remote boundaries.
    ///
    /// If no structure under defined `name` existed before, it will be created and returned
    /// instead.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as a text (in such case a sequence component of complex data type will be
    /// interpreted as a list of text chunks).
    pub fn get_or_insert_xml_text(&self, name: &str) -> XmlTextRef {
        let mut r = self.store.try_borrow_mut().expect(
            "tried to get a root level type while another transaction on the document is open",
        );
        let mut c = r.get_or_create_type(name, None, TYPE_REFS_XML_TEXT);
        c.store = Some(self.store.weak_ref());
        XmlTextRef::from(c)
    }

    /// Subscribe callback function for any changes performed within transaction scope. These
    /// changes are encoded using lib0 v1 encoding and can be decoded using [Update::decode_v1] if
    /// necessary or passed to remote peers right away. This callback is triggered on function
    /// commit.
    ///
    /// Returns a subscription, which will unsubscribe function when dropped.
    pub fn observe_update_v1<F>(&self, f: F) -> Result<UpdateSubscription, BorrowMutError>
    where
        F: Fn(&TransactionMut, &UpdateEvent) -> () + 'static,
    {
        let mut r = self.store.try_borrow_mut()?;
        let events = r.events.get_or_init();
        events.observe_update_v1(f)
    }

    /// Manually unsubscribes from a callback used in [Doc::observe_update_v1] method.
    pub fn unobserve_update_v1(&self, subscription_id: SubscriptionId) {
        let r = self.store.try_borrow().unwrap();
        if let Some(events) = r.events.as_ref() {
            events.unobserve_update_v1(subscription_id)
        }
    }

    /// Subscribe callback function for any changes performed within transaction scope. These
    /// changes are encoded using lib0 v1 encoding and can be decoded using [Update::decode_v2] if
    /// necessary or passed to remote peers right away. This callback is triggered on function
    /// commit.
    ///
    /// Returns a subscription, which will unsubscribe function when dropped.
    pub fn observe_update_v2<F>(&self, f: F) -> Result<UpdateSubscription, BorrowMutError>
    where
        F: Fn(&TransactionMut, &UpdateEvent) -> () + 'static,
    {
        let mut r = self.store.try_borrow_mut()?;
        let events = r.events.get_or_init();
        events.observe_update_v2(f)
    }

    /// Manually unsubscribes from a callback used in [Doc::observe_update_v1] method.
    pub fn unobserve_update_v2(&self, subscription_id: SubscriptionId) {
        let r = self.store.try_borrow().unwrap();
        if let Some(events) = r.events.as_ref() {
            events.unobserve_update_v2(subscription_id)
        }
    }

    /// Subscribe callback function to updates on the `Doc`. The callback will receive state updates and
    /// deletions when a document transaction is committed.
    pub fn observe_transaction_cleanup<F>(
        &self,
        f: F,
    ) -> Result<AfterTransactionSubscription, BorrowMutError>
    where
        F: Fn(&TransactionMut, &AfterTransactionEvent) -> () + 'static,
    {
        let mut r = self.store.try_borrow_mut()?;
        let events = r.events.get_or_init();
        events.observe_transaction_cleanup(f)
    }

    /// Cancels the transaction cleanup callback associated with the `subscription_id`
    pub fn unobserve_transaction_cleanup(&self, subscription_id: SubscriptionId) {
        let r = self.store.try_borrow().unwrap();
        if let Some(events) = r.events.as_ref() {
            events.unobserve_transaction_cleanup(subscription_id)
        }
    }

    /// Subscribe callback function, that will be called whenever a subdocuments inserted in this
    /// [Doc] will request a load.
    pub fn observe_subdocs<F>(&self, f: F) -> Result<SubdocsSubscription, BorrowMutError>
    where
        F: Fn(&TransactionMut, &SubdocsEvent) -> () + 'static,
    {
        let mut r = self.store.try_borrow_mut()?;
        let events = r.events.get_or_init();
        events.observe_subdocs(f)
    }

    /// Cancels the subscription created previously using [Doc::observe_subdocs].
    pub fn unobserve_subdocs(&self, subscription_id: SubscriptionId) {
        let r = self.store.try_borrow().unwrap();
        if let Some(events) = r.events.as_ref() {
            events.unobserve_subdocs(subscription_id)
        }
    }

    /// Subscribe callback function, that will be called whenever a [DocRef::destroy] has been called.
    pub fn observe_destroy<F>(&self, f: F) -> Result<DestroySubscription, BorrowMutError>
    where
        F: Fn(&TransactionMut, &Doc) -> () + 'static,
    {
        let mut r = self.store.try_borrow_mut()?;
        let events = r.events.get_or_init();
        events.observe_destroy(f)
    }

    /// Cancels the subscription created previously using [Doc::observe_subdocs].
    pub fn unobserve_destroy(&self, subscription_id: SubscriptionId) {
        let r = self.store.try_borrow().unwrap();
        if let Some(events) = r.events.as_ref() {
            events.unobserve_destroy(subscription_id)
        }
    }

    /// Sends a load request to a parent document. Works only if current document is a sub-document
    /// of an another document.
    pub fn load<T>(&self, parent_txn: &mut T)
    where
        T: WriteTxn,
    {
        let mut txn = self.transact_mut();
        if txn.store.is_subdoc() {
            if !txn.store.options.should_load {
                parent_txn
                    .subdocs_mut()
                    .loaded
                    .insert(self.addr(), self.clone());
            }
        }
        txn.store.options.should_load = true;
    }

    /// Starts destroy procedure for a current document, triggering an "destroy" callback and
    /// invalidating all event callback subscriptions.
    pub fn destroy<T>(&mut self, parent_txn: &mut T)
    where
        T: WriteTxn,
    {
        let mut txn = self.transact_mut();
        let store = txn.store_mut();
        let subdocs: Vec<_> = store.subdocs.values().cloned().collect();
        for mut subdoc in subdocs {
            subdoc.destroy(&mut txn);
        }
        if let Some(mut ptr) = txn.store.parent.take() {
            let parent_ref = ptr.clone();
            if let Block::Item(item) = ptr.deref_mut() {
                let is_deleted = item.is_deleted();
                if let ItemContent::Doc(_, content) = &mut item.content {
                    let mut options = content.options().clone();
                    options.should_load = false;
                    let new_ref = Doc::subdoc(parent_ref, options);
                    if !is_deleted {
                        parent_txn
                            .subdocs_mut()
                            .added
                            .insert(new_ref.addr(), new_ref.clone());
                    }
                    parent_txn
                        .subdocs_mut()
                        .removed
                        .insert(new_ref.addr(), new_ref.clone());

                    *content = new_ref;
                }
            }
        }
        // super.destroy(): cleanup the events
        if let Some(events) = txn.store_mut().events.take() {
            if let Some(handler) = events.destroy_events.as_ref() {
                for cb in handler.callbacks() {
                    cb(&txn, self)
                }
            }
        }
    }

    pub fn parent_doc(&self) -> Option<Doc> {
        let store = unsafe { self.store.0.as_ptr().as_ref() }.unwrap();
        if let Some(Block::Item(item)) = store.parent.as_deref() {
            if let ItemContent::Doc(Some(parent_ref), _) = &item.content {
                let store = parent_ref.0.upgrade()?;
                return Some(Doc {
                    store: StoreRef(store),
                });
            }
        }

        None
    }

    pub(crate) fn ptr_eq(a: &Doc, b: &Doc) -> bool {
        Arc::ptr_eq(&a.store.0, &b.store.0)
    }

    pub(crate) fn addr(&self) -> DocAddr {
        DocAddr::new(&self)
    }

    pub fn weak_ref(&self) -> WeakStoreRef {
        self.store.weak_ref()
    }
}

impl PartialEq for Doc {
    fn eq(&self, other: &Self) -> bool {
        self.options().guid == other.options().guid
    }
}

impl std::fmt::Display for Doc {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let options = self.options();
        write!(f, "Doc(id: {}, guid: {})", options.client_id, options.guid)
    }
}

pub type UpdateSubscription = crate::Subscription<Arc<dyn Fn(&TransactionMut, &UpdateEvent) -> ()>>;

pub type AfterTransactionSubscription =
    crate::Subscription<Arc<dyn Fn(&TransactionMut, &AfterTransactionEvent) -> ()>>;

pub type SubdocsSubscription =
    crate::Subscription<Arc<dyn Fn(&TransactionMut, &SubdocsEvent) -> ()>>;

pub type DestroySubscription = crate::Subscription<Arc<dyn Fn(&TransactionMut, &Doc) -> ()>>;

impl Default for Doc {
    fn default() -> Self {
        Doc::new()
    }
}

impl ToJson for Doc {
    fn to_json<T: ReadTxn>(&self, txn: &T) -> Any {
        let mut m = HashMap::new();
        for (key, value) in txn.root_refs() {
            m.insert(key.to_string(), value.to_json(txn));
        }
        Any::Map(Box::new(m))
    }
}

/// Configuration options of [Doc] instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Globally unique 53-bit long client identifier.
    pub client_id: ClientID,
    /// A globally unique identifier for this document.
    pub guid: Uuid,
    /// Associate this document with a collection. This only plays a role if your provider has
    /// a concept of collection.
    pub collection_id: Option<String>,
    /// How to we count offsets and lengths used in text operations.
    pub offset_kind: OffsetKind,
    /// Determines if transactions commits should try to perform GC-ing of deleted items.
    pub skip_gc: bool,
    /// If a subdocument, automatically load document. If this is a subdocument, remote peers will
    /// load the document as well automatically.
    pub auto_load: bool,
    /// Whether the document should be synced by the provider now.
    /// This is toggled to true when you call ydoc.load()
    pub should_load: bool,
}

impl Options {
    pub fn with_client_id(client_id: ClientID) -> Self {
        Options {
            client_id,
            guid: uuid_v4(&mut rand::thread_rng()),
            collection_id: None,
            offset_kind: OffsetKind::Bytes,
            skip_gc: false,
            auto_load: false,
            should_load: true,
        }
    }

    pub fn with_guid_and_client_id(guid: Uuid, client_id: ClientID) -> Self {
        Options {
            client_id,
            guid,
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
            OffsetKind::Utf32 => 2,
        };
        m.insert("encoding".to_owned(), Any::BigInt(encoding));
        m.insert("autoLoad".to_owned(), self.auto_load.into());
        m.insert("shouldLoad".to_owned(), self.should_load.into());
        Any::Map(Box::new(m))
    }
}

impl Default for Options {
    fn default() -> Self {
        let mut rng = rand::thread_rng();
        let client_id: u32 = rng.gen();
        let uuid = uuid_v4(&mut rng);
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
        options.guid = guid.into();

        if let Any::Map(opts) = decoder.read_any()? {
            for (k, v) in opts.iter() {
                match (k.as_str(), v) {
                    ("gc", Any::Bool(gc)) => options.skip_gc = !*gc,
                    ("autoLoad", Any::Bool(auto_load)) => options.auto_load = *auto_load,
                    ("collectionId", Any::String(cid)) => {
                        options.collection_id = Some(cid.to_string())
                    }
                    ("encoding", Any::BigInt(1)) => options.offset_kind = OffsetKind::Bytes,
                    ("encoding", Any::BigInt(2)) => options.offset_kind = OffsetKind::Utf32,
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
    /// Compute editable strings length and offset using Unicode code points number.
    Utf32,
}

pub trait Transact {
    /// Creates a transaction used for all kind of block store operations.
    /// Transaction cleanups & calling event handles happen when the transaction struct is dropped.
    fn try_transact(&self) -> Result<Transaction, TransactionAcqError>;

    /// Creates a transaction used for all kind of block store operations.
    /// Transaction cleanups & calling event handles happen when the transaction struct is dropped.
    fn try_transact_mut(&self) -> Result<TransactionMut, TransactionAcqError>;

    /// Creates a transaction used for all kind of block store operations.
    /// Transaction cleanups & calling event handles happen when the transaction struct is dropped.
    fn transact(&self) -> Transaction {
        self.try_transact().unwrap()
    }

    /// Creates a transaction used for all kind of block store operations.
    /// Transaction cleanups & calling event handles happen when the transaction struct is dropped.
    fn transact_mut(&self) -> TransactionMut {
        self.try_transact_mut().unwrap()
    }
}

impl Transact for Doc {
    fn try_transact(&self) -> Result<Transaction, TransactionAcqError> {
        Ok(Transaction::new(self.store.try_borrow()?))
    }

    fn try_transact_mut(&self) -> Result<TransactionMut, TransactionAcqError> {
        Ok(TransactionMut::new(self.store.try_borrow_mut()?))
    }
}

impl Transact for Branch {
    fn try_transact<'a>(&'a self) -> Result<Transaction<'a>, TransactionAcqError> {
        let store = self.store.as_ref().unwrap();
        if let Some(store) = store.0.upgrade() {
            let store_ref = store.try_borrow()?;
            let store_ref: AtomicRef<'a, Store> = unsafe { std::mem::transmute(store_ref) };
            Ok(Transaction::new(store_ref))
        } else {
            Err(TransactionAcqError::DocumentDropped)
        }
    }

    fn try_transact_mut<'a>(&'a self) -> Result<TransactionMut<'a>, TransactionAcqError> {
        let store = self.store.as_ref().unwrap();
        if let Some(store) = store.0.upgrade() {
            let store_ref = store.try_borrow_mut()?;
            let store_ref: AtomicRefMut<'a, Store> = unsafe { std::mem::transmute(store_ref) };
            Ok(TransactionMut::new(store_ref))
        } else {
            Err(TransactionAcqError::DocumentDropped)
        }
    }
}

#[derive(Error, Debug)]
pub enum TransactionAcqError {
    #[error("Failed to acquire read-only transaction. Drop read-write transaction and retry.")]
    SharedAcqFailed(BorrowError),
    #[error("Failed to acquire read-write transaction. Drop other transactions and retry.")]
    ExclusiveAcqFailed(BorrowMutError),
    #[error("All references to a parent document containing this structure has been dropped.")]
    DocumentDropped,
}

impl From<BorrowError> for TransactionAcqError {
    fn from(e: BorrowError) -> Self {
        TransactionAcqError::SharedAcqFailed(e)
    }
}

impl From<BorrowMutError> for TransactionAcqError {
    fn from(e: BorrowMutError) -> Self {
        TransactionAcqError::ExclusiveAcqFailed(e)
    }
}

impl<T> Transact for T
where
    T: AsRef<Branch>,
{
    fn try_transact(&self) -> Result<Transaction, TransactionAcqError> {
        let branch = self.as_ref();
        branch.try_transact()
    }

    fn try_transact_mut(&self) -> Result<TransactionMut, TransactionAcqError> {
        let branch = self.as_ref();
        branch.try_transact_mut()
    }
}

impl Prelim for Doc {
    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        if self.parent_doc().is_some() {
            panic!("Cannot integrate the document, because it's already being used as a sub-document elsewhere");
        }
        (ItemContent::Doc(None, self), None)
    }

    fn integrate(self, _txn: &mut TransactionMut, _inner_ref: BranchPtr) {}
}

/// For a Yjs compatibility reasons we expect subdocuments to be compared based on their reference
/// equality. This concept however doesn't really exists in Rust. Therefore we use a store reference
/// instead and specialize it for this single scenario.
#[repr(transparent)]
#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub(crate) struct DocAddr(usize);

impl DocAddr {
    pub fn new(doc: &Doc) -> Self {
        let ptr = Arc::as_ptr(&doc.store.0);
        DocAddr(ptr as usize)
    }
}

#[cfg(test)]
mod test {
    use crate::block::{Block, ItemContent};
    use crate::test_utils::exchange_updates;
    use crate::transaction::{ReadTxn, TransactionMut};
    use crate::types::ToJson;
    use crate::update::Update;
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::{Encode, Encoder, EncoderV1};
    use crate::{
        Array, ArrayPrelim, DeleteSet, Doc, GetString, Map, Options, StateVector, SubscriptionId,
        Text, Transact, Uuid,
    };
    use lib0::any::Any;
    use std::cell::{Cell, RefCell, RefMut};
    use std::collections::BTreeSet;
    use std::rc::Rc;

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
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("type");
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v1(update).unwrap());

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
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("type");
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v2(update).unwrap());

        let actual = txt.get_string(&txn);
        assert_eq!(actual, "210".to_owned());
    }

    #[test]
    fn encode_basic() {
        let doc = Doc::with_client_id(1490905955);
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
        let d1 = Doc::new();
        let txt = d1.get_or_insert_text("test");
        let mut t1 = d1.transact_mut();
        // Question: why YText.insert uses positions of blocks instead of actual cursor positions
        // in text as seen by user?
        txt.insert(&mut t1, 0, "hello");
        txt.insert(&mut t1, 5, " ");
        txt.insert(&mut t1, 6, "world");

        assert_eq!(txt.get_string(&t1), "hello world".to_string());

        // create document at B
        let d2 = Doc::new();
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
        let pending = update.integrate(&mut t2);

        assert!(pending.0.is_none());
        assert!(pending.1.is_none());

        // check if B sees the same thing that A does
        assert_eq!(txt.get_string(&t1), "hello world".to_string());
    }

    #[test]
    fn on_update() {
        let counter = Rc::new(Cell::new(0));
        let doc = Doc::new();
        let doc2 = Doc::new();
        let c = counter.clone();
        let sub = doc2.observe_update_v1(move |_: &TransactionMut, e| {
            let u = Update::decode_v1(&e.update).unwrap();
            for block in u.blocks.blocks() {
                c.set(c.get() + block.len());
            }
        });
        let txt = doc.get_or_insert_text("test");
        let mut txn = doc.transact_mut();
        {
            txt.insert(&mut txn, 0, "abc");
            let mut txn2 = doc2.transact_mut();
            let sv = txn2.state_vector().encode_v1();
            let u = txn.encode_diff_v1(&StateVector::decode_v1(sv.as_slice()).unwrap());
            txn2.apply_update(Update::decode_v1(u.as_slice()).unwrap());
        }
        assert_eq!(counter.get(), 3); // update has been propagated

        drop(sub);

        {
            txt.insert(&mut txn, 3, "de");
            let mut txn2 = doc2.transact_mut();
            let sv = txn2.state_vector().encode_v1();
            let u = txn.encode_diff_v1(&StateVector::decode_v1(sv.as_slice()).unwrap());
            txn2.apply_update(Update::decode_v1(u.as_slice()).unwrap());
        }
        assert_eq!(counter.get(), 3); // since subscription has been dropped, update was not propagated
    }

    #[test]
    fn pending_update_integration() {
        let doc = Doc::new();
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
            txn.apply_update(u);
        }
        assert_eq!(txt.get_string(&txt.transact()), "abcd".to_string());
    }

    #[test]
    fn ypy_issue_32() {
        let d1 = Doc::with_client_id(1971027812);
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
            d1.transact_mut().apply_update(u);
        }

        assert_eq!("a", source_1.get_string(&source_1.transact()));

        let d2 = Doc::new();
        let source_2 = d2.get_or_insert_text("source");
        let state_2 = d2.transact().state_vector().encode_v1();
        let update = d1
            .transact()
            .encode_state_as_update_v1(&StateVector::decode_v1(&state_2).unwrap());
        let update = Update::decode_v1(&update).unwrap();
        d2.transact_mut().apply_update(update);

        assert_eq!("a", source_2.get_string(&source_2.transact()));

        let update = Update::decode_v1(&[
            1, 2, 201, 210, 153, 56, 5, 132, 228, 254, 237, 171, 7, 0, 1, 98, 168, 201, 210, 153,
            56, 4, 1, 120, 0,
        ])
        .unwrap();
        d1.transact_mut().apply_update(update);
        assert_eq!("ab", source_1.get_string(&source_1.transact()));

        let d3 = Doc::new();
        let source_3 = d3.get_or_insert_text("source");
        let state_3 = d3.transact().state_vector().encode_v1();
        let state_3 = StateVector::decode_v1(&state_3).unwrap();
        let update = d1.transact().encode_state_as_update_v1(&state_3);
        let update = Update::decode_v1(&update).unwrap();
        d3.transact_mut().apply_update(update);

        assert_eq!("ab", source_3.get_string(&source_3.transact()));
    }

    #[test]
    fn observe_transaction_cleanup() {
        // Setup
        let doc = Doc::new();
        let text = doc.get_or_insert_text("test");
        let before_state = Rc::new(Cell::new(StateVector::default()));
        let after_state = Rc::new(Cell::new(StateVector::default()));
        let delete_set = Rc::new(Cell::new(DeleteSet::default()));
        // Create interior mutable references for the callback.
        let before_ref = Rc::clone(&before_state);
        let after_ref = Rc::clone(&after_state);
        let delete_ref = Rc::clone(&delete_set);
        // Subscribe callback

        let sub: SubscriptionId = doc
            .observe_transaction_cleanup(move |_: &TransactionMut, event| {
                before_ref.set(event.before_state.clone());
                after_ref.set(event.after_state.clone());
                delete_ref.set(event.delete_set.clone());
            })
            .unwrap()
            .into();

        {
            let mut txn = doc.transact_mut();

            // Update the document
            text.insert(&mut txn, 0, "abc");
            text.remove_range(&mut txn, 1, 2);
            txn.commit();

            // Compare values
            assert_eq!(before_state.take(), txn.before_state);
            assert_eq!(after_state.take(), txn.after_state);
            assert_eq!(delete_set.take(), txn.delete_set);
        }

        // Ensure that the subscription is successfully dropped.
        doc.unobserve_transaction_cleanup(sub);
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "should not update");
        txn.commit();
        assert_ne!(after_state.take(), txn.after_state);
    }

    #[test]
    fn partially_duplicated_update() {
        let d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("text");
        txt1.insert(&mut d1.transact_mut(), 0, "hello");
        let u = d1
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        let d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("text");
        d2.transact_mut()
            .apply_update(Update::decode_v1(&u).unwrap());

        txt1.insert(&mut d1.transact_mut(), 5, "world");
        let u = d1
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        d2.transact_mut()
            .apply_update(Update::decode_v1(&u).unwrap());

        assert_eq!(
            txt1.get_string(&txt1.transact()),
            txt2.get_string(&txt2.transact())
        );
    }

    #[test]
    fn incremental_observe_update() {
        const INPUT: &'static str = "hello";

        let d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("text");
        let acc = Rc::new(RefCell::new(String::new()));

        let a = acc.clone();
        let _sub = d1.observe_update_v1(move |_: &TransactionMut, e| {
            let u = Update::decode_v1(&e.update).unwrap();
            for mut block in u.blocks.into_blocks() {
                match block.as_block_ptr().as_deref() {
                    Some(Block::Item(item)) => {
                        if let ItemContent::String(s) = &item.content {
                            // each character is appended in individual transaction 1-by-1,
                            // therefore each update should contain a single string with only
                            // one element
                            let mut aref: RefMut<_> = a.try_borrow_mut().unwrap();
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

        assert_eq!(acc.take(), INPUT);

        // test incremental deletes
        let acc = Rc::new(RefCell::new(Vec::new()));
        let a = acc.clone();
        let _sub = d1.observe_update_v1(move |_: &TransactionMut, e| {
            let u = Update::decode_v1(&e.update).unwrap();
            for (&client_id, range) in u.delete_set.iter() {
                if client_id == 1 {
                    let mut aref: RefMut<_> = a.try_borrow_mut().unwrap();
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
        assert_eq!(acc.take(), expected);
    }

    #[test]
    fn ycrdt_issue_174() {
        let doc = Doc::new();
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
        doc.transact_mut().apply_update(update);

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
    fn snapshots_update_generation() {
        let mut options = Options::with_client_id(1);
        options.skip_gc = true;

        let d1 = Doc::with_options(options);
        let txt1 = d1.get_or_insert_text("text");
        txt1.insert(&mut d1.transact_mut(), 0, "hello");
        let snapshot = d1.transact_mut().snapshot();
        txt1.insert(&mut d1.transact_mut(), 5, " world");

        let mut encoder = EncoderV1::new();
        d1.transact_mut()
            .encode_state_from_snapshot(&snapshot, &mut encoder)
            .unwrap();
        let update = encoder.to_vec();

        let d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("text");
        d2.transact_mut()
            .apply_update(Update::decode_v1(&update).unwrap());

        assert_eq!(txt2.get_string(&txt2.transact()), "hello".to_string());
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

        let doc = Doc::new();
        let mut txn = doc.transact_mut();
        for diff in diffs {
            let u = Update::decode_v1(diff.as_slice()).unwrap();
            txn.apply_update(u);
        }
    }

    #[test]
    fn root_refs() {
        let doc = Doc::new();
        {
            let _txt = doc.get_or_insert_text("text");
            let _array = doc.get_or_insert_array("array");
            let _map = doc.get_or_insert_map("map");
            let _xml_elem = doc.get_or_insert_xml_fragment("xml_elem");
        }

        let txn = doc.transact();
        for (key, value) in txn.root_refs() {
            match key {
                "text" => assert!(value.to_ytext().is_some()),
                "array" => assert!(value.to_yarray().is_some()),
                "map" => assert!(value.to_ymap().is_some()),
                "xml_elem" => assert!(value.to_yxml_fragment().is_some()),
                "xml_text" => assert!(value.to_yxml_text().is_some()),
                other => panic!("unrecognized root type: '{}'", other),
            }
        }
    }

    #[test]
    fn integrate_block_with_parent_gc() {
        let d1 = Doc::with_client_id(1);
        let d2 = Doc::with_client_id(2);
        let d3 = Doc::with_client_id(3);

        {
            let root = d1.get_or_insert_array("array");
            let mut txn = d1.transact_mut();
            root.push_back(&mut txn, ArrayPrelim::from(["A"]));
        }

        exchange_updates(&[&d1, &d2, &d3]);

        {
            let root = d2.get_or_insert_array("array");
            let mut t2 = d2.transact_mut();
            root.remove(&mut t2, 0);
            d1.transact_mut()
                .apply_update(Update::decode_v1(&t2.encode_update_v1()).unwrap());
        }

        {
            let root = d3.get_or_insert_array("array");
            let mut t3 = d3.transact_mut();
            let a3 = root.get(&t3, 0).unwrap().to_yarray().unwrap();
            a3.push_back(&mut t3, "B");
            // D1 got update which already removed a3, but this must not cause panic
            d1.transact_mut()
                .apply_update(Update::decode_v1(&t3.encode_update_v1()).unwrap());
        }

        exchange_updates(&[&d1, &d2, &d3]);

        let r1 = d1.get_or_insert_array("array").to_json(&d1.transact());
        let r2 = d2.get_or_insert_array("array").to_json(&d2.transact());
        let r3 = d3.get_or_insert_array("array").to_json(&d3.transact());

        assert_eq!(r1, r2);
        assert_eq!(r2, r3);
        assert_eq!(r3, r1);
    }

    #[test]
    fn subdoc() {
        let doc = Doc::with_client_id(1);
        let event = Rc::new(Cell::new(None));
        let event_c = event.clone();
        let _sub = doc.observe_subdocs(move |_, e| {
            let added = e.added().map(|d| d.guid().clone()).collect();
            let removed = e.removed().map(|d| d.guid().clone()).collect();
            let loaded = e.loaded().map(|d| d.guid().clone()).collect();
            event_c.set(Some((added, removed, loaded)));
        });
        let subdocs = doc.get_or_insert_map("mysubdocs");
        let uuid_a: Uuid = "A".into();
        let doc_a = Doc::with_options({
            let mut o = Options::default();
            o.guid = uuid_a.clone();
            o
        });
        {
            let mut txn = doc.transact_mut();
            subdocs.insert(&mut txn, "a", doc_a);
            let doc_a_ref = subdocs.get(&txn, "a").unwrap().to_ydoc().unwrap();
            doc_a_ref.load(&mut txn);
        }

        let actual = event.take();
        assert_eq!(
            actual,
            Some((vec![uuid_a.clone()], vec![], vec![uuid_a.clone()]))
        );

        {
            let mut txn = doc.transact_mut();
            let doc_a_ref = subdocs.get(&txn, "a").unwrap().to_ydoc().unwrap();
            doc_a_ref.load(&mut txn);
        }
        let actual = event.take();
        assert_eq!(actual, None);

        {
            let mut txn = doc.transact_mut();
            let mut doc_a_ref = subdocs.get(&txn, "a").unwrap().to_ydoc().unwrap();
            doc_a_ref.destroy(&mut txn);
        }
        let actual = event.take();
        assert_eq!(
            actual,
            Some((vec![uuid_a.clone()], vec![uuid_a.clone()], vec![]))
        );

        {
            let mut txn = doc.transact_mut();
            let doc_a_ref = subdocs.get(&txn, "a").unwrap().to_ydoc().unwrap();
            doc_a_ref.load(&mut txn);
        }
        let actual = event.take();
        assert_eq!(actual, Some((vec![], vec![], vec![uuid_a.clone()])));

        let doc_b = Doc::with_options({
            let mut o = Options::default();
            o.guid = uuid_a.clone();
            o.should_load = false;
            o
        });
        subdocs.insert(&mut doc.transact_mut(), "b", doc_b);
        let actual = event.take();
        assert_eq!(actual, Some((vec![uuid_a.clone()], vec![], vec![])));

        {
            let mut txn = doc.transact_mut();
            let doc_b_ref = subdocs.get(&txn, "b").unwrap().to_ydoc().unwrap();
            doc_b_ref.load(&mut txn);
        }
        let actual = event.take();
        assert_eq!(actual, Some((vec![], vec![], vec![uuid_a.clone()])));

        let uuid_c: Uuid = "C".into();
        let doc_c = Doc::with_options({
            let mut o = Options::default();
            o.guid = uuid_c.clone();
            o
        });
        {
            let mut txn = doc.transact_mut();
            subdocs.insert(&mut txn, "c", doc_c);
            let doc_c_ref = subdocs.get(&txn, "c").unwrap().to_ydoc().unwrap();
            doc_c_ref.load(&mut txn);
        }
        let actual = event.take();
        assert_eq!(
            actual,
            Some((vec![uuid_c.clone()], vec![], vec![uuid_c.clone()]))
        );

        let guids: BTreeSet<_> = doc.transact().subdoc_guids().cloned().collect();
        assert_eq!(guids, BTreeSet::from([uuid_a.clone(), uuid_c.clone()]));

        let data = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        let doc2 = Doc::new();
        let event = Rc::new(Cell::new(None));
        let event_c = event.clone();
        let _sub = doc2.observe_subdocs(move |_, e| {
            let added: Vec<_> = e.added().map(|d| d.guid().clone()).collect();
            let removed: Vec<_> = e.removed().map(|d| d.guid().clone()).collect();
            let loaded: Vec<_> = e.loaded().map(|d| d.guid().clone()).collect();
            event_c.set(Some((added, removed, loaded)));
        });
        let update = Update::decode_v1(&data).unwrap();
        doc2.transact_mut().apply_update(update);
        let mut actual = event.take().unwrap();
        actual.0.sort();
        assert_eq!(
            actual,
            (
                vec![uuid_a.clone(), uuid_a.clone(), uuid_c.clone()],
                vec![],
                vec![]
            )
        );

        let subdocs = doc2.transact().get_map("mysubdocs").unwrap();
        {
            let mut txn = doc2.transact_mut();
            let doc_ref = subdocs.get(&mut txn, "a").unwrap().to_ydoc().unwrap();
            assert_eq!(doc_ref.parent_doc(), Some(doc2.clone()));
            doc_ref.load(&mut txn);
        }
        let actual = event.take();
        assert_eq!(actual, Some((vec![], vec![], vec![uuid_a.clone()])));

        let guids: BTreeSet<_> = doc2.transact().subdoc_guids().cloned().collect();
        assert_eq!(guids, BTreeSet::from([uuid_a.clone(), uuid_c.clone()]));
        {
            let mut txn = doc2.transact_mut();
            subdocs.remove(&mut txn, "a");
        }

        let actual = event.take();
        assert_eq!(actual, Some((vec![], vec![uuid_a.clone()], vec![])));

        let mut guids: Vec<_> = doc2.transact().subdoc_guids().cloned().collect();
        guids.sort();
        assert_eq!(guids, vec![uuid_a.clone(), uuid_c.clone()]);
    }

    #[test]
    fn subdoc_load_edge_cases() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("test");
        let subdoc_1 = Doc::new();
        let uuid_1 = subdoc_1.options().guid.clone();

        let event = Rc::new(RefCell::new(None));
        let event_c = event.clone();
        let _sub = doc.observe_subdocs(move |_, e| {
            let added = e.added().map(|d| d.guid().clone()).collect();
            let removed = e.removed().map(|d| d.guid().clone()).collect();
            let loaded = e.loaded().map(|d| d.guid().clone()).collect();
            let mut e: RefMut<_> = event_c.try_borrow_mut().unwrap();
            *e = Some((added, removed, loaded));
        });
        let mut doc_ref = {
            let mut txn = doc.transact_mut();
            array.insert(&mut txn, 0, subdoc_1);
            let doc_ref = array.get(&txn, 0).unwrap().to_ydoc().unwrap();
            let o = doc_ref.options();
            assert!(o.should_load);
            assert!(!o.auto_load);
            doc_ref
        };
        let last_event = event.take();
        assert_eq!(
            last_event,
            Some((vec![uuid_1.clone()], vec![], vec![uuid_1.clone()]))
        );

        // destroy and check whether lastEvent adds it again to added (it shouldn't)
        doc_ref.destroy(&mut doc.transact_mut());
        let doc_ref_2 = array.get(&doc.transact(), 0).unwrap().to_ydoc().unwrap();
        let uuid_2 = doc_ref_2.options().guid.clone();
        assert!(!Doc::ptr_eq(&doc_ref, &doc_ref_2));

        let last_event = event.take();
        assert_eq!(
            last_event,
            Some((vec![uuid_2.clone()], vec![uuid_2.clone()], vec![]))
        );

        // load
        doc_ref_2.load(&mut doc.transact_mut());
        let last_event = event.take();
        assert_eq!(last_event, Some((vec![], vec![], vec![uuid_2.clone()])));

        // apply from remote
        let doc2 = Doc::with_client_id(2);
        let event_c = event.clone();
        let _sub = doc2.observe_subdocs(move |_, e| {
            let added = e.added().map(|d| d.guid().clone()).collect();
            let removed = e.removed().map(|d| d.guid().clone()).collect();
            let loaded = e.loaded().map(|d| d.guid().clone()).collect();
            let mut e: RefMut<_> = event_c.try_borrow_mut().unwrap();
            *e = Some((added, removed, loaded));
        });
        let u = Update::decode_v1(
            &doc.transact()
                .encode_state_as_update_v1(&StateVector::default()),
        );
        doc2.transact_mut().apply_update(u.unwrap());
        let doc_ref_3 = {
            let array = doc2.get_or_insert_array("test");
            array.get(&doc2.transact(), 0).unwrap().to_ydoc().unwrap()
        };
        assert!(!doc_ref_3.options().should_load);
        assert!(!doc_ref_3.options().auto_load);
        let uuid_3 = doc_ref_3.options().guid.clone();
        let last_event = event.take();
        assert_eq!(last_event, Some((vec![uuid_3.clone()], vec![], vec![])));

        // load
        doc_ref_3.load(&mut doc2.transact_mut());
        assert!(doc_ref_3.options().should_load);
        let last_event = event.take();
        assert_eq!(last_event, Some((vec![], vec![], vec![uuid_3.clone()])));
    }

    #[test]
    fn subdoc_auto_load_edge_cases() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("test");
        let subdoc_1 = Doc::with_options({
            let mut o = Options::default();
            o.auto_load = true;
            o
        });

        let event = Rc::new(RefCell::new(None));
        let event_c = event.clone();
        let _sub = doc.observe_subdocs(move |_, e| {
            let added = e.added().map(|d| d.guid().clone()).collect();
            let removed = e.removed().map(|d| d.guid().clone()).collect();
            let loaded = e.loaded().map(|d| d.guid().clone()).collect();
            let mut e: RefMut<_> = event_c.try_borrow_mut().unwrap();
            *e = Some((added, removed, loaded));
        });

        let mut subdoc_1 = {
            let mut txn = doc.transact_mut();
            array.insert(&mut txn, 0, subdoc_1);
            array.get(&txn, 0).unwrap().to_ydoc().unwrap()
        };
        assert!(subdoc_1.options().should_load);
        assert!(subdoc_1.options().auto_load);

        let uuid_1 = subdoc_1.options().guid.clone();
        let last_event = event.take();
        assert_eq!(
            last_event,
            Some((vec![uuid_1.clone()], vec![], vec![uuid_1.clone()]))
        );

        // destroy and check whether lastEvent adds it again to added (it shouldn't)
        subdoc_1.destroy(&mut doc.transact_mut());

        let subdoc_2 = array.get(&doc.transact(), 0).unwrap().to_ydoc().unwrap();
        let uuid_2 = subdoc_2.options().guid.clone();
        assert!(!Doc::ptr_eq(&subdoc_1, &subdoc_2));

        let last_event = event.take();
        assert_eq!(
            last_event,
            Some((vec![uuid_2.clone()], vec![uuid_2.clone()], vec![]))
        );

        subdoc_2.load(&mut doc.transact_mut());
        let last_event = event.take();
        assert_eq!(last_event, Some((vec![], vec![], vec![uuid_2.clone()])));

        // apply from remote
        let doc2 = Doc::with_client_id(2);
        let event_c = event.clone();
        let _sub = doc2.observe_subdocs(move |_, e| {
            let added = e.added().map(|d| d.guid().clone()).collect();
            let removed = e.removed().map(|d| d.guid().clone()).collect();
            let loaded = e.loaded().map(|d| d.guid().clone()).collect();
            let mut e: RefMut<_> = event_c.try_borrow_mut().unwrap();
            *e = Some((added, removed, loaded));
        });
        let u = Update::decode_v1(
            &doc.transact()
                .encode_state_as_update_v1(&StateVector::default()),
        );
        doc2.transact_mut().apply_update(u.unwrap());
        let subdoc_3 = {
            let array = doc2.get_or_insert_array("test");
            array.get(&doc2.transact(), 0).unwrap().to_ydoc().unwrap()
        };
        assert!(subdoc_1.options().should_load);
        assert!(subdoc_1.options().auto_load);
        let uuid_3 = subdoc_3.options().guid.clone();
        let last_event = event.take();
        assert_eq!(
            last_event,
            Some((vec![uuid_3.clone()], vec![], vec![uuid_3.clone()]))
        );
    }
}
