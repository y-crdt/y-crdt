use crate::block::{ClientID, ItemContent, ItemPtr, Prelim};
use crate::branch::BranchPtr;
use crate::encoding::read::Error;
use crate::event::{SubdocsEvent, TransactionCleanupEvent, UpdateEvent};
use crate::store::{DocStore, StoreInner};
use crate::transaction::{Origin, TransactionMut};
use crate::types::{RootRef, ToJson};
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::utils::OptionExt;
use crate::{
    uuid_v4, uuid_v4_from, ArrayRef, BranchID, MapRef, Out, ReadTxn, TextRef, Transact,
    TransactionAcqError, Uuid, WriteTxn, XmlFragmentRef,
};
use crate::{Any, Subscription};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt::Formatter;
use std::sync::Arc;

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
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct Doc {
    pub(crate) store: DocStore,
}

unsafe impl Send for Doc {}
unsafe impl Sync for Doc {}

impl TryFrom<Out> for Doc {
    type Error = Out;

    fn try_from(value: Out) -> Result<Self, Self::Error> {
        match value {
            Out::YDoc(value) => Ok(value),
            other => Err(other),
        }
    }
}

impl Doc {
    /// Creates a new document with a randomized client identifier.
    pub fn new() -> Self {
        Self::with_options(Options::default())
    }

    #[doc(hidden)]
    pub fn into_raw(self) -> *const Doc {
        let ptr = Arc::into_raw(self.store.0);
        ptr as *const Doc
    }

    #[doc(hidden)]
    pub unsafe fn from_raw(ptr: *const Doc) -> Doc {
        let ptr = ptr as *const StoreInner;
        let cell = Arc::from_raw(ptr);
        Doc {
            store: DocStore(cell),
        }
    }

    #[doc(hidden)]
    pub fn as_raw(self) -> *const Doc {
        let ptr = Arc::as_ptr(&self.store.0);
        ptr as *const Doc
    }

    /// Creates a new document with a specified `client_id`. It's up to a caller to guarantee that
    /// this identifier is unique across all communicating replicas of that document.
    pub fn with_client_id(client_id: ClientID) -> Self {
        Self::with_options(Options::with_client_id(client_id))
    }

    /// Creates a new document with a configured set of [Options].
    pub fn with_options(options: Options) -> Self {
        Doc {
            store: DocStore::new(options, None),
        }
    }

    pub(crate) fn subdoc(parent: ItemPtr, options: Options) -> Self {
        Doc {
            store: DocStore::new(options, Some(parent)),
        }
    }

    pub(crate) fn store(&self) -> &DocStore {
        &self.store
    }

    /// A unique client identifier, that's also a unique identifier of current document replica
    /// and it's subdocuments.
    ///
    /// Default: randomly generated.
    pub fn client_id(&self) -> ClientID {
        self.store.options().client_id
    }

    /// A globally unique identifier, that's also a unique identifier of current document replica,
    /// and unlike [Doc::client_id] it's not shared with its subdocuments.
    ///
    /// Default: randomly generated UUID v4.
    pub fn guid(&self) -> Uuid {
        self.store.options().guid.clone()
    }

    /// Returns a unique collection identifier, if defined.
    ///
    /// Default: `None`.
    pub fn collection_id(&self) -> Option<Arc<str>> {
        self.store.options().collection_id.clone()
    }

    /// Informs if current document is skipping garbage collection on deleted collections
    /// on transaction commit.
    ///
    /// Default: `false`.
    pub fn skip_gc(&self) -> bool {
        self.store.options().skip_gc
    }

    /// If current document is subdocument, it will automatically for a document to load.
    ///
    /// Default: `false`.
    pub fn auto_load(&self) -> bool {
        self.store.options().auto_load
    }

    /// Whether the document should be synced by the provider now.
    /// This is toggled to true when you call [Doc::load]
    ///
    /// Default value: `true`.
    pub fn should_load(&self) -> bool {
        self.store.options().should_load
    }

