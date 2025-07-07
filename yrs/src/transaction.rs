use crate::block::{Item, ItemContent, ItemPosition, ItemPtr, Prelim, ID};
use crate::branch::{Branch, BranchPtr};
use crate::doc::DocAddr;
use crate::error::{Error, UpdateError};
use crate::event::SubdocsEvent;
use crate::gc::GCCollector;
use crate::id_set::DeleteSet;
use crate::iter::TxnIterator;
use crate::slice::BlockSlice;
use crate::store::{Store, StoreEvents, SubdocGuids, SubdocsIter};
use crate::types::{Event, Events, RootRef, TypePtr, TypeRef};
use crate::update::Update;
use crate::updates::encoder::{Encode, Encoder, EncoderV1, EncoderV2};
use crate::utils::OptionExt;
use crate::{
    merge_updates_v1, merge_updates_v2, ArrayRef, BranchID, Doc, MapRef, Out, Snapshot,
    StateVector, TextRef, Transact, XmlElementRef, XmlFragmentRef, XmlTextRef,
};
use async_lock::{RwLockReadGuard, RwLockWriteGuard};
use smallvec::SmallVec;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Formatter;
use std::hash::Hash;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::Arc;

/// Trait defining read capabilities present in a transaction. Implemented by both lightweight
/// [read-only](Transaction) and [read-write](TransactionMut) transactions.
pub trait ReadTxn: Sized {
    fn store(&self) -> &Store;

    /// Returns state vector describing current state of the updates.
    fn state_vector(&self) -> StateVector {
        self.store().blocks.get_state_vector()
    }

    /// Returns a snapshot which describes a current state of updates and removals made within
    /// the corresponding document.
    fn snapshot(&self) -> Snapshot {
        let store = self.store();
        let blocks = &store.blocks;
        let sv = blocks.get_state_vector();
        let ds = DeleteSet::from(blocks);
        Snapshot::new(sv, ds)
    }

    /// Encodes all changes from current transaction block store up to a given `snapshot`.
    /// This enables to encode state of a document at some specific point in the past.
    fn encode_state_from_snapshot<E: Encoder>(
        &self,
        snapshot: &Snapshot,
        encoder: &mut E,
    ) -> Result<(), Error> {
        self.store().encode_state_from_snapshot(snapshot, encoder)
    }

    /// Encodes the difference between remote peer state given its `state_vector` and the state
    /// of a current local peer.
    ///
    /// # Differences between alternative methods
    ///
    /// - [Self::encode_state_as_update] encodes full document state including pending updates and
    /// entire delete set.
    /// - [Self::encode_diff] encodes only the difference between the current state and
    /// the given state vector, including entire delete set. Pending updates are not included.
    /// - [TransactionMut::encode_update] encodes only inserts and deletes made within the scope
    /// of the current transaction.
    fn encode_diff<E: Encoder>(&self, state_vector: &StateVector, encoder: &mut E) {
        self.store().encode_diff(state_vector, encoder)
    }

    /// Encodes the difference between remote peer state given its `state_vector` and the state
    /// of a current local peer, using lib0 v1 encoding.
    ///
    /// # Differences between alternative methods
    ///
    /// - [Self::encode_state_as_update_v1] encodes full document state including pending updates
    /// and entire delete set.
    /// - [Self::encode_diff_v1] encodes only the difference between the current state and
    /// the given state vector, including entire delete set. Pending updates are not included.
    /// - [TransactionMut::encode_update_v1] encodes only inserts and deletes made within the scope
    /// of the current transaction.
    fn encode_diff_v1(&self, state_vector: &StateVector) -> Vec<u8> {
        let mut encoder = EncoderV1::new();
        self.encode_diff(state_vector, &mut encoder);
        encoder.to_vec()
    }

    /// Encodes the difference between remote peer state given its `state_vector` and the state
    /// of a current local peer, using lib0 v2 encoding.
    ///
    /// # Differences between alternative methods
    ///
    /// - [Self::encode_state_as_update_v2] encodes full document state including pending updates
    /// and entire delete set.
    /// - [Self::encode_diff_v2] encodes only the difference between the current state and
    /// the given state vector, including entire delete set. Pending updates are not included.
    /// - [TransactionMut::encode_update_v2] encodes only inserts and deletes made within the scope
    /// of the current transaction.
    fn encode_diff_v2(&self, state_vector: &StateVector) -> Vec<u8> {
        let mut encoder = EncoderV2::new();
        self.encode_diff(state_vector, &mut encoder);
        encoder.to_vec()
    }

    /// Encodes the difference between remote peer state given its `state_vector` and the state
    /// of a current local peer. Also includes pending updates which were not yet integrated into
    /// the main document state and entire delete set.
    ///
    /// # Differences between alternative methods
    ///
    /// - [Self::encode_state_as_update] encodes full document state including pending updates and
    /// entire delete set.
    /// - [Self::encode_diff] encodes only the difference between the current state and
    /// the given state vector, including entire delete set. Pending updates are not included.
    /// - [TransactionMut::encode_update] encodes only inserts and deletes made within the scope
    /// of the current transaction.
    fn encode_state_as_update<E: Encoder>(&self, sv: &StateVector, encoder: &mut E) {
        let store = self.store();
        store.write_blocks_from(sv, encoder);
        let ds = DeleteSet::from(&store.blocks);
        ds.encode(encoder);
    }

