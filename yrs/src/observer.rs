use crate::TransactionMut;
use arc_swap::ArcSwapOption;
use smallvec::{smallvec, SmallVec};
use std::fmt::Debug;
use std::sync::{Arc, Weak};

pub type Callback<E> = Arc<dyn Fn(&TransactionMut, &E) + 'static>;
type WeakCallback<E> = Weak<dyn Fn(&TransactionMut, &E) + 'static>;
type WeakCallbacks<E> = SmallVec<[WeakCallback<E>; 1]>;

/// Data structure used to handle publish/subscribe callbacks of specific type. Observers perform
/// subscriber changes in thread-safe manner, using atomic hardware intrinsics.
#[derive(Debug)]
pub struct Observer<E> {
    inner: ArcSwapOption<WeakCallbacks<E>>,
}

unsafe impl<E: Send> Send for Observer<E> {}
unsafe impl<E: Sync> Sync for Observer<E> {}

impl<E> Observer<E> {
    /// Creates a new [Observer] with no active callbacks.
    pub fn new() -> Self {
        Observer {
            inner: ArcSwapOption::new(None),
        }
    }

    /// Cleanup already released subscriptions. Whenever a [Subscription] is dropped, the callback is released. However,
    /// the weak reference to callback may still be kept around until it becomes touched by operations such as
    /// [Observer::subscribe] or [Observer::callbacks].
    ///
    /// This method allows to perform stale callback cleanup without waiting for callbacks to be visited.
    pub fn clean(&self) {
        self.inner.rcu(|callbacks| match callbacks {
            None => None,
            Some(links) => {
                let mut res = WeakCallbacks::with_capacity(links.len() - 1);
                for l in links.iter() {
                    if Weak::strong_count(l) != 0 {
                        res.push(l.clone());
                    }
                }
                Some(Arc::new(res))
            }
        });
    }

    /// Subscribes a callback parameter to a current [Observer].
    /// Returns a subscription object which - when dropped - will unsubscribe current callback.
    pub fn subscribe<F>(&self, callback: F) -> Subscription
    where
        F: Fn(&TransactionMut, &E) -> () + 'static,
    {
        let strong = Arc::new(callback);
        let subscription: Arc<dyn std::any::Any> = strong.clone();
        let generic: Callback<E> = strong;
        let weak = Arc::downgrade(&generic);
        self.inner.rcu(|callbacks| match callbacks {
            None => Arc::new(smallvec![weak.clone()]),
            Some(links) => {
                let mut res = WeakCallbacks::with_capacity(links.len() + 1);
                for l in links.iter() {
                    if Weak::strong_count(l) != 0 {
                        res.push(l.clone());
                    }
                }
                res.push(weak.clone());
                Arc::new(res)
            }
        });
        Subscription {
            callback: subscription,
        }
    }

    /// Returns a snapshot of callbacks subscribed to this observer at the moment when this method
    /// has been called. This snapshot can be iterated over to get access to individual callbacks
    /// and trigger them.
    pub fn callbacks(&self) -> Option<Callbacks<E>> {
        Callbacks::new(self)
    }
}

impl<E> Default for Observer<E> {
    fn default() -> Self {
        Observer {
            inner: ArcSwapOption::from(None),
        }
    }
}

#[derive(Debug)]
pub struct Callbacks<'a, E> {
    observer: &'a Observer<E>,
    callbacks: Arc<WeakCallbacks<E>>,
    index: usize,
    should_cleanup: bool,
}

unsafe impl<'a, E: Send> Send for Callbacks<'a, E> {}
unsafe impl<'a, E: Sync> Sync for Callbacks<'a, E> {}

impl<'a, E> Callbacks<'a, E> {
    fn new(observer: &'a Observer<E>) -> Option<Self> {
        let callbacks = observer.inner.load_full()?;
        Some(Callbacks {
            observer,
            callbacks,
            index: 0,
            should_cleanup: false,
        })
    }

    pub fn trigger(&mut self, txn: &TransactionMut, e: &E) {
        for cb in self {
            cb(txn, e);
        }
    }
}

impl<'a, E> Iterator for Callbacks<'a, E> {
    type Item = Callback<E>;