    /// Returns encoding used to count offsets and lengths in text operations.
    pub fn offset_kind(&self) -> OffsetKind {
        self.store.options().offset_kind
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
    pub fn get_or_insert_text<N: Into<Arc<str>>>(&self, name: N) -> TextRef {
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
    pub fn get_or_insert_map<N: Into<Arc<str>>>(&self, name: N) -> MapRef {
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
    pub fn get_or_insert_array<N: Into<Arc<str>>>(&self, name: N) -> ArrayRef {
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
    pub fn get_or_insert_xml_fragment<N: Into<Arc<str>>>(&self, name: N) -> XmlFragmentRef {
        XmlFragmentRef::root(name).get_or_create(&mut self.transact_mut())
    }

    /// Subscribe callback function for any changes performed within transaction scope. These
    /// changes are encoded using lib0 v1 encoding and can be decoded using [Update::decode_v1] if
    /// necessary or passed to remote peers right away. This callback is triggered on function
    /// commit.
    ///
    /// Returns a subscription, which will unsubscribe function when dropped.
    #[cfg(feature = "sync")]
    pub fn observe_update_v1<F>(&self, f: F) -> Result<Subscription, TransactionAcqError>
    where
        F: Fn(&TransactionMut, &UpdateEvent) + Send + Sync + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.update_v1_events.subscribe(Box::new(f)))
    }

    /// Subscribe callback function for any changes performed within transaction scope. These
    /// changes are encoded using lib0 v1 encoding and can be decoded using [Update::decode_v1] if
    /// necessary or passed to remote peers right away. This callback is triggered on function
    /// commit.
    ///
    /// Returns a subscription, which will unsubscribe function when dropped.
    #[cfg(not(feature = "sync"))]
    pub fn observe_update_v1<F>(&self, f: F) -> Result<Subscription, TransactionAcqError>
    where
        F: Fn(&TransactionMut, &UpdateEvent) + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.update_v1_events.subscribe(Box::new(f)))
    }

