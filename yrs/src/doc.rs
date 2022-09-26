use crate::block::ClientID;
use std::cell::{BorrowError, BorrowMutError};

use crate::event::{AfterTransactionEvent, EventHandler, Subscription, UpdateEvent};
use crate::store::{Store, StoreRef};
use crate::transaction::{Transaction, TransactionMut};
use crate::types::{
    TYPE_REFS_ARRAY, TYPE_REFS_MAP, TYPE_REFS_TEXT, TYPE_REFS_XML_ELEMENT, TYPE_REFS_XML_TEXT,
};
use crate::updates::encoder::{Encode, Encoder, EncoderV1, EncoderV2};
use crate::{Array, DeleteSet, Map, StateVector, SubscriptionId, Text, XmlElement, XmlText};
use rand::Rng;

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
/// use yrs::{Doc, ReadTxn, StateVector, Update};
/// use yrs::updates::decoder::Decode;
/// use yrs::updates::encoder::Encode;
///
/// let doc = Doc::new();
/// let root = doc.get_text("root-type-name");
/// let mut txn = doc.transact_mut(); // all Yrs operations happen in scope of a transaction
/// root.push(&mut txn, "hello world"); // append text to our collaborative document
///
/// // in order to exchange data with other documents we first need to create a state vector
/// let remote_doc = Doc::new();
/// let mut remote_txn = remote_doc.transact_mut();
/// let state_vector = remote_txn.state_vector().encode_v1();
///
/// // now compute a differential update based on remote document's state vector
/// let update = txn.encode_diff_v1(&StateVector::decode_v1(&state_vector).unwrap());
///
/// // both update and state vector are serializable, we can pass the over the wire
/// // now apply update to a remote document
/// remote_txn.apply_update(Update::decode_v1(update.as_slice()).unwrap());
/// ```
pub struct Doc {
    /// A unique client identifier, that's also a unique identifier of current document replica.
    pub client_id: ClientID,
    store: StoreRef,
}

unsafe impl Send for Doc {}

impl Doc {
    /// Creates a new document with a randomized client identifier.
    pub fn new() -> Self {
        Self::with_options(Options::default())
    }

    /// Creates a new document with a specified `client_id`. It's up to a caller to guarantee that
    /// this identifier is unique across all communicating replicas of that document.
    pub fn with_client_id(client_id: ClientID) -> Self {
        Self::with_options(Options::with_client_id(client_id))
    }

    pub fn with_options(options: Options) -> Self {
        Doc {
            client_id: options.client_id,
            store: Store::new(options).into(),
        }
    }

    /// Returns a [Text] data structure stored under a given `name`. Text structures are used for
    /// collaborative text editing: they expose operations to append and remove chunks of text,
    /// which are free to execute concurrently by multiple peers over remote boundaries.
    ///
    /// If not structure under defined `name` existed before, it will be created and returned
    /// instead.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as a text (in such case a sequence component of complex data type will be
    /// interpreted as a list of text chunks).
    pub fn get_text(&self, name: &str) -> Text {
        let mut r = self.store.try_borrow_mut().expect(
            "tried to get a root level type while another transaction on the document is open",
        );
        let mut c = r.get_or_create_type(name, None, TYPE_REFS_TEXT);
        c.store = Some(self.store.clone());
        Text::from(c)
    }

    /// Returns a [Map] data structure stored under a given `name`. Maps are used to store key-value
    /// pairs associated together. These values can be primitive data (similar but not limited to
    /// a JavaScript Object Notation) as well as other shared types (Yrs maps, arrays, text
    /// structures etc.), enabling to construct a complex recursive tree structures.
    ///
    /// If not structure under defined `name` existed before, it will be created and returned
    /// instead.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as a map (in such case a map component of complex data type will be
    /// interpreted as native map).
    pub fn get_map(&self, name: &str) -> Map {
        let mut r = self.store.try_borrow_mut().expect(
            "tried to get a root level type while another transaction on the document is open",
        );
        let mut c = r.get_or_create_type(name, None, TYPE_REFS_MAP);
        c.store = Some(self.store.clone());
        Map::from(c)
    }

