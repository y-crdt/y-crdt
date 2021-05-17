use crate::block_store::StateVector;
use crate::store::Store;
use crate::transaction::Transaction;
use crate::updates::decoder::DecoderV1;
use crate::*;
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

    /// Apply a document update.
    pub fn apply_update(&self, tr: &mut Transaction, update: &[u8]) {
        let mut decoder = DecoderV1::from(update);
        tr.store.integrate(&mut decoder)
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
        assert_eq!(actual, "210".to_owned());
    }
}
