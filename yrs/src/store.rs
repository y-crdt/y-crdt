use crate::block::ItemContent;
use crate::block_store::{BlockStore, SquashResult, StateVector};
use crate::event::{EventHandler, UpdateEvent};
use crate::id_set::DeleteSet;
use crate::types;
use crate::types::{BranchRef, TypePtr, TypeRefs, TYPE_REFS_UNDEFINED};
use crate::update::PendingUpdate;
use crate::updates::encoder::{Encode, Encoder};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::rc::Rc;

/// Store is a core element of a document. It contains all of the information, like block store
/// map of root types, pending updates waiting to be applied once a missing update information
/// arrives and all subscribed callbacks.
pub(crate) struct Store {
    /// An unique identifier of a current document replica.
    pub client_id: u64,

    /// Root types (a.k.a. top-level types). These types are defined by users at the document level,
    /// they have their own unique names and represent core shared types that expose operations
    /// which can be called concurrently by remote peers in a conflict-free manner.
    pub types: HashMap<Rc<String>, BranchRef>,

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

    /// A subscription handler. It contains all callbacks with registered by user functions that
    /// are supposed to be called, once a new update arrives.
    pub(crate) update_events: EventHandler<UpdateEvent>,
}

impl Store {
    /// Create a new empty store in context of a given `client_id`.
    pub fn new(client_id: u64) -> Self {
        Store {
            client_id,
            types: Default::default(),
            blocks: BlockStore::new(),
            pending: None,
            pending_ds: None,
            update_events: EventHandler::new(),
        }
    }

    /// Get the latest clock sequence number observed and integrated into a current store client.
    /// This is exclusive value meaning it describes a clock value of the beginning of the next
    /// block that's about to be inserted. You cannot use that clock value to find any existing
    /// block content.
    pub fn get_local_state(&self) -> u32 {
        self.blocks.get_state(&self.client_id)
    }

    /// Returns a branch reference to a complex type identified by its pointer. Returns `None` if
    /// no such type could be found or was ever defined.
    pub fn get_type(&self, ptr: &TypePtr) -> Option<&BranchRef> {
        match ptr {
            TypePtr::Id(id) => {
                // @todo the item might not exist
                let item = self.blocks.get_item(id)?;
                if let ItemContent::Type(c) = &item.content {
                    Some(c)
                } else {
                    None
                }
            }
            TypePtr::Named(name) => self.types.get(name),
            TypePtr::Unknown => None,
        }
    }

    /// Retrieves a complex data structure reference given its type pointer. In case of root types
    /// (defined by the user at document level) of such type didn't exist before, it will be created
    /// and returned. For other (recursively nested) types, they will be returned only if they
    /// already existed. Otherwise a `None` will be returned.
    pub fn init_type_from_ptr(&mut self, ptr: &types::TypePtr) -> Option<BranchRef> {
        match ptr {
            types::TypePtr::Named(name) => {
                if let Some(inner) = self.types.get(name) {
                    Some(inner.clone())
                } else {
                    let inner = self.init_type_ref(name.clone(), None, TYPE_REFS_UNDEFINED);
                    Some(inner)
                }
            }
            _ => {
                if let Some(inner) = self.get_type(ptr) {
                    return Some(inner.clone());
                } else {
                    None
                }
            }
        }
    }

    /// Creates or returns a new empty root type (defined by user at the document level). This type
    /// has user-specified `name`, a `node_name` (which is used only in case of XML elements) and a
    /// type ref describing which shared type does this instance represent.
    pub fn create_type(
        &mut self,
        name: &str,
        node_name: Option<String>,
        type_ref: TypeRefs,
    ) -> BranchRef {
        let rc = Rc::new(name.to_owned());
        self.init_type_ref(rc.clone(), node_name, type_ref)
    }

