use std::collections::HashMap;

use crate::Origin;

#[cfg(feature = "sync")]
mod pending {
    use crate::Origin;
    use std::sync::{Arc, Mutex, Weak};

    #[derive(Clone)]
    pub(crate) struct PendingRemovals(Arc<Mutex<Vec<Origin>>>);

    impl Default for PendingRemovals {
        fn default() -> Self {
            PendingRemovals(Arc::new(Mutex::new(Vec::new())))
        }
    }

    impl PendingRemovals {
        pub fn push(&self, id: Origin) {
            if let Ok(mut queue) = self.0.lock() {
                queue.push(id);
            }
        }

        pub fn drain(&self) -> Vec<Origin> {
            match self.0.lock() {
                Ok(mut queue) => std::mem::take(&mut *queue),
                Err(_) => Vec::new(),
            }
        }

        pub fn downgrade(&self) -> WeakRemovals {
            WeakRemovals(Arc::downgrade(&self.0))
        }
    }

    pub(crate) struct WeakRemovals(Weak<Mutex<Vec<Origin>>>);

    impl WeakRemovals {
        pub fn push(&self, id: Origin) {
            if let Some(arc) = self.0.upgrade() {
                if let Ok(mut queue) = arc.lock() {
                    queue.push(id);
                }
            }
        }
    }
}

#[cfg(not(feature = "sync"))]
mod pending {
    use crate::Origin;
    use std::cell::RefCell;
    use std::rc::{Rc, Weak};

    #[derive(Clone)]
    pub(crate) struct PendingRemovals(Rc<RefCell<Vec<Origin>>>);

    impl Default for PendingRemovals {
        fn default() -> Self {
            PendingRemovals(Rc::new(RefCell::new(Vec::new())))
        }
    }

    impl PendingRemovals {
        pub fn push(&self, id: Origin) {
            self.0.borrow_mut().push(id);
        }

        pub fn drain(&self) -> Vec<Origin> {
            std::mem::take(&mut *self.0.borrow_mut())
        }

        pub fn downgrade(&self) -> WeakRemovals {
            WeakRemovals(Rc::downgrade(&self.0))
        }
    }

    pub(crate) struct WeakRemovals(Weak<RefCell<Vec<Origin>>>);

    impl WeakRemovals {
        pub fn push(&self, id: Origin) {
            if let Some(rc) = self.0.upgrade() {
                rc.borrow_mut().push(id);
            }
        }
    }
}

use pending::{PendingRemovals, WeakRemovals};

/// Subscription handle returned by [Observer::subscribe], which will unsubscribe the
/// corresponding callback when dropped.
pub struct Subscription {
    id: Origin,
    pending: WeakRemovals,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.pending.push(std::mem::take(&mut self.id));
    }
}

#[cfg(feature = "sync")]
unsafe impl Send for Subscription {}
#[cfg(feature = "sync")]
unsafe impl Sync for Subscription {}

/// Data structure used to handle publish/subscribe callbacks of a specific type.
pub struct Observer<F> {
    callbacks: HashMap<Origin, F>,
    pending: PendingRemovals,
}

impl<F> Observer<F> {
    /// Creates a new [Observer] with no active callbacks.
    pub fn new() -> Self {
        Observer {
            callbacks: HashMap::new(),
            pending: PendingRemovals::default(),
        }
    }

    /// Returns `true` if this observer has any active subscribers.
    pub fn has_subscribers(&self) -> bool {
        !self.callbacks.is_empty()
    }

    /// Subscribes a callback to this observer with an auto-generated key.
    /// Returns a [Subscription] which will unsubscribe the callback when dropped.
    pub fn subscribe(&mut self, callback: F) -> Subscription {
        self.drain_pending();
        let id = Origin::from(fastrand::usize(0..usize::MAX));
        self.callbacks.insert(id.clone(), callback);
        Subscription {
            id,
            pending: self.pending.downgrade(),
        }
    }

    /// Subscribes a callback with a specific key. If a callback with the same key already
    /// exists, it will be replaced.
    pub fn subscribe_with(&mut self, id: impl Into<Origin>, callback: F) {
        self.drain_pending();
        self.callbacks.insert(id.into(), callback);
    }

    /// Removes a callback by its key. Returns `true` if the callback was found and removed.
    pub fn unsubscribe(&mut self, id: &Origin) -> bool {
        self.callbacks.remove(id).is_some()
    }

    /// Calls `each` for every registered callback, giving mutable access to the callback.
    pub fn trigger<E: FnMut(&mut F)>(&mut self, mut each: E) {
        self.drain_pending();
        for callback in self.callbacks.values_mut() {
            each(callback);
        }
    }

    fn drain_pending(&mut self) {
        for id in self.pending.drain() {
            self.callbacks.remove(&id);
        }
    }
}

