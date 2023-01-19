//! Yrs (read: "wires") is a high performance CRDT implementation based on the idea of **Shared Types**.
//! It is a compatible port of the [Yjs](https://github.com/yjs/yjs) CRDT.
//!
//! **Shared Types** work just like normal data types, but they automatically sync with other peers.
//!
//! A **Document** is the access point to create shared types, and to listen to update events.
//!
//! # Quick start
//!
//! Let's discuss basic features of Yrs. We'll introduce these concepts starting from
//! the following code snippet:
//!
//! ```rust
//! use yrs::{Doc, GetString, ReadTxn, StateVector, Text, Transact, Update};
//! use yrs::updates::decoder::Decode;
//! use yrs::updates::encoder::Encode;
//!
//! let doc = Doc::new();
//! let text = doc.get_or_insert_text("article");
//!
//! {
//!   let mut txn = doc.transact_mut();
//!   text.insert(&mut txn, 0, "hello");
//!   text.insert(&mut txn, 5, " world");
//!   // other rich text operations include formatting or inserting embedded elements
//! } // transaction is automatically committed when dropped
//!
//! assert_eq!(text.get_string(&doc.transact()), "hello world".to_owned());
//!
//! // synchronize state with remote replica
//! let remote_doc = Doc::new();
//! let remote_text = remote_doc.get_or_insert_text("article");
//! let remote_timestamp = remote_doc.transact().state_vector().encode_v1();
//!
//! // get update with contents not observed by remote_doc
//! let update = doc.transact().encode_diff_v1(&StateVector::decode_v1(&remote_timestamp).unwrap());
//! // apply update on remote doc
//! remote_doc.transact_mut().apply_update(Update::decode_v1(&update).unwrap());
//!
//! assert_eq!(text.get_string(&doc.transact()), remote_text.get_string(&remote_doc.transact()));
//! ```
//!
//! The [Doc] is a core structure of Yrs. All other structures and operations are performed in
//! context of their document. All documents gets randomly generated [Doc::client_id] (which can be
//! also defined explicitly), which must be unique per active peer. It's crucial, as potential
//! concurrent changes made by different peers sharing the same [ClientID] will cause a document
//! state corruption.
//!
//! Next line defines [TextRef] - a shared collection specialized in providing collaborative rich text
//! operations. Here it has been defined by calling [Doc::get_or_insert_text] method.
//! Shared types defined at the document level are so called root types. Root level types sharing
//! the same name across different peers are considered to be replicas of the same logical entity,
//! regardless of their type. It's highly recommended for all collaborating clients to define all
//! root level types they are going to use up front, during document creation. A list of supported
//! shared types include: [TextRef], [ArrayRef], [MapRef], [XmlTextRef], [XmlFragmentRef] and [XmlElementRef].
//!
//! Next section of code performs an update over the defined text replica. In Yrs all operations
//! must be executed in a scope of transaction created by the document there were defined in. We
//! can differentiate two transaction types:
//!
//! 1. [Read-only transactions](Transaction), created via [Doc::transact]. They are used only to
//!    access the contents of an underlying document store but they never alter it. They are useful
//!    for methods like reading the structure state or for serialization. It's allowed to have
//!    multiple active read-only transactions as long as no read-write transaction is in progress.
//! 2. [Read-write transactions](TransactionMut), create via [Doc::transact_mut]. These can be used
//!    to modify the internal document state. These transactions work as intelligent batches. They
//!    are automatically committed when dropped, performing tasks like state cleaning, metadata
//!    compression and triggering event callbacks. Read-write transactions require exclusive access
//!    to an underlying document store - no other transaction (neither read-write nor read-only one)
//!    can be active while read-write transaction is to be created.
//!
//! In order to synchronize state between the document replicas living on a different peer processes,
//! there are two possible cases:
//!
//! 1. Peer who wishes to receive an update first encodes its document's [state vector](ReadTxn::state_vector).
//!    It's a logical timestamp describing which updates that has been observed by this document instance
//!    so far. This [StateVector] can be later serialized and passed to the remote collaborator. This
//!    collaborator can then deserialize it back and [generate an update](ReadTxn::encode_diff) which
//!    will contain all new changes performed since provided state vector. Finally this update can
//!    be passed back to the requester, [deserialized](Update::decode) and integrated into a document
//!    store via [TransactionMut::apply_update].
//! 2. Another propagation mechanism relies on subscribing to [Doc::observe_update_v1] or
//!    [Doc::observe_update_v2] events, which will be fired whenever an referenced document will
//!    detect new changes.
//!
//! While 2nd option can produce smaller binary payload than the 1st one at times and doesn't require
//! request-response cycles, it cannot pass the document state prior the observer callback
//! registration and expects that all changes will surely be delivered to other peer. A practical
//! approach (used i.e. by [y-sync protocol](https://crates.io/crates/y-sync)) is usually a combination
//! of both variants: use 1st one on connection initialization between two peers followed by
//! 2nd approach to deliver subsequent changes.
//!
//! # Transaction event lifecycle
//!
//! Yrs provides a variety of lifecycle events, which enable users to react on various situations
//! and changes performed. Some of these events are used by Yrs own features (eg. [UndoManager]).
//! They are always triggered once performed update is committed by dropping or
//! [committing](TransactionMut::commit) a read-write transaction.
//!
//! An order in which these updates are fired is as follows:
//!
//! 1. Observers on updated shared types: [TextRef::observe], [ArrayRef::observe], [MapRef::observe],
//!    [XmlTextRef::observe], [XmlFragmentRef::observe] and [XmlElementRef::observe].
//! 2. Deep observers (special kind of observers that are bubbled up from nested shared types through
//!    their parent collections hierarchy): [TextRef::observe_deep], [ArrayRef::observe_deep], [MapRef::observe_deep],
//!    [XmlTextRef::observe_deep], [XmlFragmentRef::observe_deep] and [XmlElementRef::observe_deep].
//! 3. After transaction callbacks: [Doc::observe_after_transaction].
//! 4. After transaction cleanup callbacks (moment after all changes performed by transaction have
//!    been compressed an integrated into document store): [Doc::observe_transaction_cleanup].
//! 5. Update callbacks: [Doc::observe_update_v1] and [Doc::observe_update_v2]. Useful when we want
//!    to encode and propagate incremental changes made by transaction to other peers.
//! 6. Sub-document change callbacks: [Doc::observe_subdocs].
//!
//! # Other reference materials
//!
//! - [A short walkthrough over YATA](https://bartoszsypytkowski.com/yata/) - a conflict resolution
//!   algorithm used by Yrs/Yjs.
//! - [Deep dive into internal architecture of Yrs](https://bartoszsypytkowski.com/yrs-architecture/).
//! - [Detailed explanation of conflict-free reordering algorithm](https://bartoszsypytkowski.com/yata-move/) used by Yrs.

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
pub mod undo;

