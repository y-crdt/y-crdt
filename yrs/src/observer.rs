use crate::atomic::AtomicRef;
use std::fmt::{Debug, Formatter};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub type SubscriptionId = u32;

#[derive(Debug)]
pub struct Observer<F: Clone> {
    seq_nr: AtomicU32,
    state: Arc<AtomicRef<Inner<F>>>,
}

impl<F: Clone> Observer<F> {
    pub fn new() -> Self {
        Observer {
            seq_nr: AtomicU32::new(0),
            state: Arc::new(AtomicRef::new(Inner::default())),
        }
    }

    pub fn subscribe(&self, f: F) -> Subscription<F> {
        let subscription_id = self.seq_nr.fetch_add(1, Ordering::SeqCst);
        let handle = Handle::new(subscription_id, f);
        self.state.update(move |subs| {
            let mut subs = subs.cloned().unwrap_or_else(Inner::default);
            subs.insert(handle.clone());
            subs
        });
        Subscription::new(subscription_id, self.state.clone())
    }

    pub fn unsubscribe(&self, subscription_id: SubscriptionId) {
        self.state.update(move |s| {
            let mut s = s.cloned().unwrap_or_else(Inner::default);
            s.remove(subscription_id);
            s
        });
    }

    pub fn callbacks(&self) -> Callbacks<F> {
        Callbacks::new(self)
    }
}

impl<F: Clone> Default for Observer<F> {
    fn default() -> Self {
        Observer {
            seq_nr: AtomicU32::new(0),
            state: Arc::new(AtomicRef::default()),
        }
    }
}

#[derive(Debug)]
pub struct Callbacks<F: Clone> {
    inner: Option<Arc<Inner<F>>>,
    index: usize,
}

impl<F: Clone> Callbacks<F> {
    fn new(o: &Observer<F>) -> Self {
        let inner = o.state.get();
        Callbacks { inner, index: 0 }
    }
}

impl<F: Clone> Iterator for Callbacks<F> {
    type Item = F;

    fn next(&mut self) -> Option<Self::Item> {
        let inner = self.inner.as_ref()?;
        if self.index >= inner.handles.len() {
            None
        } else {
            let result = &inner.handles[self.index];
            self.index += 1;
            Some(result.callback.clone())
        }
    }
}

#[derive(Debug, Clone)]
pub struct Subscription<F: Clone> {
    subscription_id: SubscriptionId,
    observer: Arc<AtomicRef<Inner<F>>>,
}

impl<F: Clone> Subscription<F> {
    fn new(subscription_id: SubscriptionId, observer: Arc<AtomicRef<Inner<F>>>) -> Self {
        Subscription {
            subscription_id,
            observer,
        }
    }
}

impl<F: Clone> Into<SubscriptionId> for Subscription<F> {
    fn into(self) -> SubscriptionId {
        let subscription_id = self.subscription_id;
        std::mem::forget(self);
        subscription_id
    }
}

impl<F: Clone> Drop for Subscription<F> {
    fn drop(&mut self) {
        self.observer.update(|s| {
            let mut s = s.unwrap().clone();
            s.remove(self.subscription_id);
            s
        })
    }
}

struct Handle<F: Clone> {
    subscription_id: SubscriptionId,
    callback: F,
}

impl<F: Clone> Handle<F> {
    fn new(subscription_id: SubscriptionId, f: F) -> Self {
        Handle {
            subscription_id,
            callback: f,
        }
    }
}

impl<F: Clone> Clone for Handle<F> {
    fn clone(&self) -> Self {
        Handle {
            subscription_id: self.subscription_id,
            callback: self.callback.clone(),
        }
    }
}

impl<F: Clone> Debug for Handle<F> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Handle(#{})", self.subscription_id)
    }
}

#[derive(Debug)]
struct Inner<F: Clone> {
    handles: Vec<Handle<F>>,
}

impl<F: Clone> Inner<F> {
    fn insert(&mut self, handle: Handle<F>) {
        self.handles.push(handle);
    }

    fn remove(&mut self, subscription_id: SubscriptionId) {
        let mut i = 0;
        while i < self.handles.len() {
            if self.handles[i].subscription_id == subscription_id {
                break;
            }
            i += 1;
        }

        if i != self.handles.len() {
            self.handles.remove(i);
        }
    }
}

impl<F: Clone> Default for Inner<F> {
    fn default() -> Self {
        Inner {
            handles: Vec::default(),
        }
    }
}

impl<F: Clone> Clone for Inner<F> {
    fn clone(&self) -> Self {
        let handles = self.handles.clone();
        Inner { handles }
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
        let o: Observer<Arc<dyn Fn(u32) -> ()>> = Observer::new();
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
