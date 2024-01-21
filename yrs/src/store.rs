use crate::block::{BlockCell, ClientID, ItemContent, ItemPtr};
use crate::block_store::BlockStore;
use crate::doc::{
    AfterTransactionSubscription, DestroySubscription, DocAddr, Options, SubdocsSubscription,
};
use crate::error::Error;
use crate::event::SubdocsEvent;
use crate::id_set::DeleteSet;
use crate::slice::ItemSlice;
use crate::types::{Branch, BranchPtr, Path, PathSegment, TypeRef};
use crate::update::PendingUpdate;
use crate::updates::encoder::{Encode, Encoder};
use crate::StateVector;
use crate::{
    Doc, Observer, OffsetKind, Snapshot, SubscriptionId, TransactionCleanupEvent,
    TransactionCleanupSubscription, TransactionMut, UpdateEvent, UpdateSubscription, Uuid, ID,
};
use atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut, BorrowError, BorrowMutError};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::sync::{Arc, Weak};

/// Store is a core element of a document. It contains all of the information, like block store
/// map of root types, pending updates waiting to be applied once a missing update information
/// arrives and all subscribed callbacks.
pub struct Store {
    pub(crate) options: Options,

    /// Root types (a.k.a. top-level types). These types are defined by users at the document level,
    /// they have their own unique names and represent core shared types that expose operations
    /// which can be called concurrently by remote peers in a conflict-free manner.
    pub(crate) types: HashMap<Arc<str>, Arc<Branch>>,

    /// Registry of all alive nodes in the document store.
    pub(crate) node_registry: HashSet<BranchPtr>,

    /// A block store of a current document. It represent all blocks (inserted or tombstoned
    /// operations) integrated - and therefore visible - into a current document.
    pub(crate) blocks: BlockStore,

    /// A pending update. It contains blocks, which are not yet integrated into `blocks`, usually
    /// because due to issues in update exchange, there were some missing blocks that need to be
    /// integrated first before the data from `pending` can be applied safely.
    pub(crate) pending: Option<PendingUpdate>,

    /// A pending delete set. Just like `pending`, it contains deleted ranges of blocks that have
    /// not been yet applied due to missing blocks that prevent `pending` update to be integrated
    /// into `blocks`.
    pub(crate) pending_ds: Option<DeleteSet>,

    pub(crate) subdocs: HashMap<DocAddr, Doc>,

    pub(crate) events: Option<Box<StoreEvents>>,

    /// Pointer to a parent block - present only if a current document is a sub-document of another
    /// document.
    pub(crate) parent: Option<ItemPtr>,

    /// Dependencies between items and weak links pointing to these items.
    pub(crate) linked_by: HashMap<ItemPtr, HashSet<BranchPtr>>,
}

impl Store {
    /// Create a new empty store in context of a given `client_id`.
    pub(crate) fn new(options: Options) -> Self {
        Store {
            options,
            types: HashMap::default(),
            node_registry: HashSet::default(),
            blocks: BlockStore::default(),
            subdocs: HashMap::default(),
            linked_by: HashMap::default(),
            events: None,
            pending: None,
            pending_ds: None,
            parent: None,
        }
    }

    pub fn is_subdoc(&self) -> bool {
        self.parent.is_some()
    }

    /// Get the latest clock sequence number observed and integrated into a current store client.
    /// This is exclusive value meaning it describes a clock value of the beginning of the next
    /// block that's about to be inserted. You cannot use that clock value to find any existing
    /// block content.
    pub fn get_local_state(&self) -> u32 {
        self.blocks.get_clock(&self.options.client_id)
    }

    /// Returns a branch reference to a complex type identified by its pointer. Returns `None` if
    /// no such type could be found or was ever defined.
    pub(crate) fn get_type<K: Into<Arc<str>>>(&self, key: K) -> Option<BranchPtr> {
        let ptr = BranchPtr::from(self.types.get(&key.into())?);
        Some(ptr)
    }