    /// Returns an [Array] data structure stored under a given `name`. Array structures are used for
    /// storing a sequences of elements in ordered manner, positioning given element accordingly
    /// to its index.
    ///
    /// If not structure under defined `name` existed before, it will be created and returned
    /// instead.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as an array (in such case a sequence component of complex data type will be
    /// interpreted as a list of inserted values).
    pub fn get_array(&self, name: &str) -> Array {
        let mut r = self.store.try_borrow_mut().expect(
            "tried to get a root level type while another transaction on the document is open",
        );
        let mut c = r.get_or_create_type(name, None, TYPE_REFS_ARRAY);
        c.store = Some(self.store.clone());
        Array::from(c)
    }

    /// Returns a [XmlElement] data structure stored under a given `name`. XML elements represent
    /// nodes of XML document. They can contain attributes (key-value pairs, both of string type)
    /// as well as other nested XML elements or text values, which are stored in their insertion
    /// order.
    ///
    /// If not structure under defined `name` existed before, it will be created and returned
    /// instead.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as a XML element (in such case a map component of complex data type will be
    /// interpreted as map of its attributes, while a sequence component - as a list of its child
    /// XML nodes).
    pub fn get_xml_element(&self, name: &str) -> XmlElement {
        let mut r = self.store.try_borrow_mut().expect(
            "tried to get a root level type while another transaction on the document is open",
        );
        let mut c = r.get_or_create_type(name, Some("UNDEFINED".into()), TYPE_REFS_XML_ELEMENT);
        c.store = Some(self.store.clone());
        XmlElement::from(c)
    }

    /// Returns a [XmlText] data structure stored under a given `name`. Text structures are used for
    /// collaborative text editing: they expose operations to append and remove chunks of text,
    /// which are free to execute concurrently by multiple peers over remote boundaries.
    ///
    /// If not structure under defined `name` existed before, it will be created and returned
    /// instead.
    ///
    /// If a structure under defined `name` already existed, but its type was different it will be
    /// reinterpreted as a text (in such case a sequence component of complex data type will be
    /// interpreted as a list of text chunks).
    pub fn get_xml_text(&self, name: &str) -> XmlText {
        let mut r = self.store.try_borrow_mut().expect(
            "tried to get a root level type while another transaction on the document is open",
        );
        let mut c = r.get_or_create_type(name, None, TYPE_REFS_XML_TEXT);
        c.store = Some(self.store.clone());
        XmlText::from(c)
    }

    /// Creates a transaction used for all kind of block store operations.
    /// Transaction cleanups & calling event handles happen when the transaction struct is dropped.
    pub fn try_transact(&self) -> Result<Transaction, BorrowError> {
        Ok(Transaction::new(self.store.try_borrow()?))
    }

    /// Creates a transaction used for all kind of block store operations.
    /// Transaction cleanups & calling event handles happen when the transaction struct is dropped.
    pub fn try_transact_mut(&self) -> Result<TransactionMut, BorrowMutError> {
        Ok(TransactionMut::new(self.store.try_borrow_mut()?))
    }

    /// Creates a transaction used for all kind of block store operations.
    /// Transaction cleanups & calling event handles happen when the transaction struct is dropped.
    pub fn transact(&self) -> Transaction {
        self.try_transact().expect("cannot read document store contents because another read-write transaction is in progress")
    }

    /// Creates a transaction used for all kind of block store operations.
    /// Transaction cleanups & calling event handles happen when the transaction struct is dropped.
    pub fn transact_mut(&self) -> TransactionMut {
        self.try_transact_mut()
            .expect("only one read-write transaction can be active at the same time")
    }

    /// Subscribe callback function for any changes performed within transaction scope. These
    /// changes are encoded using lib0 v1 encoding and can be decoded using [Update::decode_v1] if
    /// necessary or passed to remote peers right away. This callback is triggered on function
    /// commit.
    ///
    /// Returns a subscription, which will unsubscribe function when dropped.
    pub fn observe_update_v1<F>(&mut self, f: F) -> Subscription<UpdateEvent>
    where
        F: Fn(&TransactionMut, &UpdateEvent) -> () + 'static,
    {
        let mut r = self.store.try_borrow_mut().unwrap();
        let eh = r.update_v1_events.get_or_insert_with(EventHandler::new);
        eh.subscribe(f)
    }

