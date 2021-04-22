#![feature(shrink_to)]
//! Yrs "wires" is a high performance CRDT implementation based on the idea of **Shared
//! Types**. It is a compatible port of the [Yjs](https://github.com/yjs/yjs) CRDT.
//!
//! **Shared Types** work just like normal data types, but they automatically
//! sync with other peers. In Rust, they can automatically sync with their
//! counterparts in other threads.
//!
//! A **Shared Document** is the access point to create shared types,
//! and to listen to update events.
//!
//! # Quick Start
//!
//! ```
//! // create a shared document
//! let doc = yrs::Doc::new();
//!
//! // Retrieve a shared type named "my text type".
//! let ytype = doc.get_type("my text type");
//!
//! // Perform changes..
//! // All modifications must be associated to a Transaction.
//! let tr = doc.transact();
//! ytype.insert(&tr, 0, 'x');
//!
//! // Encode the document state to a binary update message.
//! let update = doc.encode_state_as_update();
//!
//! // Retrieve the document state encoded in the update message.
//! let doc2 = yrs::Doc::new();
//! doc2.apply_update(&update);
//!
//! // check document content
//! assert_eq!(doc2.get_type("my text type").to_string(), "x");
//! ```
//!
//! # Implement a Provider
//!
//! A **provider** connects to a shared document and automatically syncs updates
//! through a medium. A provider could sync document updates to other peers through
//! a network protocol, or sync document updates to a database so that they are
//! available without a network connection. You can combine providers with each
//! other to make your application more resilient.
//!
//! In Yjs, we already have a rich collection of providers that allow you to
//! build resilient applications that sync through multiple communication
//! mediums all at once. We don't have this ecosystem yet in Yrs, but you can
//! build them easily on your own.
//!
//! ```
//! use std::rc::Rc;
//!
//! // syncs document updates to another document
//! struct MyProvider {
//!     doc: yrs::Doc
//! }
//!
//! impl yrs::Subscriber<yrs::events::UpdateEvent> for MyProvider {
//!     fn on_change (&self, event: yrs::events::UpdateEvent) {
//!         self.doc.apply_update(&event.update);
//!     }
//! }
//!
//! let doc1 = yrs::Doc::new();
//! let doc2 = yrs::Doc::new();
//!
//! // register update observer
//! let provider = Rc::from(MyProvider {
//!     doc: doc2.clone()
//! });
//! doc1.on_update(Rc::downgrade(&provider));
//!
//! let my_type = doc1.get_type("my first shared type");
//!
//! {
//!     // All changes must happen within a transaction.
//!     // When the transaction is dropped, the yrs::Doc fires event (e.g. the update event)
//!     let tr = doc1.transact();
//!     my_type.insert(&tr, 0, 'a');
//! } // transaction is dropped and changes are automatically synced to doc2
//!
//! println!("synced document state: {}", doc2.get_type("my first shared type").to_string());
//! assert_eq!(doc2.get_type("my first shared type").to_string(), "a");
//! ```
//!

mod utils;
mod block;
mod block_store;
mod doc;
mod transaction;
mod updates;
mod types;
mod store;
mod id_set;

use utils::client_hasher::ClientHasher;
use std::cell::{Cell, RefCell, RefMut};
use std::collections::HashMap;
use std::hash::BuildHasherDefault;

pub struct Doc {
    pub client_id: u64,
    store: RefCell<Store>
}

#[derive(Default)]
pub struct StateVector(HashMap<u64, u32, BuildHasherDefault<ClientHasher>>);

pub struct Transaction <'a> {
    pub store: RefMut<'a, Store>,
    pub start_state_vector: StateVector,
}

pub struct ClientBlockList {
    pub list: Vec<block::Block>,
    pub integrated_len: usize,
}

pub struct BlockStore {
    pub clients: HashMap<u64, ClientBlockList, BuildHasherDefault<ClientHasher>>,
}

pub struct Store {
    client_id: u64,
    pub type_refs: HashMap<String, u32>,
    pub types: Vec<(types::Inner, String)>,
    pub blocks: BlockStore,
}