    /// Returns a branch reference to a complex type identified by its pointer. Returns `None` if
    /// no such type could be found or was ever defined.
    pub(crate) fn get_or_create_type<K: Into<Arc<str>>>(
        &mut self,
        key: K,
        type_ref: TypeRef,
    ) -> BranchPtr {
        let key = key.into();
        match self.types.entry(key.clone()) {
            Entry::Occupied(mut e) => {
                let branch = Arc::get_mut(e.get_mut()).unwrap();
                branch.repair_type_ref(type_ref);
                BranchPtr::from(e.get_mut())
            }
            Entry::Vacant(e) => {
                let mut branch = Branch::new(type_ref);
                let branch_ref = BranchPtr::from(&mut branch);
                self.node_registry.insert(branch_ref);
                e.insert(branch);
                branch_ref
            }
        }
    }

    pub(crate) fn get_type_key(&self, ptr: BranchPtr) -> Option<&Arc<str>> {
        let branch = ptr.deref() as *const Branch;
        for (k, v) in self.types.iter() {
            let target = v.as_ref() as *const Branch;
            if std::ptr::eq(target, branch) {
                return Some(k);
            }
        }
        None
    }

    /// Encodes all changes from current transaction block store up to a given `snapshot`.
    /// This enables to encode state of a document at some specific point in the past.
    pub fn encode_state_from_snapshot<E: Encoder>(
        &self,
        snapshot: &Snapshot,
        encoder: &mut E,
    ) -> Result<(), Error> {
        if !self.options.skip_gc {
            return Err(Error::Gc);
        }
        self.write_blocks_to(&snapshot.state_map, encoder);
        snapshot.delete_set.encode(encoder);

        Ok(())
    }

    pub(crate) fn write_blocks_to<E: Encoder>(&self, sv: &StateVector, encoder: &mut E) {
        let local_sv = self.blocks.get_state_vector();
        let mut diff = Vec::with_capacity(sv.len());
        for (&client_id, &clock) in sv.iter() {
            if local_sv.contains_client(&client_id) {
                diff.push((client_id, clock.min(local_sv.get(&client_id))));
            }
        }
        // Write items with higher client ids first
        // This heavily improves the conflict algorithm.
        diff.sort_by(|a, b| b.0.cmp(&a.0));

        encoder.write_var(diff.len());
        for (client, clock) in diff {
            let blocks = self.blocks.get_client(&client).unwrap();
            let clock = clock.min(blocks.clock() + 1);
            let last_idx = blocks.find_pivot(clock - 1).unwrap();
            // write # encoded structs
            encoder.write_var(last_idx + 1);
            encoder.write_client(client);
            encoder.write_var(0);
            for i in 0..last_idx {
                let block = blocks[i].as_slice();
                block.encode(encoder, Some(self));
            }
            let last_block = &blocks[last_idx];
            // write first struct with an offset
            let mut slice = last_block.as_slice();
            slice.trim_end(slice.clock_end() - (clock - 1));
            slice.encode(encoder, Some(self));
        }
    }

    /// Compute a diff to sync with another client.
    ///
    /// This is the most efficient method to sync with another client by only
    /// syncing the differences.
    ///
    /// The sync protocol in Yrs/js is:
    /// * Send StateVector to the other client.
    /// * The other client comutes a minimal diff to sync by using the StateVector.
    pub fn encode_diff<E: Encoder>(&self, sv: &StateVector, encoder: &mut E) {
        //TODO: this could be actually 2 steps:
        // 1. create Diff of block store and remote state vector (it can have lifetime of bock store)
        // 2. make Diff implement Encode trait and encode it
        // this way we can add some extra utility method on top of Diff (like introspection) without need of decoding it.
        self.write_blocks_from(sv, encoder);
        let delete_set = DeleteSet::from(&self.blocks);
        delete_set.encode(encoder);
    }

