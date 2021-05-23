use crate::block_store::StateVector;
use crate::id_set::DeleteSet;
use crate::store::Store;
use crate::transaction::Transaction;
use crate::update::Update;
use crate::updates::decoder::{Decode, DecoderV1};
use crate::updates::encoder::Encode;
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

    fn with_client_id(client_id: u64) -> Self {
        Doc {
            client_id,
            store: RefCell::from(Store::new(client_id)),
        }
    }

    pub fn encode_state_as_update(&self, txn: &mut Transaction) -> Vec<u8> {
        txn.store.encode_v1()
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

    /// Apply a document update.
    pub fn apply_update(&self, tr: &mut Transaction, update: &[u8]) {
        let mut decoder = DecoderV1::from(update);
        let update = Update::decode(&mut decoder);
        let ds = DeleteSet::decode(&mut decoder);
        tr.apply_update(update, ds)
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
    use crate::update::Update;
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::{Encode, Encoder, EncoderV1};
    use crate::{Doc, StateVector};

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
        assert_eq!(actual, "210".to_owned());
    }

    #[test]
    fn encode_basic() {
        let doc = Doc::with_client_id(1490905955);
        let mut t = doc.transact();
        let mut txt = t.get_text("type");
        txt.insert(&mut t, 0, "0");
        txt.insert(&mut t, 0, "1");
        txt.insert(&mut t, 0, "2");

        let encoded = doc.encode_state_as_update(&mut t);
        let expected = &[
            1, 3, 227, 214, 245, 198, 5, 0, 4, 1, 4, 116, 121, 112, 101, 1, 48, 68, 227, 214, 245,
            198, 5, 0, 1, 49, 68, 227, 214, 245, 198, 5, 1, 1, 50, 0,
        ];
        assert_eq!(encoded.as_slice(), expected);
    }

    #[test]
    fn integrate() {
        // create new document at A and add some initial text to it
        let mut d1 = Doc::new();
        let mut t1 = d1.transact();
        let txt = t1.get_text("test");
        // Question: why YText.insert uses positions of blocks instead of actual cursor positions
        // in text as seen by user?
        txt.insert(&mut t1, 0, "hello");
        txt.insert(&mut t1, 1, " ");
        txt.insert(&mut t1, 2, "world");

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
}