    /// Manually unsubscribes from a callback used in [Doc::observe_update_v1] method.
    pub fn unobserve_update_v1(&mut self, subscription_id: SubscriptionId) {
        let mut r = self.store.try_borrow_mut().unwrap();
        r.update_v1_events
            .as_mut()
            .unwrap()
            .unsubscribe(subscription_id);
    }

    /// Subscribe callback function for any changes performed within transaction scope. These
    /// changes are encoded using lib0 v1 encoding and can be decoded using [Update::decode_v2] if
    /// necessary or passed to remote peers right away. This callback is triggered on function
    /// commit.
    ///
    /// Returns a subscription, which will unsubscribe function when dropped.
    pub fn observe_update_v2<F>(&mut self, f: F) -> Subscription<UpdateEvent>
    where
        F: Fn(&TransactionMut, &UpdateEvent) -> () + 'static,
    {
        let mut r = self.store.try_borrow_mut().unwrap();
        let eh = r.update_v2_events.get_or_insert_with(EventHandler::new);
        eh.subscribe(f)
    }

    /// Manually unsubscribes from a callback used in [Doc::observe_update_v1] method.
    pub fn unobserve_update_v2(&mut self, subscription_id: SubscriptionId) {
        let mut r = self.store.try_borrow_mut().unwrap();
        r.update_v2_events
            .as_mut()
            .unwrap()
            .unsubscribe(subscription_id);
    }

    /// Subscribe callback function to updates on the `Doc`. The callback will receive state updates and
    /// deletions when a document transaction is committed.
    pub fn observe_transaction_cleanup<F>(&mut self, f: F) -> Subscription<AfterTransactionEvent>
    where
        F: Fn(&TransactionMut, &AfterTransactionEvent) -> () + 'static,
    {
        let mut r = self.store.try_borrow_mut().unwrap();
        r.after_transaction_events
            .get_or_insert_with(EventHandler::new)
            .subscribe(f)
    }
    /// Cancels the transaction cleanup callback associated with the `subscription_id`
    pub fn unobserve_transaction_cleanup(&mut self, subscription_id: SubscriptionId) {
        let mut r = self.store.try_borrow_mut().unwrap();
        if let Some(handler) = r.after_transaction_events.as_mut() {
            (*handler).unsubscribe(subscription_id);
        }
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
    pub client_id: ClientID,
    /// How to we count offsets and lengths used in text operations.
    pub offset_kind: OffsetKind,
    /// Determines if transactions commits should try to perform GC-ing of deleted items.
    pub skip_gc: bool,
}

impl Options {
    pub fn with_client_id(client_id: ClientID) -> Self {
        Options {
            client_id,
            offset_kind: OffsetKind::Bytes,
            skip_gc: false,
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        let client_id: u32 = rand::thread_rng().gen();
        Self::with_client_id(client_id as ClientID)
    }
}

/// Determines how string length and offsets of [Text]/[XmlText] are being determined.
#[repr(u8)]
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
    use crate::block::{Block, ItemContent};
    use crate::transaction::{ReadTxn, TransactionMut};
    use crate::update::Update;
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::{Encode, Encoder, EncoderV1};
    use crate::{DeleteSet, Doc, Options, StateVector, SubscriptionId};
    use lib0::any::Any;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    #[test]
    fn apply_update_basic_v1() {
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
        let txt = doc.get_text("type");
        let mut tr = doc.transact_mut();
        tr.apply_update(Update::decode_v1(update).unwrap());

        let actual = txt.to_string();
        assert_eq!(actual, "210".to_owned());
    }

