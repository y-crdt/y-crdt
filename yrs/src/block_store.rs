use crate::block::{Block, BlockPtr, BlockSlice, ClientID, ID};
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::utils::client_hasher::ClientHasher;
use crate::*;
use lib0::error::Error;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::ops::Deref;
use std::vec::Vec;

/// State vector is a compact representation of all known blocks inserted and integrated into
/// a given document. This descriptor can be serialized and used to determine a difference between
/// seen and unseen inserts of two replicas of the same document, potentially existing in different
/// processes.
///
/// Another popular name for the concept represented by state vector is
/// [Version Vector](https://en.wikipedia.org/wiki/Version_vector).
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct StateVector(HashMap<ClientID, u32, BuildHasherDefault<ClientHasher>>);

impl StateVector {
    /// Checks if current state vector contains any data.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns a number of unique clients observed by a document, current state vector corresponds
    /// to.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Calculates a state vector from a document's block store.
    pub(crate) fn from(ss: &BlockStore) -> Self {
        let mut sv = StateVector::default();
        for (client_id, client_struct_list) in ss.clients.iter() {
            sv.0.insert(*client_id, client_struct_list.get_state());
        }
        sv
    }

    pub fn new(map: HashMap<ClientID, u32, BuildHasherDefault<ClientHasher>>) -> Self {
        StateVector(map)
    }

    /// Checks if current state vector includes given block identifier. Blocks, which identifiers
    /// can be found in a state vectors don't need to be encoded as part of an update, because they
    /// were already observed by their remote peer, current state vector refers to.
    pub fn contains(&self, id: &ID) -> bool {
        id.clock <= self.get(&id.client)
    }

    pub fn contains_client(&self, client_id: &ClientID) -> bool {
        self.0.contains_key(client_id)
    }

    /// Get the latest clock sequence number value for a given `client_id` as observed from
    /// the perspective of a current state vector.
    pub fn get(&self, client_id: &ClientID) -> u32 {
        match self.0.get(client_id) {
            Some(state) => *state,
            None => 0,
        }
    }

    /// Updates a state vector observed clock sequence number for a given `client` by incrementing
    /// it by a given `delta`.
    pub fn inc_by(&mut self, client: ClientID, delta: u32) {
        if delta > 0 {
            let e = self.0.entry(client).or_default();
            *e = *e + delta;
        }
    }

    /// Updates a state vector observed clock sequence number for a given `client` by setting it to
    /// a minimum value between an already present one and the provided `clock`. In case if state
    /// vector didn't contain any value for that `client`, a `clock` value will be used.
    pub fn set_min(&mut self, client: ClientID, clock: u32) {
        match self.0.entry(client) {
            Entry::Occupied(e) => {
                let value = e.into_mut();
                *value = (*value).min(clock);
            }
            Entry::Vacant(e) => {
                e.insert(clock);
            }
        }
    }

    /// Updates a state vector observed clock sequence number for a given `client` by setting it to
    /// a maximum value between an already present one and the provided `clock`. In case if state
    /// vector didn't contain any value for that `client`, a `clock` value will be used.
    pub fn set_max(&mut self, client: ClientID, clock: u32) {
        let e = self.0.entry(client).or_default();
        *e = (*e).max(clock);
    }

    /// Returns an iterator which enables to traverse over all clients and their known clock values
    /// described by a current state vector.
    pub fn iter(&self) -> std::collections::hash_map::Iter<ClientID, u32> {
        self.0.iter()
    }

    /// Merges another state vector into a current one. Since vector's clock values can only be
    /// incremented, whenever a conflict between two states happen (both state vectors have
    /// different clock values for the same client entry), a highest of these to is considered to
    /// be the most up-to-date.
    pub fn merge(&mut self, other: Self) {
        for (client, clock) in other.0 {
            let e = self.0.entry(client).or_default();
            *e = (*e).max(clock);
        }
    }
}

impl Decode for StateVector {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let len = decoder.read_var::<u32>()? as usize;
        let mut sv = HashMap::with_capacity_and_hasher(len, BuildHasherDefault::default());
        let mut i = 0;
        while i < len {
            let client = decoder.read_var()?;
            let clock = decoder.read_var()?;
            sv.insert(client, clock);
            i += 1;
        }
        Ok(StateVector(sv))
    }
}

impl Encode for StateVector {
    fn encode<E: Encoder>(&self, encoder: &mut E) -> Result<(), Error> {
        encoder.write_var(self.len());
        for (&client, &clock) in self.iter() {
            encoder.write_var(client);
            encoder.write_var(clock);
        }

        Ok(())
    }
}

