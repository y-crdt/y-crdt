use crate::transaction::TransactionMut;
use crate::{DeleteSet, StateVector};
use rand::RngCore;
use std::collections::HashMap;
use std::ptr::NonNull;

#[repr(transparent)]
pub(crate) struct EventHandler<T>(Box<Subscriptions<T>>);

pub type SubscriptionId = u32;

type Subscriptions<V> =
    HashMap<SubscriptionId, Box<dyn for<'doc> Fn(&TransactionMut<'doc>, &V) -> ()>>;

impl<V> EventHandler<V> {
    pub fn new() -> Self {
        EventHandler(Box::new(Subscriptions::new()))
    }

    pub fn subscribe<F>(&mut self, f: F) -> Subscription<V>
    where
        F: Fn(&TransactionMut, &V) -> () + 'static,
    {
        let mut rng = rand::thread_rng();
        let id = rng.next_u32();
        self.0.insert(id, Box::new(f));
        let subscriptions = NonNull::from(self.0.as_mut());
        Subscription { id, subscriptions }
    }

    pub fn unsubscribe(&mut self, subscription_id: u32) {
        self.0.remove(&subscription_id);
    }

    pub fn publish(&self, txn: &TransactionMut, arg: &V) {
        for f in self.0.values() {
            f(txn, arg);
        }
    }

    pub fn has_subscribers(&self) -> bool {
        !self.0.is_empty()
    }

    fn subscription_count(&self) -> usize {
        self.0.len()
    }
}

impl<T> Default for EventHandler<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// A subscription handle to a custom user-defined callback for an event handler. When dropped,
/// it will unsubscribe corresponding callback.
pub struct Subscription<T> {
    id: SubscriptionId,
    subscriptions: NonNull<Subscriptions<T>>,
}

impl<T> Into<SubscriptionId> for Subscription<T> {
    fn into(self) -> SubscriptionId {
        let id = self.id;
        std::mem::forget(self);
        id
    }
}

impl<T> Drop for Subscription<T> {
    fn drop(&mut self) {
        let subs = unsafe { self.subscriptions.as_mut() };
        subs.remove(&self.id);
    }
}

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
pub struct AfterTransactionEvent {
    pub before_state: StateVector,
    pub after_state: StateVector,
    pub delete_set: DeleteSet,
}

#[cfg(test)]
mod test {
    use crate::event::EventHandler;
    use crate::Doc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn subscription() {
        let doc = Doc::new();
        let txn = doc.transact_mut(); // just for sake of parameter passing

        let mut eh: EventHandler<u32> = EventHandler::new();
        let s1_state = Arc::new(AtomicU32::new(0));
        let s2_state = Arc::new(AtomicU32::new(0));

        {
            let a = s1_state.clone();
            let b = s2_state.clone();

            let _s1 = eh.subscribe(move |_, value| a.store(*value, Ordering::Release));
            let _s2 = eh.subscribe(move |_, value| b.store(*value * 2, Ordering::Release));
            assert_eq!(eh.subscription_count(), 2);

            eh.publish(&txn, &1);
            assert_eq!(s1_state.load(Ordering::Acquire), 1);
            assert_eq!(s2_state.load(Ordering::Acquire), 2);

            eh.publish(&txn, &2);
            assert_eq!(s1_state.load(Ordering::Acquire), 2);
            assert_eq!(s2_state.load(Ordering::Acquire), 4);
        }

        // both subscriptions left the scope, they should be dropped
        assert_eq!(eh.subscription_count(), 0);

        // subscriptions were dropped, we don't expect updates to be propagated
        eh.publish(&txn, &3);
        assert_eq!(s1_state.load(Ordering::Acquire), 2);
        assert_eq!(s2_state.load(Ordering::Acquire), 4);
    }
}
