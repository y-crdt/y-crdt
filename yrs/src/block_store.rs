use crate::block::{
    Block, BlockPtr, Item, ItemContent, BLOCK_GC_REF_NUMBER, BLOCK_SKIP_REF_NUMBER, HAS_ORIGIN,
    HAS_RIGHT_ORIGIN, ID,
};
use crate::utils::client_hasher::ClientHasher;
use crate::*;
use lib0::decoding::Decoder;
use lib0::encoding::Encoder;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::vec::Vec;
use updates::decoder::UpdateDecoder;

#[derive(Default, Debug, Clone)]
pub struct StateVector(HashMap<u64, u32, BuildHasherDefault<ClientHasher>>);

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
        encoder.write_uvar(len);
        for (client_id, clock) in self.iter() {
            encoder.write_uvar(*client_id);
            encoder.write_uvar(*clock);
        }
        encoder.buf
    }
    pub fn decode(encoded_sv: &[u8]) -> Self {
        let mut decoder = Decoder::new(encoded_sv);
        let len: u32 = decoder.read_uvar();
        let mut sv = Self::empty();
        for _ in 0..len {
            // client, clock
            sv.0.insert(decoder.read_uvar(), decoder.read_uvar());
        }
        sv
    }
}

#[derive(Debug)]
pub struct ClientBlockList {
    pub list: Vec<block::Block>,
    pub integrated_len: usize,
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
}

#[derive(Debug)]
pub struct BlockStore {
    pub clients: HashMap<u64, ClientBlockList, BuildHasherDefault<ClientHasher>>,
}

impl BlockStore {
    pub fn new() -> Self {
        Self {
            clients: HashMap::<u64, ClientBlockList, BuildHasherDefault<ClientHasher>>::default(),
        }
    }
    pub fn from(update_decoder: &mut updates::decoder::DecoderV1) -> Self {
        let mut store = Self::new();
        let updates_count: u32 = update_decoder.rest_decoder.read_uvar();
        for _ in 0..updates_count {
            let blocks_len = update_decoder.rest_decoder.read_uvar::<u32>() as usize;
            let client = update_decoder.read_client();
            let mut clock: u32 = update_decoder.rest_decoder.read_uvar();
            let blocks = store.get_client_blocks_with_capacity_mut(client, blocks_len);
            let id = block::ID { client, clock };
            for _ in 0..blocks_len {
                let info = update_decoder.read_info();
                match info {
                    BLOCK_SKIP_REF_NUMBER => {
                        let len: u32 = update_decoder.rest_decoder.read_uvar();
                        let skip = block::Skip { id, len };
                        blocks.list.push(block::Block::Skip(skip));
                        clock += len;
                    }
                    BLOCK_GC_REF_NUMBER => {
                        let len: u32 = update_decoder.rest_decoder.read_uvar();
                        let skip = block::GC { id, len };
                        blocks.list.push(block::Block::GC(skip));
                        clock += len;
                    }
                    info => {
                        let cant_copy_parent_info = info & (HAS_ORIGIN | HAS_RIGHT_ORIGIN) == 0;
                        let origin = if info & HAS_ORIGIN != 0 {
                            Some(update_decoder.read_left_id())
                        } else {
                            None
                        };
                        let right_origin = if info & HAS_RIGHT_ORIGIN != 0 {
                            Some(update_decoder.read_right_id())
                        } else {
                            None
                        };
                        let parent = if cant_copy_parent_info {
                            types::TypePtr::Named(update_decoder.read_string().to_owned())
                        } else {
                            types::TypePtr::Id(block::BlockPtr::from(update_decoder.read_left_id()))
                        };
                        let parent_sub = if cant_copy_parent_info && (info & 0b00100000 != 0) {
                            Some(update_decoder.read_string().to_owned())
                        } else {
                            None
                        };
                        let content =
                            ItemContent::decode(update_decoder, info, BlockPtr::from(id.clone())); //TODO: What BlockPtr here is supposed to mean
                        let item: block::Item = Item {
                            id,
                            left: None,
                            right: None,
                            origin,
                            right_origin,
                            content,
                            parent,
                            parent_sub,
                            deleted: false,
                        };
                        clock += item.len();
                        blocks.list.push(block::Block::Item(item));
                    }
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
            encoder.write_uvar(*client_id);
            encoder.write_uvar(*clock);
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
        self.clients[&ptr.id.client].list[ptr.pivot as usize]
            .as_item()
            .unwrap()
    }
    pub fn get_state(&self, client: u64) -> u32 {
        if let Some(client_structs) = self.clients.get(&client) {
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

#[cfg(test)]
mod test {
    use crate::block::{Block, Item, ItemContent};
    use crate::types::TypePtr;
    use crate::updates::decoder::DecoderV1;
    use crate::{BlockStore, ID};
    use lib0::decoding::Decoder;

    #[test]
    fn block_store_from_basic() {
        /* Generated with:

           ```js
           var Y = require('yjs');

           var doc = new Y.Doc()
           var map = doc.getMap()
           map.set('keyB', 'valueB')

           // Merge changes from remote
           var update = Y.encodeStateAsUpdate(doc)
           ```
        */
        let update: &[u8] = &[
            1, 1, 176, 249, 159, 198, 7, 0, 40, 1, 0, 4, 107, 101, 121, 66, 1, 119, 6, 118, 97,
            108, 117, 101, 66, 0,
        ];
        let mut decoder = Decoder::new(update);
        let mut decoder = DecoderV1::new(&mut decoder);
        let store = BlockStore::from(&mut decoder);

        let id = ID::new(2026372272, 0);
        let block = store.find(&id);
        let expected = Some(Block::Item(Item {
            id,
            left: None,
            right: None,
            origin: None,
            right_origin: None,
            content: ItemContent::Any(vec!["valueB".into()]),
            parent: TypePtr::Named("\u{0}".to_owned()),
            parent_sub: Some("keyB".to_owned()),
            deleted: false,
        }));
        assert_eq!(block, expected.as_ref());
    }
}