    pub(crate) fn write_blocks_from<E: Encoder>(&self, sv: &StateVector, encoder: &mut E) {
        let local_sv = self.blocks.get_state_vector();
        let mut diff = Self::diff_state_vectors(&local_sv, sv);

        // Write items with higher client ids first
        // This heavily improves the conflict algorithm.
        diff.sort_by(|a, b| b.0.cmp(&a.0));

        encoder.write_var(diff.len());
        for (client, clock) in diff {
            let blocks = self.blocks.get_client(&client).unwrap();
            let clock = clock.max(blocks.get(0).map(|i| i.clock_start()).unwrap_or_default()); // make sure the first id exists
            let start = blocks.find_pivot(clock).unwrap();
            // write # encoded structs
            encoder.write_var(blocks.len() - start);
            encoder.write_client(client);
            encoder.write_var(clock);
            let first_block = blocks.get(start).unwrap();
            // write first struct with an offset
            let offset = clock - first_block.clock_start();
            let mut slice = first_block.as_slice();
            slice.trim_start(offset);
            slice.encode(encoder, Some(self));
            for i in (start + 1)..blocks.len() {
                blocks[i].as_slice().encode(encoder, Some(self));
            }
        }
    }

    fn diff_state_vectors(local_sv: &StateVector, remote_sv: &StateVector) -> Vec<(ClientID, u32)> {
        let mut diff = Vec::new();
        for (client, &remote_clock) in remote_sv.iter() {
            let local_clock = local_sv.get(client);
            if local_clock > remote_clock {
                diff.push((*client, remote_clock));
            }
        }
        for (client, _) in local_sv.iter() {
            if remote_sv.get(client) == 0 {
                diff.push((*client, 0));
            }
        }
        diff
    }

    pub fn get_type_from_path(&self, path: &Path) -> Option<BranchPtr> {
        let mut i = path.iter();
        if let Some(PathSegment::Key(root_name)) = i.next() {
            let mut current = self.get_type(root_name.clone())?;
            while let Some(segment) = i.next() {
                match segment {
                    PathSegment::Key(key) => {
                        let child = current.map.get(key)?;
                        if let ItemContent::Type(child_branch) = &child.content {
                            current = BranchPtr::from(child_branch.as_ref());
                        } else {
                            return None;
                        }
                    }
                    PathSegment::Index(index) => {
                        if let Some((ItemContent::Type(child_branch), _)) = current.get_at(*index) {
                            current = child_branch.into();
                        } else {
                            return None;
                        }
                    }
                }
            }
            Some(current)
        } else {
            None
        }
    }

    /// Consumes current block slice view, materializing it into actual block representation equivalent,
    /// splitting underlying block along [ItemSlice::start]/[ItemSlice::end] offsets.
    ///
    /// Returns a block created this way, that represents the boundaries that current [ItemSlice]
    /// was representing.
    pub(crate) fn materialize(&mut self, mut slice: ItemSlice) -> ItemPtr {
        let id = slice.id().clone();
        let blocks = self.blocks.get_client_mut(&id.client).unwrap();
        let mut links = None;
        let item = slice.ptr.deref();
        if item.info.is_linked() {
            links = self.linked_by.get(&slice.ptr).cloned();
        }

        let mut index = None;
        let mut ptr = if slice.adjacent_left() {
            slice.ptr
        } else {
            let mut i = blocks.find_pivot(id.clock).unwrap();
            if let Some(new) = slice.ptr.splice(slice.start, OffsetKind::Utf16) {
                if let Some(source) = links.clone() {
                    let dest = self.linked_by.entry(ItemPtr::from(&new)).or_default();
                    dest.extend(source);
                }
                blocks.insert(i + 1, BlockCell::Block(new));
                i += 1;
                //todo: txn merge blocks insert?
                index = Some(i);
            }
            let ptr = blocks[i].as_item().unwrap();
            slice = ItemSlice::new(ptr, 0, slice.end - slice.start);
            ptr
        };

        if !slice.adjacent_right() {
            // split block on the right side
            let i = if let Some(i) = index {
                i
            } else {
                let last_id = slice.last_id();
                blocks.find_pivot(last_id.clock).unwrap()
            };
            let new = ptr.splice(slice.len(), OffsetKind::Utf16).unwrap();
            if let Some(source) = links {
                let dest = self.linked_by.entry(ItemPtr::from(&new)).or_default();
                dest.extend(source);
            }
            blocks.insert(i + 1, BlockCell::Block(new));
            //todo: txn merge blocks insert?
        }

        ptr
    }

