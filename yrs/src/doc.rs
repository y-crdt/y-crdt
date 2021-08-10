use crate::block_store::StateVector;
use crate::event::{Subscription, UpdateEvent};
use crate::id_set::DeleteSet;
use crate::store::Store;
use crate::transaction::Transaction;
use crate::update::Update;
use crate::updates::decoder::{Decode, DecoderV1};
use crate::updates::encoder::{Encode, Encoder, EncoderV1};
use crate::*;
use rand::Rng;
use std::cell::RefCell;

/// A Y.Doc instance.
pub struct Doc {
    pub client_id: u64,
    store: RefCell<Store>,
}

impl Doc {
    pub fn new() -> Self {
        let client_id: u64 = rand::thread_rng().gen();
        Self::with_client_id(client_id)
    }

    pub fn with_client_id(client_id: u64) -> Self {
        Doc {
            client_id,
            store: RefCell::from(Store::new(client_id)),
        }
    }

    pub fn encode_state_as_update(&self, txn: &Transaction<'_>) -> Vec<u8> {
        txn.store.encode_v1()
    }

    pub fn encode_state_vector(&self, txn: &Transaction<'_>) -> Vec<u8> {
        txn.store.blocks.get_state_vector().encode_v1()
    }

    pub fn encode_delta_as_update(
        &self,
        remote_sv: &StateVector,
        txn: &Transaction<'_>,
    ) -> Vec<u8> {
        let mut encoder = EncoderV1::new();
        txn.store.encode_diff(remote_sv, &mut encoder);
        encoder.to_vec()
    }

    pub fn get_text(&self, txn: &mut Transaction, string: &str) -> types::Text {
        txn.get_text(string)
    }
    /// Creates a transaction. Transaction cleanups & calling event handles
    /// happen when the transaction struct is dropped.
    pub fn transact(&self) -> Transaction {
        Transaction::new(self.store.borrow_mut())
    }

    /// Apply a document update.
    pub fn apply_update(&self, tr: &mut Transaction, update: &[u8]) {
        let mut decoder = DecoderV1::from(update);
        let update = Update::decode(&mut decoder);
        let ds = DeleteSet::decode(&mut decoder);
        tr.apply_update(update, ds)
    }

    /// Retrieve document state vector in order to encode the document diff.
    pub fn get_state_vector(&self, tr: &Transaction) -> StateVector {
        tr.store.blocks.get_state_vector()
    }

    /// Subscribe callback function for incoming update events.
    /// Returns a subscription, which will unsubscribe function
    /// when dropped.
    pub fn on_update<F>(&mut self, f: F) -> Subscription<UpdateEvent>
    where
        F: Fn(&UpdateEvent) -> () + 'static,
    {
        let mut store = self.store.borrow_mut();
        store.update_events.subscribe(f)
    }
}

impl Default for Doc {
    fn default() -> Self {
        Doc::new()
    }
}

#[cfg(test)]
mod test {
    use crate::update::Update;
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::{Encode, Encoder, EncoderV1};
    use crate::{Doc, StateVector};
    use std::cell::Cell;
    use std::rc::Rc;

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

        let actual = doc.get_text(&mut tr, "type").to_string(&tr);
        assert_eq!(actual, "210".to_owned());
    }

    #[test]
    fn encode_basic() {
        let doc = Doc::with_client_id(1490905955);
        let mut t = doc.transact();
        let txt = t.get_text("type");
        txt.insert(&mut t, 0, "0");
        txt.insert(&mut t, 0, "1");
        txt.insert(&mut t, 0, "2");

        let encoded = doc.encode_state_as_update(&t);
        let expected = &[
            1, 3, 227, 214, 245, 198, 5, 0, 4, 1, 4, 116, 121, 112, 101, 1, 48, 68, 227, 214, 245,
            198, 5, 0, 1, 49, 68, 227, 214, 245, 198, 5, 1, 1, 50, 0,
        ];
        assert_eq!(encoded.as_slice(), expected);
    }

    #[test]
    fn integrate() {
        // create new document at A and add some initial text to it
        let d1 = Doc::new();
        let mut t1 = d1.transact();
        let txt = t1.get_text("test");
        // Question: why YText.insert uses positions of blocks instead of actual cursor positions
        // in text as seen by user?
        txt.insert(&mut t1, 0, "hello");
        txt.insert(&mut t1, 5, " ");
        txt.insert(&mut t1, 6, "world");

        assert_eq!(txt.to_string(&t1), "hello world".to_string());

        // create document at B
        let d2 = Doc::new();
        let mut t2 = d2.transact();
        let sv = d2.get_state_vector(&mut t2).encode_v1();

        // create an update A->B based on B's state vector
        let mut encoder = EncoderV1::new();
        t1.store
            .encode_diff(&StateVector::decode_v1(sv.as_slice()), &mut encoder);
        let binary = encoder.to_vec();

        // decode an update incoming from A and integrate it at B
        let update = Update::decode_v1(binary.as_slice());
        let pending = update.integrate(&mut t2);

        assert!(pending.is_none());

        // check if B sees the same thing that A does
        let txt = t2.get_text("test");
        assert_eq!(txt.to_string(&t2), "hello world".to_string());
    }

    #[test]
    fn on_update() {
        let counter = Rc::new(Cell::new(0));
        let doc = Doc::new();
        let mut doc2 = Doc::new();
        let c = counter.clone();
        let sub = doc2.on_update(move |e| {
            for block in e.update.blocks() {
                c.set(c.get() + block.len());
            }
        });
        let mut txn = doc.transact();
        let mut txn2 = doc2.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "abc");
        let u = doc.encode_delta_as_update(&doc2.get_state_vector(&txn2), &txn);
        doc2.apply_update(&mut txn2, u.as_slice());
        assert_eq!(counter.get(), 3); // update has been propagated

        drop(sub);

        txt.insert(&mut txn, 3, "de");
        let u = doc.encode_delta_as_update(&doc2.get_state_vector(&txn2), &txn);
        doc2.apply_update(&mut txn2, u.as_slice());
        assert_eq!(counter.get(), 3); // since subscription has been dropped, update was not propagated
    }
}