impl<F> Default for Observer<F> {
    fn default() -> Self {
        Self::new()
    }
}


#[cfg(test)]
mod test {
    use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
    use std::sync::Arc;

    use crate::observer::Observer;
    use crate::Origin;

    #[test]
    fn subscription() {
        let mut o: Observer<Box<dyn FnMut(&u32) + Send + Sync + 'static>> = Observer::new();
        let s1_state = Arc::new(AtomicU32::new(0));
        let s2_state = Arc::new(AtomicU32::new(0));

        {
            let a = s1_state.clone();
            let b = s2_state.clone();

            let _s1 = o.subscribe(Box::new(move |&value| {
                a.store(value, Ordering::Release);
            }));
            let _s2 = o.subscribe(Box::new(move |&value| {
                b.store(value * 2, Ordering::Release);
            }));

            o.trigger(|fun| fun(&1));
            assert_eq!(s1_state.load(Ordering::Acquire), 1);
            assert_eq!(s2_state.load(Ordering::Acquire), 2);

            o.trigger(|fun| fun(&2));
            assert_eq!(s1_state.load(Ordering::Acquire), 2);
            assert_eq!(s2_state.load(Ordering::Acquire), 4);
        }

        // subscriptions were dropped, pending removals queued
        // next trigger will drain them
        o.trigger(|fun| fun(&3));
        assert_eq!(s1_state.load(Ordering::Acquire), 2);
        assert_eq!(s2_state.load(Ordering::Acquire), 4);
    }

    #[test]
    fn subscribers_predicate() {
        let mut o: Observer<Box<dyn FnMut(&u32) + Send + Sync + 'static>> = Observer::new();
        assert!(!o.has_subscribers());

        let sub = o.subscribe(Box::new(move |_| {}));
        assert!(o.has_subscribers());

        drop(sub);
        o.drain_pending();
        assert!(!o.has_subscribers());
    }

    #[test]
    fn subscribe_with_replaced_old_callback() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut o: Observer<Box<dyn FnMut(u32) + Send + Sync + 'static>> = Observer::new();
        let ta = tx.clone();
        o.subscribe_with(
            123usize,
            Box::new(move |i| ta.send(format!("a-{i}")).unwrap()),
        );
        o.trigger(|fun| fun(1));
        assert_eq!(rx.try_recv().unwrap(), "a-1");

        // override the callback with the same key
        o.subscribe_with(
            123usize,
            Box::new(move |i| tx.send(format!("b-{i}")).unwrap()),
        );
        o.trigger(|fun| fun(2));
        assert_eq!(rx.try_recv().unwrap(), "b-2");
    }

    struct DropCounter(Arc<AtomicI32>);

    impl DropCounter {
        fn new(counter: Arc<AtomicI32>) -> Self {
            counter.fetch_add(1, Ordering::SeqCst);
            DropCounter(counter)
        }
    }

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn drop_subscription() {
        let counter = Arc::new(AtomicI32::new(0));
        let mut o: Observer<DropCounter> = Observer::new();
        for _ in 0..100 {
            assert_eq!(counter.load(Ordering::SeqCst), 0);
            let sub = o.subscribe(DropCounter::new(counter.clone()));
            assert_eq!(counter.load(Ordering::SeqCst), 1);
            drop(sub);
            o.drain_pending(); // flush deferred removals
        }
    }

    #[test]
    fn drop_subscription2() {
        let counter = Arc::new(AtomicI32::new(0));
        let mut o: Observer<DropCounter> = Observer::new();
        let mut subscriptions = Vec::new();
        for _ in 0..100 {
            let sub = o.subscribe(DropCounter::new(counter.clone()));
            subscriptions.push(sub);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 100);
        drop(subscriptions);
        o.drain_pending();
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn unsubscribe() {
        let counter = Arc::new(AtomicI32::new(0));
        let mut o: Observer<DropCounter> = Observer::new();
        for i in 0..100 {
            assert_eq!(counter.load(Ordering::SeqCst), 0);

            o.subscribe_with(i, DropCounter::new(counter.clone()));

            assert_eq!(counter.load(Ordering::SeqCst), 1);

            let unsubscribed = o.unsubscribe(&Origin::from(i));
            assert!(unsubscribed, "unsubscribe failed for {}", i);
        }
    }

    #[test]
    fn unsubscribe2() {
        let counter = Arc::new(AtomicI32::new(0));
        let mut o: Observer<DropCounter> = Observer::new();
        for i in 0..100 {
            o.subscribe_with(i, DropCounter::new(counter.clone()));
        }

        assert_eq!(counter.load(Ordering::SeqCst), 100);

        for i in 0..100 {
            let unsubscribed = o.unsubscribe(&Origin::from(i));
            assert!(unsubscribed, "unsubscribe failed for {}", i);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
}