    /// Returns a collection of sub documents linked within the structures of this document store.
    pub fn subdocs(&self) -> SubdocsIter {
        SubdocsIter(self.subdocs.values())
    }

    /// Returns a collection of globally unique identifiers of sub documents linked within
    /// the structures of this document store.
    pub fn subdoc_guids(&self) -> SubdocGuids {
        SubdocGuids(self.subdocs.values())
    }

    pub(crate) fn follow_redone(&self, id: &ID) -> (Option<ItemPtr>, u32) {
        let mut next_id = Some(*id);
        let mut ptr = None;
        let mut diff = 0;
        while {
            if let Some(mut next) = next_id {
                if diff > 0 {
                    next.clock += diff;
                    next_id = Some(next.clone());
                }
                ptr = self.blocks.get_item(&next);
                if let Some(item) = ptr.as_deref() {
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
        (ptr, diff)
    }

    pub fn is_alive(&self, branch_ptr: &BranchPtr) -> bool {
        self.node_registry.contains(branch_ptr)
    }

    pub(crate) fn register(&mut self, branch: &mut Arc<Branch>) -> BranchPtr {
        let ptr = BranchPtr::from(branch);
        self.node_registry.insert(ptr);
        ptr
    }

    pub(crate) fn deregister(&mut self, branch: &mut Arc<Branch>) {
        let ptr = BranchPtr::from(branch);
        self.node_registry.remove(&ptr);
    }
}

impl Encode for Store {
    /// Encodes the document state to a binary format.
    ///
    /// Document updates are idempotent and commutative. Caveats:
    /// * It doesn't matter in which order document updates are applied.
    /// * As long as all clients receive the same document updates, all clients
    ///   end up with the same content.
    /// * Even if an update contains known information, the unknown information
    ///   is extracted and integrated into the document structure.
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        self.encode_diff(&StateVector::default(), encoder)
    }
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct(&self.options.client_id.to_string());
        if !self.types.is_empty() {
            s.field("root types", &self.types);
        }
        if !self.blocks.is_empty() {
            s.field("blocks", &self.blocks);
        }
        if let Some(pending) = self.pending.as_ref() {
            s.field("pending", pending);
        }
        if let Some(pending_ds) = self.pending_ds.as_ref() {
            s.field("pending delete set", pending_ds);
        }

        if let Some(parent) = self.parent.as_ref() {
            s.field("parent block", parent.id());
        }
        if !self.linked_by.is_empty() {
            s.field("links", &self.linked_by);
        }
        s.finish()
    }
}

#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct WeakStoreRef(pub(crate) Weak<AtomicRefCell<Store>>);

impl PartialEq for WeakStoreRef {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}

#[repr(transparent)]
#[derive(Debug, Clone)]
pub(crate) struct StoreRef(pub(crate) Arc<AtomicRefCell<Store>>);

impl StoreRef {
    pub fn try_borrow(&self) -> Result<AtomicRef<Store>, BorrowError> {
        self.0.try_borrow()
    }

    pub fn try_borrow_mut(&self) -> Result<AtomicRefMut<Store>, BorrowMutError> {
        self.0.try_borrow_mut()
    }

    pub fn weak_ref(&self) -> WeakStoreRef {
        WeakStoreRef(Arc::downgrade(&self.0))
    }

    pub fn options(&self) -> &Options {
        let store = unsafe { self.0.as_ptr().as_ref().unwrap() };
        &store.options
    }
}

impl From<Store> for StoreRef {
    fn from(store: Store) -> Self {
        StoreRef(Arc::new(AtomicRefCell::new(store)))
    }
}

#[repr(transparent)]
pub struct SubdocsIter<'doc>(std::collections::hash_map::Values<'doc, DocAddr, Doc>);

impl<'doc> Iterator for SubdocsIter<'doc> {
    type Item = &'doc Doc;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

#[repr(transparent)]
pub struct SubdocGuids<'doc>(std::collections::hash_map::Values<'doc, DocAddr, Doc>);

impl<'doc> Iterator for SubdocGuids<'doc> {
    type Item = &'doc Uuid;