    /// Encodes the difference between remote peer state given its `state_vector` and the state
    /// of a current local peer, using lib0 v1 encoding. Also includes pending updates which were
    /// not yet integrated into the main document state and entire delete set.
    ///
    /// # Differences between alternative methods
    ///
    /// - [Self::encode_state_as_update_v1] encodes full document state including pending updates
    /// and entire delete set.
    /// - [Self::encode_diff_v1] encodes only the difference between the current state and
    /// the given state vector, including entire delete set. Pending updates are not included.
    /// - [TransactionMut::encode_update_v1] encodes only inserts and deletes made within the scope
    /// of the current transaction.
    fn encode_state_as_update_v1(&self, sv: &StateVector) -> Vec<u8> {
        let mut encoder = EncoderV1::new();
        self.encode_state_as_update(sv, &mut encoder);
        // check for pending data
        merge_pending_v1(encoder.to_vec(), self.store())
    }

    /// Encodes the difference between remote peer state given its `state_vector` and the state
    /// of a current local peer, using lib0 v2 encoding. Also includes pending updates which were
    /// not yet integrated into the main document state and entire delete set.
    ///
    /// # Differences between alternative methods
    ///
    /// - [Self::encode_state_as_update_v2] encodes full document state including pending updates
    /// and entire delete set.
    /// - [Self::encode_diff_v2] encodes only the difference between the current state and
    /// the given state vector, including entire delete set. Pending updates are not included.
    /// - [TransactionMut::encode_update_v2] encodes only inserts and deletes made within the scope
    /// of the current transaction.
    fn encode_state_as_update_v2(&self, sv: &StateVector) -> Vec<u8> {
        let mut encoder = EncoderV2::new();
        self.encode_state_as_update(sv, &mut encoder);

        // check for pending data
        merge_pending_v2(encoder.to_vec(), self.store())
    }

    /// Returns an iterator over top level (root) shared types available in current [Doc].
    fn root_refs(&self) -> RootRefs {
        let store = self.store();
        RootRefs(store.types.iter())
    }

    /// Returns a collection of globally unique identifiers of sub documents linked within
    /// the structures of this document store.
    fn subdoc_guids(&self) -> SubdocGuids {
        let store = self.store();
        store.subdoc_guids()
    }

    /// Returns a collection of sub documents linked within the structures of this document store.
    fn subdocs(&self) -> SubdocsIter {
        let store = self.store();
        store.subdocs()
    }

    /// Returns a [TextRef] data structure stored under a given `name`. Text structures are used for
    /// collaborative text editing: they expose operations to append and remove chunks of text,
    /// which are free to execute concurrently by multiple peers over remote boundaries.
    ///
    /// If not structure under defined `name` existed before, [None] will be returned.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as a text (in such case a sequence component of complex data type will be
    /// interpreted as a list of text chunks).
    #[inline]
    fn get_text<N: Into<Arc<str>>>(&self, name: N) -> Option<TextRef> {
        TextRef::root(name).get(self)
    }

    /// Returns an [ArrayRef] data structure stored under a given `name`. Array structures are used for
    /// storing a sequences of elements in ordered manner, positioning given element accordingly
    /// to its index.
    ///
    /// If not structure under defined `name` existed before, [None] will be returned.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as an array (in such case a sequence component of complex data type will be
    /// interpreted as a list of inserted values).
    #[inline]
    fn get_array<N: Into<Arc<str>>>(&self, name: N) -> Option<ArrayRef> {
        ArrayRef::root(name).get(self)
    }

    /// Returns a [MapRef] data structure stored under a given `name`. Maps are used to store key-value
    /// pairs associated. These values can be primitive data (similar but not limited to
    /// a JavaScript Object Notation) as well as other shared types (Yrs maps, arrays, text
    /// structures etc.), enabling to construct a complex recursive tree structures.
    ///
    /// If not structure under defined `name` existed before, [None] will be returned.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as a map (in such case a map component of complex data type will be
    /// interpreted as native map).
    #[inline]
    fn get_map<N: Into<Arc<str>>>(&self, name: N) -> Option<MapRef> {
        MapRef::root(name).get(self)
    }

    /// Returns a [XmlFragmentRef] data structure stored under a given `name`. XML elements represent
    /// nodes of XML document. They can contain attributes (key-value pairs, both of string type)
    /// and other nested XML elements or text values, which are stored in their insertion
    /// order.
    ///
    /// If not structure under defined `name` existed before, [None] will be returned.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as a XML element (in such case a map component of complex data type will be
    /// interpreted as map of its attributes, while a sequence component - as a list of its child
    /// XML nodes).
    #[inline]
    fn get_xml_fragment<N: Into<Arc<str>>>(&self, name: N) -> Option<XmlFragmentRef> {
        XmlFragmentRef::root(name).get(self)
    }