/// Snapshot describes a state of a document store at a given point in (logical) time. In practice
/// it's a combination of [StateVector] (a summary of all observed insert/update operations)
/// and a [DeleteSet] (a summary of all observed deletions).
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Compressed information about all deleted blocks at current snapshot time.
    pub delete_set: DeleteSet,
    /// Logical clock describing a current snapshot time.
    pub state_map: StateVector,
}

impl Snapshot {
    pub fn new(state_map: StateVector, delete_set: DeleteSet) -> Self {
        Snapshot {
            state_map,
            delete_set,
        }
    }

    pub(crate) fn is_visible(&self, id: &ID) -> bool {
        self.state_map.get(&id.client) > id.clock && !self.delete_set.is_deleted(id)
    }
}

impl Encode for Snapshot {
    fn encode<E: Encoder>(&self, encoder: &mut E) -> Result<(), Error> {
        self.delete_set.encode(encoder)?;
        self.state_map.encode(encoder)
    }
}

impl Decode for Snapshot {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let ds = DeleteSet::decode(decoder)?;
        let sm = StateVector::decode(decoder)?;
        Ok(Snapshot::new(sm, ds))
    }
}

/// A resizable list of blocks inserted by a single client.
pub(crate) struct ClientBlockList {
    list: Vec<Box<block::Block>>,
}

impl PartialEq for ClientBlockList {
    fn eq(&self, other: &Self) -> bool {
        if self.list.len() != other.list.len() {
            false
        } else {
            let mut i = 0;
            while i < self.list.len() {
                let x = self.get(i);
                let y = other.get(i);
                if x != y {
                    return false;
                }
                i += 1;
            }
            true
        }
    }
}

impl ClientBlockList {
    fn new() -> ClientBlockList {
        ClientBlockList { list: Vec::new() }
    }

    /// Creates a new instance of aclient block list with a predefined capacity.
    pub fn with_capacity(capacity: usize) -> ClientBlockList {
        ClientBlockList {
            list: Vec::with_capacity(capacity),
        }
    }

    pub fn try_get(&self, index: usize) -> Option<BlockPtr> {
        let block_ref = self.list.get(index)?;
        Some(BlockPtr::from(block_ref))
    }

    pub fn get(&self, index: usize) -> BlockPtr {
        let block_ref = &self.list[index];
        BlockPtr::from(block_ref)
    }

    /// Gets the last clock sequence number representing the state of inserts made by client
    /// represented by this block list. This is an exclusive value meaning, that it actually
    /// describes a clock sequence number that **will be** assigned, when a new block will be
    /// appended to current list.
    pub fn get_state(&self) -> u32 {
        let item = self.get(self.list.len() - 1);
        item.id().clock + item.len()
    }

    /// Returns first block on the list - since we only initialize [ClientBlockList]
    /// when we're sure, we're about to add new elements to it, it always should
    /// stay non-empty.
    pub(crate) fn first(&self) -> BlockPtr {
        self.get(0)
    }

    /// Returns last block on the list - since we only initialize [ClientBlockList]
    /// when we're sure, we're about to add new elements to it, it always should
    /// stay non-empty.
    pub(crate) fn last(&self) -> BlockPtr {
        self.get(self.len() - 1)
    }

    /// Given a block's identifier clock value, return an offset under which this block could be
    /// found using binary search algorithm.
    pub(crate) fn find_pivot(&self, clock: u32) -> Option<usize> {
        let mut left = 0;
        let mut right = self.list.len() - 1;
        let mut block = self.get(right);
        let mut current_clock = block.id().clock;
        if current_clock == clock {
            Some(right)
        } else {
            //todo: does it even make sense to pivot the search?
            // If a good split misses, it might actually increase the time to find the correct item.
            // Currently, the only advantage is that search with pivoting might find the item on the first try.
            //let clock = clock.min(right as u32);
            let div = current_clock + block.len() - 1;
            let mut mid = ((clock / div) * right as u32) as usize;
            while left <= right {
                block = self.get(mid);
                current_clock = block.id().clock;
                if current_clock <= clock {
                    if clock < current_clock + block.len() {
                        return Some(mid);
                    }
                    left = mid + 1;
                } else {
                    right = mid - 1;
                }
                mid = (left + right) / 2;
            }

            None
        }
    }

    /// Attempts to find a Block which contains given clock sequence number within current block
    /// list. Clocks are considered to work in left-side inclusive way, meaning that block with
    /// an ID (<client-id>, 0) and length 2, with contain all elements with clock values
    /// corresponding to {0,1} but not 2.
    pub(crate) fn get_block(&self, clock: u32) -> Option<BlockPtr> {
        let idx = self.find_pivot(clock)?;
        self.try_get(idx)
    }