    #[test]
    fn apply_update_basic_v2() {
        /* Result of calling following code:
        ```javascript
        const doc = new Y.Doc()
        const ytext = doc.getText('type')
        doc.transact(function () {
            for (let i = 0; i < 3; i++) {
                ytext.insert(0, (i % 10).toString())
            }
        })
        const update = Y.encodeStateAsUpdateV2(doc)
        ```
         */
        let update = &[
            0, 0, 6, 195, 187, 207, 162, 7, 1, 0, 2, 0, 2, 3, 4, 0, 68, 11, 7, 116, 121, 112, 101,
            48, 49, 50, 4, 65, 1, 1, 1, 0, 0, 1, 3, 0, 0,
        ];
        let doc = Doc::new();
        let txt = doc.get_text("type");
        let mut tr = doc.transact_mut();
        tr.apply_update(Update::decode_v2(update).unwrap());

        let actual = txt.to_string();
        assert_eq!(actual, "210".to_owned());
    }

    #[test]
    fn encode_basic() {
        let doc = Doc::with_client_id(1490905955);
        let txt = doc.get_text("type");
        let mut t = doc.transact_mut();
        txt.insert(&mut t, 0, "0");
        txt.insert(&mut t, 0, "1");
        txt.insert(&mut t, 0, "2");

        let encoded = t.encode_state_as_update_v1(&StateVector::default());
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
        let txt = d1.get_text("test");
        let mut t1 = d1.transact_mut();
        // Question: why YText.insert uses positions of blocks instead of actual cursor positions
        // in text as seen by user?
        txt.insert(&mut t1, 0, "hello");
        txt.insert(&mut t1, 5, " ");
        txt.insert(&mut t1, 6, "world");

        assert_eq!(txt.to_string(), "hello world".to_string());

        // create document at B
        let d2 = Doc::new();
        let txt = d2.get_text("test");
        let mut t2 = d2.transact_mut();
        let sv = t2.state_vector().encode_v1();

        // create an update A->B based on B's state vector
        let mut encoder = EncoderV1::new();
        t1.encode_diff(
            &StateVector::decode_v1(sv.as_slice()).unwrap(),
            &mut encoder,
        );
        let binary = encoder.to_vec();

        // decode an update incoming from A and integrate it at B
        let update = Update::decode_v1(binary.as_slice()).unwrap();
        let pending = update.integrate(&mut t2);

        assert!(pending.0.is_none());
        assert!(pending.1.is_none());

        // check if B sees the same thing that A does
        assert_eq!(txt.to_string(), "hello world".to_string());
    }

    #[test]
    fn on_update() {
        let counter = Rc::new(Cell::new(0));
        let doc = Doc::new();
        let mut doc2 = Doc::new();
        let c = counter.clone();
        let sub = doc2.observe_update_v1(move |_: &TransactionMut, e| {
            let u = Update::decode_v1(&e.update).unwrap();
            for block in u.blocks.blocks() {
                c.set(c.get() + block.len());
            }
        });
        let txt = doc.get_text("test");
        let mut txn = doc.transact_mut();
        {
            txt.insert(&mut txn, 0, "abc");
            let mut txn2 = doc2.transact_mut();
            let sv = txn2.state_vector().encode_v1();
            let u = txn.encode_diff_v1(&StateVector::decode_v1(sv.as_slice()).unwrap());
            txn2.apply_update(Update::decode_v1(u.as_slice()).unwrap());
        }
        assert_eq!(counter.get(), 3); // update has been propagated

        drop(sub);

        {
            txt.insert(&mut txn, 3, "de");
            let mut txn2 = doc2.transact_mut();
            let sv = txn2.state_vector().encode_v1();
            let u = txn.encode_diff_v1(&StateVector::decode_v1(sv.as_slice()).unwrap());
            txn2.apply_update(Update::decode_v1(u.as_slice()).unwrap());
        }
        assert_eq!(counter.get(), 3); // since subscription has been dropped, update was not propagated
    }

