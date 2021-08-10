use crate::block::{Block, BlockPtr, Item, ID};
use crate::types::TypePtr;
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::utils::client_hasher::ClientHasher;
use crate::*;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::ops::{Index, IndexMut};
use std::vec::Vec;

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct StateVector(HashMap<u64, u32, BuildHasherDefault<ClientHasher>>);

impl StateVector {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn from(ss: &BlockStore) -> Self {
        let mut sv = StateVector::default();
        for (client_id, client_struct_list) in ss.clients.iter() {
            sv.0.insert(*client_id, client_struct_list.get_state());
        }
        sv
    }

    pub fn contains(&self, id: &ID) -> bool {
        id.clock <= self.get(&id.client)
    }

    pub fn get(&self, client_id: &u64) -> u32 {
        match self.0.get(client_id) {
            Some(state) => *state,
            None => 0,
        }
    }

    pub fn inc_by(&mut self, client: u64, delta: u32) {
        if delta > 0 {
            let e = self.0.entry(client).or_default();
            *e = *e + delta;
        }
    }

    pub fn set_min(&mut self, client: u64, clock: u32) {
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
    pub fn set_max(&mut self, client: u64, clock: u32) {
        let e = self.0.entry(client).or_default();
        *e = (*e).max(clock);
    }

    pub fn iter(&self) -> std::collections::hash_map::Iter<u64, u32> {
        self.0.iter()
    }

    pub fn merge(&mut self, other: Self) {
        for (client, clock) in other.0 {
            let e = self.0.entry(client).or_default();
            *e = (*e).max(clock);
        }
    }
}

impl Decode for StateVector {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        let len = decoder.read_uvar::<u32>() as usize;
        let mut sv = HashMap::with_capacity_and_hasher(len, BuildHasherDefault::default());
        let mut i = 0;
        while i < len {
            let client = decoder.read_uvar();
            let clock = decoder.read_uvar();
            sv.insert(client, clock);
            i += 1;
        }
        StateVector(sv)
    }
}

