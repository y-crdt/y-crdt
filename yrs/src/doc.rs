use crate::updates::encoder::DSEncoder;
use crate::*;

use crate::block_store::StateVector;
use crate::store::Store;
use crate::transaction::Transaction;
use lib0::decoding::Decoder;
use rand::Rng;
use std::cell::RefCell;

/// A Y.Doc instance.
pub struct Doc {
    pub client_id: u64,
    store: RefCell<Store>,
}

impl Doc {
    pub fn new() -> Doc {
        let client_id: u64 = rand::thread_rng().gen();
        Doc {
            client_id,
            store: RefCell::from(Store::new(client_id)),
        }
    }
    pub fn get_type(&self, tr: &Transaction, string: &str) -> types::Text {
        let ptr = types::TypePtr::Named(string.to_owned());
        types::Text::from(ptr)
    }
    /// Creates a transaction. Transaction cleanups & calling event handles
    /// happen when the transaction struct is dropped.
    ///
    /// # Example
    ///
    /// ```
    /// use std::rc::Rc;
    ///
    /// struct MyObserver {}
    ///
    /// impl yrs::Subscriber<yrs::events::UpdateEvent> for MyObserver {
    ///   fn on_change (&self, event: yrs::events::UpdateEvent) {
    ///     println!("Observer called!")
    ///   }
    /// }
    ///
    /// let doc = yrs::Doc::new();
    ///
    /// // register update observer
    /// let provider = Rc::from(MyObserver {});
    /// doc.on_update(Rc::downgrade(&provider));
    /// {
    ///   let tr = doc.transact();
    ///   doc.get_type("my type").insert(&tr, 0, 'a');
    ///   doc.get_type("my type").insert(&tr, 0, 'a');
    ///   // the block ends and `tr` is going to be dropped
    /// } // => "Observer called!"
    ///
    /// ```
    pub fn transact(&self) -> Transaction {
        Transaction::new(self.store.borrow_mut())
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
    pub fn encode_state_as_update(&self, tr: &Transaction) -> Vec<u8> {
        let mut update_encoder = updates::encoder::EncoderV1::new();
        tr.store
            .write_blocks(&mut update_encoder, &StateVector::empty());
        // @todo this is not satisfactory. We would copy the complete buffer every time this method is called.
        // Instead we should implement `write_state_as_update` and fill an existing object that implements the Write trait.
        update_encoder.to_buffer()
    }
    /// Compute a diff to sync with another client.
    ///
    /// This is the most efficient method to sync with another client by only
    /// syncing the differences.
    ///
    /// The sync protocol in Yrs/js is:
    /// * Send StateVector to the other client.
    /// * The other client comutes a minimal diff to sync by using the StateVector.
    ///
    /// ```
    /// let doc1 = yrs::Doc::new();
    /// let doc2 = yrs::Doc::new();
    ///
    /// let state_vector = doc1.get_state_vector();
    /// // encode state vector to a binary format that you can send to other peers.
    /// let state_vector_encoded: Vec<u8> = state_vector.encode();
    ///
    /// let diff = doc2.encode_diff_as_update(&yrs::StateVector::decode(&state_vector_encoded));
    ///
    /// // apply all missing changes from doc2 to doc1.
    /// doc1.apply_update(&diff);
    /// ```
    pub fn encode_diff_as_update(&self, tr: &Transaction, sv: &StateVector) -> Vec<u8> {
        let mut update_encoder = updates::encoder::EncoderV1::new();
        tr.store.write_blocks(&mut update_encoder, sv);
        update_encoder.to_buffer()
    }
    /// Apply a document update.
    pub fn apply_update(&self, tr: &mut Transaction, update: &[u8]) {
        let decoder = &mut Decoder::new(update);
        let update_decoder = &mut updates::decoder::DecoderV1::new(decoder);
        tr.store.read_blocks(update_decoder)
    }
    // Retrieve document state vector in order to encode the document diff.
    pub fn get_state_vector(&self, tr: &mut Transaction) -> StateVector {
        tr.store.blocks.get_state_vector()
    }
}

impl Default for Doc {
    fn default() -> Self {
        Doc::new()
    }
}

#[cfg(test)]
mod test {
    use crate::Doc;

    #[test]
    fn apply_update_basic() {
        /* Result of calling following code:
        ```javascript
        const doc = new Y.Doc()
        const ytext = doc.getText('type')
        doc.transact(function () {
            for (let i = 0; i < 3; i++) {
                ytext.insert(0, (i % 10).toString())
            }
        })
        const update = Y.encodeStateAsUpdate(doc)
        ```
         */
        let update = &[
            1, 3, 227, 214, 245, 198, 5, 0, 4, 1, 4, 116, 121, 112, 101, 1, 48, 68, 227, 214, 245,
            198, 5, 0, 1, 49, 68, 227, 214, 245, 198, 5, 1, 1, 50, 0,
        ];
        let doc = Doc::new();
        let mut tr = doc.transact();
        doc.apply_update(&mut tr, update);

        let actual = doc.get_type(&tr, "type").to_string(&tr);
        assert_eq!(actual, "321".to_owned());
    }
}
