use crate::block::{
    Block, BlockPtr, Item, ItemContent, Skip, BLOCK_GC_REF_NUMBER, BLOCK_SKIP_REF_NUMBER, GC,
    HAS_ORIGIN, HAS_PARENT_SUB, HAS_RIGHT_ORIGIN,
};
use crate::id_set::IdSet;
use crate::store::Store;
use crate::types::TypePtr;
use crate::updates::decoder::{Decode, Decoder};
use crate::utils::client_hasher::ClientHasher;
use crate::{StateVector, ID};
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::hash::BuildHasherDefault;

pub struct Update {
    clients: HashMap<u64, Vec<Block>, BuildHasherDefault<ClientHasher>>,
}

impl Update {
    fn new() -> Self {
        Update {
            clients: HashMap::default(),
        }
    }

    pub fn merge(&mut self, other: Self) {
        for (client, blocks) in other.clients {
            match self.clients.entry(client) {
                Entry::Occupied(e) => {
                    todo!()
                }
                Entry::Vacant(e) => {
                    e.insert(blocks);
                }
            }
        }
    }

    fn build_work_queue(&self) -> Vec<(u64, usize)> {
        let mut total_len = 0;
        let mut filter: HashMap<u64, BlockFilter, BuildHasherDefault<ClientHasher>> =
            HashMap::with_capacity_and_hasher(self.clients.len(), BuildHasherDefault::default());
        for (client, vec) in self.clients.iter() {
            let len = vec.len();
            total_len += len;
            filter.insert(*client, BlockFilter::with_capacity(len));
        }

        let mut work_q = Vec::with_capacity(total_len);

        for (client, bits) in filter.iter() {
            let blocks = self.clients.get(client).unwrap();

            // iterate over non-visited blocks
            for index in bits.unset() {
                // mark block index as visited
                if bits.set(index) {
                    let mut block = &blocks[index];
                    work_q.push((block.id().client, index));

                    while let Some(dependency) = block.dependency() {
                        let (bits, blocks) = if dependency.client == *client {
                            (bits, blocks)
                        } else {
                            if let Some(blocks) = self.clients.get(&dependency.client) {
                                let bits = filter.get(&dependency.client).unwrap();
                                (bits, blocks)
                            } else {
                                break; // the update doesn't contain client, dependency refers to
                            }
                        };

                        let dependency_index = blocks
                            .binary_search_by(|b| b.id().clock.cmp(&dependency.clock))
                            .ok();
                        if let Some(index) = dependency_index {
                            //TODO: check if dependency is in missing set
                            if bits.set(index) {
                                block = &blocks[index];
                                work_q.push((block.id().client, index));
                            } else {
                                break; // we already visited that block, it's on the work_q
                            }
                        } else {
                            break; // we haven't found the dependency block in a current update
                        }
                    }
                }
            }
        }

        work_q
    }

    pub fn integrate(mut self, store: &mut Store) -> Option<PendingUpdate> {
        //TODO: check if it's valid to insert the block into current block store
        let state_vector = store.blocks.get_state_vector();
        let mut jobs = self.build_work_queue();

        while let Some((client, index)) = jobs.pop() {
            let blocks = self.clients.get_mut(&client).unwrap();
            let len = blocks.len();
            let block = &mut blocks[index];

            block.integrate(store, index as u32);
            let blocks = store
                .blocks
                .get_client_blocks_with_capacity_mut(client, len);
            blocks.push(unsafe { std::ptr::read(block as *const Block) });
        }

        todo!()
    }

    fn decode_block<D: Decoder>(id: ID, decoder: &mut D) -> Block {
        let info = decoder.read_info();
        match info {
            BLOCK_SKIP_REF_NUMBER => {
                let len: u32 = decoder.read_uvar();
                Block::Skip(Skip { id, len })
            }
            BLOCK_GC_REF_NUMBER => {
                let len: u32 = decoder.read_uvar();
                Block::GC(GC { id, len })
            }
            info => {
                let cant_copy_parent_info = info & (HAS_ORIGIN | HAS_RIGHT_ORIGIN) == 0;
                let origin = if info & HAS_ORIGIN != 0 {
                    Some(decoder.read_left_id())
                } else {
                    None
                };
                let left = origin.as_ref().map(|id| BlockPtr::from(id.clone()));
                let right_origin = if info & HAS_RIGHT_ORIGIN != 0 {
                    Some(decoder.read_right_id())
                } else {
                    None
                };
                let right = right_origin.as_ref().map(|id| BlockPtr::from(id.clone()));
                let parent = if cant_copy_parent_info {
                    if decoder.read_parent_info() {
                        TypePtr::Named(decoder.read_string().to_owned())
                    } else {
                        TypePtr::Id(BlockPtr::from(decoder.read_left_id()))
                    }
                } else {
                    let parent = if let Some(id) = origin.as_ref() {
                        id.clone()
                    } else if let Some(id) = right_origin.as_ref() {
                        id.clone()
                    } else {
                        panic!(
                            "Couldn't decode item (id: {:?}) - no parent was provided",
                            id
                        )
                    };
                    TypePtr::Id(BlockPtr::from(parent))
                };
                let parent_sub = if cant_copy_parent_info && (info & HAS_PARENT_SUB != 0) {
                    Some(decoder.read_string().to_owned())
                } else {
                    None
                };
                let content = ItemContent::decode(decoder, info, BlockPtr::from(id.clone())); //TODO: What BlockPtr here is supposed to mean
                let item: Item = Item {
                    id,
                    left,
                    right,
                    origin,
                    right_origin,
                    content,
                    parent,
                    parent_sub,
                    deleted: false,
                };
                Block::Item(item)
            }
        }
    }
}

