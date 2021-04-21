use crate::*;
use updates::decoder::UpdateDecoder;
use lib0::decoding::Decoder;
use lib0::encoding::Encoder;
use std::collections::HashMap;
use std::vec::Vec;
use crate::block::Block;

impl StateVector {
    pub fn empty() -> Self {
        StateVector::default()
    }
    pub fn size(&self) -> usize {
        self.0.len()
    }
    pub fn from(ss: &BlockStore) -> Self {
        let mut sv = StateVector::default();
        for (client_id, client_struct_list) in ss.clients.iter() {
            sv.0.insert(*client_id, client_struct_list.get_state());
        }
        sv
    }
    pub fn get_state(&self, client_id: u64) -> u32 {
        match self.0.get(&client_id) {
            Some(state) => *state,
            None => 0,
        }
    }
    pub fn iter(&self) -> std::collections::hash_map::Iter<u64, u32> {
        self.0.iter()
    }
    pub fn encode(&self) -> Vec<u8> {
        let len = self.size();
        // expecting to write at most two u32 values (using variable encoding
        // we will probably write less)
        let mut encoder = Encoder::with_capacity(len * 14); // Upper bound: 9 for client, 5 for clock
        encoder.write_var_uint(len);
        for (client_id, clock) in self.iter() {
            encoder.write_var_uint(*client_id);
            encoder.write_var_uint(*clock);
        }
        encoder.buf
    }
    pub fn decode(encoded_sv: &[u8]) -> Self {
        let mut decoder = Decoder::new(encoded_sv);
        let len: u32 = decoder.read_var_uint();
        let mut sv = Self::empty();
        for _ in 0..len {
            // client, clock
            sv.0.insert(decoder.read_var_uint(), decoder.read_var_uint());
        }
        sv
    }
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
    pub fn find_pivot(&self, clock: u32) -> u32 {
        panic!("implement findIndexSS");
    }
    pub fn find_item_clean_start(&mut self, tr: &mut Transaction, clock_start: u32) {

    }
    pub fn iterate(&self, tr: &Transaction, clock_start: u32, len: u32, f: fn(block::Block)) {
        if len > 0 {
            let clock_end = clock_start + len;
        }
    }
}

impl BlockStore {
    pub fn new() -> Self {
        Self {
            clients: HashMap::<u64, ClientBlockList, BuildHasherDefault<ClientHasher>>::default(),
            local_block_list: ClientBlockList::new(),
        }
    }
    pub fn from (update_decoder: &mut updates::decoder::DecoderV1) -> Self {
        let mut store = Self::new();
        let num_of_state_updates: u32 = update_decoder.rest_decoder.read_var_uint();
        for i in 0..num_of_state_updates {
            let number_of_structs = update_decoder.rest_decoder.read_var_uint::<u32>() as usize;
            let client = update_decoder.read_client();
            let clock: u32 = update_decoder.rest_decoder.read_var_uint();
            let structs = store.get_client_structs_list_with_capacity(client, number_of_structs as usize);
            let id = block::ID { client, clock };
            for j in 0..number_of_structs {
                let info = update_decoder.read_info();
                if info == 10 {
                    // is a Skip
                    let len: u32 = update_decoder.rest_decoder.read_var_uint();
                    let skip = block::Skip {
                        id,
                        len
                    };
                    structs.list.push(block::Block::Skip(skip));
                    clock += len;
                } else if info & 0b11111 != 0 {
                    // is an Item
                    let cantCopyParentInfo = info & 0b11000000 == 0;
                    let left = if info & 0b10000000 > 0 { Some(update_decoder.read_left_id()) } else { None };
                    let right = if info & 0b01000000 > 0 { Some(update_decoder.read_right_id()) } else { None };
                    let parent = if cantCopyParentInfo {
                        types::TypePtr::Named(update_decoder.read_string().to_owned())
                    } else {
                        types::TypePtr::Id(block::BlockPtr::from(update_decoder.read_left_id()))
                    };
                    let parent_sub = if cantCopyParentInfo && info & 0b00100000 > 0 {
                        Some(update_decoder.read_string())
                    } else {
                        None
                    };
                    let item: block::Item = todo!();
                    structs.list.push(block::Block::Item(item));
                    clock += 1;

                } else {
                    // is a GC
                    let len: u32 = update_decoder.rest_decoder.read_var_uint();
                    let skip = block::GC {
                        id,
                        len
                    };
                    structs.list.push(block::Block::GC(skip));
                    clock += len;
                }
            }
        }

        store
    }
    pub fn encode_state_vector(&self) -> Vec<u8> {
        let sv = self.get_state_vector();
        // expecting to write at most two u32 values (using variable encoding
        // we will probably write less)
        let mut encoder = Encoder::with_capacity(sv.size() * 8);
        for (client_id, clock) in sv.iter() {
            encoder.write_var_uint(*client_id);
            encoder.write_var_uint(*clock);
        }
        encoder.buf
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
        self.clients[&ptr.id.client].list[ptr.pivot as usize].as_item().unwrap()
    }
    pub fn get_state(&self, client: u64) -> u32 {
        if let Some(client_structs) = self.clients.get(&client) {
            client_structs.get_state()
        } else {
            0
        }
    }
    pub fn get_client_structs_list(&mut self, client_id: u64) -> &mut ClientBlockList {
        self.clients
            .entry(client_id)
            .or_insert_with(ClientBlockList::new)
    }
    pub fn get_client_structs_list_with_capacity(
        &mut self,
        client_id: u64,
        capacity: usize,
    ) -> &mut ClientBlockList {
        self.clients
            .entry(client_id)
            .or_insert_with(|| ClientBlockList::with_capacity(capacity))
    }
}