    fn next(&mut self) -> Option<Self::Item> {
        let d = self.0.next()?;
        Some(&d.options().guid)
    }
}

#[derive(Default)]
pub(crate) struct StoreEvents {
    /// Handles subscriptions for the transaction cleanup event. Events are called with the
    /// newest updates once they are committed and compacted.
    pub(crate) transaction_cleanup_events:
        Option<Observer<Arc<dyn Fn(&TransactionMut, &TransactionCleanupEvent) -> ()>>>,

    /// Handles subscriptions for the `afterTransactionCleanup` event. Events are called with the
    /// newest updates once they are committed and compacted.
    pub(crate) after_transaction_events: Option<Observer<Arc<dyn Fn(&mut TransactionMut) -> ()>>>,

    /// A subscription handler. It contains all callbacks with registered by user functions that
    /// are supposed to be called, once a new update arrives.
    pub(crate) update_v1_events: Option<Observer<Arc<dyn Fn(&TransactionMut, &UpdateEvent) -> ()>>>,

    /// A subscription handler. It contains all callbacks with registered by user functions that
    /// are supposed to be called, once a new update arrives.
    pub(crate) update_v2_events: Option<Observer<Arc<dyn Fn(&TransactionMut, &UpdateEvent) -> ()>>>,

    /// Handles subscriptions for subdocs events.
    pub(crate) subdocs_events: Option<Observer<Arc<dyn Fn(&TransactionMut, &SubdocsEvent) -> ()>>>,

    pub(crate) destroy_events: Option<Observer<Arc<dyn Fn(&TransactionMut, &Doc) -> ()>>>,
}

impl StoreEvents {
    /// Subscribe callback function for any changes performed within transaction scope. These
    /// changes are encoded using lib0 v1 encoding and can be decoded using [Update::decode_v1] if
    /// necessary or passed to remote peers right away. This callback is triggered on function
    /// commit.
    ///
    /// Returns a subscription, which will unsubscribe function when dropped.
    pub fn observe_update_v1<F>(&mut self, f: F) -> Result<UpdateSubscription, BorrowMutError>
    where
        F: Fn(&TransactionMut, &UpdateEvent) -> () + 'static,
    {
        let eh = self.update_v1_events.get_or_insert_with(Observer::new);
        Ok(eh.subscribe(Arc::new(f)))
    }

    /// Manually unsubscribes from a callback used in [Doc::observe_update_v1] method.
    pub fn unobserve_update_v1(&self, subscription_id: SubscriptionId) {
        if let Some(handler) = self.update_v1_events.as_ref() {
            handler.unsubscribe(subscription_id);
        }
    }

    pub fn emit_update_v1(&self, txn: &TransactionMut) {
        if let Some(eh) = self.update_v1_events.as_ref() {
            if !txn.delete_set.is_empty() || txn.after_state != txn.before_state {
                // produce update only if anything changed
                let update = UpdateEvent::new_v1(txn);
                for fun in eh.callbacks() {
                    fun(txn, &update);
                }
            }
        }
    }

    /// Subscribe callback function for any changes performed within transaction scope. These
    /// changes are encoded using lib0 v1 encoding and can be decoded using [Update::decode_v2] if
    /// necessary or passed to remote peers right away. This callback is triggered on function
    /// commit.
    ///
    /// Returns a subscription, which will unsubscribe function when dropped.
    pub fn observe_update_v2<F>(&mut self, f: F) -> Result<UpdateSubscription, BorrowMutError>
    where
        F: Fn(&TransactionMut, &UpdateEvent) -> () + 'static,
    {
        let eh = self.update_v2_events.get_or_insert_with(Observer::new);
        Ok(eh.subscribe(Arc::new(f)))
    }

    /// Manually unsubscribes from a callback used in [Doc::observe_update_v1] method.
    pub fn unobserve_update_v2(&self, subscription_id: SubscriptionId) {
        if let Some(handler) = self.update_v2_events.as_ref() {
            handler.unsubscribe(subscription_id);
        }
    }

