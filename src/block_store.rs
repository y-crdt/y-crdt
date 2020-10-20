
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::vec::Vec;
use crate::*;

impl StateVector {
    pub fn empty () -> Self {
        StateVector::default()
    }
    pub fn len (&self) -> usize {
        self.0.len()
    }
    pub fn from (ss: &StructStore) -> Self {
        let mut sv = StateVector::default();
        sv.0.insert(ss.client_id, ss.local_struct_list.get_state());
        for (client_id, client_struct_list) in ss.structs.iter() {
            sv.0.insert(*client_id, client_struct_list.get_state());
        }
        sv
    }
    pub fn get_state (&self, client_id: u32) -> u32 {
        match self.0.get(&client_id) {
            Some(state) => *state,
            None => 0
        }
    }
    pub fn iter (&self) -> std::collections::hash_map::Iter<u32, u32> {
        self.0.iter()
    }
}

pub struct UserStructList {
    pub list: Vec<Item>,
    pub integrated_len: usize,
}

impl UserStructList {
    #[inline]
    fn new () -> UserStructList {
        UserStructList {
            list: Vec::new(),
            integrated_len: 0
        }
    }
    #[inline]
    pub fn with_capacity (capacity: usize) -> UserStructList {
        UserStructList {
            list: Vec::with_capacity(capacity),
            integrated_len: 0
        }
    }
    #[inline]
    pub fn get_state (&self) -> u32 {
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

pub struct StructStore {
    pub structs: HashMap::<u32, UserStructList, BuildHasherDefault<ClientHasher>>,
    pub client_id: u32,
    pub local_struct_list: UserStructList,
    // contains structs that can't be integrated because they depend on other structs
    // unintegrated: HashMap::<u32, Vec<Item>, BuildHasherDefault<ClientHasher>>,
}

impl StructStore {
    pub fn new (client_id: u32) -> StructStore {
        StructStore {
            structs: HashMap::<u32, UserStructList, BuildHasherDefault<ClientHasher>>::default(),
            client_id,
            local_struct_list: UserStructList::new(),
            // unintegrated: HashMap::<u32, Vec<Item>, BuildHasherDefault<ClientHasher>>::default()
        }
    }
    pub fn encode_state_vector (&self) -> Vec<u8> {
        let sv = self.get_state_vector();
        // expecting to write at most two u32 values (using variable encoding
        // we will probably write less)
        let mut encoder = encoding::Encoder::with_capacity(sv.len() * 8);
        for (client_id, clock) in sv.iter() {
            encoder.write_var_u32(*client_id);
            encoder.write_var_u32(*clock);
        }
        encoder.buf
    }
    pub fn get_state_vector (&self) -> StateVector {
        StateVector::from(self)
    }
    #[inline(always)]
    pub fn find_item_ptr(&self, id: &ID) -> StructPtr {
        StructPtr {
            id: *id,
            pivot: id.clock,
        }
    }
    #[inline(always)]
    pub fn get_item_mut (&mut self, ptr: &StructPtr) -> &mut Item {
        if ptr.id.client == self.client_id {
            unsafe {
                self.local_struct_list.list.get_unchecked_mut(ptr.pivot as usize)
            }
        } else {
            unsafe {
                // this is not a dangerous expectation because we really checked
                // beforehand that these items existed (once a reference ptr was created we
                // know that the item existed)
                self.structs.get_mut(&ptr.id.client).unwrap().list.get_unchecked_mut(ptr.pivot as usize)
            }
        }
    }
    #[inline(always)]
    pub fn get_item (&self, ptr: &StructPtr) -> &Item {
        if ptr.id.client == self.client_id {
            &self.local_struct_list.list[ptr.pivot as usize]
        } else {
            // this is not a dangerous expectation because we really checked
            // beforehand that these items existed (once a reference was created we
            // know that the item existed)
            &self.structs[&ptr.id.client].list[ptr.pivot as usize]
        }
    }
    #[inline(always)]
    pub fn get_state (&self, client: u32) -> u32 {
        if client == self.client_id {
            self.local_struct_list.get_state()
        } else if let Some(client_structs) = self.structs.get(&client) {
                client_structs.get_state()
        } else {
            0
        }
    }
    pub fn get_local_state (&self) -> u32 {
        self.local_struct_list.get_state()
    }
    /*
    fn get_client_structs_list (&mut self, client_id: u32) -> &mut UserStructList {
        if client_id == self.client_id {
            &mut self.local_struct_list
        } else {
            self.structs.entry(client_id).or_insert_with(UserStructList::new)
        }
    }
    */
}
