use crate::block::{HAS_ORIGIN, HAS_RIGHT_ORIGIN};
use crate::block_store::{BlockStore, StateVector};
use crate::updates::decoder::Decoder;
use crate::updates::encoder::Encoder;
use crate::{block, types};
use std::collections::HashMap;

pub struct Store {
    client_id: u64,
    pub type_refs: HashMap<String, u32>,
    pub types: Vec<(types::Inner, String)>,
    pub blocks: BlockStore,
}

impl Store {
    pub fn new(client_id: u64) -> Self {
        Store {
            client_id,
            type_refs: Default::default(),
            types: Default::default(),
            blocks: BlockStore::new(),
        }
    }

    pub fn get_state(&self, client: &u64) -> u32 {
        self.blocks.get_state(client)
    }

    pub fn get_local_state(&self) -> u32 {
        self.blocks.get_state(&self.client_id)
    }
    pub fn get_type(&self, ptr: &types::TypePtr) -> Option<&types::Inner> {
        match ptr {
            types::TypePtr::NamedRef(name_ref) => self.types.get(*name_ref as usize).map(|t| &t.0),
            types::TypePtr::Id(id) => {
                // @todo the item might not exist
                if let block::ItemContent::Type(t) = &self.blocks.get_item(id).content {
                    Some(t)
                } else {
                    None
                }
            }
            types::TypePtr::Named(name) => self
                .get_type_ref(name)
                .map(|tref| &self.types[tref as usize].0),
        }
    }

    pub fn init_type_from_ptr(&mut self, ptr: &types::TypePtr) -> Option<&types::Inner> {
        match ptr {
            types::TypePtr::Named(name) => {
                let id = self.init_type_ref(name);
                self.types.get(id as usize).map(|t| &t.0)
            }
            _ => {
                if let Some(inner) = self.get_type(ptr) {
                    return Some(inner);
                } else {
                    None
                }
            }
        }
    }

    pub fn get_type_ref(&self, string: &str) -> Option<u32> {
        self.type_refs.get(string).map(|r| *r)
    }
    pub fn init_type_ref(&mut self, string: &str) -> u32 {
        let types = &mut self.types;
        *self.type_refs.entry(string.to_owned()).or_insert_with(|| {
            let name_ref = types.len() as u32;
            let ptr = types::TypePtr::NamedRef(name_ref);
            let inner = types::Inner::new(ptr, None, types::TYPE_REFS_ARRAY);
            types.push((inner, string.to_owned()));
            name_ref
        })
    }
    pub fn create_item(&mut self, pos: &block::ItemPosition, content: block::ItemContent) {
        let parent = self.get_type(&pos.parent).unwrap();
        let left = pos.after;
        let right = match pos.after.as_ref() {
            Some(left_id) => self.blocks.get_item(left_id).right,
            None => parent.start.get(),
        };
        let id = block::ID {
            client: self.client_id,
            clock: self.get_local_state(),
        };
        let pivot = self
            .blocks
            .get_client_blocks_mut(self.client_id)
            .integrated_len as u32;
        let item = block::Item {
            id,
            content,
            left,
            right,
            origin: pos.after.as_ref().map(|l| l.id),
            right_origin: right.map(|r| r.id),
            parent: pos.parent.clone(),
            deleted: false,
            parent_sub: None,
        };
        item.integrate(self, pivot as u32);
        let local_block_list = self.blocks.get_client_blocks_mut(self.client_id);
        local_block_list.list.push(block::Block::Item(item));
        local_block_list.integrated_len += 1;
    }

    pub fn read_blocks<D: Decoder>(&mut self, decoder: &mut D) {
        let number_of_clients: u32 = decoder.read_uvar();
        for _ in 0..number_of_clients {
            let number_of_structs: u32 = decoder.read_uvar();
            let client = decoder.read_client();
            let mut clock = decoder.read_uvar();
            for _ in 0..number_of_structs {
                let info = decoder.read_info();
                // we will get parent from either left, right. Otherwise, we
                // read it from update_decoder.
                let mut parent: Option<types::TypePtr> = None;
                let (origin, left) = if info & HAS_ORIGIN == HAS_ORIGIN {
                    let id = decoder.read_left_id();
                    let ptr = self.blocks.find_item_ptr(&id);
                    parent = Some(self.blocks.get_item(&ptr).parent.clone());
                    (Some(id), Some(ptr))
                } else {
                    (None, None)
                };
                let (right_origin, right) = if info & HAS_RIGHT_ORIGIN == HAS_RIGHT_ORIGIN {
                    let id = decoder.read_right_id();
                    let ptr = self.blocks.find_item_ptr(&id);
                    if info & HAS_ORIGIN != HAS_ORIGIN {
                        // only set parent if not already done so above
                        parent = Some(self.blocks.get_item(&ptr).parent.clone());
                    }
                    (Some(id), Some(ptr))
                } else {
                    (None, None)
                };
                if info & (HAS_RIGHT_ORIGIN | HAS_ORIGIN) == 0 {
                    // neither origin nor right_origin is defined
                    let parent_info = decoder.read_parent_info();
                    if parent_info {
                        let type_name = decoder.read_string();
                        let type_name_ref = self.init_type_ref(type_name);
                        parent = Some(types::TypePtr::NamedRef(type_name_ref as u32));
                    } else {
                        let id = decoder.read_left_id();
                        parent = Some(types::TypePtr::Id(block::BlockPtr::from(id)));
                    }
                };
                let string_content = decoder.read_string();
                let content = block::ItemContent::String(string_content.to_owned());
                let item = block::Item {
                    id: block::ID { client, clock },
                    left,
                    right,
                    origin,
                    right_origin,
                    content,
                    parent: parent.unwrap(),
                    deleted: false,
                    parent_sub: None,
                };
                item.integrate(self, clock); // todo compute pivot beforehand
                                             // add item to struct list
                                             // @todo try borow of index and generalize in ss
                let client_struct_list = self
                    .blocks
                    .get_client_blocks_with_capacity_mut(client, number_of_structs as usize);
                client_struct_list.list.push(block::Block::Item(item));
                client_struct_list.integrated_len += 1;

                // struct integration done. Now increase clock
                clock += 1;
            }
        }
    }

    pub fn write_blocks<E: Encoder>(&self, encoder: &mut E, sv: &StateVector) {
        // turns this into a vector because at some point we want to sort this
        // @todo Sort for better perf!
        let blocks: Vec<_> = self.blocks.iter_over(sv).collect();
        encoder.write_uvar(blocks.len());

        for (client_id, client_blocks) in blocks.iter() {
            let start_clock = sv.get(*client_id);
            let start_pivot = client_blocks.find_pivot(start_clock).unwrap() as u32;
            encoder.write_client(**client_id);
            encoder.write_uvar(client_blocks.integrated_len as u32 - start_pivot);
            encoder.write_uvar(start_clock); // initial clock
            for i in (start_pivot as usize)..(client_blocks.integrated_len) {
                client_blocks.list[i].encode(self, encoder);
            }
        }
    }

    pub fn get_type_name(&self, type_name_ref: u32) -> &str {
        &self.types[type_name_ref as usize].1
    }
}
