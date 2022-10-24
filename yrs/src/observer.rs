use crate::atomic::AtomicRef;
use std::fmt::{write, Debug, Formatter};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub type SubscriptionId = u32;

#[derive(Debug, Default)]
pub struct Observer<T> {
    seq_nr: AtomicU32,
    state: AtomicRef<Inner<T>>,
}

impl<T> Observer<T> {
    pub fn new() -> Self {
        Observer {
            seq_nr: AtomicU32::new(0),
            state: AtomicRef::new(Inner::default()),
        }
    }

    pub fn subscribe<F>(&self, f: F) -> Subscription<T>
    where
        F: Fn(&T) -> () + 'static,
    {
        let subscription_id = self.seq_nr.fetch_add(1, Ordering::SeqCst);
        let handle = Handle::new(subscription_id, f);
        self.state.update(move |subs| subs.insert(handle.clone()));
        Subscription::new(subscription_id, self.state.clone())
    }

    pub fn unsubscribe(&self, subscription_id: SubscriptionId) {
        self.state.update(move |s| s.remove(subscription_id));
    }

    pub fn publish(&self, args: &T) {
        let state = self.state.get();
        for sub in state.handles.iter() {
            (sub.callback)(args)
        }
    }
}

#[derive(Debug, Clone)]
pub struct Subscription<T> {
    subscription_id: SubscriptionId,
    observer: AtomicRef<Inner<T>>,
}

impl<T> Subscription<T> {
    fn new(subscription_id: SubscriptionId, observer: AtomicRef<Inner<T>>) -> Self {
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
        self.observer
            .update(|state| state.remove(self.subscription_id))
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

#[derive(Debug, Clone)]
struct Inner<T> {
    handles: Vec<Handle<T>>,
}

impl<T> Inner<T> {
    fn insert(&self, handle: Handle<T>) -> Self {
        let mut handles = self.handles.to_vec();
        handles.push(handle);
        Inner { handles }
    }

    fn remove(&self, subscription_id: SubscriptionId) -> Self {
        let mut i = 0;
        while i < self.handles.len() {
            if self.handles[i].subscription_id == subscription_id {
                break;
            }
            i += 1;
        }

        let mut handles = self.handles.to_vec();
        if i != self.handles.len() {
            handles.remove(i);
        }
        Inner { handles }
    }
}

impl<T> Default for Inner<T> {
    fn default() -> Self {
        Inner {
            handles: Vec::default(),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::observer::Observer;
    use crate::Doc;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn subscription() {
        let mut eh: Observer<u32> = Observer::new();
        let s1_state = Arc::new(AtomicU32::new(0));
        let s2_state = Arc::new(AtomicU32::new(0));

        {
            let a = s1_state.clone();
            let b = s2_state.clone();

            let _s1 = eh.subscribe(move |value| a.store(*value, Ordering::Release));
            let _s2 = eh.subscribe(move |value| b.store(*value * 2, Ordering::Release));

            eh.publish(&1);
            assert_eq!(s1_state.load(Ordering::Acquire), 1);
            assert_eq!(s2_state.load(Ordering::Acquire), 2);

            eh.publish(&2);
            assert_eq!(s1_state.load(Ordering::Acquire), 2);
            assert_eq!(s2_state.load(Ordering::Acquire), 4);
        }

        // subscriptions were dropped, we don't expect updates to be propagated
        eh.publish(&3);
        assert_eq!(s1_state.load(Ordering::Acquire), 2);
        assert_eq!(s2_state.load(Ordering::Acquire), 4);
    }
}
