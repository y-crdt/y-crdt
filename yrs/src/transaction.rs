use crate::*;

use updates::encoder::*;
use std::cell::RefMut;

impl <'a> Transaction <'a> {
    pub fn new (store: RefMut<'a, Store>) -> Transaction {
        let start_state_vector = store.blocks.get_state_vector();
        Transaction {
            store,
            start_state_vector,
        }
    }
    /// Encodes the document state to a binary format.
    ///
    /// Document updates are idempotent and commutative. Caveats:
    /// * It doesn't matter in which order document updates are applied.
    /// * As long as all clients receive the same document updates, all clients
    ///   end up with the same content.
    /// * Even if an update contains known information, the unknown information
    ///   is extracted and integrated into the document structure.
    ///
    /// ```
    /// let doc1 = yrs::Doc::new();
    /// let doc2 = yrs::Doc::new();
    ///
    /// // some content
    /// doc1.get_type("my type").insert(&doc1.transact(), 0, 'a');
    ///
    /// let update = doc1.encode_state_as_update();
    ///
    /// doc2.apply_update(&update);
    ///
    /// assert_eq!(doc1.get_type("my type").to_string(), "a");
    /// ```
    ///
    pub fn encode_update (&self) -> Vec<u8> {
        let mut update_encoder = updates::encoder::EncoderV1::new();
        self.store.write_structs(&mut update_encoder, &self.start_state_vector);
        update_encoder.to_buffer()
    }
}