    #[test]
    fn pending_update_integration() {
        let doc = Doc::new();
        let txt = doc.get_text("source");

        let updates = [
            vec![
                1, 2, 242, 196, 218, 129, 3, 0, 40, 1, 5, 115, 116, 97, 116, 101, 5, 100, 105, 114,
                116, 121, 1, 121, 40, 1, 7, 99, 111, 110, 116, 101, 120, 116, 4, 112, 97, 116, 104,
                1, 119, 13, 117, 110, 116, 105, 116, 108, 101, 100, 52, 46, 116, 120, 116, 0,
            ],
            vec![
                1, 1, 242, 196, 218, 129, 3, 2, 40, 1, 7, 99, 111, 110, 116, 101, 120, 116, 13,
                108, 97, 115, 116, 95, 109, 111, 100, 105, 102, 105, 101, 100, 1, 119, 27, 50, 48,
                50, 50, 45, 48, 52, 45, 49, 51, 84, 49, 48, 58, 49, 48, 58, 53, 55, 46, 48, 55, 51,
                54, 50, 51, 90, 0,
            ],
            vec![
                1, 2, 242, 196, 218, 129, 3, 3, 4, 1, 6, 115, 111, 117, 114, 99, 101, 1, 97, 168,
                242, 196, 218, 129, 3, 0, 1, 120, 0,
            ],
            vec![
                1, 1, 242, 196, 218, 129, 3, 4, 168, 242, 196, 218, 129, 3, 0, 1, 120, 1, 242, 196,
                218, 129, 3, 1, 0, 1,
            ],
            vec![
                1, 1, 152, 182, 129, 244, 193, 193, 227, 4, 0, 168, 242, 196, 218, 129, 3, 4, 1,
                121, 1, 242, 196, 218, 129, 3, 2, 0, 1, 4, 1,
            ],
            vec![
                1, 2, 242, 196, 218, 129, 3, 5, 132, 242, 196, 218, 129, 3, 3, 1, 98, 168, 152,
                190, 167, 244, 1, 0, 1, 120, 0,
            ],
            vec![
                1, 1, 242, 196, 218, 129, 3, 6, 168, 152, 190, 167, 244, 1, 0, 1, 120, 1, 152, 190,
                167, 244, 1, 1, 0, 1,
            ],
            vec![
                1, 1, 242, 196, 218, 129, 3, 7, 132, 242, 196, 218, 129, 3, 5, 1, 99, 0,
            ],
            vec![
                1, 1, 242, 196, 218, 129, 3, 8, 132, 242, 196, 218, 129, 3, 7, 1, 100, 0,
            ],
        ];

        for u in updates {
            let mut txn = doc.transact_mut();
            let u = Update::decode_v1(u.as_slice()).unwrap();
            txn.apply_update(u);
        }
        assert_eq!(txt.to_string(), "abcd".to_string());
    }

    #[test]
    fn ypy_issue_32() {
        let d1 = Doc::with_client_id(1971027812);
        let source_1 = d1.get_text("source");
        source_1.push(&mut d1.transact_mut(), "a");

        let updates = [
            vec![
                1, 2, 201, 210, 153, 56, 0, 40, 1, 5, 115, 116, 97, 116, 101, 5, 100, 105, 114,
                116, 121, 1, 121, 40, 1, 7, 99, 111, 110, 116, 101, 120, 116, 4, 112, 97, 116, 104,
                1, 119, 13, 117, 110, 116, 105, 116, 108, 101, 100, 52, 46, 116, 120, 116, 0,
            ],
            vec![
                1, 1, 201, 210, 153, 56, 2, 168, 201, 210, 153, 56, 0, 1, 120, 1, 201, 210, 153,
                56, 1, 0, 1,
            ],
            vec![
                1, 1, 201, 210, 153, 56, 3, 40, 1, 7, 99, 111, 110, 116, 101, 120, 116, 13, 108,
                97, 115, 116, 95, 109, 111, 100, 105, 102, 105, 101, 100, 1, 119, 27, 50, 48, 50,
                50, 45, 48, 52, 45, 49, 54, 84, 49, 52, 58, 48, 51, 58, 53, 51, 46, 57, 51, 48, 52,
                54, 56, 90, 0,
            ],
            vec![
                1, 1, 201, 210, 153, 56, 4, 168, 201, 210, 153, 56, 2, 1, 121, 1, 201, 210, 153,
                56, 1, 2, 1,
            ],
        ];
        for u in updates {
            let u = Update::decode_v1(&u).unwrap();
            d1.transact_mut().apply_update(u);
        }

        assert_eq!("a", source_1.to_string());

        let d2 = Doc::new();
        let source_2 = d2.get_text("source");
        let state_2 = d2.transact().state_vector().encode_v1();
        let update = d1
            .transact()
            .encode_state_as_update_v1(&StateVector::decode_v1(&state_2).unwrap());
        let update = Update::decode_v1(&update).unwrap();
        d2.transact_mut().apply_update(update);

        assert_eq!("a", source_2.to_string());

        let update = Update::decode_v1(&[
            1, 2, 201, 210, 153, 56, 5, 132, 228, 254, 237, 171, 7, 0, 1, 98, 168, 201, 210, 153,
            56, 4, 1, 120, 0,
        ])
        .unwrap();
        d1.transact_mut().apply_update(update);
        assert_eq!("ab", source_1.to_string());

        let d3 = Doc::new();
        let source_3 = d3.get_text("source");
        let state_3 = d3.transact().state_vector().encode_v1();
        let state_3 = StateVector::decode_v1(&state_3).unwrap();
        let update = d1.transact().encode_state_as_update_v1(&state_3);
        let update = Update::decode_v1(&update).unwrap();
        d3.transact_mut().apply_update(update);

        assert_eq!("ab", source_3.to_string());
    }

