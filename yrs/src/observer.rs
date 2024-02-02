use crate::types::Event;
use crate::TransactionMut;
use arc_swap::{ArcSwapOption, Guard, RefCnt};
use smallvec::{smallvec, SmallVec};
use std::fmt::Debug;
use std::sync::{Arc, Weak};

pub type SubscriptionId = usize;

pub trait Func: RefCnt + Sized {
    type Weak;
    fn weak(this: &Self) -> Self::Weak;
}

impl<T> Func for Arc<T> {
    type Weak = Weak<T>;

    fn weak(this: &Self) -> Self::Weak {
        Arc::downgrade(this)
    }
}

type WeakCallbacks<F> = SmallVec<[F; 1]>;

/// Data structure used to handle publish/subscribe callbacks of specific type. Observers perform
/// subscriber changes in thread-safe manner, using atomic hardware intrinsics.
///
/// # Example
///
/// ```rust
/// use std::sync::Arc;
/// use yrs::Observer;
///
/// let observer: Observer<Arc<dyn Fn(u32)->()>> = Observer::new();
/// let a = observer.subscribe(Arc::new(|arg| println!("A: {}", arg)));
/// let b = observer.subscribe(Arc::new(|arg| println!("B: {}", arg)));
///
/// // get snapshot of all active callbacks
/// for cb in observer.callbacks() {
///     cb(1);
/// }
///
/// drop(a); // unsubscribe callback
/// ```
#[derive(Debug)]
pub struct Observer<F: Func> {
    inner: ArcSwapOption<WeakCallbacks<F::Weak>>,
}

impl<F: Func> Observer<F> {
    /// Creates a new [Observer] with no active callbacks.
    pub fn new() -> Self {
        Observer {
            inner: ArcSwapOption::new(None),
        }
    }

    pub fn is_empty(&self) -> bool {
        let guard = self.inner.load();
        if let Some(callbacks) = guard.as_ref() {
            callbacks.is_empty()
        } else {
            true
        }
    }

    /// Subscribes a callback parameter to a current [Observer].
    /// Returns a subscription object which - when dropped - will unsubscribe current callback.
    pub fn subscribe(&self, callback: F) -> Subscription<F> {
        let link = F::weak(callback);
        self.inner.rcu(|callbacks| match callbacks {
            None => Arc::new(smallvec![link]),
            Some(links) => {
                let mut res = WeakCallbacks::with_capacity(links.len() + 1);
                for l in links.iter() {
                    if Weak::strong_count(l) != 0 {
                        res.push(l.clone());
                    }
                }
                res.push(link);
                Arc::new(res)
            }
        });
        Subscription::new(callback)
    }

    /// Returns a snapshot of callbacks subscribed to this observer at the moment when this method
    /// has been called. This snapshot can be iterated over to get access to individual callbacks
    /// and trigger them.
    pub fn callbacks(&self) -> Callbacks<F> {
        Callbacks::new(self)
    }
}

impl<F: Func> Default for Observer<F> {
    fn default() -> Self {
        Observer {
            inner: ArcSwapOption::from(None),
        }
    }
}

#[derive(Debug)]
pub struct Callbacks<F: Func> {
    inner: Guard<Option<Arc<WeakCallbacks<F>>>>,
    index: usize,
    should_cleanup: bool,
}

impl<F: Func> Callbacks<F> {
    fn new(o: &Observer<F>) -> Self {
        let inner = o.inner.load();
        Callbacks {
            inner,
            index: 0,
            should_cleanup: false,
        }
    }
}

impl<F: Func> Iterator for Callbacks<F> {
    type Item = Arc<F>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(callbacks) = self.inner.as_deref() {
            while callbacks.len() < self.index {
                let next = &callbacks[self.index];
                self.index += 1;
                if let Some(next) = next.upgrade() {
                    return Some(next);
                } else {
                    self.should_cleanup = true;
                }
            }
        }
        None
    }
}

