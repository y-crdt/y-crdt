use crate::block::{ClientID, ItemContent};
use crate::block_store::{BlockStore, StateVector};
use crate::doc::Options;
use crate::event::{AfterTransactionEvent, EventHandler};
use crate::id_set::DeleteSet;
use crate::types::{Branch, BranchPtr, Path, PathSegment, TypeRefs};
use crate::update::PendingUpdate;
use crate::updates::encoder::{Encode, Encoder};
use crate::{Snapshot, UpdateEvent};
use lib0::error::Error;
use std::cell::UnsafeCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

/// Store is a core element of a document. It contains all of the information, like block store
/// map of root types, pending updates waiting to be applied once a missing update information
/// arrives and all subscribed callbacks.
pub(crate) struct Store {
    pub options: Options,

    /// Root types (a.k.a. top-level types). These types are defined by users at the document level,
    /// they have their own unique names and represent core shared types that expose operations
    /// which can be called concurrently by remote peers in a conflict-free manner.
    pub types: HashMap<Rc<str>, Box<Branch>>,

    /// A block store of a current document. It represent all blocks (inserted or tombstoned
    /// operations) integrated - and therefore visible - into a current document.
    pub(crate) blocks: BlockStore,

    /// A pending update. It contains blocks, which are not yet integrated into `blocks`, usually
    /// because due to issues in update exchange, there were some missing blocks that need to be
    /// integrated first before the data from `pending` can be applied safely.
    pub pending: Option<PendingUpdate>,

    /// A pending delete set. Just like `pending`, it contains deleted ranges of blocks that have
    /// not been yet applied due to missing blocks that prevent `pending` update to be integrated
    /// into `blocks`.
    pub pending_ds: Option<DeleteSet>,

    /// Handles subscriptions for the `afterTransactionCleanup` event. Events are called with the
    /// newest updates once they are committed and compacted.
    pub(crate) after_transaction_events: Option<EventHandler<AfterTransactionEvent>>,

    /// A subscription handler. It contains all callbacks with registered by user functions that
    /// are supposed to be called, once a new update arrives.
    pub(crate) update_v1_events: Option<EventHandler<UpdateEvent>>,

    /// A subscription handler. It contains all callbacks with registered by user functions that
    /// are supposed to be called, once a new update arrives.
    pub(crate) update_v2_events: Option<EventHandler<UpdateEvent>>,
}

impl Store {
    /// Create a new empty store in context of a given `client_id`.
    pub fn new(options: Options) -> Self {
        Store {
            options,
            types: Default::default(),
            blocks: BlockStore::new(),
            pending: None,
            pending_ds: None,
            update_v1_events: None,
            update_v2_events: None,
            after_transaction_events: None,
        }
    }

    /// Get the latest clock sequence number observed and integrated into a current store client.
    /// This is exclusive value meaning it describes a clock value of the beginning of the next
    /// block that's about to be inserted. You cannot use that clock value to find any existing
    /// block content.
    pub fn get_local_state(&self) -> u32 {
        self.blocks.get_state(&self.options.client_id)
    }

    /// Returns a branch reference to a complex type identified by its pointer. Returns `None` if
    /// no such type could be found or was ever defined.
    pub fn get_type<K: Into<Rc<str>>>(&self, key: K) -> Option<BranchPtr> {
        let ptr = BranchPtr::from(self.types.get(&key.into())?);
        Some(ptr)
    }

    /// Returns a branch reference to a complex type identified by its pointer. Returns `None` if
    /// no such type could be found or was ever defined.
    pub fn get_or_create_type<K: Into<Rc<str>>>(
        &mut self,
        key: K,
        node_name: Option<Rc<str>>,
        type_ref: TypeRefs,
    ) -> BranchPtr {
        let key = key.into();
        match self.types.entry(key.clone()) {
            Entry::Occupied(mut e) => {
                let branch = e.get_mut();
                branch.repair_type_ref(type_ref);
                BranchPtr::from(branch)
            }
            Entry::Vacant(e) => {
                let mut branch = Branch::new(type_ref, node_name);
                let branch_ref = BranchPtr::from(&mut branch);
                e.insert(branch);
                branch_ref
            }
        }
    }

    pub(crate) fn get_type_key(&self, ptr: BranchPtr) -> Option<&Rc<str>> {
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
            return Err(Error::Other("Cannot encode past state from the snapshot for a document with GC option flag set on".to_string()));
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
            let blocks = self.blocks.get(&client).unwrap();
            let clock = clock.min(blocks.last().last_id().clock);
            let last_idx = blocks.find_pivot(clock).unwrap();
            // write # encoded structs
            encoder.write_var(last_idx + 1);
            encoder.write_client(client);
            encoder.write_var(0);
            for i in 0..last_idx {
                let block = blocks.get(i);
                block.encode(Some(self), encoder);
            }
            let last_block = blocks.get(last_idx);
            // write first struct with an offset
            let offset = last_block.last_id().clock - clock;
            last_block.encode_to(Some(self), encoder, offset);
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
            let blocks = self.blocks.get(&client).unwrap();
            let clock = clock.max(blocks.first().id().clock); // make sure the first id exists
            let start = blocks.find_pivot(clock).unwrap();
            // write # encoded structs
            encoder.write_var(blocks.len() - start);
            encoder.write_client(client);
            encoder.write_var(clock);
            let first_block = blocks.get(start);
            // write first struct with an offset
            let offset = clock - first_block.id().clock;
            first_block.encode_from(Some(self), encoder, offset);
            for i in (start + 1)..blocks.len() {
                blocks.get(i).encode(Some(self), encoder);
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
                        let child = current.map.get(key)?.as_item()?;
                        if let ItemContent::Type(child_branch) = &child.content {
                            current = child_branch.into();
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

        s.finish()
    }
}

#[repr(transparent)]
#[derive(Debug, Clone)]
pub(crate) struct StoreRef(Rc<UnsafeCell<Store>>);

impl Deref for StoreRef {
    type Target = Store;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { (self.0.get() as *const Self::Target).as_ref().unwrap() }
    }
}

impl DerefMut for StoreRef {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.0.get().as_mut().unwrap() }
    }
}

impl From<Store> for StoreRef {
    fn from(store: Store) -> Self {
        StoreRef(Rc::new(UnsafeCell::new(store)))
    }
}
