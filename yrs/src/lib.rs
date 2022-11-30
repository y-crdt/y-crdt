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

mod alt;
pub mod block;
mod block_store;
mod doc;
mod event;
mod id_set;
mod store;
mod transaction;
pub mod types;
mod update;
pub mod updates;
mod utils;

#[cfg(test)]
mod compatibility_tests;

pub mod atomic;
mod block_iter;
mod moving;
pub mod observer;
#[cfg(test)]
mod test_utils;

pub use crate::alt::{
    diff_updates_v1, diff_updates_v2, encode_state_vector_from_update_v1,
    encode_state_vector_from_update_v2, merge_updates_v1, merge_updates_v2,
};
pub use crate::block::ID;
pub use crate::block_store::Snapshot;
pub use crate::block_store::StateVector;
pub use crate::doc::Doc;
pub use crate::doc::DocRef;
pub use crate::doc::OffsetKind;
pub use crate::doc::Options;
pub use crate::doc::Transact;
pub use crate::doc::{
    AfterTransactionSubscription, DestroySubscription, SubdocsSubscription, UpdateSubscription,
};
pub use crate::event::{AfterTransactionEvent, SubdocsEvent, SubdocsEventIter, UpdateEvent};
pub use crate::id_set::DeleteSet;
pub use crate::observer::{Observer, Subscription, SubscriptionId};
pub use crate::store::Store;
pub use crate::transaction::ReadTxn;
pub use crate::transaction::RootRefs;
pub use crate::transaction::Transaction;
pub use crate::transaction::TransactionMut;
pub use crate::transaction::WriteTxn;
pub use crate::types::array::Array;
pub use crate::types::array::ArrayPrelim;
pub use crate::types::array::ArrayRef;
pub use crate::types::map::Map;
pub use crate::types::map::MapPrelim;
pub use crate::types::map::MapRef;
pub use crate::types::text::Text;
pub use crate::types::text::TextPrelim;
pub use crate::types::text::TextRef;
pub use crate::types::xml::Xml;
pub use crate::types::xml::XmlElementPrelim;
pub use crate::types::xml::XmlElementRef;
pub use crate::types::xml::XmlFragment;
pub use crate::types::xml::XmlFragmentPrelim;
pub use crate::types::xml::XmlFragmentRef;
pub use crate::types::xml::XmlNode;
pub use crate::types::xml::XmlTextPrelim;
pub use crate::types::xml::XmlTextRef;
pub use crate::types::GetString;
pub use crate::types::Observable;
pub use crate::update::Update;
use rand::RngCore;

pub type Uuid = std::sync::Arc<str>;

pub fn uuid_v4<R: RngCore>(rng: &mut R) -> Uuid {
    let mut b = [0u8; 16];
    rng.fill_bytes(&mut b);
    let uuid = format!(
        "{:x}{:x}{:x}{:x}-{:x}{:x}-{:x}{:x}-{:x}{:x}-{:x}{:x}{:x}{:x}{:x}{:x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    );
    uuid.into()
}