    fn next(&mut self) -> Option<Self::Item> {
        let callbacks = &*self.callbacks;
        while self.index < callbacks.len() {
            let weak = &callbacks[self.index];
            self.index += 1;
            if let Some(strong) = weak.upgrade() {
                return Some(strong);
            }
        }
        None
    }
}

impl<'a, E> Drop for Callbacks<'a, E> {
    fn drop(&mut self) {
        if self.should_cleanup {
            self.observer.clean();
        }
    }
}

pub type CallbackMut<E> = Arc<dyn Fn(&mut TransactionMut, &mut E) + 'static>;
type WeakCallbackMut<E> = Weak<dyn Fn(&mut TransactionMut, &mut E) + 'static>;
type WeakCallbacksMut<E> = SmallVec<[WeakCallbackMut<E>; 1]>;

/// Data structure used to handle publish/subscribe callbacks of specific type. Observers perform
/// subscriber changes in thread-safe manner, using atomic hardware intrinsics.
#[derive(Debug)]
pub struct ObserverMut<E> {
    inner: ArcSwapOption<WeakCallbacksMut<E>>,
}

unsafe impl<E: Send> Send for ObserverMut<E> {}
unsafe impl<E: Sync> Sync for ObserverMut<E> {}

impl<E> ObserverMut<E> {
    /// Creates a new [ObserverMut with no active callbacks.
    pub fn new() -> Self {
        ObserverMut {
            inner: ArcSwapOption::new(None),
        }
    }

    /// Cleanup already released subscriptions. Whenever a [Subscription] is dropped, the callback is released. However,
    /// the weak reference to callback may still be kept around until it becomes touched by operations such as
    /// [Observer::subscribe] or [Observer::callbacks].
    ///
    /// This method allows to perform stale callback cleanup without waiting for callbacks to be visited.
    pub fn clean(&self) {
        self.inner.rcu(|callbacks| match callbacks {
            None => None,
            Some(links) => {
                let mut res = WeakCallbacksMut::with_capacity(links.len() - 1);
                for l in links.iter() {
                    if Weak::strong_count(l) != 0 {
                        res.push(l.clone());
                    }
                }
                Some(Arc::new(res))
            }
        });
    }

    /// Subscribes a callback parameter to a current [Observer].
    /// Returns a subscription object which - when dropped - will unsubscribe current callback.
    pub fn subscribe<F>(&self, callback: F) -> Subscription
    where
        F: Fn(&mut TransactionMut, &mut E) + 'static,
    {
        let strong = Arc::new(callback);
        let subscription: Arc<dyn std::any::Any> = strong.clone();
        let generic: CallbackMut<E> = strong;
        let weak = Arc::downgrade(&generic);
        self.inner.rcu(|callbacks| match callbacks {
            None => Arc::new(smallvec![weak.clone()]),
            Some(links) => {
                let mut res = WeakCallbacksMut::with_capacity(links.len() + 1);
                for l in links.iter() {
                    if Weak::strong_count(l) != 0 {
                        res.push(l.clone());
                    }
                }
                res.push(weak.clone());
                Arc::new(res)
            }
        });
        Subscription {
            callback: subscription,
        }
    }

    /// Returns a snapshot of callbacks subscribed to this observer at the moment when this method
    /// has been called. This snapshot can be iterated over to get access to individual callbacks
    /// and trigger them.
    pub fn callbacks(&self) -> Option<CallbacksMut<E>> {
        CallbacksMut::new(self)
    }
}

impl<E> Default for ObserverMut<E> {
    fn default() -> Self {
        ObserverMut {
            inner: ArcSwapOption::from(None),
        }
    }
}

#[derive(Debug)]
pub struct CallbacksMut<'a, E> {
    observer: &'a ObserverMut<E>,
    callbacks: Arc<WeakCallbacksMut<E>>,
    index: usize,
    should_cleanup: bool,
}

unsafe impl<'a, E: Send> Send for CallbacksMut<'a, E> {}
unsafe impl<'a, E: Sync> Sync for CallbacksMut<'a, E> {}

impl<'a, E> CallbacksMut<'a, E> {
    fn new(observer: &'a ObserverMut<E>) -> Option<Self> {
        let callbacks = observer.inner.load_full()?;
        Some(CallbacksMut {
            observer,
            callbacks,
            index: 0,
            should_cleanup: false,
        })
    }

