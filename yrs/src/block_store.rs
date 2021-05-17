use crate::block::{Block, ID};
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::utils::client_hasher::ClientHasher;
use crate::*;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::ops::{Index, IndexMut};
use std::vec::Vec;

#[derive(Default, Debug, Clone)]
pub struct StateVector(HashMap<u64, u32, BuildHasherDefault<ClientHasher>>);

impl StateVector {
    pub fn empty() -> Self {
        StateVector::default()
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

    pub fn get_state(&self, client_id: &u64) -> u32 {
        match self.0.get(client_id) {
            Some(state) => *state,
            None => 0,
        }
    }

    pub fn iter(&self) -> std::collections::hash_map::Iter<u64, u32> {
        self.0.iter()
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

#[derive(Debug)]
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

    pub fn find_pivot(&self, clock: u32) -> Option<usize> {
        let mut left = 0;
        let mut right = self.list.len() - 1;
        let mut mid = &self.list[right];
        let mut mid_clock = mid.id().clock;
        if mid_clock == clock {
            Some(right)
        } else {
            //todo: does it even make sense to pivot the search?
            // If a good split misses, it might actually increase the time to find the correct item.
            // Currently, the only advantage is that search with pivoting might find the item on the first try.
            let mut mid_idx = ((clock / (mid_clock + mid.len() - 1)) * right as u32) as usize;
            while left <= right {
                mid = &self.list[mid_idx];
                mid_clock = mid.id().clock;
                if mid_clock <= clock {
                    if clock < mid_clock + mid.len() {
                        return Some(mid_idx);
                    }
                    left = mid_idx + 1;
                } else {
                    right = mid_idx - 1;
                }
                mid_idx = (left + right) / 2;
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

    pub fn insert(&mut self, index: usize, block: block::Block) {
        self.list.insert(index, block);
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

#[derive(Debug)]
pub struct BlockStore {
    clients: HashMap<u64, ClientBlockList, BuildHasherDefault<ClientHasher>>,
}

pub type Iter<'a> = std::collections::hash_map::Iter<'a, u64, ClientBlockList>;

impl BlockStore {
    pub fn new() -> Self {
        Self {
            clients: HashMap::<u64, ClientBlockList, BuildHasherDefault<ClientHasher>>::default(),
        }
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

    pub fn get_item_mut(&mut self, ptr: &block::BlockPtr) -> &mut block::Item {
        unsafe {
            // this is not a dangerous expectation because we really checked
            // beforehand that these items existed (once a reference ptr was created we
            // know that the item existed)
            self.clients
                .get_mut(&ptr.id.client)
                .unwrap()
                .list
                .get_unchecked_mut(ptr.pivot as usize)
                .as_item_mut()
                .unwrap()
        }
    }

    pub fn get_block(&self, ptr: &block::BlockPtr) -> &block::Block {
        &self.clients[&ptr.id.client].list[ptr.pivot as usize]
    }

    pub fn get_item(&self, ptr: &block::BlockPtr) -> &block::Item {
        // this is not a dangerous expectation because we really checked
        // beforehand that these items existed (once a reference was created we
        // know that the item existed)
        self.clients[&ptr.id.client].list[ptr.pivot as usize]
            .as_item()
            .unwrap()
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
}
