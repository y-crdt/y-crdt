use crate::*;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::vec::Vec;

#[wasm_bindgen]
#[derive(Default)]
pub struct StateVector(HashMap<u32, u32, BuildHasherDefault<ClientHasher>>);

impl StateVector {
    pub fn empty() -> Self {
        StateVector::default()
    }
    pub fn size(&self) -> usize {
        self.0.len()
    }
    pub fn from(ss: &BlockStore) -> Self {
        let mut sv = StateVector::default();
        sv.0.insert(ss.client_id, ss.local_block_list.get_state());
        for (client_id, client_struct_list) in ss.clients.iter() {
            sv.0.insert(*client_id, client_struct_list.get_state());
        }
        sv
    }
    pub fn get_state(&self, client_id: u32) -> u32 {
        match self.0.get(&client_id) {
            Some(state) => *state,
            None => 0,
        }
    }
    pub fn iter(&self) -> std::collections::hash_map::Iter<u32, u32> {
        self.0.iter()
    }
    pub fn encode(&self) -> Vec<u8> {
        let len = self.size();
        // expecting to write at most two u32 values (using variable encoding
        // we will probably write less)
        let mut encoder = encoding::Encoder::with_capacity(len * 8);
        encoder.write_var_u32(len as u32);
        for (client_id, clock) in self.iter() {
            encoder.write_var_u32(*client_id);
            encoder.write_var_u32(*clock);
        }
        encoder.buf
    }
    pub fn decode(encoded_sv: &[u8]) -> Self {
        let mut decoder = encoding::Decoder::new(encoded_sv);
        let len = decoder.read_var_u32();
        let mut sv = Self::empty();
        for _ in 0..len {
            // client, clock
            sv.0.insert(decoder.read_var_u32(), decoder.read_var_u32());
        }
        sv
    }
}

pub struct ClientBlockList {
    pub list: Vec<Item>,
    pub integrated_len: usize,
}

impl ClientBlockList {
    #[inline]
    fn new() -> ClientBlockList {
        ClientBlockList {
            list: Vec::new(),
            integrated_len: 0,
        }
    }
    #[inline]
    pub fn with_capacity(capacity: usize) -> ClientBlockList {
        ClientBlockList {
            list: Vec::with_capacity(capacity),
            integrated_len: 0,
        }
    }
    #[inline]
    pub fn get_state(&self) -> u32 {
        if self.integrated_len == 0 {
            0
        } else {
            let item = &self.list[self.integrated_len - 1];
            item.id.clock + 1
        }
    }
    pub fn find_pivot(&self, clock: u32) -> u32 {
        clock
    }
}

pub struct BlockStore {
    pub clients: HashMap<u32, ClientBlockList, BuildHasherDefault<ClientHasher>>,
    pub client_id: u32,
    pub local_block_list: ClientBlockList,
    // contains structs that can't be integrated because they depend on other structs
    // unintegrated: HashMap::<u32, Vec<Item>, BuildHasherDefault<ClientHasher>>,
}

impl BlockStore {
    pub fn new(client_id: u32) -> BlockStore {
        BlockStore {
            clients: HashMap::<u32, ClientBlockList, BuildHasherDefault<ClientHasher>>::default(),
            client_id,
            local_block_list: ClientBlockList::new(),
            // unintegrated: HashMap::<u32, Vec<Item>, BuildHasherDefault<ClientHasher>>::default()
        }
    }
    pub fn encode_state_vector(&self) -> Vec<u8> {
        let sv = self.get_state_vector();
        // expecting to write at most two u32 values (using variable encoding
        // we will probably write less)
        let mut encoder = encoding::Encoder::with_capacity(sv.size() * 8);
        for (client_id, clock) in sv.iter() {
            encoder.write_var_u32(*client_id);
            encoder.write_var_u32(*clock);
        }
        encoder.buf
    }
    pub fn get_state_vector(&self) -> StateVector {
        StateVector::from(self)
    }
    #[inline(always)]
    pub fn find_item_ptr(&self, id: &ID) -> BlockPtr {
        BlockPtr {
            id: *id,
            pivot: id.clock,
        }
    }
    #[inline(always)]
    pub fn get_item_mut(&mut self, ptr: &BlockPtr) -> &mut Item {
        if ptr.id.client == self.client_id {
            unsafe {
                self.local_block_list
                    .list
                    .get_unchecked_mut(ptr.pivot as usize)
            }
        } else {
            unsafe {
                // this is not a dangerous expectation because we really checked
                // beforehand that these items existed (once a reference ptr was created we
                // know that the item existed)
                self.clients
                    .get_mut(&ptr.id.client)
                    .unwrap()
                    .list
                    .get_unchecked_mut(ptr.pivot as usize)
            }
        }
    }
    #[inline(always)]
    pub fn get_item(&self, ptr: &BlockPtr) -> &Item {
        if ptr.id.client == self.client_id {
            &self.local_block_list.list[ptr.pivot as usize]
        } else {
            // this is not a dangerous expectation because we really checked
            // beforehand that these items existed (once a reference was created we
            // know that the item existed)
            &self.clients[&ptr.id.client].list[ptr.pivot as usize]
        }
    }
    #[inline(always)]
    pub fn get_state(&self, client: u32) -> u32 {
        if client == self.client_id {
            self.local_block_list.get_state()
        } else if let Some(client_structs) = self.clients.get(&client) {
            client_structs.get_state()
        } else {
            0
        }
    }
    pub fn get_local_state(&self) -> u32 {
        self.local_block_list.get_state()
    }
    #[inline(always)]
    pub fn get_client_structs_list(&mut self, client_id: u32) -> &mut ClientBlockList {
        if client_id == self.client_id {
            &mut self.local_block_list
        } else {
            self.clients
                .entry(client_id)
                .or_insert_with(ClientBlockList::new)
        }
    }
    #[inline(always)]
    pub fn get_client_structs_list_with_capacity(
        &mut self,
        client_id: u32,
        capacity: usize,
    ) -> &mut ClientBlockList {
        if client_id == self.client_id {
            &mut self.local_block_list
        } else {
            self.clients
                .entry(client_id)
                .or_insert_with(|| ClientBlockList::with_capacity(capacity))
        }
    }
}
