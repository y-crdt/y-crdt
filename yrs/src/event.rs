use crate::wrap::Wrap;
use crate::{DeleteSet, StateVector, TransactionMut};

/// An update event passed to a callback subscribed with [Doc::observe_update_v1]/[Doc::observe_update_v2].
pub struct UpdateEvent {
    /// A binary which contains information about all inserted and deleted changes performed within
    /// the scope of its [TransactionMut].
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
pub struct TransactionCleanupEvent {
    pub before_state: StateVector,
    pub after_state: StateVector,
    pub delete_set: DeleteSet,
}

impl TransactionCleanupEvent {
    pub fn new(txn: &TransactionMut) -> Self {
        TransactionCleanupEvent {
            before_state: txn.before_state().clone(),
            after_state: txn.after_state().clone(),
            delete_set: txn.delete_set().cloned().unwrap_or_default(),
        }
    }
}

/// Event used to communicate load requests from the underlying subdocuments.
#[derive(Debug)]
pub struct SubdocsEvent {
    pub(crate) loaded: Vec<Wrap<crate::Doc>>,
    pub(crate) added: Vec<Wrap<crate::Doc>>,
    pub(crate) removed: Vec<crate::Doc>,
}

impl SubdocsEvent {
    pub(crate) fn new(
        added: Vec<Wrap<crate::Doc>>,
        removed: Vec<crate::Doc>,
        loaded: Vec<Wrap<crate::Doc>>,
    ) -> Self {
        SubdocsEvent {
            loaded,
            added,
            removed,
        }
    }
    /// Returns an iterator over all sub-documents living in a parent document, that have requested
    /// to be loaded within a scope of committed transaction.
    pub fn loaded(&self) -> &[Wrap<crate::Doc>] {
        &self.loaded
    }

    /// Returns an iterator over all sub-documents added to a current document within a scope of
    /// committed transaction.
    pub fn added(&self) -> &[Wrap<crate::Doc>] {
        &self.added
    }

    /// Returns an iterator over all sub-documents removed from a current document within a scope of
    /// committed transaction.
    pub fn removed(&self) -> &[crate::Doc] {
        &self.removed
    }
}