    pub fn emit_update_v2(&self, txn: &TransactionMut) {
        if let Some(eh) = self.update_v2_events.as_ref() {
            if !txn.delete_set.is_empty() || txn.after_state != txn.before_state {
                // produce update only if anything changed
                let update = UpdateEvent::new_v2(txn);
                for fun in eh.callbacks() {
                    fun(txn, &update);
                }
            }
        }
    }

    /// Subscribe callback function to updates on the `Doc`. The callback will receive state updates and
    /// deletions when a document transaction is committed.
    pub fn observe_after_transaction<F>(
        &mut self,
        f: F,
    ) -> Result<AfterTransactionSubscription, BorrowMutError>
    where
        F: Fn(&mut TransactionMut) -> () + 'static,
    {
        let subscription = self
            .after_transaction_events
            .get_or_insert_with(Observer::new)
            .subscribe(Arc::new(f));
        Ok(subscription)
    }

    /// Cancels the transaction cleanup callback associated with the `subscription_id`
    pub fn unobserve_after_transaction(&self, subscription_id: SubscriptionId) {
        if let Some(handler) = self.after_transaction_events.as_ref() {
            (*handler).unsubscribe(subscription_id);
        }
    }

    pub fn emit_after_transaction(&self, txn: &mut TransactionMut) {
        if let Some(eh) = self.after_transaction_events.as_ref() {
            for cb in eh.callbacks() {
                cb(txn);
            }
        }
    }

    /// Subscribe callback function to updates on the `Doc`. The callback will receive state updates and
    /// deletions when a document transaction is committed.
    pub fn observe_transaction_cleanup<F>(
        &mut self,
        f: F,
    ) -> Result<TransactionCleanupSubscription, BorrowMutError>
    where
        F: Fn(&TransactionMut, &TransactionCleanupEvent) -> () + 'static,
    {
        let subscription = self
            .transaction_cleanup_events
            .get_or_insert_with(Observer::new)
            .subscribe(Arc::new(f));
        Ok(subscription)
    }

    /// Cancels the transaction cleanup callback associated with the `subscription_id`
    pub fn unobserve_transaction_cleanup(&self, subscription_id: SubscriptionId) {
        if let Some(handler) = self.transaction_cleanup_events.as_ref() {
            (*handler).unsubscribe(subscription_id);
        }
    }

    pub fn emit_transaction_cleanup(&self, txn: &TransactionMut) {
        if let Some(eh) = self.transaction_cleanup_events.as_ref() {
            let event = TransactionCleanupEvent::new(txn);
            for fun in eh.callbacks() {
                fun(txn, &event);
            }
        }
    }

    /// Subscribe callback function, that will be called whenever a subdocuments inserted in this
    /// [Doc] will request a load.
    pub fn observe_subdocs<F>(&mut self, f: F) -> Result<SubdocsSubscription, BorrowMutError>
    where
        F: Fn(&TransactionMut, &SubdocsEvent) -> () + 'static,
    {
        let subscription = self
            .subdocs_events
            .get_or_insert_with(Observer::new)
            .subscribe(Arc::new(f));
        Ok(subscription)
    }

    /// Cancels the subscription created previously using [Doc::observe_subdocs].
    pub fn unobserve_subdocs(&self, subscription_id: SubscriptionId) {
        if let Some(handler) = self.subdocs_events.as_ref() {
            (*handler).unsubscribe(subscription_id);
        }
    }

    /// Subscribe callback function, that will be called whenever a [DocRef::destroy] has been called.
    pub fn observe_destroy<F>(&mut self, f: F) -> Result<DestroySubscription, BorrowMutError>
    where
        F: Fn(&TransactionMut, &Doc) -> () + 'static,
    {
        let subscription = self
            .destroy_events
            .get_or_insert_with(Observer::new)
            .subscribe(Arc::new(f));
        Ok(subscription)
    }

    /// Cancels the subscription created previously using [Doc::observe_destroy].
    pub fn unobserve_destroy(&self, subscription_id: SubscriptionId) {
        if let Some(handler) = self.destroy_events.as_ref() {
            (*handler).unsubscribe(subscription_id);
        }
    }
}