    #[test]
    fn observe_transaction_cleanup() {
        // Setup
        let mut doc = Doc::new();
        let text = doc.get_text("test");
        let before_state = Rc::new(Cell::new(StateVector::default()));
        let after_state = Rc::new(Cell::new(StateVector::default()));
        let delete_set = Rc::new(Cell::new(DeleteSet::default()));
        // Create interior mutable references for the callback.
        let before_ref = Rc::clone(&before_state);
        let after_ref = Rc::clone(&after_state);
        let delete_ref = Rc::clone(&delete_set);
        // Subscribe callback

        let sub: SubscriptionId = doc
            .observe_transaction_cleanup(move |_: &TransactionMut, event| {
                before_ref.set(event.before_state.clone());
                after_ref.set(event.after_state.clone());
                delete_ref.set(event.delete_set.clone());
            })
            .into();

        {
            let mut txn = doc.transact_mut();

            // Update the document
            text.insert(&mut txn, 0, "abc");
            text.remove_range(&mut txn, 1, 2);
            txn.commit();

            // Compare values
            assert_eq!(before_state.take(), txn.before_state);
            assert_eq!(after_state.take(), txn.after_state);
            assert_eq!(delete_set.take(), txn.delete_set);
        }

        // Ensure that the subscription is successfully dropped.
        doc.unobserve_transaction_cleanup(sub);
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "should not update");
        assert_ne!(after_state.take(), txn.after_state);
    }

    #[test]
    fn partially_duplicated_update() {
        let d1 = Doc::with_client_id(1);
        let txt1 = d1.get_text("text");
        txt1.insert(&mut d1.transact_mut(), 0, "hello");
        let u = d1
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        let d2 = Doc::with_client_id(2);
        let txt2 = d2.get_text("text");
        d2.transact_mut()
            .apply_update(Update::decode_v1(&u).unwrap());

        txt1.insert(&mut d1.transact_mut(), 5, "world");
        let u = d1
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        d2.transact_mut()
            .apply_update(Update::decode_v1(&u).unwrap());

        assert_eq!(txt1.to_string(), txt2.to_string());
    }

    #[test]
    fn incremental_observe_update() {
        const INPUT: &'static str = "hello";

        let mut d1 = Doc::with_client_id(1);
        let txt1 = d1.get_text("text");
        let acc = Rc::new(RefCell::new(String::new()));

        let a = acc.clone();
        let _sub = d1.observe_update_v1(move |_: &TransactionMut, e| {
            let u = Update::decode_v1(&e.update).unwrap();
            for mut block in u.blocks.into_blocks() {
                match block.as_block_ptr().as_deref() {
                    Some(Block::Item(item)) => {
                        if let ItemContent::String(s) = &item.content {
                            // each character is appended in individual transaction 1-by-1,
                            // therefore each update should contain a single string with only
                            // one element
                            let mut aref = a.borrow_mut();
                            aref.push_str(s.as_str());
                        } else {
                            panic!("unexpected content type")
                        }
                    }
                    _ => {}
                }
            }
        });

        for c in INPUT.chars() {
            // append characters 1-by-1 (1 transactions per character)
            txt1.push(&mut d1.transact_mut(), &c.to_string());
        }

        assert_eq!(acc.take(), INPUT);

        // test incremental deletes
        let acc = Rc::new(RefCell::new(Vec::new()));
        let a = acc.clone();
        let _sub = d1.observe_update_v1(move |_: &TransactionMut, e| {
            let u = Update::decode_v1(&e.update).unwrap();
            for (&client_id, range) in u.delete_set.iter() {
                if client_id == 1 {
                    let mut aref = a.borrow_mut();
                    for r in range.iter() {
                        aref.push(r.clone());
                    }
                }
            }
        });

        for _ in 0..INPUT.len() as u32 {
            txt1.remove_range(&mut d1.transact_mut(), 0, 1);
        }

        let expected = vec![(0..1), (1..2), (2..3), (3..4), (4..5)];
        assert_eq!(acc.take(), expected);
    }