    fn get<S: AsRef<str>>(&self, name: S) -> Option<Out> {
        let value = self.store().types.get(name.as_ref())?;
        let ptr = BranchPtr::from(&*value);
        match &ptr.type_ref {
            TypeRef::Array => Some(Out::YArray(ArrayRef::from(ptr))),
            TypeRef::Map => Some(Out::YMap(MapRef::from(ptr))),
            TypeRef::Text => Some(Out::YText(TextRef::from(ptr))),
            TypeRef::XmlElement(_) => Some(Out::YXmlElement(XmlElementRef::from(ptr))),
            TypeRef::XmlFragment => Some(Out::YXmlFragment(XmlFragmentRef::from(ptr))),
            TypeRef::XmlHook => None,
            TypeRef::XmlText => Some(Out::YXmlText(XmlTextRef::from(ptr))),
            TypeRef::SubDoc => Some(Out::YDoc(ptr.as_subdoc()?)),
            #[cfg(feature = "weak")]
            TypeRef::WeakLink(_) => Some(Out::YWeakLink(crate::WeakRef::from(ptr))),
            TypeRef::Undefined => Some(Out::UndefinedRef(ptr)),
        }
    }

    /// If current document has been inserted as a sub-document, returns a reference to a parent
    /// document, which contains it.
    fn parent_doc(&self) -> Option<Doc> {
        if let Some(item) = self.store().parent.as_deref() {
            if let ItemContent::Doc(parent_doc, _) = &item.content {
                return parent_doc.clone();
            }
        }

        None
    }

    /// If current document has been inserted as a sub-document, returns its [BranchID].
    fn branch_id(&self) -> Option<BranchID> {
        if let Some(item) = self.store().parent {
            Some(BranchID::Nested(item.id))
        } else {
            None
        }
    }

    /// Returns `true` if current document has any pending updates that are not yet
    /// integrated into the document.
    fn has_missing_updates(&self) -> bool {
        let store = self.store();
        store.pending.is_some() || store.pending_ds.is_some()
    }
}

