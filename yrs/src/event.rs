use crate::doc::DocAddr;
use crate::transaction::Subdocs;
use crate::{DeleteSet, Doc, StateVector, TransactionMut};
use std::collections::HashMap;

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
            before_state: txn.before_state.clone(),
            after_state: txn.after_state.clone(),
            delete_set: txn.delete_set.clone(),
        }
    }
}

/// Event used to communicate load requests from the underlying subdocuments.
#[derive(Debug, Clone)]
pub struct SubdocsEvent {
    pub(crate) added: HashMap<DocAddr, Doc>,
    pub(crate) removed: HashMap<DocAddr, Doc>,
    pub(crate) loaded: HashMap<DocAddr, Doc>,
}

impl SubdocsEvent {
    pub(crate) fn new(inner: Box<Subdocs>) -> Self {
        SubdocsEvent {
            added: inner.added,
            removed: inner.removed,
            loaded: inner.loaded,
        }
    }

    /// Returns an iterator over all sub-documents added to a current document within a scope of
    /// committed transaction.
    pub fn added(&self) -> SubdocsEventIter {
        SubdocsEventIter(self.added.values())
    }

    /// Returns an iterator over all sub-documents removed from a current document within a scope of
    /// committed transaction.
    pub fn removed(&self) -> SubdocsEventIter {
        SubdocsEventIter(self.removed.values())
    }

    /// Returns an iterator over all sub-documents living in a parent document, that have requested
    /// to be loaded within a scope of committed transaction.
    pub fn loaded(&self) -> SubdocsEventIter {
        SubdocsEventIter(self.loaded.values())
    }
}

#[repr(transparent)]
pub struct SubdocsEventIter<'a>(std::collections::hash_map::Values<'a, DocAddr, Doc>);

impl<'a> Iterator for SubdocsEventIter<'a> {
    type Item = &'a Doc;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

impl<'a> ExactSizeIterator for SubdocsEventIter<'a> {
    fn len(&self) -> usize {
        self.0.len()
    }
}