    pub(crate) fn init_type_ref(
        &mut self,
        name: Rc<String>,
        node_name: Option<String>,
        type_ref: TypeRefs,
    ) -> BranchRef {
        let e = self.types.entry(name.clone());
        let value = e.or_insert_with(|| {
            let type_ptr = types::TypePtr::Named(name.clone());
            let inner = types::Branch::new(type_ptr, type_ref, node_name);
            BranchRef::new(inner)
        });
        value.clone()
    }

    /// Compute a diff to sync with another client.
    ///
    /// This is the most efficient method to sync with another client by only
    /// syncing the differences.
    ///
    /// The sync protocol in Yrs/js is:
    /// * Send StateVector to the other client.
    /// * The other client comutes a minimal diff to sync by using the StateVector.
    pub fn encode_diff<E: Encoder>(&self, remote_sv: &StateVector, encoder: &mut E) {
        //TODO: this could be actually 2 steps:
        // 1. create Diff of block store and remote state vector (it can have lifetime of bock store)
        // 2. make Diff implement Encode trait and encode it
        // this way we can add some extra utility method on top of Diff (like introspection) without need of decoding it.
        self.write_blocks(remote_sv, encoder);
        let delete_set = DeleteSet::from(&self.blocks);
        delete_set.encode(encoder);
    }

    fn write_blocks<E: Encoder>(&self, remote_sv: &StateVector, encoder: &mut E) {
        let local_sv = self.blocks.get_state_vector();
        let mut diff = Self::diff_state_vectors(&local_sv, remote_sv);

        // Write items with higher client ids first
        // This heavily improves the conflict algorithm.
        diff.sort_by(|a, b| b.0.cmp(&a.0));

        encoder.write_uvar(diff.len());
        for (client, clock) in diff {
            let blocks = self.blocks.get(&client).unwrap();
            let clock = clock.max(blocks.first().id().clock); // make sure the first id exists
            let start = blocks.find_pivot(clock).unwrap();
            // write # encoded structs
            encoder.write_uvar(blocks.integrated_len() - start);
            encoder.write_client(client);
            encoder.write_uvar(clock);
            let first_block = &blocks[start];
            // write first struct with an offset
            first_block.encode(encoder);
            for i in (start + 1)..blocks.integrated_len() {
                blocks[i].encode(encoder);
            }
        }
    }

    fn diff_state_vectors(local_sv: &StateVector, remote_sv: &StateVector) -> Vec<(u64, u32)> {
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

    pub(crate) fn gc_cleanup(&self, compaction: SquashResult) {
        if let Some(parent_sub) = compaction.parent_sub {
            if let Some(parent) = self.get_type(&compaction.parent) {
                let mut inner = parent.borrow_mut();
                match inner.map.entry(parent_sub) {
                    Entry::Occupied(e) => {
                        let cell = e.into_mut();
                        if cell.id == compaction.old_right {
                            *cell = compaction.replacement;
                        }
                    }
                    Entry::Vacant(e) => {
                        e.insert(compaction.replacement);
                    }
                }
            }
        }
        if let Some(right) = compaction.new_right {
            if let Some(item) = self.blocks.get_item_mut(&right) {
                item.left = Some(compaction.replacement);
            }
        }
    }

    pub(crate) fn get_root_type_key(&self, value: &BranchRef) -> Option<&Rc<String>> {
        for (k, v) in self.types.iter() {
            if v == value {
                return Some(k);
            }
        }

        None
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

impl std::fmt::Display for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Store(ID: {}) {{", self.client_id)?;
        if !self.types.is_empty() {
            writeln!(f, "\ttypes: {{")?;
            for (k, v) in self.types.iter() {
                writeln!(f, "\t\t'{}': {}", k.as_str(), *v.borrow())?;
            }

            writeln!(f, "\t}}")?;
        }
        if !self.blocks.is_empty() {
            writeln!(f, "\tblocks: {}", self.blocks)?;
        }

        writeln!(f, "}}")
    }
}