impl Decode for Update {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        let mut missing = IdSet::new();
        let clients_len: u32 = decoder.read_uvar();
        let mut total_len: usize = 0;
        let mut clients =
            HashMap::with_capacity_and_hasher(clients_len as usize, BuildHasherDefault::default());
        for _ in 0..clients_len {
            let blocks_len = decoder.read_uvar::<u32>() as usize;
            total_len += blocks_len;

            let client = decoder.read_client();
            let mut clock: u32 = decoder.read_uvar();
            let blocks = clients
                .entry(client)
                .or_insert_with(|| Vec::with_capacity(blocks_len));
            let id = ID::new(client, clock);

            for _ in 0..blocks_len {
                let block = Self::decode_block(id, decoder);
                clock += block.len();
                blocks.push(block);
            }
        }

        Update { clients }
    }
}

pub struct PendingUpdate {
    pub update: Update,
    pub missing: StateVector,
}

impl PendingUpdate {
    fn merge(&mut self, other: &Self) {
        todo!()
    }
}

struct BlockFilter(RefCell<Box<[u8]>>);

impl BlockFilter {
    fn with_capacity(capacity: usize) -> Self {
        let capacity = 1 + capacity / 8;
        let bits = unsafe { Box::new_zeroed_slice(capacity).assume_init() };
        BlockFilter(RefCell::new(bits))
    }

    #[inline]
    fn parse_index(index: usize) -> (usize, u8) {
        let byte_position = index / 8;
        let mask = 1u8 << (index & 0x07);
        (byte_position, mask)
    }

    fn get(&self, index: usize) -> bool {
        let (position, mask) = Self::parse_index(index);
        self.0.borrow()[position] & mask == mask
    }

    /// Marks block as visited. This is used when we construct a work queue -
    /// during that process we traverse over the chain of block dependencies
    /// and add them to the queue. [BlockFilter] is then used to detect potential
    /// duplicates.
    ///
    /// Returns true if value under index was false prior the call.
    /// Returns false, if value was already set before.
    fn set(&self, index: usize) -> bool {
        let (position, mask) = Self::parse_index(index);
        let e = &mut self.0.borrow_mut()[position];
        let result = *e & mask == 0;
        *e = (*e) | mask;
        result
    }

    /// Iterates over unset bits back to front, returning their indexes.
    fn unset(&self) -> IterUnset<'_> {
        IterUnset::new(self)
    }
}

struct IterUnset<'a> {
    bits: &'a BlockFilter,
    current: usize,
}

impl<'a> IterUnset<'a> {
    fn new(bits: &'a BlockFilter) -> Self {
        IterUnset {
            bits,
            current: bits.0.borrow().len(),
        }
    }
}

impl<'a> Iterator for IterUnset<'a> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        while self.current != 0 && self.bits.get(self.current - 1) {
            self.current -= 1;
        }
        if self.current == 0 {
            None
        } else {
            self.current -= 1;
            Some(self.current)
        }
    }
}

#[cfg(test)]
mod test {
    use crate::block::{Block, Item, ItemContent};
    use crate::types::TypePtr;
    use crate::update::Update;
    use crate::updates::decoder::{Decode, DecoderV1};
    use crate::ID;

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
        let mut decoder = DecoderV1::from(update);
        let u = Update::decode(&mut decoder);

        let id = ID::new(2026372272, 0);
        let block = u.clients.get(&id.client).unwrap();
        let mut expected = Vec::new();
        expected.push(Block::Item(Item {
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
        assert_eq!(block, &expected);
    }
}
