use crate::{DeleteSet, StateVector};

/// An update event passed to a callback registered in the event handler. Contains data about the
/// state of an update.
pub struct UpdateEvent {
    /// An update that's about to be applied. Update contains information about all inserted blocks,
    /// which have been send from a remote peer.
    pub update: Vec<u8>,
}

impl UpdateEvent {
    pub(crate) fn new(update: Vec<u8>) -> Self {
        UpdateEvent { update }
    }
}

/// Holds transaction update information from a commit after state vectors have been compressed.
#[derive(Debug, Clone)]
pub struct AfterTransactionEvent {
    pub before_state: StateVector,
    pub after_state: StateVector,
    pub delete_set: DeleteSet,
}
