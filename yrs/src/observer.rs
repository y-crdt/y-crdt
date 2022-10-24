use crate::atomic::AtomicRef;
use std::fmt::{Debug, Formatter};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub type SubscriptionId = u32;

#[derive(Debug, Default)]
pub struct Observer<T> {
    seq_nr: AtomicU32,
    state: Arc<AtomicRef<Inner<T>>>,
}

impl<T> Observer<T> {
    pub fn new() -> Self {
        Observer {
            seq_nr: AtomicU32::new(0),
            state: Arc::new(AtomicRef::new(Inner::default())),
        }
    }

    pub fn subscribe<F>(&self, f: F) -> Subscription<T>
    where
        F: Fn(&T) -> () + 'static,
    {
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

    pub fn publish(&self, args: &T) {
        if let Some(state) = self.state.get() {
            for sub in state.handles.iter() {
                (sub.callback)(args)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Subscription<T> {
    subscription_id: SubscriptionId,
    observer: Arc<AtomicRef<Inner<T>>>,
}

impl<T> Subscription<T> {
    fn new(subscription_id: SubscriptionId, observer: Arc<AtomicRef<Inner<T>>>) -> Self {
        Subscription {
            subscription_id,
            observer,
        }
    }
}

impl<T> Into<SubscriptionId> for Subscription<T> {
    fn into(self) -> SubscriptionId {
        let subscription_id = self.subscription_id;
        std::mem::forget(self);
        subscription_id
    }
}

impl<T> Drop for Subscription<T> {
    fn drop(&mut self) {
        self.observer.update(|s| {
            let mut s = s.unwrap().clone();
            s.remove(self.subscription_id);
            s
        })
    }
}

struct Handle<T> {
    subscription_id: SubscriptionId,
    callback: Arc<dyn Fn(&T) -> ()>,
}

impl<T> Handle<T> {
    fn new<F>(subscription_id: SubscriptionId, f: F) -> Self
    where
        F: Fn(&T) -> () + 'static,
    {
        Handle {
            subscription_id,
            callback: Arc::new(f),
        }
    }
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Handle {
            subscription_id: self.subscription_id,
            callback: self.callback.clone(),
        }
    }
}

impl<T> Debug for Handle<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Handle(#{})", self.subscription_id)
    }
}

#[derive(Debug)]
struct Inner<T> {
    handles: Vec<Handle<T>>,
}

impl<T> Inner<T> {
    fn insert(&mut self, handle: Handle<T>) {
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

impl<T> Default for Inner<T> {
    fn default() -> Self {
        Inner {
            handles: Vec::default(),
        }
    }
}

impl<T> Clone for Inner<T> {
    fn clone(&self) -> Self {
        let handles = self.handles.clone();
        Inner { handles }
    }
}

#[cfg(test)]
mod test {
    use crate::observer::Observer;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::thread::spawn;

    #[test]
    fn subscription() {
        let o: Observer<u32> = Observer::new();
        let s1_state = Arc::new(AtomicU32::new(0));
        let s2_state = Arc::new(AtomicU32::new(0));

        {
            let a = s1_state.clone();
            let b = s2_state.clone();

            let _s1 = o.subscribe(move |value| a.store(*value, Ordering::Release));
            let _s2 = o.subscribe(move |value| b.store(*value * 2, Ordering::Release));

            o.publish(&1);
            assert_eq!(s1_state.load(Ordering::Acquire), 1);
            assert_eq!(s2_state.load(Ordering::Acquire), 2);

            o.publish(&2);
            assert_eq!(s1_state.load(Ordering::Acquire), 2);
            assert_eq!(s2_state.load(Ordering::Acquire), 4);
        }

        // subscriptions were dropped, we don't expect updates to be propagated
        o.publish(&3);
        assert_eq!(s1_state.load(Ordering::Acquire), 2);
        assert_eq!(s2_state.load(Ordering::Acquire), 4);
    }

    #[test]
    fn multi_threading() {
        let o: Observer<u32> = Observer::new();

        let s1_state = Arc::new(AtomicU32::new(0));
        let a = s1_state.clone();
        let sub1 = o.subscribe(move |value| a.store(*value, Ordering::Release));

        let s2_state = Arc::new(AtomicU32::new(0));
        let b = s2_state.clone();
        let sub2 = o.subscribe(move |value| b.store(*value, Ordering::Release));

        let handle = spawn(move || {
            o.publish(&1);
            drop(sub1);
            drop(sub2);
        });

        handle.join().unwrap();

        assert_eq!(s1_state.load(Ordering::Acquire), 1);
        assert_eq!(s2_state.load(Ordering::Acquire), 1);
    }
}