pub use crate::alt::{
    diff_updates_v1, diff_updates_v2, encode_state_vector_from_update_v1,
    encode_state_vector_from_update_v2, merge_updates_v1, merge_updates_v2,
};
pub use crate::block::ID;
pub use crate::block_store::Snapshot;
pub use crate::block_store::StateVector;
pub use crate::doc::Doc;
pub use crate::doc::OffsetKind;
pub use crate::doc::Options;
pub use crate::doc::Transact;
pub use crate::doc::{
    DestroySubscription, SubdocsSubscription, TransactionCleanupSubscription, UpdateSubscription,
};
pub use crate::event::{SubdocsEvent, SubdocsEventIter, TransactionCleanupEvent, UpdateEvent};
pub use crate::id_set::DeleteSet;
pub use crate::moving::AbsolutePosition;
pub use crate::moving::Assoc;
pub use crate::moving::RelativeIndex;
pub use crate::moving::RelativePosition;
pub use crate::moving::RelativePositionContext;
pub use crate::observer::{Observer, Subscription, SubscriptionId};
pub use crate::store::Store;
pub use crate::transaction::Origin;
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
pub use crate::undo::UndoManager;
pub use crate::update::Update;
use rand::RngCore;

pub type Uuid = std::sync::Arc<str>;

/// Generate random v4 UUID.
/// (See: https://www.rfc-editor.org/rfc/rfc4122#section-4.4)
pub fn uuid_v4<R: RngCore>(rng: &mut R) -> Uuid {
    let mut b = [0u8; 16];
    rng.fill_bytes(&mut b);

    // According to RFC 4122 - Section 4.4, UUID v4 requires setting up following:
    b[6] = b[6] & 0x0f | 0x40; // time_hi_and_version (bits 4-7 of 7th octet)
    b[8] = b[8] & 0x3f | 0x80; // clock_seq_hi_and_reserved (bit 6 & 7 of 9th octet)

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
