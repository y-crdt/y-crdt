use crate::transaction::Subdocs;
use crate::{DeleteSet, DocRef, StateVector, TransactionMut};
use std::collections::HashMap;
use uuid::Uuid;

/// An update event passed to a callback registered in the event handler. Contains data about the
/// state of an update.
pub struct UpdateEvent {
    /// An update that's about to be applied. Update contains information about all inserted blocks,
    /// which have been send from a remote peer.
    pub update: Vec<u8>,
}

impl UpdateEvent {
    pub(crate) fn new_v1(txn: &TransactionMut) -> Self {
        UpdateEvent {
            update: txn.encode_update_v1(),
        }
    }
    pub(crate) fn new_v2(txn: &TransactionMut) -> Self {
        UpdateEvent {
            update: txn.encode_update_v2(),
        }
    }
}

/// Holds transaction update information from a commit after state vectors have been compressed.
#[derive(Debug, Clone)]
pub struct AfterTransactionEvent {
    pub before_state: StateVector,
    pub after_state: StateVector,
    pub delete_set: DeleteSet,
}

impl AfterTransactionEvent {
    pub fn new(txn: &TransactionMut) -> Self {
        AfterTransactionEvent {
            before_state: txn.before_state.clone(),
            after_state: txn.after_state.clone(),
            delete_set: txn.delete_set.clone(),
        }
    }
}

/// Event used to communicate load requests from the underlying subdocuments.
#[derive(Debug, Clone)]
pub struct SubdocsEvent {
    pub added: HashMap<Uuid, DocRef>,
    pub removed: HashMap<Uuid, DocRef>,
    pub loaded: HashMap<Uuid, DocRef>,
}

impl SubdocsEvent {
    pub(crate) fn new(inner: Box<Subdocs>) -> Self {
        SubdocsEvent {
            added: inner.added,
            removed: inner.removed,
            loaded: inner.loaded,
        }
    }
}
