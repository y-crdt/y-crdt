use crate::block::{
    Block, BlockPtr, Item, ItemContent, Skip, BLOCK_GC_REF_NUMBER, BLOCK_SKIP_REF_NUMBER, GC,
    HAS_ORIGIN, HAS_RIGHT_ORIGIN,
};
use crate::id_set::IdSet;
use crate::types::TypePtr;
use crate::updates::decoder::{Decode, Decoder};
use crate::utils::client_hasher::ClientHasher;
use crate::ID;
use std::collections::{HashMap, VecDeque};
use std::hash::BuildHasherDefault;

pub struct Update {
    /// similar to `BlockStore.clients`, but optimized for picking up the updates.
    clients: HashMap<u64, VecDeque<Block>, BuildHasherDefault<ClientHasher>>,
    work_queue: VecDeque<BlockPtr>,
}

impl Update {
    fn new() -> Self {
        Update {
            clients: HashMap::default(),
            work_queue: VecDeque::new(),
        }
    }
}

impl Decode for Update {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        let mut missing = IdSet::new();
        let mut work_queue = VecDeque::new();
        let clients_len: u32 = decoder.read_uvar();
        let mut clients =
            HashMap::with_capacity_and_hasher(clients_len as usize, BuildHasherDefault::default());
        for _ in 0..clients_len {
            let blocks_len = decoder.read_uvar::<u32>() as usize;
            let client = decoder.read_client();
            let mut clock: u32 = decoder.read_uvar();
            let blocks = clients
                .entry(client)
                .or_insert_with(|| VecDeque::with_capacity(blocks_len));
            let id = ID::new(client, clock);
            for _ in 0..blocks_len {
                let info = decoder.read_info();
                match info {
                    BLOCK_SKIP_REF_NUMBER => {
                        let len: u32 = decoder.read_uvar();
                        let skip = Skip { id, len };
                        blocks.push_back(Block::Skip(skip));
                        clock += len;
                    }
                    BLOCK_GC_REF_NUMBER => {
                        let len: u32 = decoder.read_uvar();
                        let skip = GC { id, len };
                        blocks.push_back(Block::GC(skip));
                        clock += len;
                    }
                    info => {
                        let cant_copy_parent_info = info & (HAS_ORIGIN | HAS_RIGHT_ORIGIN) == 0;
                        let origin = if info & HAS_ORIGIN != 0 {
                            Some(decoder.read_left_id())
                        } else {
                            None
                        };
                        let right_origin = if info & HAS_RIGHT_ORIGIN != 0 {
                            Some(decoder.read_right_id())
                        } else {
                            None
                        };
                        let parent = if cant_copy_parent_info {
                            TypePtr::Named(decoder.read_string().to_owned())
                        } else {
                            TypePtr::Id(BlockPtr::from(decoder.read_left_id()))
                        };
                        let parent_sub = if cant_copy_parent_info && (info & 0b00100000 != 0) {
                            Some(decoder.read_string().to_owned())
                        } else {
                            None
                        };
                        let content =
                            ItemContent::decode(decoder, info, BlockPtr::from(id.clone())); //TODO: What BlockPtr here is supposed to mean
                        let item: Item = Item {
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
                        blocks.push_back(Block::Item(item));
                    }
                }
            }
        }
        Update {
            clients,
            work_queue,
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
    use std::collections::VecDeque;

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
        let mut expected = VecDeque::new();
        expected.push_back(Block::Item(Item {
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