    #[test]
    fn ycrdt_issue_174() {
        let doc = Doc::new();
        let bin = &[
            0, 0, 11, 176, 133, 128, 149, 31, 205, 190, 199, 196, 21, 7, 3, 0, 3, 5, 0, 17, 168, 1,
            8, 0, 40, 0, 8, 0, 40, 0, 8, 0, 40, 0, 33, 1, 39, 110, 91, 49, 49, 49, 114, 111, 111,
            116, 105, 51, 50, 114, 111, 111, 116, 115, 116, 114, 105, 110, 103, 114, 111, 111, 116,
            97, 95, 108, 105, 115, 116, 114, 111, 111, 116, 97, 95, 109, 97, 112, 114, 111, 111,
            116, 105, 51, 50, 95, 108, 105, 115, 116, 114, 111, 111, 116, 105, 51, 50, 95, 109, 97,
            112, 114, 111, 111, 116, 115, 116, 114, 105, 110, 103, 95, 108, 105, 115, 116, 114,
            111, 111, 116, 115, 116, 114, 105, 110, 103, 95, 109, 97, 112, 65, 1, 4, 3, 4, 6, 4, 6,
            4, 5, 4, 8, 4, 7, 4, 11, 4, 10, 3, 0, 5, 1, 6, 0, 1, 0, 1, 0, 1, 2, 65, 8, 2, 8, 0,
            125, 2, 119, 5, 119, 111, 114, 108, 100, 118, 2, 1, 98, 119, 1, 97, 1, 97, 125, 1, 118,
            2, 1, 98, 119, 1, 98, 1, 97, 125, 2, 125, 1, 125, 2, 119, 1, 97, 119, 1, 98, 8, 0, 1,
            141, 223, 163, 226, 10, 1, 0, 1,
        ];
        let update = Update::decode_v2(bin).unwrap();
        doc.transact_mut().apply_update(update);

        let root = doc.get_map("root");
        let mut txn = doc.transact_mut();
        let actual = root.to_json();
        let expected = Any::from_json(
            r#"{
              "string": "world",
              "a_list": [{"b": "a", "a": 1}],
              "i32_map": {"1": 2},
              "a_map": {
                "1": {"a": 2, "b": "b"}
              },
              "string_list": ["a"],
              "i32": 2,
              "string_map": {"1": "b"},
              "i32_list": [1]
            }"#,
        )
        .unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn snapshots_update_generation() {
        let mut options = Options::with_client_id(1);
        options.skip_gc = true;

        let d1 = Doc::with_options(options);
        let txt1 = d1.get_text("text");
        txt1.insert(&mut d1.transact_mut(), 0, "hello");
        let snapshot = d1.transact_mut().snapshot();
        txt1.insert(&mut d1.transact_mut(), 5, " world");

        let mut encoder = EncoderV1::new();
        d1.transact_mut()
            .encode_state_from_snapshot(&snapshot, &mut encoder)
            .unwrap();
        let update = encoder.to_vec();

        let d2 = Doc::with_client_id(2);
        let txt2 = d2.get_text("text");
        d2.transact_mut()
            .apply_update(Update::decode_v1(&update).unwrap());

        assert_eq!(txt2.to_string(), "hello".to_string());
    }
}