impl<F> Drop for Callbacks<F> {
    fn drop(&mut self) {
        if self.should_cleanup {
            if let Some(callbacks) = self.inner.as_deref_mut() {
                let mut i = 0;
                while i < callbacks.len() {
                    let cb = &callbacks[i];
                    if Weak::strong_count(cb) == 0 {
                        callbacks.swap_remove(i);
                    } else {
                        i += 1;
                    }
                }
            }
        }
    }
}

pub type SharedRefObserver = Subscription<Arc<dyn Fn(&TransactionMut, &Event) -> () + 'static>>;

/// Subscription handle returned by [Observer::subscribe] methods, which will unsubscribe corresponding
/// callback when dropped.
///
/// If implicit callback unsubscribe on drop is undesired, this structure can be cast [into](Subscription::into)
/// [SubscriptionId] which is an identifier of the same subscription, which in turn must be used
/// manually via [Observer::unsubscribe] to perform usubscribe.
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct Subscription<F> {
    inner: Arc<F>,
}

impl<F> Subscription<F> {
    fn new(inner: Arc<F>) -> Self {
        Subscription { inner }
    }
}

impl<F> Into<SubscriptionId> for Subscription<F> {
    fn into(self) -> SubscriptionId {
        let ptr = Arc::as_ptr(&self.inner);
        ptr as SubscriptionId
    }
}

#[cfg(test)]
mod test {
    use crate::observer::Observer;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::thread::spawn;

    #[test]
    fn subscription() {
        let o: Observer<dyn Fn(u32) -> ()> = Observer::new();
        let s1_state = Arc::new(AtomicU32::new(0));
        let s2_state = Arc::new(AtomicU32::new(0));

        {
            let a = s1_state.clone();
            let b = s2_state.clone();

            let _s1 = o.subscribe(Arc::new(move |value| a.store(value, Ordering::Release)));
            let _s2 = o.subscribe(Arc::new(move |value| b.store(value * 2, Ordering::Release)));

            for fun in o.callbacks() {
                fun(1)
            }
            assert_eq!(s1_state.load(Ordering::Acquire), 1);
            assert_eq!(s2_state.load(Ordering::Acquire), 2);

            for fun in o.callbacks() {
                fun(2)
            }
            assert_eq!(s1_state.load(Ordering::Acquire), 2);
            assert_eq!(s2_state.load(Ordering::Acquire), 4);
        }

        // subscriptions were dropped, we don't expect updates to be propagated
        for fun in o.callbacks() {
            fun(3)
        }
        assert_eq!(s1_state.load(Ordering::Acquire), 2);
        assert_eq!(s2_state.load(Ordering::Acquire), 4);
    }

    #[test]
    fn multi_threading() {
        let o: Observer<Arc<dyn Fn(u32) -> ()>> = Observer::new();

        let s1_state = Arc::new(AtomicU32::new(0));
        let a = s1_state.clone();
        let sub1 = o.subscribe(Arc::new(move |value| a.store(value, Ordering::Release)));

        let s2_state = Arc::new(AtomicU32::new(0));
        let b = s2_state.clone();
        let sub2 = o.subscribe(Arc::new(move |value| b.store(value, Ordering::Release)));

        let handle = spawn(move || {
            for fun in o.callbacks() {
                fun(1)
            }
            drop(sub1);
            drop(sub2);
        });

        handle.join().unwrap();

        assert_eq!(s1_state.load(Ordering::Acquire), 1);
        assert_eq!(s2_state.load(Ordering::Acquire), 1);
    }

    #[test]
    fn multi_param() {
        struct Wrapper {
            observer: Observer<Arc<dyn Fn(&u32, &u32) -> ()>>,
        }
        let o = Wrapper {
            observer: Observer::new(),
        };
        let state = Rc::new(Cell::new(0));
        let s = state.clone();
        let _sub = o.observer.subscribe(Arc::new(move |a, b| {
            let cell = s.as_ref();
            cell.set(*a + *b);
        }));

        for fun in o.observer.callbacks() {
            fun(&1, &2)
        }
        assert_eq!(state.get(), 3);
    }
}
