use crate::block_store::StateVector;
use crate::event::{Subscription, UpdateEvent};
use crate::store::Store;
use crate::transaction::Transaction;
use crate::update::Update;
use crate::updates::decoder::{Decode, DecoderV1};
use crate::updates::encoder::{Encode, Encoder, EncoderV1};
use rand::Rng;
use std::cell::UnsafeCell;
use std::rc::Rc;

/// A Yrs document type. Documents are most important units of collaborative resources management.
/// All shared collections live within a scope of their corresponding documents. All updates are
/// generated on per document basis (rather than individual shared type). All operations on shared
/// collections happen via [Transaction], which lifetime is also bound to a document.
///
/// Document manages so called root types, which are top-level shared types definitions (as opposed
/// to recursively nested types).
///
/// A basic workflow sample:
///
/// ```
/// use yrs::Doc;
///
/// let doc = Doc::new();
/// let mut txn = doc.transact(); // all Yrs operations happen in scope of a transaction
/// let root = txn.get_text("root-type-name");
/// root.push(&mut txn, "hello world"); // append text to our collaborative document
///
/// // in order to exchange data with other documents we first need to create a state vector
/// let remote_doc = Doc::new();
/// let mut remote_txn = remote_doc.transact();
/// let state_vector = remote_doc.get_state_vector(&remote_txn);
///
/// // now compute a differential update based on remote document's state vector
/// let update = doc.encode_delta_as_update_v1(&txn, &state_vector);
///
/// // both update and state vector are serializable, we can pass the over the wire
/// // now apply update to a remote document
/// remote_doc.apply_update_v1(&mut remote_txn, update.as_slice());
/// ```
pub struct Doc {
    /// A unique client identifier, that's also a unique identifier of current document replica.
    pub client_id: u64,
    store: Rc<UnsafeCell<Store>>,
}

unsafe impl Send for Doc {}

impl Doc {
    /// Creates a new document with a randomized client identifier.
    pub fn new() -> Self {
        Self::with_options(Options::default())
    }

    /// Creates a new document with a specified `client_id`. It's up to a caller to guarantee that
    /// this identifier is unique across all communicating replicas of that document.
    pub fn with_client_id(client_id: u64) -> Self {
        Self::with_options(Options::with_client_id(client_id))
    }

    pub fn with_options(options: Options) -> Self {
        Doc {
            client_id: options.client_id,
            store: Rc::new(UnsafeCell::new(Store::new(options))),
        }
    }

    /// Encode entire state of a current block store using ver. 1 encoding.
    /// This state can be persisted so that later the entire document will be recovered.
    /// To apply state update use [Self::apply_update] method.
    pub fn encode_state_as_update_v1(&self, txn: &Transaction) -> Vec<u8> {
        txn.encode_update_v1()
    }

    /// Encode state vector of a current block store using ver. 1 encoding.
    pub fn encode_state_vector_v1(&self, txn: &Transaction) -> Vec<u8> {
        txn.store().blocks.get_state_vector().encode_v1()
    }

    /// Encodes a difference between current block store and a remote one based on its state vector.
    /// Such update contains only blocks not observed by a remote peer together with a delete set.
    pub fn encode_delta_as_update_v1(&self, txn: &Transaction, remote_sv: &StateVector) -> Vec<u8> {
        let mut encoder = EncoderV1::new();
        txn.encode_diff(remote_sv, &mut encoder);
        encoder.to_vec()
    }

    /// Creates a transaction used for all kind of block store operations.
    /// Transaction cleanups & calling event handles happen when the transaction struct is dropped.
    pub fn transact(&self) -> Transaction {
        Transaction::new(self.store.clone())
    }

    /// Apply a document update assuming it's encoded using lib0 ver.1 data format.
    pub fn apply_update_v1(&self, tr: &mut Transaction, update: &[u8]) {
        let mut decoder = DecoderV1::from(update);
        let update = Update::decode(&mut decoder);
        tr.apply_update(update)
    }

    /// Retrieve document state vector in order to encode the document diff. This state vector
    /// contains compressed information about all inserted blocks observed by the current block
    /// store.
    pub fn get_state_vector(&self, tr: &Transaction) -> StateVector {
        tr.store().blocks.get_state_vector()
    }

    /// Subscribe callback function for incoming update events. Returns a subscription, which will
    /// unsubscribe function when dropped.
    pub fn on_update<F>(&mut self, f: F) -> Subscription<UpdateEvent>
    where
        F: Fn(&Transaction, &UpdateEvent) -> () + 'static,
    {
        let store = unsafe { &mut *self.store.get() };
        store.update_events.subscribe(f)
    }
}

impl Default for Doc {
    fn default() -> Self {
        Doc::new()
    }
}

/// Configuration options of [Doc] instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Globally unique 53-bit long client identifier.
    pub client_id: u64,
    /// How to we count offsets and lengths used in text operations.
    pub offset_kind: OffsetKind,
    /// Determines if transactions commits should try to perform GC-ing of deleted items.
    pub skip_gc: bool,
}

impl Options {
    pub fn with_client_id(client_id: u64) -> Self {
        Options {
            client_id,
            offset_kind: OffsetKind::Bytes,
            skip_gc: false,
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        let client_id: u64 = rand::thread_rng().gen();
        Self::with_client_id(client_id & 0x3fffffffffffff)
    }
}

/// Determines how string length and offsets of [Text]/[XmlText] are being determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetKind {
    /// Compute editable strings length and offset using UTF-8 byte count.
    Bytes,
    /// Compute editable strings length and offset using UTF-16 chars count.
    Utf16,
    /// Compute editable strings length and offset using Unicode code points number.
    Utf32,
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
        doc.apply_update_v1(&mut tr, update);

        let actual = tr.get_text("type").to_string();
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

        let encoded = doc.encode_state_as_update_v1(&t);
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

        assert_eq!(txt.to_string(), "hello world".to_string());

        // create document at B
        let d2 = Doc::new();
        let mut t2 = d2.transact();
        let sv = d2.get_state_vector(&mut t2).encode_v1();

        // create an update A->B based on B's state vector
        let mut encoder = EncoderV1::new();
        t1.encode_diff(&StateVector::decode_v1(sv.as_slice()), &mut encoder);
        let binary = encoder.to_vec();

        // decode an update incoming from A and integrate it at B
        let update = Update::decode_v1(binary.as_slice());
        let pending = update.integrate(&mut t2);

        assert!(pending.0.is_none());
        assert!(pending.1.is_none());

        // check if B sees the same thing that A does
        let txt = t2.get_text("test");
        assert_eq!(txt.to_string(), "hello world".to_string());
    }

    #[test]
    fn on_update() {
        let counter = Rc::new(Cell::new(0));
        let doc = Doc::new();
        let mut doc2 = Doc::new();
        let c = counter.clone();
        let sub = doc2.on_update(move |_txn, e| {
            for block in e.update.blocks.blocks() {
                c.set(c.get() + block.len());
            }
        });
        let mut txn = doc.transact();
        let mut txn2 = doc2.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "abc");
        let u = doc.encode_delta_as_update_v1(&txn, &doc2.get_state_vector(&txn2));
        doc2.apply_update_v1(&mut txn2, u.as_slice());
        assert_eq!(counter.get(), 3); // update has been propagated

        drop(sub);

        txt.insert(&mut txn, 3, "de");
        let u = doc.encode_delta_as_update_v1(&txn, &doc2.get_state_vector(&txn2));
        doc2.apply_update_v1(&mut txn2, u.as_slice());
        assert_eq!(counter.get(), 3); // since subscription has been dropped, update was not propagated
    }
}
