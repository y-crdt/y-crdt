use crate::update::Update;
use rand::RngCore;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

pub(crate) struct EventHandler<T>(Rc<RefCell<Subscriptions<T>>>);

type Subscriptions<T> = HashMap<u32, Box<dyn Fn(&T) -> ()>>;

impl<T> EventHandler<T> {
    pub fn new() -> Self {
        EventHandler(Rc::new(RefCell::new(Subscriptions::new())))
    }

    pub fn subscribe<F>(&mut self, f: F) -> Subscription<T>
    where
        F: Fn(&T) -> () + 'static,
    {
        let mut rng = rand::thread_rng();
        let id = rng.next_u32();
        self.0.borrow_mut().insert(id, Box::new(f));
        let subscriptions = Rc::downgrade(&self.0);
        Subscription { id, subscriptions }
    }

    pub fn publish(&self, arg: &T) {
        let subscriptions = self.0.borrow_mut();
        for f in subscriptions.values() {
            f(arg);
        }
    }

    pub fn has_subscribers(&self) -> bool {
        !self.0.borrow().is_empty()
    }

    fn subscription_count(&self) -> usize {
        self.0.borrow().len()
    }
}

/// A subscription handle to a custom user-defined callback for an event handler. When dropped,
/// it will unsubscribe corresponding callback.
pub struct Subscription<T> {
    id: u32,
    subscriptions: Weak<RefCell<Subscriptions<T>>>,
}

impl<T> Drop for Subscription<T> {
    fn drop(&mut self) {
        if let Some(cell) = self.subscriptions.upgrade() {
            cell.borrow_mut().remove(&self.id);
        }
    }
}

/// An update event passed to a callback registered in the event handler. Contains data about the
/// state of an update.
pub struct UpdateEvent {
    /// An update that's about to be applied. Update contains information about all inserted blocks,
    /// which have been send from a remote peer.
    pub update: Update,
}

impl UpdateEvent {
    pub(crate) fn new(update: Update) -> Self {
        UpdateEvent { update }
    }
}

#[cfg(test)]
mod test {
    use crate::event::EventHandler;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn subscription() {
        let mut eh: EventHandler<u32> = EventHandler::new();
        let s1_state = Arc::new(AtomicU32::new(0));
        let s2_state = Arc::new(AtomicU32::new(0));

        {
            let a = s1_state.clone();
            let b = s2_state.clone();

            let _s1 = eh.subscribe(move |value| a.store(*value, Ordering::Release));
            let _s2 = eh.subscribe(move |value| b.store(*value * 2, Ordering::Release));
            assert_eq!(eh.subscription_count(), 2);

            eh.publish(&1);
            assert_eq!(s1_state.load(Ordering::Acquire), 1);
            assert_eq!(s2_state.load(Ordering::Acquire), 2);

            eh.publish(&2);
            assert_eq!(s1_state.load(Ordering::Acquire), 2);
            assert_eq!(s2_state.load(Ordering::Acquire), 4);
        }

        // both subscriptions left the scope, they should be dropped
        assert_eq!(eh.subscription_count(), 0);

        // subscriptions were dropped, we don't expect updates to be propagated
        eh.publish(&3);
        assert_eq!(s1_state.load(Ordering::Acquire), 2);
        assert_eq!(s2_state.load(Ordering::Acquire), 4);
    }
}