impl Encode for StateVector {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_uvar(self.len());
        for (&client, &clock) in self.iter() {
            encoder.write_uvar(client);
            encoder.write_uvar(clock);
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct ClientBlockList {
    list: Vec<block::Block>,
    integrated_len: usize,
}

impl ClientBlockList {
    fn new() -> ClientBlockList {
        ClientBlockList {
            list: Vec::new(),
            integrated_len: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> ClientBlockList {
        ClientBlockList {
            list: Vec::with_capacity(capacity),
            integrated_len: 0,
        }
    }

    pub fn get_state(&self) -> u32 {
        if self.integrated_len == 0 {
            0
        } else {
            let item = &self.list[self.integrated_len - 1];
            item.id().clock + item.len()
        }
    }

    /// Returns first block on the list - since we only initialize [ClientBlockList]
    /// when we're sure, we're about to add new elements to it, it always should
    /// stay non-empty.
    pub fn first(&self) -> &Block {
        &self.list[0]
    }

    /// Returns last block on the list - since we only initialize [ClientBlockList]
    /// when we're sure, we're about to add new elements to it, it always should
    /// stay non-empty.
    pub fn last(&self) -> &Block {
        &self.list[self.integrated_len - 1]
    }

    pub fn find(&mut self, ptr: &BlockPtr) -> Option<&mut Block> {
        let pivot = match self.list.get_mut(ptr.pivot()) {
            Some(block) if *block.id() == ptr.id => Some(ptr.pivot()),
            _ => self.find_pivot(ptr.id.clock),
        };
        self.list.get_mut(pivot?)
    }

    pub fn find_pivot(&self, clock: u32) -> Option<usize> {
        let mut left = 0;
        let mut right = self.list.len() - 1;
        let mut block = &self.list[right];
        let mut current_clock = block.id().clock;
        if current_clock == clock {
            Some(right)
        } else {
            //todo: does it even make sense to pivot the search?
            // If a good split misses, it might actually increase the time to find the correct item.
            // Currently, the only advantage is that search with pivoting might find the item on the first try.
            let div = (current_clock + block.len() - 1);
            let mut mid = ((clock / div) * right as u32) as usize;
            while left <= right {
                block = &self.list[mid];
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

    pub fn find_block(&self, clock: u32) -> Option<&Block> {
        let idx = self.find_pivot(clock)?;
        Some(&self.list[idx])
    }

    pub fn push(&mut self, block: block::Block) {
        self.list.push(block);
        self.integrated_len += 1;
    }

    fn insert(&mut self, index: usize, block: block::Block) {
        self.list.insert(index, block);
        self.integrated_len += 1;
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn integrated_len(&self) -> usize {
        self.integrated_len
    }

    pub fn iter(&self) -> ClientBlockListIter<'_> {
        self.list.iter()
    }

    pub fn clear(&mut self) {
        self.integrated_len = 0;
        self.list.clear();
    }

    pub(crate) fn compact_left(&mut self, pos: usize) -> Option<CompactionResult> {
        let replacement = {
            let (l, r) = self.list.split_at_mut(pos);
            let left = &mut l[pos - 1];
            let right = &r[0];
            if left.is_deleted() == right.is_deleted() && left.same_type(right) {
                if left.try_merge(right) {
                    let new_ptr = BlockPtr::new(left.id().clone(), pos as u32 - 1);
                    Some(new_ptr)
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(replacement) = replacement {
            let block = self.list.remove(pos);
            self.integrated_len -= 1;
            if let Block::Item(item) = block {
                return Some(CompactionResult {
                    parent: item.parent,
                    parent_sub: item.parent_sub,
                    new_right: item.right,
                    old_right: item.id,
                    replacement,
                });
            }
        }

        None
    }
}

pub(crate) struct CompactionResult {
    pub parent: TypePtr,
    pub parent_sub: Option<String>,
    /// Pointer to a block that resulted from compaction of two adjacent blocks.
    pub replacement: BlockPtr,
    /// Pointer to a neighbor, that's now on the right side of the `replacement` block.
    pub new_right: Option<BlockPtr>,
    /// ID of the block that was compacted into left block. Left block ID is in `replacement`.
    pub old_right: ID,
}

impl Default for ClientBlockList {
    fn default() -> Self {
        Self::new()
    }
}

impl Index<usize> for ClientBlockList {
    type Output = block::Block;

    fn index(&self, index: usize) -> &Self::Output {
        &self.list[index]
    }
}

impl IndexMut<usize> for ClientBlockList {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.list[index]
    }
}

pub type ClientBlockListIter<'a> = std::slice::Iter<'a, block::Block>;

#[derive(Debug, PartialEq)]
pub struct BlockStore {
    clients: HashMap<u64, ClientBlockList, BuildHasherDefault<ClientHasher>>,
}

pub type Iter<'a> = std::collections::hash_map::Iter<'a, u64, ClientBlockList>;

impl BlockStore {
    pub fn from(clients: HashMap<u64, ClientBlockList, BuildHasherDefault<ClientHasher>>) -> Self {
        Self { clients }
    }

    pub fn new() -> Self {
        Self {
            clients: HashMap::<u64, ClientBlockList, BuildHasherDefault<ClientHasher>>::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    pub fn contains_client(&self, client: &u64) -> bool {
        self.clients.contains_key(client)
    }

    pub fn get(&self, client: &u64) -> Option<&ClientBlockList> {
        self.clients.get(client)
    }

    pub fn get_mut(&mut self, client: &u64) -> Option<&mut ClientBlockList> {
        self.clients.get_mut(client)
    }

    pub fn remove(&mut self, client: &u64) -> Option<ClientBlockList> {
        self.clients.remove(client)
    }

    pub fn iter(&self) -> Iter<'_> {
        self.clients.iter()
    }

    pub fn get_state_vector(&self) -> StateVector {
        StateVector::from(self)
    }

    pub fn find_item_ptr(&self, id: &block::ID) -> block::BlockPtr {
        let x = block::BlockPtr::from(*id);
        x
    }

    pub fn get_item_mut(&mut self, ptr: &block::BlockPtr) -> Option<&mut block::Item> {
        let blocks = self.clients.get_mut(&ptr.id.client)?;
        let block = blocks.list.get_mut(ptr.pivot())?;
        block.as_item_mut()
    }

    pub fn get_block(&self, ptr: &block::BlockPtr) -> Option<&block::Block> {
        let clients = self.clients.get(&ptr.id.client)?;
        match clients.list.get(ptr.pivot()) {
            Some(block) if block.id().clock == ptr.id.clock => Some(block),
            _ => {
                // ptr.pivot missed - go slow path to find it
                let pivot = clients.find_pivot(ptr.id.clock)?;
                ptr.fix_pivot(pivot as u32);
                Some(&clients.list[pivot])
            }
        }
    }

    pub fn get_item(&self, ptr: &block::BlockPtr) -> Option<&block::Item> {
        let block = self.get_block(ptr)?;
        block.as_item()
    }

    pub fn get_state(&self, client: &u64) -> u32 {
        if let Some(client_structs) = self.clients.get(client) {
            client_structs.get_state()
        } else {
            0
        }
    }

    pub fn get_client_blocks_mut(&mut self, client_id: u64) -> &mut ClientBlockList {
        self.clients
            .entry(client_id)
            .or_insert_with(ClientBlockList::new)
    }

    pub fn get_client_blocks_with_capacity_mut(
        &mut self,
        client_id: u64,
        capacity: usize,
    ) -> &mut ClientBlockList {
        self.clients
            .entry(client_id)
            .or_insert_with(|| ClientBlockList::with_capacity(capacity))
    }

    pub fn find(&self, id: &ID) -> Option<&Block> {
        let blocks = self.clients.get(&id.client)?;
        blocks.find_block(id.clock)
    }

    pub fn get_item_from_type_ptr(&self, ptr: &TypePtr) -> Option<&Item> {
        if let TypePtr::Id(ptr) = ptr {
            if let Some(Block::Item(item)) = &self.get_block(ptr) {
                return Some(item);
            }
        }

        None
    }

    pub fn insert(&mut self, client: u64, blocks: ClientBlockList) -> Option<ClientBlockList> {
        self.clients.insert(client, blocks)
    }

    /// Given block pointer, tries to split it, returning a pointers to left and right halves
    /// of a newly split block.
    ///
    /// If split was not necessary (eg. because block `ptr` was not inside of any block),
    /// the right half returned wll be None.
    ///
    /// If no block for given `ptr` was found, then both returned options will be None.
    pub fn split_block(&mut self, ptr: &BlockPtr) -> (Option<BlockPtr>, Option<BlockPtr>) {
        let mut pivot = ptr.pivot();
        if let Some(mut blocks) = self.clients.get_mut(&ptr.id.client) {
            let block: &mut Block = {
                match blocks.list.get_mut(pivot) {
                    // check if ptr clock fits into block found by pivot
                    Some(b) if ptr.id.clock >= b.id().clock && ptr.id.clock < b.clock_end() => b,
                    _ => {
                        // search by pivot missed: perform standard lookup to find correct block
                        if let Some(p) = blocks.find_pivot(ptr.id.clock) {
                            pivot = p;
                            &mut blocks.list[pivot]
                        } else {
                            return (None, None);
                        }
                    }
                }
            };

            let left_split_ptr = BlockPtr::new(block.id().clone(), pivot as u32);
            let right_split_ptr = match block {
                Block::Item(item) => {
                    let len = item.len();
                    if ptr.id.clock > item.id.clock && ptr.id.clock <= item.id.clock + len {
                        let index = pivot + 1;
                        let diff = ptr.id.clock - item.id.clock;
                        let right_split = item.split(diff);
                        let right_split_id = right_split.id.clone();
                        let right_ptr = right_split.right.clone();
                        if let Some(right_ptr) = right_ptr {
                            blocks = if right_ptr.id.client == ptr.id.client {
                                blocks
                            } else {
                                self.clients.get_mut(&right_ptr.id.client).unwrap()
                            };
                            let right = blocks.find(&right_ptr).unwrap();
                            if let Some(right_item) = right.as_item_mut() {
                                right_item.left =
                                    Some(BlockPtr::new(right_split.id.clone(), index as u32));
                            }
                        }
                        blocks.insert(index, Block::Item(right_split));
                        Some(BlockPtr::new(right_split_id, index as u32))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            (Some(left_split_ptr), right_split_ptr)
        } else {
            (None, None)
        }
    }
}

impl std::fmt::Display for ClientBlockList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[")?;
        let mut i = 0;
        writeln!(f, "")?;
        while i < self.list.len() {
            let block = &self.list[i];
            writeln!(f, "\t\t{}", block)?;
            if i == self.integrated_len {
                writeln!(f, "---")?;
            }
            i += 1;
        }
        write!(f, "\t]")
    }
}

impl std::fmt::Display for BlockStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{{")?;
        for (k, v) in self.iter() {
            writeln!(f, "\t{} ->{}", k, v)?;
        }
        writeln!(f, "}}")
    }
}