    /// Subscribe callback function for any changes performed within transaction scope. These
    /// changes are encoded using lib0 v1 encoding and can be decoded using [Update::decode_v1] if
    /// necessary or passed to remote peers right away. This callback is triggered on function
    /// commit.
    ///
    /// Provided `key` will be used to identify a subscription, which will be used to unsubscribe.
    #[cfg(feature = "sync")]
    pub fn observe_update_v1_with<K, F>(&self, key: K, f: F) -> Result<(), TransactionAcqError>
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &UpdateEvent) + Send + Sync + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        events
            .update_v1_events
            .subscribe_with(key.into(), Box::new(f));
        Ok(())
    }

    /// Subscribe callback function for any changes performed within transaction scope. These
    /// changes are encoded using lib0 v1 encoding and can be decoded using [Update::decode_v1] if
    /// necessary or passed to remote peers right away. This callback is triggered on function
    /// commit.
    ///
    /// Provided `key` will be used to identify a subscription, which will be used to unsubscribe.
    #[cfg(not(feature = "sync"))]
    pub fn observe_update_v1_with<K, F>(&self, key: K, f: F) -> Result<(), TransactionAcqError>
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &UpdateEvent) + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        events
            .update_v1_events
            .subscribe_with(key.into(), Box::new(f));
        Ok(())
    }

    pub fn unobserve_update_v1<K>(&self, key: K) -> Result<bool, TransactionAcqError>
    where
        K: Into<Origin>,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.update_v1_events.unsubscribe(&key.into()))
    }

    /// Subscribe callback function for any changes performed within transaction scope. These
    /// changes are encoded using lib0 v2 encoding and can be decoded using [Update::decode_v2] if
    /// necessary or passed to remote peers right away. This callback is triggered on function
    /// commit.
    ///
    /// Returns a subscription, which will unsubscribe function when dropped.
    #[cfg(feature = "sync")]
    pub fn observe_update_v2<F>(&self, f: F) -> Result<Subscription, TransactionAcqError>
    where
        F: Fn(&TransactionMut, &UpdateEvent) + Send + Sync + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.update_v2_events.subscribe(Box::new(f)))
    }

    /// Subscribe callback function for any changes performed within transaction scope. These
    /// changes are encoded using lib0 v2 encoding and can be decoded using [Update::decode_v2] if
    /// necessary or passed to remote peers right away. This callback is triggered on function
    /// commit.
    ///
    /// Returns a subscription, which will unsubscribe function when dropped.
    #[cfg(not(feature = "sync"))]
    pub fn observe_update_v2<F>(&self, f: F) -> Result<Subscription, TransactionAcqError>
    where
        F: Fn(&TransactionMut, &UpdateEvent) + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.update_v2_events.subscribe(Box::new(f)))
    }

    /// Subscribe callback function for any changes performed within transaction scope. These
    /// changes are encoded using lib0 v2 encoding and can be decoded using [Update::decode_v2] if
    /// necessary or passed to remote peers right away. This callback is triggered on function
    /// commit.
    ///
    /// Provided `key` will be used to identify a subscription, which will be used to unsubscribe.
    #[cfg(feature = "sync")]
    pub fn observe_update_v2_with<K, F>(&self, key: K, f: F) -> Result<(), TransactionAcqError>
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &UpdateEvent) + Send + Sync + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        events
            .update_v2_events
            .subscribe_with(key.into(), Box::new(f));
        Ok(())
    }

    /// Subscribe callback function for any changes performed within transaction scope. These
    /// changes are encoded using lib0 v2 encoding and can be decoded using [Update::decode_v2] if
    /// necessary or passed to remote peers right away. This callback is triggered on function
    /// commit.
    ///
    /// Provided `key` will be used to identify a subscription, which will be used to unsubscribe.
    #[cfg(not(feature = "sync"))]
    pub fn observe_update_v2_with<K, F>(&self, key: K, f: F) -> Result<(), TransactionAcqError>
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &UpdateEvent) + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        events
            .update_v2_events
            .subscribe_with(key.into(), Box::new(f));
        Ok(())
    }

    pub fn unobserve_update_v2<K>(&self, key: K) -> Result<bool, TransactionAcqError>
    where
        K: Into<Origin>,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.update_v2_events.unsubscribe(&key.into()))
    }

    /// Subscribe callback function to updates on the `Doc`. The callback will receive state updates and
    /// deletions when a document transaction is committed.
    #[cfg(feature = "sync")]
    pub fn observe_transaction_cleanup<F>(&self, f: F) -> Result<Subscription, TransactionAcqError>
    where
        F: Fn(&TransactionMut, &TransactionCleanupEvent) + Send + Sync + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.transaction_cleanup_events.subscribe(Box::new(f)))
    }

    /// Subscribe callback function to updates on the `Doc`. The callback will receive state updates and
    /// deletions when a document transaction is committed.
    #[cfg(not(feature = "sync"))]
    pub fn observe_transaction_cleanup<F>(&self, f: F) -> Result<Subscription, TransactionAcqError>
    where
        F: Fn(&TransactionMut, &TransactionCleanupEvent) + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.transaction_cleanup_events.subscribe(Box::new(f)))
    }

    /// Subscribe callback function to updates on the `Doc`. The callback will receive state updates and
    /// deletions when a document transaction is committed.
    #[cfg(feature = "sync")]
    pub fn observe_transaction_cleanup_with<K, F>(
        &self,
        key: K,
        f: F,
    ) -> Result<(), TransactionAcqError>
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &TransactionCleanupEvent) + Send + Sync + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        events
            .transaction_cleanup_events
            .subscribe_with(key.into(), Box::new(f));
        Ok(())
    }

    /// Subscribe callback function to updates on the `Doc`. The callback will receive state updates and
    /// deletions when a document transaction is committed.
    #[cfg(not(feature = "sync"))]
    pub fn observe_transaction_cleanup_with<K, F>(
        &self,
        key: K,
        f: F,
    ) -> Result<(), TransactionAcqError>
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &TransactionCleanupEvent) + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        events
            .transaction_cleanup_events
            .subscribe_with(key.into(), Box::new(f));
        Ok(())
    }

    pub fn unobserve_transaction_cleanup<K>(&self, key: K) -> Result<bool, TransactionAcqError>
    where
        K: Into<Origin>,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.transaction_cleanup_events.unsubscribe(&key.into()))
    }

    #[cfg(feature = "sync")]
    pub fn observe_after_transaction<F>(&self, f: F) -> Result<Subscription, TransactionAcqError>
    where
        F: Fn(&mut TransactionMut) + Send + Sync + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.after_transaction_events.subscribe(Box::new(f)))
    }

    #[cfg(feature = "sync")]
    pub fn observe_after_transaction_with<K, F>(
        &self,
        key: K,
        f: F,
    ) -> Result<(), TransactionAcqError>
    where
        K: Into<Origin>,
        F: Fn(&mut TransactionMut) + Send + Sync + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        events
            .after_transaction_events
            .subscribe_with(key.into(), Box::new(f));
        Ok(())
    }

    #[cfg(not(feature = "sync"))]
    pub fn observe_after_transaction_with<K, F>(
        &self,
        key: K,
        f: F,
    ) -> Result<(), TransactionAcqError>
    where
        K: Into<Origin>,
        F: Fn(&mut TransactionMut) + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        events
            .after_transaction_events
            .subscribe_with(key.into(), Box::new(f));
        Ok(())
    }

    pub fn unobserve_after_transaction<K>(&self, key: K) -> Result<bool, TransactionAcqError>
    where
        K: Into<Origin>,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.after_transaction_events.unsubscribe(&key.into()))
    }

    /// Subscribe callback function, that will be called whenever a subdocuments inserted in this
    /// [Doc] will request a load.
    #[cfg(feature = "sync")]
    pub fn observe_subdocs<F>(&self, f: F) -> Result<Subscription, TransactionAcqError>
    where
        F: Fn(&TransactionMut, &SubdocsEvent) + Send + Sync + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.subdocs_events.subscribe(Box::new(f)))
    }

    /// Subscribe callback function, that will be called whenever a subdocuments inserted in this
    /// [Doc] will request a load.
    #[cfg(not(feature = "sync"))]
    pub fn observe_subdocs<F>(&self, f: F) -> Result<Subscription, TransactionAcqError>
    where
        F: Fn(&TransactionMut, &SubdocsEvent) + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.subdocs_events.subscribe(Box::new(f)))
    }

    /// Subscribe callback function, that will be called whenever a subdocuments inserted in this
    /// [Doc] will request a load.
    #[cfg(feature = "sync")]
    pub fn observe_subdocs_with<K, F>(&self, key: K, f: F) -> Result<(), TransactionAcqError>
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &SubdocsEvent) + Send + Sync + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        events
            .subdocs_events
            .subscribe_with(key.into(), Box::new(f));
        Ok(())
    }

    /// Subscribe callback function, that will be called whenever a subdocuments inserted in this
    /// [Doc] will request a load.
    #[cfg(not(feature = "sync"))]
    pub fn observe_subdocs_with<K, F>(&self, key: K, f: F) -> Result<(), TransactionAcqError>
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &SubdocsEvent) + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        events
            .subdocs_events
            .subscribe_with(key.into(), Box::new(f));
        Ok(())
    }

    pub fn unobserve_subdocs<K>(&self, key: K) -> Result<bool, TransactionAcqError>
    where
        K: Into<Origin>,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.subdocs_events.unsubscribe(&key.into()))
    }

    /// Subscribe callback function, that will be called whenever a [DocRef::destroy] has been called.
    #[cfg(feature = "sync")]
    pub fn observe_destroy<F>(&self, f: F) -> Result<Subscription, TransactionAcqError>
    where
        F: Fn(&TransactionMut, &Doc) + Send + Sync + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.destroy_events.subscribe(Box::new(f)))
    }

    /// Subscribe callback function, that will be called whenever a [DocRef::destroy] has been called.
    #[cfg(not(feature = "sync"))]
    pub fn observe_destroy<F>(&self, f: F) -> Result<Subscription, TransactionAcqError>
    where
        F: Fn(&TransactionMut, &Doc) + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.destroy_events.subscribe(Box::new(f)))
    }

    /// Subscribe callback function, that will be called whenever a [DocRef::destroy] has been called.
    #[cfg(feature = "sync")]
    pub fn observe_destroy_with<K, F>(&self, key: K, f: F) -> Result<(), TransactionAcqError>
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &Doc) + Send + Sync + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        events
            .destroy_events
            .subscribe_with(key.into(), Box::new(f));
        Ok(())
    }

    pub fn unobserve_destroy<K>(&self, key: K) -> Result<bool, TransactionAcqError>
    where
        K: Into<Origin>,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        Ok(events.destroy_events.unsubscribe(&key.into()))
    }

    /// Subscribe callback function, that will be called whenever a [DocRef::destroy] has been called.
    #[cfg(not(feature = "sync"))]
    pub fn observe_destroy_with<K, F>(&self, key: K, f: F) -> Result<(), TransactionAcqError>
    where
        K: Into<Origin>,
        F: Fn(&TransactionMut, &Doc) + 'static,
    {
        let mut store = self
            .store
            .try_write()
            .ok_or(TransactionAcqError::ExclusiveAcqFailed)?;
        let events = store.events.get_or_init();
        events
            .destroy_events
            .subscribe_with(key.into(), Box::new(f));
        Ok(())
    }

    /// Sends a load request to a parent document. Works only if current document is a sub-document
    /// of a document.
    pub fn load<T>(&self, parent_txn: &mut T)
    where
        T: WriteTxn,
    {
        let should_load = self.store.set_should_load(true);
        if !should_load {
            let txn = self.transact();
            if txn.store().is_subdoc() {
                parent_txn
                    .subdocs_mut()
                    .loaded
                    .insert(self.addr(), self.clone());
            }
        }
    }

    /// Starts destroy procedure for a current document, triggering an "destroy" callback and
    /// invalidating all event callback subscriptions.
    pub fn destroy<T>(&self, parent_txn: &mut T)
    where
        T: WriteTxn,
    {
        let mut txn = self.transact_mut();
        let store = txn.store_mut();
        let subdocs: Vec<_> = store.subdocs.values().cloned().collect();
        for subdoc in subdocs {
            subdoc.destroy(&mut txn);
        }
        if let Some(mut item) = txn.store.parent.take() {
            let parent_ref = item.clone();
            let is_deleted = item.is_deleted();
            if let ItemContent::Doc(_, content) = &mut item.content {
                let mut options = (**content.store.options()).clone();
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
        // super.destroy(): cleanup the events
        if let Some(events) = txn.store_mut().events.take() {
            events.destroy_events.trigger(|cb| cb(&txn, self));
        }
    }

    /// If current document has been inserted as a sub-document, returns a reference to a parent
    /// document, which contains it.
    pub fn parent_doc(&self) -> Option<Doc> {
        let txn = self.transact();
        txn.parent_doc()
    }

    pub fn branch_id(&self) -> Option<BranchID> {
        let txn = self.transact();
        txn.branch_id()
    }

    pub fn ptr_eq(a: &Doc, b: &Doc) -> bool {
        Arc::ptr_eq(&a.store.0, &b.store.0)
    }

    pub(crate) fn addr(&self) -> DocAddr {
        DocAddr::new(&self)
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

impl TryFrom<ItemPtr> for Doc {
    type Error = ItemPtr;

    fn try_from(item: ItemPtr) -> Result<Self, Self::Error> {
        if let ItemContent::Doc(_, doc) = &item.content {
            Ok(doc.clone())
        } else {
            Err(item)
        }
    }
}

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
        Any::from(m)
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
    pub guid: Uuid,
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
            guid: uuid_v4(),
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
        options.guid = guid.into();

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

impl Prelim for Doc {
    type Return = Doc;

    fn into_content(self, txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        if txn.parent_doc().is_some() {
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
    use crate::block::{BlockCell, ClientID, ItemContent, GC};
    use crate::error::Error;
    use crate::test_utils::exchange_updates;
    use crate::transaction::{ReadTxn, TransactionMut};
    use crate::types::ToJson;
    use crate::update::Update;
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::{Encode, Encoder, EncoderV1};
    use crate::{
        any, uuid_v4, Any, Array, ArrayPrelim, ArrayRef, DeleteSet, Doc, GetString, Map, MapRef,
        OffsetKind, Options, Snapshot, StateVector, Subscription, Text, TextPrelim, TextRef,
        Transact, Uuid, WriteTxn, XmlElementPrelim, XmlFragment, XmlFragmentRef, XmlTextPrelim,
        XmlTextRef, ID,
    };
    use arc_swap::ArcSwapOption;
    use assert_matches2::assert_matches;
    use std::collections::BTreeSet;
    use std::iter::FromIterator;
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
        let doc = Doc::new();
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
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("type");
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v2(update).unwrap())
            .unwrap();

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
        let pending = update.integrate(&mut t2).unwrap();

        assert!(pending.0.is_none());
        assert!(pending.1.is_none());

        // check if B sees the same thing that A does
        assert_eq!(txt.get_string(&t1), "hello world".to_string());
    }

    #[test]
    fn on_update() {
        let counter = Arc::new(AtomicU32::new(0));
        let doc = Doc::new();
        let doc2 = Doc::new();
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
            txn.apply_update(u).unwrap();
        }
        assert_eq!(txt.get_string(&doc.transact()), "abcd".to_string());
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
            d1.transact_mut().apply_update(u).unwrap();
        }

        assert_eq!("a", source_1.get_string(&d1.transact()));

        let d2 = Doc::new();
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

        let d3 = Doc::new();
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
        let doc = Doc::new();
        let text = doc.get_or_insert_text("test");
        let before_state = Arc::new(ArcSwapOption::default());
        let after_state = Arc::new(ArcSwapOption::default());
        let delete_set = Arc::new(ArcSwapOption::default());
        // Create interior mutable references for the callback.
        let before_ref = before_state.clone();
        let after_ref = after_state.clone();
        let delete_ref = delete_set.clone();
        // Subscribe callback

        let sub: Subscription = doc
            .observe_transaction_cleanup(move |_: &TransactionMut, event| {
                before_ref.store(Some(event.before_state.clone().into()));
                after_ref.store(Some(event.after_state.clone().into()));
                delete_ref.store(Some(event.delete_set.clone().into()));
            })
            .unwrap();

        {
            let mut txn = doc.transact_mut();

            // Update the document
            text.insert(&mut txn, 0, "abc");
            text.remove_range(&mut txn, 1, 2);
            txn.commit();

            // Compare values
            assert_eq!(
                before_state.swap(None),
                Some(Arc::new(txn.before_state.clone()))
            );
            assert_eq!(
                after_state.swap(None),
                Some(Arc::new(txn.after_state.clone()))
            );
            assert_eq!(
                delete_set.swap(None),
                Some(Arc::new(txn.delete_set.clone()))
            );
        }

        // Ensure that the subscription is successfully dropped.
        drop(sub);
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "should not update");
        txn.commit();
        assert_ne!(
            after_state.swap(None),
            Some(Arc::new(txn.after_state.clone()))
        );
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

        let d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("text");
        let acc = Arc::new(Mutex::new(String::new()));

        let a = acc.clone();
        let _sub = d1.observe_update_v1(move |_: &TransactionMut, e| {
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
        let _sub = d1.observe_update_v1(move |_: &TransactionMut, e| {
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

        let d1 = Doc::with_options(options);
        let txt1 = d1.get_or_insert_text("text");
        txt1.insert(&mut d1.transact_mut(), 0, "hello");
        let snapshot = d1.transact_mut().snapshot();
        txt1.insert(&mut d1.transact_mut(), 5, "_world");

        let mut encoder = EncoderV1::new();
        d1.transact_mut()
            .encode_state_from_snapshot(&snapshot, &mut encoder)
            .unwrap();
        let update = Update::decode_v1(&encoder.to_vec()).unwrap();

        let d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("text");
        d2.transact_mut().apply_update(update).unwrap();

        assert_eq!(txt2.get_string(&d2.transact()), "hello".to_string());
    }

    #[test]
    fn snapshot_non_splitting_text() {
        let mut options = Options::default();
        options.skip_gc = true;

        let doc = Doc::with_options(options.clone().into());
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

        let remote_doc = Doc::with_options(options);
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

        let doc = Doc::new();
        let mut txn = doc.transact_mut();
        for diff in diffs {
            let u = Update::decode_v1(diff.as_slice()).unwrap();
            txn.apply_update(u).unwrap();
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
                "text" => assert!(value.cast::<TextRef>().is_ok()),
                "array" => assert!(value.cast::<ArrayRef>().is_ok()),
                "map" => assert!(value.cast::<MapRef>().is_ok()),
                "xml_elem" => assert!(value.cast::<XmlFragmentRef>().is_ok()),
                "xml_text" => assert!(value.cast::<XmlTextRef>().is_ok()),
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
                .apply_update(Update::decode_v1(&t2.encode_update_v1()).unwrap())
                .unwrap();
        }

        {
            let root = d3.get_or_insert_array("array");
            let mut t3 = d3.transact_mut();
            let a3 = root.get(&t3, 0).unwrap().cast::<ArrayRef>().unwrap();
            a3.push_back(&mut t3, "B");
            // D1 got update which already removed a3, but this must not cause panic
            d1.transact_mut()
                .apply_update(Update::decode_v1(&t3.encode_update_v1()).unwrap())
                .unwrap();
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
        let event = Arc::new(ArcSwapOption::default());
        let event_c = event.clone();
        let _sub = doc.observe_subdocs(move |_, e| {
            let added = e.added().map(|d| d.guid().clone()).collect();
            let removed = e.removed().map(|d| d.guid().clone()).collect();
            let loaded = e.loaded().map(|d| d.guid().clone()).collect();
            event_c.store(Some(Arc::new((added, removed, loaded))));
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
            let doc_a_ref = subdocs.insert(&mut txn, "a", doc_a);
            doc_a_ref.load(&mut txn);
        }

        let actual = event.swap(None);
        assert_eq!(
            actual,
            Some((vec![uuid_a.clone()], vec![], vec![uuid_a.clone()]).into())
        );

        {
            let mut txn = doc.transact_mut();
            let doc_a_ref = subdocs.get(&txn, "a").unwrap().cast::<Doc>().unwrap();
            doc_a_ref.load(&mut txn);
        }
        let actual = event.swap(None);
        assert_eq!(actual, None);

        {
            let mut txn = doc.transact_mut();
            let doc_a_ref = subdocs.get(&txn, "a").unwrap().cast::<Doc>().unwrap();
            doc_a_ref.destroy(&mut txn);
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
            let doc_a_ref = subdocs.get(&txn, "a").unwrap().cast::<Doc>().unwrap();
            doc_a_ref.load(&mut txn);
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
            let doc_b_ref = subdocs.get(&txn, "b").unwrap().cast::<Doc>().unwrap();
            doc_b_ref.load(&mut txn);
        }
        let actual = event.swap(None);
        assert_eq!(
            actual,
            Some(Arc::new((vec![], vec![], vec![uuid_a.clone()])))
        );

        let uuid_c: Uuid = "C".into();
        let doc_c = Doc::with_options({
            let mut o = Options::default();
            o.guid = uuid_c.clone();
            o
        });
        {
            let mut txn = doc.transact_mut();
            let doc_c_ref = subdocs.insert(&mut txn, "c", doc_c);
            doc_c_ref.load(&mut txn);
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

        let guids: BTreeSet<_> = doc.transact().subdoc_guids().collect();
        assert_eq!(guids, BTreeSet::from([uuid_a.clone(), uuid_c.clone()]));

        let data = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        let doc2 = Doc::new();
        let event = Arc::new(ArcSwapOption::default());
        let event_c = event.clone();
        let _sub = doc2.observe_subdocs(move |_, e| {
            let added: Vec<_> = e.added().map(|d| d.guid().clone()).collect();
            let removed: Vec<_> = e.removed().map(|d| d.guid().clone()).collect();
            let loaded: Vec<_> = e.loaded().map(|d| d.guid().clone()).collect();
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
            let doc_ref = subdocs.get(&mut txn, "a").unwrap().cast::<Doc>().unwrap();
            doc_ref.load(&mut txn);
        }
        let actual = event.swap(None);
        assert_eq!(
            actual,
            Some(Arc::new((vec![], vec![], vec![uuid_a.clone()])))
        );

        let guids: BTreeSet<_> = doc2.transact().subdoc_guids().collect();
        assert_eq!(guids, BTreeSet::from([uuid_a.clone(), uuid_c.clone()]));
        {
            let mut txn = doc2.transact_mut();
            subdocs.remove(&mut txn, "a");
        }

        let actual = event.swap(None);
        assert_eq!(
            actual,
            Some(Arc::new((vec![], vec![uuid_a.clone()], vec![])))
        );

        let mut guids: Vec<_> = doc2.transact().subdoc_guids().collect();
        guids.sort();
        assert_eq!(guids, vec![uuid_a.clone(), uuid_c.clone()]);
    }

    #[test]
    fn subdoc_load_edge_cases() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("test");
        let subdoc_1 = Doc::new();
        let uuid_1 = subdoc_1.guid().clone();

        let event = Arc::new(ArcSwapOption::default());
        let event_c = event.clone();
        let _sub = doc.observe_subdocs(move |_, e| {
            let added = e.added().map(|d| d.guid().clone()).collect();
            let removed = e.removed().map(|d| d.guid().clone()).collect();
            let loaded = e.loaded().map(|d| d.guid().clone()).collect();

            event_c.store(Some(Arc::new((added, removed, loaded))));
        });
        let doc_ref = {
            let mut txn = doc.transact_mut();
            let doc_ref = array.insert(&mut txn, 0, subdoc_1);
            assert!(doc_ref.should_load());
            assert!(!doc_ref.auto_load());
            doc_ref
        };
        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some((vec![uuid_1.clone()], vec![], vec![uuid_1.clone()]).into())
        );

        // destroy and check whether lastEvent adds it again to added (it shouldn't)
        doc_ref.destroy(&mut doc.transact_mut());
        let doc_ref_2 = array
            .get(&doc.transact(), 0)
            .unwrap()
            .cast::<Doc>()
            .unwrap();
        let uuid_2 = doc_ref_2.guid();
        assert!(!Doc::ptr_eq(&doc_ref, &doc_ref_2));

        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some((vec![uuid_2.clone()], vec![uuid_2.clone()], vec![]).into())
        );

        // load
        doc_ref_2.load(&mut doc.transact_mut());
        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some(Arc::new((vec![], vec![], vec![uuid_2.clone()])))
        );

        // apply from remote
        let doc2 = Doc::with_client_id(2);
        let event_c = event.clone();
        let _sub = doc2.observe_subdocs(move |_, e| {
            let added = e.added().map(|d| d.guid().clone()).collect();
            let removed = e.removed().map(|d| d.guid().clone()).collect();
            let loaded = e.loaded().map(|d| d.guid().clone()).collect();

            event_c.store(Some(Arc::new((added, removed, loaded))));
        });
        let u = Update::decode_v1(
            &doc.transact()
                .encode_state_as_update_v1(&StateVector::default()),
        );
        doc2.transact_mut().apply_update(u.unwrap()).unwrap();
        let doc_ref_3 = {
            let array = doc2.get_or_insert_array("test");
            array
                .get(&doc2.transact(), 0)
                .unwrap()
                .cast::<Doc>()
                .unwrap()
        };
        assert!(!doc_ref_3.should_load());
        assert!(!doc_ref_3.auto_load());
        let uuid_3 = doc_ref_3.guid();
        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some(Arc::new((vec![uuid_3.clone()], vec![], vec![])))
        );

        // load
        doc_ref_3.load(&mut doc2.transact_mut());
        assert!(doc_ref_3.should_load());
        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some(Arc::new((vec![], vec![], vec![uuid_3.clone()])))
        );
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

        let event = Arc::new(ArcSwapOption::default());
        let event_c = event.clone();
        let _sub = doc.observe_subdocs(move |_, e| {
            let added = e.added().map(|d| d.guid().clone()).collect();
            let removed = e.removed().map(|d| d.guid().clone()).collect();
            let loaded = e.loaded().map(|d| d.guid().clone()).collect();

            event_c.store(Some(Arc::new((added, removed, loaded))));
        });

        let subdoc_1 = {
            let mut txn = doc.transact_mut();
            array.insert(&mut txn, 0, subdoc_1)
        };
        assert!(subdoc_1.should_load());
        assert!(subdoc_1.auto_load());

        let uuid_1 = subdoc_1.guid();
        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some(Arc::new((
                vec![uuid_1.clone()],
                vec![],
                vec![uuid_1.clone()]
            )))
        );

        // destroy and check whether lastEvent adds it again to added (it shouldn't)
        subdoc_1.destroy(&mut doc.transact_mut());

        let subdoc_2 = array
            .get(&doc.transact(), 0)
            .unwrap()
            .cast::<Doc>()
            .unwrap();
        let uuid_2 = subdoc_2.guid();
        assert!(!Doc::ptr_eq(&subdoc_1, &subdoc_2));

        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some(Arc::new((
                vec![uuid_2.clone()],
                vec![uuid_2.clone()],
                vec![]
            )))
        );

        subdoc_2.load(&mut doc.transact_mut());
        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some(Arc::new((vec![], vec![], vec![uuid_2.clone()])))
        );

        // apply from remote
        let doc2 = Doc::with_client_id(2);
        let event_c = event.clone();
        let _sub = doc2.observe_subdocs(move |_, e| {
            let added = e.added().map(|d| d.guid()).collect();
            let removed = e.removed().map(|d| d.guid()).collect();
            let loaded = e.loaded().map(|d| d.guid()).collect();

            event_c.store(Some(Arc::new((added, removed, loaded))));
        });
        let u = Update::decode_v1(
            &doc.transact()
                .encode_state_as_update_v1(&StateVector::default()),
        );
        doc2.transact_mut().apply_update(u.unwrap()).unwrap();
        let subdoc_3 = {
            let array = doc2.get_or_insert_array("test");
            array
                .get(&doc2.transact(), 0)
                .unwrap()
                .cast::<Doc>()
                .unwrap()
        };
        assert!(subdoc_1.should_load());
        assert!(subdoc_1.auto_load());
        let uuid_3 = subdoc_3.guid();
        let last_event = event.swap(None);
        assert_eq!(
            last_event,
            Some(Arc::new((
                vec![uuid_3.clone()],
                vec![],
                vec![uuid_3.clone()]
            )))
        );
    }

    #[test]
    fn to_json() {
        let doc = Doc::new();
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
        let sub_doc = Doc::new();
        let sub_text = sub_doc.get_or_insert_text("sub-text");
        let sub_doc = map.insert(&mut txn, "sub-doc", sub_doc);
        let mut sub_txn = sub_doc.transact_mut();
        sub_text.push(&mut sub_txn, "sample");

        let actual = doc.to_json(&txn);
        let expected = any!({
            "text": "hello",
            "array": [1,2,3],
            "map": {
                "key1": "value1",
                "sub-doc": {
                    "guid": sub_doc.guid().as_ref()
                }
            },
            "xml-fragment": "<div></div>world<xml-element><body></body></xml-element>",
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn apply_snapshot_updates() {
        let update = {
            let doc = Doc::with_options(Options {
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

        let doc = Doc::with_client_id(1);
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

        let d1 = Doc::new();
        let _sub = {
            let updates = updates.clone();
            d1.observe_update_v1(move |_, e| {
                let mut u = updates.lock().unwrap();
                u.push(Update::decode_v1(&e.update).unwrap());
            })
            .unwrap()
        };

        let map = d1.get_or_insert_map("map");
        map.insert(&mut d1.transact_mut(), "a", 1);
        map.insert(&mut d1.transact_mut(), "a", 1.1);
        map.insert(&mut d1.transact_mut(), "b", 2);

        assert_eq!(map.to_json(&d1.transact()), any!({"a": 1.1, "b": 2}));

        let d2 = Doc::new();

        {
            let mut updates = updates.lock().unwrap();
            let u3 = updates.pop().unwrap();
            let u2 = updates.pop().unwrap();
            let u1 = updates.pop().unwrap();
            let mut txn = d2.transact_mut();
            txn.apply_update(u1).unwrap();
            assert!(txn.store.pending.is_none()); // applied
            txn.apply_update(u3).unwrap();
            assert!(txn.store.pending.is_some()); // pending update waiting for u2
            txn.apply_update(u2).unwrap();
            assert!(txn.store.pending.is_none()); // applied after fixing the missing update
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
        let d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("text");

        let e = Arc::new(ArcSwapOption::default());
        let e_copy = e.clone();
        d1.observe_after_transaction_with("key", move |txn| {
            e_copy.swap(Some(Arc::new((
                txn.before_state.clone(),
                txn.after_state.clone(),
                txn.delete_set.clone(),
            ))));
        })
        .unwrap();

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

        d1.unobserve_after_transaction("key").unwrap();

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
        let doc = Doc::with_options(Options {
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

        // force GC and check if original content is hard deleted
        doc.transact_mut().gc(None);

        let txn = doc.transact();
        let block = txn.store().blocks.get_block(&ID::new(1, 1)).unwrap();
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

    fn restore_from_snapshot(doc: &Doc, snapshot: &Snapshot) -> Result<Doc, Error> {
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
}
