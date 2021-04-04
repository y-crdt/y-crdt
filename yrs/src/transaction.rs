use std::cell::RefMut;

use crate::*;

impl <'a> Transaction <'a> {
    pub fn new (store: RefMut<'a, Store>) -> Transaction {
        let start_state_vector = store.blocks.get_state_vector();
        Transaction {
            store,
            start_state_vector,
        }
    }
}