    pub fn trigger(&mut self, txn: &mut TransactionMut, e: &mut E) {
        for cb in self {
            cb(txn, e);
        }
    }
}

impl<'a, E> Iterator for CallbacksMut<'a, E> {
    type Item = CallbackMut<E>;

    fn next(&mut self) -> Option<Self::Item> {
        let callbacks = &*self.callbacks;
        while self.index < callbacks.len() {
            let weak = &callbacks[self.index];
            self.index += 1;
            if let Some(strong) = weak.upgrade() {
                return Some(strong);
            }
        }
        None
    }
}

impl<'a, E> Drop for CallbacksMut<'a, E> {
    fn drop(&mut self) {
        if self.should_cleanup {
            self.observer.clean();
        }
    }
}

/// Subscription handle returned by [Observer::subscribe] methods, which will unsubscribe corresponding
/// callback when dropped.
///
/// If implicit callback unsubscribe on drop is undesired, this structure can be cast [into](Subscription::into)
/// [SubscriptionId] which is an identifier of the same subscription, which in turn must be used
/// manually via [Observer::unsubscribe] to perform usubscribe.
#[repr(C)]
pub struct Subscription {
    callback: Arc<dyn std::any::Any>,
}

unsafe impl Send for Subscription {}
unsafe impl Sync for Subscription {}

#[cfg(test)]
mod test {
    use crate::observer::Observer;
    use crate::Transact;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::thread::spawn;

    #[test]
    fn subscription() {
        let doc = crate::Doc::new();
        let txn = doc.transact_mut();
        let o: Observer<u32> = Observer::new();
        let s1_state = Arc::new(AtomicU32::new(0));
        let s2_state = Arc::new(AtomicU32::new(0));

        {
            let a = s1_state.clone();
            let b = s2_state.clone();

            let _s1 = o.subscribe(move |_, &value| a.store(value, Ordering::Release));
            let _s2 = o.subscribe(move |_, &value| b.store(value * 2, Ordering::Release));

            for fun in o.callbacks().unwrap() {
                fun(&txn, &1)
            }
            assert_eq!(s1_state.load(Ordering::Acquire), 1);
            assert_eq!(s2_state.load(Ordering::Acquire), 2);

            for fun in o.callbacks().unwrap() {
                fun(&txn, &2)
            }
            assert_eq!(s1_state.load(Ordering::Acquire), 2);
            assert_eq!(s2_state.load(Ordering::Acquire), 4);
        }

        // subscriptions were dropped, we don't expect updates to be propagated
        for fun in o.callbacks().unwrap() {
            fun(&txn, &3)
        }
        assert_eq!(s1_state.load(Ordering::Acquire), 2);
        assert_eq!(s2_state.load(Ordering::Acquire), 4);
    }

    #[test]
    fn multi_threading() {
        let o: Observer<u32> = Observer::new();

        let s1_state = Arc::new(AtomicU32::new(0));
        let a = s1_state.clone();
        let sub1 = o.subscribe(move |_, &value| a.store(value, Ordering::Release));

        let s2_state = Arc::new(AtomicU32::new(0));
        let b = s2_state.clone();
        let sub2 = o.subscribe(move |_, &value| b.store(value, Ordering::Release));

        let handle = spawn(move || {
            let doc = crate::Doc::new();
            let txn = doc.transact_mut();
            for fun in o.callbacks().unwrap() {
                fun(&txn, &1)
            }
            drop(sub1);
            drop(sub2);
        });

        handle.join().unwrap();

        assert_eq!(s1_state.load(Ordering::Acquire), 1);
        assert_eq!(s2_state.load(Ordering::Acquire), 1);
    }

    #[test]
    fn unsubscribe_calls_drop() {
        let o: Observer<()> = Observer::new();
        let c = Arc::new(());
        let subscription = {
            let inner = c.clone();
            o.subscribe(move |_, _| {
                let count = Arc::strong_count(&inner);
                assert_eq!(count, 2);
            })
        };
        let doc = crate::Doc::new();
        let txn = doc.transact_mut();
        for cb in o.callbacks().unwrap() {
            cb(&txn, &());
        }
        drop(subscription);

        let count = Arc::strong_count(&c);
        assert_eq!(count, 1);
    }
}
