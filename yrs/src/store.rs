use crate::block_store::{BlockStore, CompactionResult, StateVector};
use crate::event::{EventHandler, UpdateEvent};
use crate::id_set::DeleteSet;
use crate::types::{Inner, TypeRefs};
use crate::update::PendingUpdate;
use crate::updates::encoder::{Encode, Encoder};
use crate::{block, types};
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::rc::Rc;

pub struct Store {
    pub client_id: u64,
    pub types: HashMap<Rc<String>, Rc<RefCell<types::Inner>>>,
    pub blocks: BlockStore,
    pub pending: Option<PendingUpdate>,
    pub pending_ds: Option<DeleteSet>,
    pub(crate) update_events: EventHandler<UpdateEvent>,
}

impl Store {
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

    pub fn get_local_state(&self) -> u32 {
        self.blocks.get_state(&self.client_id)
    }

    pub fn get_type(&self, ptr: &types::TypePtr) -> Option<&Rc<RefCell<types::Inner>>> {
        match ptr {
            types::TypePtr::Id(id) => {
                // @todo the item might not exist
                let item = self.blocks.get_item(id)?;
                if let block::ItemContent::Type(c) = &item.content {
                    Some(c)
                } else {
                    None
                }
            }
            types::TypePtr::Named(name) => self.types.get(name),
        }
    }

    pub fn init_type_from_ptr(
        &mut self,
        ptr: &types::TypePtr,
        type_refs: TypeRefs,
    ) -> Option<Rc<RefCell<types::Inner>>> {
        match ptr {
            types::TypePtr::Named(name) => {
                let inner = self.init_type_ref(name.clone(), type_refs);
                Some(inner)
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
    pub fn create_type(&mut self, name: &str, type_ref: TypeRefs) -> Rc<RefCell<Inner>> {
        let rc = Rc::new(name.to_owned());
        self.init_type_ref(rc.clone(), type_ref)
    }

    pub(crate) fn init_type_ref(
        &mut self,
        string: Rc<String>,
        type_ref: TypeRefs,
    ) -> Rc<RefCell<Inner>> {
        let e = self.types.entry(string.clone());
        let value = e.or_insert_with(|| {
            let type_ptr = types::TypePtr::Named(string.clone());
            let inner = types::Inner::new(type_ptr, None, type_ref);
            Rc::new(RefCell::new(inner))
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

    pub(crate) fn gc_cleanup(&mut self, compaction: CompactionResult) {
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