pub trait WriteTxn: Sized {
    fn store_mut(&mut self) -> &mut Store;
    fn subdocs_mut(&mut self) -> &mut Subdocs;

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
    fn get_or_insert_text<N: Into<Arc<str>>>(&mut self, name: N) -> TextRef {
        TextRef::root(name).get_or_create(self)
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
    fn get_or_insert_map<N: Into<Arc<str>>>(&mut self, name: N) -> MapRef {
        MapRef::root(name).get_or_create(self)
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
    fn get_or_insert_array<N: Into<Arc<str>>>(&mut self, name: N) -> ArrayRef {
        ArrayRef::root(name).get_or_create(self)
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
    fn get_or_insert_xml_fragment<N: Into<Arc<str>>>(&mut self, name: N) -> XmlFragmentRef {
        XmlFragmentRef::root(name).get_or_create(self)
    }

    /// Prunes a pending updates from the current document and returns them.
    /// Returns `None` if current document didn't have any pending updates.
    fn prune_pending(&mut self) -> Option<Update> {
        let mut merge = Vec::with_capacity(2);
        let store = self.store_mut();
        if let Some(pending) = store.pending.take() {
            merge.push(pending.update);
        }
        if let Some(pending_ds) = store.pending_ds.take() {
            let mut u = Update::new();
            u.delete_set = pending_ds.clone();
            merge.push(u);
        }
        if merge.is_empty() {
            None
        } else {
            Some(Update::merge_updates(merge))
        }
    }
}

fn merge_pending_v1(update: Vec<u8>, store: &Store) -> Vec<u8> {
    let mut merge = VecDeque::new();
    if let Some(pending) = store.pending.as_ref() {
        merge.push_back(pending.update.encode_v1());
    }
    if let Some(pending_ds) = store.pending_ds.as_ref() {
        let mut u = Update::new();
        u.delete_set = pending_ds.clone();
        merge.push_back(u.encode_v1());
    }
    if merge.is_empty() {
        update
    } else {
        merge.push_front(update);
        merge_updates_v1(merge).unwrap()
    }
}

fn merge_pending_v2(update: Vec<u8>, store: &Store) -> Vec<u8> {
    let mut merge = VecDeque::new();
    if let Some(pending) = store.pending.as_ref() {
        merge.push_back(pending.update.encode_v2());
    }
    if let Some(pending_ds) = store.pending_ds.as_ref() {
        let mut u = Update::new();
        u.delete_set = pending_ds.clone();
        merge.push_back(u.encode_v2());
    }
    if merge.is_empty() {
        update
    } else {
        merge.push_front(update);
        merge_updates_v2(merge).unwrap()
    }
}

/// A very lightweight read-only transaction. These transactions are guaranteed to not modify the
/// contents of an underlying [Doc] and can be used to read it or for serialization purposes.
/// For this reason it's allowed to have a multiple active read-only transactions, but it's
/// not allowed to have any active [read-write transactions](TransactionMut) at the same time.
#[derive(Debug)]
pub struct Transaction<'doc> {
    store: RwLockReadGuard<'doc, Store>,
}

impl<'doc> Transaction<'doc> {
    pub(crate) fn new(store: RwLockReadGuard<'doc, Store>) -> Self {
        Transaction { store }
    }
}

impl<'doc> ReadTxn for Transaction<'doc> {
    #[inline]
    fn store(&self) -> &Store {
        self.store.deref()
    }
}

/// Read-write transaction. It can be used to modify an underlying state of the corresponding [Doc].
/// Read-write transactions require an exclusive access to document store - only one such
/// transaction can be present per [Doc] at the same time (read-only [Transaction]s are not allowed
/// to coexists at the same time as well).
///
/// This transaction type stores the information about all of the changes performed in its scope.
/// These will be used during [TransactionMut::commit] call to optimize metadata of incoming updates,
/// triggering necessary event callbacks etc. For performance reasons it's preferred to batch as
/// many updates as possible using the same transaction.
///
/// In Yrs transactions are always auto-committing all of their changes when dropped. Rollbacks are
/// not supported (if some operations needs to be undone, this can be achieved using [UndoManager])
pub struct TransactionMut<'doc> {
    pub(crate) store: RwLockWriteGuard<'doc, Store>,
    /// State vector of a current transaction at the moment of its creation.
    pub(crate) before_state: StateVector,
    /// Current state vector of a transaction, which includes all performed updates.
    pub(crate) after_state: StateVector,
    /// ID's of the blocks to be merged.
    pub(crate) merge_blocks: Vec<ID>,
    /// Describes the set of deleted items by ids.
    pub(crate) delete_set: DeleteSet,
    /// We store the reference that last moved an item. This is needed to compute the delta
    /// when multiple ContentMove move the same item.
    pub(crate) prev_moved: HashMap<ItemPtr, ItemPtr>,
    /// All types that were directly modified (property added or child inserted/deleted).
    /// New types are not included in this Set.
    pub(crate) changed: HashMap<TypePtr, HashSet<Option<Arc<str>>>>,
    pub(crate) changed_parent_types: Vec<BranchPtr>,
    pub(crate) subdocs: Option<Box<Subdocs>>,
    pub(crate) origin: Option<Origin>,
    doc: Doc,
    committed: bool,
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

    fn subdocs_mut(&mut self) -> &mut Subdocs {
        self.subdocs.get_or_init()
    }
}

impl<'doc> Drop for TransactionMut<'doc> {
    fn drop(&mut self) {
        self.commit()
    }
}

impl<'doc> TransactionMut<'doc> {
    pub(crate) fn new(
        doc: Doc,
        store: RwLockWriteGuard<'doc, Store>,
        origin: Option<Origin>,
    ) -> Self {
        let begin_timestamp = store.blocks.get_state_vector();
        TransactionMut {
            store,
            doc,
            origin,
            before_state: begin_timestamp,
            merge_blocks: Vec::default(),
            delete_set: DeleteSet::new(),
            after_state: StateVector::default(),
            changed: HashMap::default(),
            changed_parent_types: Vec::default(),
            prev_moved: HashMap::default(),
            subdocs: None,
            committed: false,
        }
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    pub fn events(&self) -> Option<&StoreEvents> {
        self.store.events.as_deref()
    }

    pub fn events_mut(&mut self) -> &mut StoreEvents {
        self.store.events.get_or_init()
    }

    /// Corresponding document's state vector at the moment when current transaction was created.
    pub fn before_state(&self) -> &StateVector {
        &self.before_state
    }

    /// State vector of the transaction after [Transaction::commit] has been called.
    pub fn after_state(&self) -> &StateVector {
        &self.after_state
    }

    /// Data about deletions performed in the scope of current transaction.
    pub fn delete_set(&self) -> &DeleteSet {
        &self.delete_set
    }

    /// Returns origin of the transaction if any was defined. Read-write transactions can get an
    /// origin assigned via [Transact::try_transact_mut_with]/[Transact::transact_mut_with] methods.
    pub fn origin(&self) -> Option<&Origin> {
        self.origin.as_ref()
    }

    /// Returns a list of root level types changed in a scope of the current transaction. This
    /// list is not filled right away, but as a part of [TransactionMut::commit] process.
    pub fn changed_parent_types(&self) -> &[BranchPtr] {
        &self.changed_parent_types
    }

    #[inline]
    pub(crate) fn store(&self) -> &Store {
        &self.store
    }

    #[inline]
    pub(crate) fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    /// Encodes changes made within the scope of the current transaction using lib0 v1 encoding.
    ///
    /// Document updates are idempotent and commutative. Caveats:
    /// * It doesn't matter in which order document updates are applied.
    /// * As long as all clients receive the same document updates, all clients
    ///   end up with the same content.
    /// * Even if an update contains known information, the unknown information
    ///   is extracted and integrated into the document structure.
    pub fn encode_update_v1(&self) -> Vec<u8> {
        let mut encoder = EncoderV1::new();
        self.encode_update(&mut encoder);
        encoder.to_vec()
    }

    /// Encodes changes made within the scope of the current transaction using lib0 v2 encoding.
    ///
    /// Document updates are idempotent and commutative. Caveats:
    /// * It doesn't matter in which order document updates are applied.
    /// * As long as all clients receive the same document updates, all clients
    ///   end up with the same content.
    /// * Even if an update contains known information, the unknown information
    ///   is extracted and integrated into the document structure.
    pub fn encode_update_v2(&self) -> Vec<u8> {
        let mut encoder = EncoderV2::new();
        self.encode_update(&mut encoder);
        encoder.to_vec()
    }

    /// Encodes changes made within the scope of the current transaction.
    ///
    /// Document updates are idempotent and commutative. Caveats:
    /// * It doesn't matter in which order document updates are applied.
    /// * As long as all clients receive the same document updates, all clients
    ///   end up with the same content.
    /// * Even if an update contains known information, the unknown information
    ///   is extracted and integrated into the document structure.
    pub fn encode_update<E: Encoder>(&self, encoder: &mut E) {
        let store = self.store();
        store.write_blocks_from(&self.before_state, encoder);
        self.delete_set.encode(encoder);
    }

    /// Applies given `id_set` onto current transaction to run multi-range deletion.
    /// Returns a remaining of original ID set, that couldn't be applied.
    pub(crate) fn apply_delete(&mut self, ds: &DeleteSet) -> Option<DeleteSet> {
        let mut unapplied = DeleteSet::new();
        for (client, ranges) in ds.iter() {
            if let Some(mut blocks) = self.store.blocks.get_client_mut(client) {
                let state = blocks.clock();

                for range in ranges.iter() {
                    let clock = range.start;
                    let clock_end = range.end;

                    if clock < state {
                        if state < clock_end {
                            unapplied.insert(ID::new(*client, clock), clock_end - state);
                        }
                        // We can ignore the case of GC and Delete structs, because we are going to skip them
                        if let Some(mut index) = blocks.find_pivot(clock) {
                            // We can ignore the case of GC and Delete structs, because we are going to skip them
                            let ptr = &mut blocks[index];
                            if let Some(item) = ptr.as_item() {
                                // split the first item if necessary
                                if !item.is_deleted() && item.id.clock < clock {
                                    if let Some(split) = self
                                        .store
                                        .blocks
                                        .split_block_inner(item, clock - item.id.clock)
                                    {
                                        if item.moved.is_some() {
                                            if let Some(&prev_moved) = self.prev_moved.get(&item) {
                                                self.prev_moved.insert(split, prev_moved);
                                            }
                                        }

                                        index += 1;
                                        self.merge_blocks.push(*split.id());
                                    }
                                    blocks = self.store.blocks.get_client_mut(client).unwrap();
                                }

                                while index < blocks.len() {
                                    let block = &mut blocks[index];
                                    if let Some(item) = block.as_item() {
                                        if item.id.clock < clock_end {
                                            if !item.is_deleted() {
                                                if item.id.clock + item.len() > clock_end {
                                                    if let Some(split) =
                                                        self.store.blocks.split_block_inner(
                                                            item,
                                                            clock_end - item.id.clock,
                                                        )
                                                    {
                                                        if item.moved.is_some() {
                                                            if let Some(&prev_moved) =
                                                                self.prev_moved.get(&item)
                                                            {
                                                                self.prev_moved
                                                                    .insert(split, prev_moved);
                                                            }
                                                        }
                                                        if item.info.is_linked() {
                                                            if let Some(links) = self
                                                                .store
                                                                .linked_by
                                                                .get(&item)
                                                                .cloned()
                                                            {
                                                                self.store
                                                                    .linked_by
                                                                    .insert(split, links);
                                                            }
                                                        }

                                                        self.merge_blocks.push(*split.id());
                                                        index += 1;
                                                    }
                                                }
                                                self.delete(item);
                                                blocks = self
                                                    .store
                                                    .blocks
                                                    .get_client_mut(client)
                                                    .unwrap();
                                                // just to make the borrow checker happy
                                            }
                                        } else {
                                            break;
                                        }
                                    }
                                    index += 1;
                                }
                            }
                        }
                    } else {
                        unapplied.insert(ID::new(*client, clock), clock_end - clock);
                    }
                }
            }
        }

        if unapplied.is_empty() {
            None
        } else {
            Some(unapplied)
        }
    }

    /// Delete item under given pointer.
    /// Returns true if block was successfully deleted, false if it was already deleted in the past.
    pub(crate) fn delete(&mut self, mut item: ItemPtr) -> bool {
        let mut recurse = Vec::new();
        let mut result = false;

        let ptr = item.clone();
        let store = self.store.deref();
        if !item.is_deleted() {
            if item.parent_sub.is_none() && item.is_countable() {
                if let TypePtr::Branch(mut parent) = item.parent {
                    parent.block_len -= item.len();
                    parent.content_len -= item.content_len(store.offset_kind);
                }
            }

            item.mark_as_deleted();
            self.delete_set.insert(item.id.clone(), item.len());
            if let Some(parent) = item.parent.as_branch() {
                self.add_changed_type(*parent, item.parent_sub.clone());
            } else {
                // parent has been GC'ed
            }

            match &mut item.content {
                ItemContent::Doc(_, doc) => {
                    let subdocs = self.subdocs.get_or_init();
                    let addr = doc.addr();
                    if subdocs.added.remove(&addr).is_none() {
                        subdocs.removed.insert(addr, doc.clone());
                    }
                }
                ItemContent::Type(inner) => {
                    let branch_ptr = BranchPtr::from(inner);
                    #[cfg(feature = "weak")]
                    if let crate::types::TypeRef::WeakLink(source) = &branch_ptr.type_ref {
                        source.unlink_all(self, branch_ptr);
                    }
                    let mut ptr = branch_ptr.start;
                    self.changed.remove(&TypePtr::Branch(branch_ptr));

                    while let Some(item) = ptr.as_deref() {
                        if !item.is_deleted() {
                            recurse.push(ptr.unwrap());
                        }

                        ptr = item.right.clone();
                    }

                    for ptr in branch_ptr.map.values() {
                        recurse.push(ptr.clone());
                    }
                }
                ItemContent::Move(m) => m.delete(self, ptr),
                _ => { /* nothing to do for other content types */ }
            }
            if item.info.is_linked() {
                // notify links that current element has been removed
                if let Some(linked_by) = self.store.linked_by.remove(&item) {
                    for link in linked_by {
                        self.add_changed_type(link, item.parent_sub.clone());
                    }
                }
            }
            result = true;
        }

        for &ptr in recurse.iter() {
            let id = *ptr.id();
            if !self.delete(ptr) {
                // Whis will be gc'd later and we want to merge it if possible
                // We try to merge all deleted items after each transaction,
                // but we have no knowledge about that this needs to be merged
                // since it is not in transaction.ds. Hence we add it to transaction._mergeStructs
                self.merge_blocks.push(id);
            }
        }

        result
    }

    /// Applies a deserialized [Update] contents into a document owning current transaction. Update
    /// payload can be generated by methods such as [TransactionMut::encode_diff] or passed to
    /// [Doc::observe_update_v1]/[Doc::observe_update_v2] callbacks. Updates are allowed to contain
    /// duplicate blocks (already presen in current document store) - these will be ignored.
    ///
    /// # Pending updates
    ///
    /// Remote update integration requires that all to-be-integrated blocks must have their direct
    /// predecessors already in place. Out of order updates from the same peer will be stashed
    /// internally and their integration will be postponed until missing blocks arrive first.
    pub fn apply_update(&mut self, update: Update) -> Result<(), UpdateError> {
        let (remaining, remaining_ds) = update.integrate(self)?;
        let mut retry = false;
        {
            let store = self.store_mut();
            store.pending = if let Some(mut pending) = store.pending.take() {
                // check if we can apply something
                for (client, &clock) in pending.missing.iter() {
                    if clock < store.blocks.get_clock(client) {
                        retry = true;
                        break;
                    }
                }

                if let Some(remaining) = remaining {
                    // merge restStructs into store.pending
                    for (&client, &clock) in remaining.missing.iter() {
                        pending.missing.set_min(client, clock);
                    }
                    pending.update = Update::merge_updates(vec![pending.update, remaining.update]);
                }
                Some(pending)
            } else {
                remaining
            };
        }
        if let Some(pending) = self.store_mut().pending_ds.take() {
            let ds2 = self.apply_delete(&pending);
            let ds = match (remaining_ds, ds2) {
                (Some(mut a), Some(b)) => {
                    a.delete_set.merge(b);
                    Some(a.delete_set)
                }
                (Some(x), _) => Some(x.delete_set),
                (_, Some(x)) => Some(x),
                _ => None,
            };
            self.store_mut().pending_ds = ds;
        } else {
            self.store_mut().pending_ds = remaining_ds.map(|update| update.delete_set);
        }

        if retry {
            let store = self.store_mut();
            if let Some(pending) = store.pending.take() {
                let ds = store.pending_ds.take().unwrap_or_default();
                let mut ds_update = Update::new();
                ds_update.delete_set = ds;
                self.apply_update(pending.update)?;
                self.apply_update(ds_update)?;
            }
        }

        Ok(())
    }

    pub(crate) fn create_item<T: Prelim>(
        &mut self,
        pos: &ItemPosition,
        value: T,
        parent_sub: Option<Arc<str>>,
    ) -> Option<ItemPtr> {
        let (left, right, origin, id) = {
            let store = self.store_mut();
            let left = pos.left;
            let right = pos.right;
            let origin = if let Some(item) = pos.left.as_deref() {
                Some(item.last_id())
            } else {
                None
            };
            let client_id = store.client_id;
            let id = ID::new(client_id, store.get_local_state());

            (left, right, origin, id)
        };
        let (mut content, remainder) = value.into_content(self);
        let inner_ref = if let ItemContent::Type(inner_ref) = &mut content {
            Some(BranchPtr::from(inner_ref))
        } else {
            None
        };
        let mut block = Item::new(
            id,
            left,
            origin,
            right,
            right.map(|r| r.id().clone()),
            pos.parent.clone(),
            parent_sub,
            content,
        )?;
        let mut block_ptr = ItemPtr::from(&mut block);

        block_ptr.integrate(self, 0);

        self.store_mut().blocks.push_block(block);

        if let Some(remainder) = remainder {
            remainder.integrate(self, inner_ref.unwrap().into())
        }

        Some(block_ptr)
    }

    fn call_type_observers(
        changed_parent_types: &mut Vec<BranchPtr>,
        all_links: &HashMap<ItemPtr, HashSet<BranchPtr>>,
        branch: BranchPtr,
        changed_parents: &mut HashMap<BranchPtr, Vec<usize>>,
        event_cache: &Vec<Event>,
        visited: &mut HashSet<BranchPtr>,
    ) {
        let mut current = branch;
        loop {
            changed_parent_types.push(current);
            if current.deep_observers.has_subscribers() {
                let entries = changed_parents.entry(current).or_default();
                entries.push(event_cache.len() - 1);
            }

            if let Some(item) = current.item {
                if item.info.is_linked() {
                    if let Some(linked_by) = all_links.get(&item) {
                        for &link in linked_by.iter() {
                            if visited.insert(link) {
                                Self::call_type_observers(
                                    changed_parent_types,
                                    all_links,
                                    link,
                                    changed_parents,
                                    event_cache,
                                    visited,
                                )
                            }
                        }
                    }
                }
                if let TypePtr::Branch(parent) = item.parent {
                    current = parent;
                    continue;
                }
            }

            break;
        }
    }

    /// Commits current transaction. This step involves cleaning up and optimizing changes performed
    /// during lifetime of a transaction. Such changes include squashing delete sets data,
    /// squashing blocks that have been appended one after another to preserve memory and triggering
    /// events.
    ///
    /// This step is performed automatically when a transaction is about to be dropped (its life
    /// scope comes to an end).
    pub fn commit(&mut self) {
        if self.committed {
            return;
        }
        self.committed = true;

        // 1. sort and merge delete set
        self.delete_set.squash();
        self.after_state = self.store.blocks.get_state_vector();
        // 2. emit 'beforeObserverCalls'
        // 3. for each change observed by the transaction call 'afterTransaction'
        if !self.changed.is_empty() {
            let mut changed_parents: HashMap<BranchPtr, Vec<usize>> = HashMap::new();
            let mut event_cache = Vec::new();

            for (ptr, subs) in self.changed.iter() {
                if let TypePtr::Branch(branch) = ptr {
                    if let Some(e) = branch.trigger(self, subs.clone()) {
                        event_cache.push(e);
                        Self::call_type_observers(
                            &mut self.changed_parent_types,
                            &self.store.linked_by,
                            *branch,
                            &mut changed_parents,
                            &event_cache,
                            &mut HashSet::default(),
                        );
                    }
                }
            }

            // deep observe events
            for (&branch, events) in changed_parents.iter() {
                // sort events by path length so that top-level events are fired first.
                let mut unsorted: Vec<&Event> = Vec::with_capacity(events.len());

                for &i in events.iter() {
                    let e = &mut event_cache[i];
                    e.set_current_target(branch);
                }

                for &i in events.iter() {
                    unsorted.push(&event_cache[i]);
                }

                // We don't need to check for events.length
                // because we know it has at least one element
                let events = Events::new(&mut unsorted);
                branch.trigger_deep(self, &events);
            }
        }

        if let Some(events) = self.store.events.take() {
            events.emit_after_transaction(self);
            self.store.events = Some(events);
        }

        // 4. try GC delete set
        if !self.store.skip_gc {
            GCCollector::collect(self);
        }

        // 5. try merge delete set
        self.delete_set.try_squash_with(&mut self.store);

        // 6. get transaction after state and try to merge to left
        for (client, &clock) in self.after_state.iter() {
            let before_clock = self.before_state.get(client);
            if before_clock != clock {
                let blocks = self.store.blocks.get_client_mut(client).unwrap();
                let first_change = blocks.find_pivot(before_clock).unwrap().max(1);
                let mut i = blocks.len() - 1;
                while i >= first_change {
                    blocks.squash_left(i);
                    i -= 1;
                }
            }
        }

        // 7. get merge_structs and try to merge to left
        for id in self.merge_blocks.iter() {
            if let Some(blocks) = self.store.blocks.get_client_mut(&id.client) {
                if let Some(replaced_pos) = blocks.find_pivot(id.clock) {
                    if replaced_pos + 1 < blocks.len() {
                        blocks.squash_left(replaced_pos + 1);
                    } else if replaced_pos > 0 {
                        blocks.squash_left(replaced_pos);
                    }
                }
            }
        }

        if let Some(events) = self.store.events.as_ref() {
            // 8. emit 'afterTransactionCleanup'
            events.emit_transaction_cleanup(self);
            // 9. emit 'update'
            events.emit_update_v1(self);
            // 10. emit 'updateV2'
            events.emit_update_v2(self);
        }

        // 11. add and remove subdocs
        let store = self.store.deref_mut();
        if let Some(mut subdocs) = self.subdocs.take() {
            let client_id = store.client_id;
            for (guid, subdoc) in subdocs.added.iter_mut() {
                let mut txn = subdoc.transact_mut();
                txn.store.client_id = client_id;
                txn.doc
                    .store()
                    .set_subdoc_data(client_id, self.doc.collection_id());
                store.subdocs.insert(guid.clone(), subdoc.clone());
            }
            for guid in subdocs.removed.keys() {
                store.subdocs.remove(guid);
            }

            let store = self.store.deref();
            let mut removed = if let Some(events) = store.events.as_ref() {
                if events.subdocs_events.has_subscribers() {
                    let e = SubdocsEvent::new(subdocs);
                    events.subdocs_events.trigger(|cb| cb(self, &e));
                    e.removed
                } else {
                    subdocs.removed
                }
            } else {
                subdocs.removed
            };

            for (_, subdoc) in removed.iter_mut() {
                subdoc.destroy(self);
            }
        }
    }

    /// Perform garbage collection of deleted blocks, even if a document was created with `skip_gc`
    /// option.
    ///
    /// If `delete_set` is provided, it will be used to limit the scope of garbage collection
    /// to only those blocks that are present in the delete set. If `delete_set` is `None`, all
    /// deleted blocks will be considered for garbage collection.
    pub fn gc(&mut self, delete_set: Option<&DeleteSet>) {
        GCCollector::collect_all(self, delete_set);
    }

    pub(crate) fn add_changed_type(&mut self, parent: BranchPtr, parent_sub: Option<Arc<str>>) {
        let trigger = if let Some(ptr) = parent.item {
            (ptr.id().clock < self.before_state.get(&ptr.id().client)) && !ptr.is_deleted()
        } else {
            true
        };
        if trigger {
            let e = self.changed.entry(parent.into()).or_default();
            e.insert(parent_sub.clone());
        }
    }

    /// Checks if item with a given `id` has been added to a block store within this transaction.
    pub(crate) fn has_added(&self, id: &ID) -> bool {
        id.clock >= self.before_state.get(&id.client)
    }

    /// Checks if item with a given `id` has been deleted within this transaction.
    pub(crate) fn has_deleted(&self, id: &ID) -> bool {
        self.delete_set.is_deleted(id)
    }

    pub(crate) fn split_by_snapshot(&mut self, snapshot: &Snapshot) {
        let mut merge_blocks: Vec<ID> = Vec::new();
        let blocks = &mut self.store.blocks;
        for (&client, &clock) in snapshot.state_map.iter() {
            if let Some(ptr) = blocks.get_item(&ID::new(client, clock)) {
                let ptr_clock = ptr.id.clock;
                if ptr_clock < clock {
                    if let Some(right) = blocks.split_block_inner(ptr, clock - ptr_clock) {
                        if right.moved.is_some() {
                            if let Some(&prev_moved) = self.prev_moved.get(&ptr) {
                                self.prev_moved.insert(right, prev_moved);
                            }
                        }

                        merge_blocks.push(*right.id());
                    }
                }
            }
        }

        self.merge_blocks.append(&mut merge_blocks);
        let mut deleted = snapshot.delete_set.deleted_blocks();
        while let Some(slice) = deleted.next(self) {
            if let BlockSlice::Item(slice) = slice {
                //TODO: we technically don't need to physically split underlying item in two
                // if we were to use block slices all the way down.

                // split the blocks by delete set
                let ptr = self.store.materialize(slice);
                self.merge_blocks.push(ptr.id);
            }
        }
    }

    #[cfg(feature = "weak")]
    pub(crate) fn unlink(&mut self, mut source: ItemPtr, link: BranchPtr) {
        let all_links = &mut self.store.linked_by;
        let prune = if let Some(linked_by) = all_links.get_mut(&source) {
            linked_by.remove(&link) && linked_by.is_empty()
        } else {
            false
        };
        if prune {
            all_links.remove(&source);
            source.info.clear_linked();
            if source.is_countable() {
                // since linked property is blocking items from merging,
                // it may turn out that source item can be merged now
                self.merge_blocks.push(source.id);
            }
        }
    }
}

/// Iterator struct used to traverse over all of the root level types defined in a corresponding [Doc].
pub struct RootRefs<'doc>(std::collections::hash_map::Iter<'doc, Arc<str>, Box<Branch>>);

impl<'doc> Iterator for RootRefs<'doc> {
    type Item = (&'doc str, Out);

    fn next(&mut self) -> Option<Self::Item> {
        let (key, branch) = self.0.next()?;
        let key = key.as_ref();
        let ptr = BranchPtr::from(branch);
        Some((key, ptr.into()))
    }
}

#[derive(Default)]
pub struct Subdocs {
    pub(crate) added: HashMap<DocAddr, Doc>,
    pub(crate) removed: HashMap<DocAddr, Doc>,
    pub(crate) loaded: HashMap<DocAddr, Doc>,
}

/// A binary marker that can be assigned to a read-write transaction upon creation via
/// [Transact::try_transact_mut_with]/[Transact::transact_mut_with]. It can be used to classify
/// transaction updates within a specific context, which exists for the duration of a transaction
/// (it's **not persisted** in the document store itself), i.e. *you can use unique document client
/// identifiers to differentiate updates incoming from remote nodes from those performed locally*.
#[repr(transparent)]
#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct Origin(SmallVec<[u8; std::mem::size_of::<usize>()]>);

impl AsRef<[u8]> for Origin {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<'a, T> From<Pin<&'a T>> for Origin {
    fn from(p: Pin<&T>) -> Self {
        let ptr = Pin::get_ref(p) as *const T as usize;
        Origin(SmallVec::from_const(ptr.to_be_bytes()))
    }
}

impl<'a> From<&'a [u8]> for Origin {
    fn from(slice: &'a [u8]) -> Self {
        Origin(SmallVec::from_slice(slice))
    }
}

impl<'a> From<&'a str> for Origin {
    fn from(v: &'a str) -> Self {
        Origin(SmallVec::from_slice(v.as_ref()))
    }
}

impl From<String> for Origin {
    fn from(v: String) -> Self {
        Origin(SmallVec::from(Vec::from(v)))
    }
}

impl std::fmt::Debug for Origin {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Origin(")?;
        for b in self.0.iter() {
            write!(f, "{:02x?}", b)?;
        }
        write!(f, ")")
    }
}

macro_rules! impl_origin {
    ($t:ty) => {
        impl From<$t> for Origin {
            fn from(v: $t) -> Origin {
                Origin(SmallVec::from_slice(&v.to_be_bytes()))
            }
        }
    };
}

impl_origin!(u8);
impl_origin!(u16);
impl_origin!(u32);
impl_origin!(u64);
impl_origin!(u128);
impl_origin!(usize);
impl_origin!(i8);
impl_origin!(i16);
impl_origin!(i32);
impl_origin!(i64);
impl_origin!(i128);
impl_origin!(isize);