    /// Pushes a new block at the end of this block list.
    pub(crate) fn push(&mut self, block: Box<block::Block>) {
        self.list.push(block);
    }

    /// Inserts a new block at a given `index` position within this block list. This method may
    /// panic if `index` is greater than a length of the list.
    pub(crate) fn insert(&mut self, index: usize, block: Box<block::Block>) {
        self.list.insert(index, block);
    }

    /// Returns a number of blocks stored within this list.
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub(crate) fn iter(&self) -> ClientBlockListIter<'_> {
        ClientBlockListIter(self.list.iter())
    }

    /// Attempts to squash block at a given `index` with a corresponding block on its left side.
    /// If this succeeds, block under a given `index` will be removed, and its contents will be
    /// squashed into its left neighbor. In such case a squash result will be returned in order to
    /// later on rewire left/right neighbor changes that may have occurred as a result of squashing
    /// and block removal.
    pub(crate) fn squash_left(&mut self, index: usize) {
        let (l, r) = self.list.split_at_mut(index);
        let mut left = BlockPtr::from(&mut l[index - 1]);
        let right = BlockPtr::from(&r[0]);
        if left.is_deleted() == right.is_deleted() && left.same_type(right.deref()) {
            if left.try_squash(right) {
                let mut right = self.list.remove(index);
                let right_ptr = BlockPtr::from(&mut right);
                if let Block::Item(item) = *right {
                    if let Some(parent_sub) = item.parent_sub {
                        let mut parent = item.parent.as_branch().unwrap().clone();
                        if let Entry::Occupied(mut e) = parent.map.entry(parent_sub) {
                            let r = e.get_mut();
                            if *r == right_ptr {
                                *r = left;
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Default for ClientBlockList {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) struct ClientBlockListIter<'a>(std::slice::Iter<'a, Box<block::Block>>);

impl<'a> Iterator for ClientBlockListIter<'a> {
    type Item = &'a Block;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.0.next()?;
        Some(next.as_ref())
    }
}

/// Block store is a collection of all blocks known to a document owning instance of this type.
/// Blocks are organized per client ID and contain a resizable list of all blocks inserted by that
/// client.
#[derive(PartialEq)]
pub(crate) struct BlockStore {
    clients: HashMap<ClientID, ClientBlockList, BuildHasherDefault<ClientHasher>>,
}

pub(crate) type Iter<'a> = std::collections::hash_map::Iter<'a, ClientID, ClientBlockList>;

impl BlockStore {
    /// Creates a new empty block store instance.
    pub fn new() -> Self {
        Self {
            clients:
                HashMap::<ClientID, ClientBlockList, BuildHasherDefault<ClientHasher>>::default(),
        }
    }

    /// Checks if block store is empty. Empty block store doesn't contain any blocks, neither active
    /// nor tombstoned.
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    pub fn contains(&self, id: &ID) -> bool {
        if let Some(clients) = self.clients.get(&id.client) {
            id.clock <= clients.last().last_id().clock
        } else {
            false
        }
    }

    /// Returns an immutable reference to a block list for a particular `client`. Returns `None` if
    /// no block list existed for provided `client` in current block store.
    pub fn get(&self, client: &ClientID) -> Option<&ClientBlockList> {
        self.clients.get(client)
    }

    /// Returns a mutable reference to a block list for a particular `client`. Returns `None` if
    /// no block list existed for provided `client` in current block store.
    pub fn get_mut(&mut self, client: &ClientID) -> Option<&mut ClientBlockList> {
        self.clients.get_mut(client)
    }

    /// Returns an iterator over the client and block lists pairs known to a current block store.
    pub fn iter(&self) -> Iter<'_> {
        self.clients.iter()
    }

    pub(crate) fn blocks(&self) -> Blocks<'_> {
        Blocks::new(self)
    }

    /// Returns a state vector, which is a compact representation of the state of blocks integrated
    /// into a current block store. This state vector can later be encoded and send to a remote
    /// peers in order to calculate differences between two stored and produce a compact update,
    /// that can be applied in order to fill missing update information.
    pub fn get_state_vector(&self) -> StateVector {
        StateVector::from(self)
    }

    /// Returns immutable reference to a block, given its pointer. Returns `None` if not such
    /// block could be found.
    pub(crate) fn get_block(&self, id: &ID) -> Option<BlockPtr> {
        let clients = self.clients.get(&id.client)?;
        let pivot = clients.find_pivot(id.clock)?;
        clients.try_get(pivot)
    }

    /// Returns a block slice that represents a range of data within a particular block containing
    /// provided [ID], starting from that [ID] until the end of the block.
    ///
    /// Example: *for a block `A:1..=5` and id `A:3`, the returned slice will represent `A:3..=5`*.
    pub(crate) fn get_item_clean_start(&self, id: &ID) -> Option<BlockSlice> {
        let blocks = self.clients.get(&id.client)?;
        let index = blocks.find_pivot(id.clock)?;
        let ptr = blocks.get(index);
        let offset = id.clock - ptr.id().clock;
        Some(BlockSlice::new(ptr, offset, ptr.len() - 1))
    }

    /// Returns a block slice that represents a range of data within a particular block containing
    /// provided [ID], starting from the beginning of the block until the that [ID] (inclusive).
    ///
    /// Example: *for a block `A:1..=5` and id `A:3`, the returned slice will represent `A:1..=3`*.
    pub(crate) fn get_item_clean_end(&self, id: &ID) -> Option<BlockSlice> {
        let blocks = self.clients.get(&id.client)?;
        let index = blocks.find_pivot(id.clock)?;
        let ptr = blocks.get(index);
        let block_id = ptr.id();
        let offset = id.clock - block_id.clock;
        if offset != ptr.len() - 1 {
            Some(BlockSlice::new(ptr, 0, offset))
        } else {
            Some(BlockSlice::from(ptr))
        }
    }

    /// Returns the last observed clock sequence number for a given `client`. This is exclusive
    /// value meaning it describes a clock value of the beginning of the next block that's about
    /// to be inserted. You cannot use that clock value to find any existing block content.
    pub fn get_state(&self, client: &ClientID) -> u32 {
        if let Some(client_structs) = self.clients.get(client) {
            client_structs.get_state()
        } else {
            0
        }
    }

    /// Returns a mutable reference to block list for the given `client`. In case when no such list
    /// existed, a new one will be created and returned.
    pub(crate) fn get_client_blocks_mut(&mut self, client: ClientID) -> &mut ClientBlockList {
        self.clients
            .entry(client)
            .or_insert_with(ClientBlockList::new)
    }

    /// Returns a mutable reference to block list for the given `client`. In case when no such list
    /// existed, a new one will be created with predefined `capacity` and returned.
    pub(crate) fn get_client_blocks_with_capacity_mut(
        &mut self,
        client: ClientID,
        capacity: usize,
    ) -> &mut ClientBlockList {
        self.clients
            .entry(client)
            .or_insert_with(|| ClientBlockList::with_capacity(capacity))
    }

    /// Given block pointer, tries to split it, returning a true, if block was split in result of
    /// calling this action, and false otherwise.
    pub fn split_block(
        &mut self,
        mut block: BlockPtr,
        offset: u32,
        encoding: OffsetKind,
    ) -> Option<BlockPtr> {
        let id = block.id().clone();
        let blocks = self.clients.get_mut(&id.client)?;
        let index = blocks.find_pivot(id.clock)?;
        let mut right = block.splice(offset, encoding)?;
        let right_ptr = BlockPtr::from(&mut right);
        blocks.insert(index + 1, right);

        Some(right_ptr)
    }

    pub(crate) fn split_block_inner(&mut self, block: BlockPtr, offset: u32) -> Option<BlockPtr> {
        self.split_block(block, offset, OffsetKind::Utf16)
    }
}

impl std::fmt::Debug for ClientBlockList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for ClientBlockList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.list.iter()).finish()
    }
}

impl std::fmt::Debug for BlockStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for BlockStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("");
        for (k, v) in self.clients.iter() {
            s.field(&k.to_string(), v);
        }
        s.finish()
    }
}

pub(crate) struct Blocks<'a> {
    current_client: std::vec::IntoIter<(&'a ClientID, &'a ClientBlockList)>,
    current_block: Option<ClientBlockListIter<'a>>,
}

impl<'a> Blocks<'a> {
    fn new(update: &'a BlockStore) -> Self {
        let mut client_blocks: Vec<(&'a ClientID, &'a ClientBlockList)> =
            update.clients.iter().collect();
        // sorting to return higher client ids first
        client_blocks.sort_by(|a, b| b.0.cmp(a.0));
        let mut current_client = client_blocks.into_iter();

        let current_block = current_client.next().map(|(_, v)| v.iter());
        Blocks {
            current_client,
            current_block,
        }
    }
}

impl<'a> Iterator for Blocks<'a> {
    type Item = &'a Block;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(blocks) = self.current_block.as_mut() {
            let block = blocks.next();
            if block.is_some() {
                return block;
            }
        }

        if let Some(entry) = self.current_client.next() {
            self.current_block = Some(entry.1.iter());
            self.next()
        } else {
            None
        }
    }
}
