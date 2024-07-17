use std::sync::{Arc, Weak};

use arc_swap::{ArcSwapOption, AsRaw, Guard};

use crate::Origin;

/// Data structure used to handle publish/subscribe callbacks of specific type. Observers perform
/// subscriber changes in thread-safe manner, using atomic hardware intrinsics.
pub struct Observer<F> {
    inner: ArcSwapOption<Inner<F>>,
}

impl<F> Observer<F>
where
    F: 'static,
{
    /// Creates a new [Observer] with no active callbacks.
    pub fn new() -> Self {
        Observer {
            inner: ArcSwapOption::new(None),
        }
    }

    pub fn has_subscribers(&self) -> bool {
        if let Some(inner) = &*self.inner.load() {
            inner.head.load().is_some()
        } else {
            false
        }
    }

    /// Cleanup already released subscriptions. Whenever a [Subscription] is dropped, the callback is released. However,
    /// the weak reference to callback may still be kept around until it becomes touched by operations such as
    /// [Observer::subscribe] or [Observer::callbacks].
    ///
    /// This method allows to perform stale callback cleanup without waiting for callbacks to be visited.
    pub fn clean(&self) {
        self.inner.swap(None);
    }

    fn inner(&self) -> Arc<Inner<F>> {
        let cur = self.inner.load_full();
        match cur {
            Some(inner) => inner,
            None => {
                // inner was not initialized yet, we need to create a new one
                let inner = Arc::new(Inner {
                    head: ArcSwapOption::new(None),
                });
                let old: Option<Arc<Inner<F>>> = None;
                let prev = self.inner.compare_and_swap(&old, Some(inner.clone()));
                // there's a slight possibility that inner was initialized twice, in that case
                // return first swapped inner
                Guard::into_inner(prev).unwrap_or(inner)
            }
        }
    }

    fn remove(mut prev: Arc<Node<F>>, id: &Origin) -> bool {
        while let Some(next) = prev.next.load_full() {
            if &next.uid == id {
                prev.next.store(next.next.load_full());
                return true;
            }
            prev = next;
        }
        false
    }

    pub fn unsubscribe(&self, id: &Origin) -> bool {
        if let Some(inner) = &*self.inner.load() {
            inner.remove(id)
        } else {
            false
        }
    }

    /// Returns a snapshot of callbacks subscribed to this observer at the moment when this method
    /// has been called. This snapshot can be iterated over to get access to individual callbacks
    /// and trigger them.
    pub fn trigger<E>(&self, mut each: E)
    where
        E: FnMut(&F),
    {
        if let Some(inner) = &*self.inner.load() {
            let mut next = inner.head.load();
            while let Some(node) = &*next {
                each(&node.callback);
                next = node.next.load();
            }
        }
    }

    /// Subscribes a callback parameter to a current [Observer].
    /// Returns a subscription object which - when dropped - will unsubscribe current callback.
    /// If the `id` was already present in the observer, current callback will be ignored.
    pub fn subscribe_with(&self, id: Origin, callback: F) {
        let inner = self.inner();
        let mut node = Arc::new(Node::new(id.clone(), callback));
        let cur = inner.head.load();
        let head = loop {
            {
                // update new node next pointer to point to current head
                // it's safe to unwrap, since until current node is successfully inserted
                // there will be no more that a single Arc reference to it
                let n = Arc::get_mut(&mut node).unwrap();
                n.next.store(cur.clone());
            }

            let prev = inner.head.compare_and_swap(&*cur, Some(node.clone()));
            let swapped = std::ptr::eq(prev.as_raw(), cur.as_raw());
            if swapped {
                // we successfully swapped the head, we can exit the loop
                break node;
            }
        };
        // remove all previous nodes that share the same ID
        Self::remove(head.clone(), &id);
    }
}

#[cfg(feature = "sync")]
impl<F> Observer<F>
where
    F: Send + Sync + 'static,
{
    pub fn subscribe(&self, callback: F) -> Subscription {
        let mut rng = fastrand::Rng::new();
        let id = rng.usize(0..usize::MAX);
        let origin = Origin::from(id);
        self.subscribe_with(origin.clone(), callback);
        Arc::new(Cancel {
            id: origin,
            inner: Arc::downgrade(&self.inner()),
        })
    }
}

#[cfg(not(feature = "sync"))]
impl<F> Observer<F>
where
    F: 'static,
{
    pub fn subscribe(&self, callback: F) -> Subscription {
        let mut rng = fastrand::Rng::new();
        let id = rng.usize(0..usize::MAX);
        let origin = Origin::from(id);
        self.subscribe_with(origin.clone(), callback);
        Arc::new(Cancel {
            id: origin,
            inner: Arc::downgrade(&self.inner()),
        })
    }
}

#[cfg(feature = "sync")]
impl<F> Default for Observer<F>
where
    F: Send + Sync + 'static,
{
    fn default() -> Self {
        Observer::new()
    }
}

#[cfg(not(feature = "sync"))]
impl<F> Default for Observer<F>
where
    F: 'static,
{
    fn default() -> Self {
        Observer::new()
    }
}

struct Inner<F> {
    head: ArcSwapOption<Node<F>>,
}

impl<F> Inner<F>
where
    F: 'static,
{
    fn remove(&self, id: &Origin) -> bool {
        while let Some(head) = self.head.load_full() {
            if &head.uid == id {
                // the element to remove is the head of the list
                // we need to swap head pointer of self to the next element
                let next = head.next.load_full();
                let prev = self.head.compare_and_swap(&head, next);
                if !std::ptr::eq(prev.as_raw(), Arc::as_ptr(&head)) {
                    // head changed, retry
                    continue;
                } else {
                    return true;
                }
            } else {
                // the element to remove is somewhere in the middle of the list
                // we need to find it and repoint its predecessor's next pointer
                // to its successor
                return Observer::remove(head.clone(), id);
            }
        }
        false
    }
}

struct Node<T> {
    uid: Origin,
    callback: T,
    next: ArcSwapOption<Node<T>>,
}

impl<F> Node<F> {
    fn new(uid: Origin, callback: F) -> Self {
        Node {
            uid,
            callback,
            next: Default::default(),
        }
    }
}

#[cfg(feature = "sync")]
struct Cancel<F>
where
    F: Send + Sync + 'static,
{
    id: Origin,
    inner: Weak<Inner<F>>,
}

#[cfg(feature = "sync")]
impl<F> Drop for Cancel<F>
where
    F: Send + Sync + 'static,
{
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            inner.remove(&self.id);
        }
    }
}

#[cfg(not(feature = "sync"))]
struct Cancel<F>
where
    F: 'static,
{
    id: Origin,
    inner: Weak<Inner<F>>,
}

#[cfg(not(feature = "sync"))]
impl<F> Drop for crate::observer::Cancel<F>
where
    F: 'static,
{
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            inner.remove(&self.id);
        }
    }
}

/// Subscription handle returned by [Observer::subscribe] methods, which will unsubscribe corresponding
/// callback when dropped.
///
/// If you need to send the Subscription handle to another thread, you may wish to enable
/// the `sync` feature such that Subscription implements `Send+Sync`.
#[cfg(feature = "sync")]
pub type Subscription = Arc<dyn Drop + Send + Sync + 'static>;

/// Subscription handle returned by [Observer::subscribe] methods, which will unsubscribe corresponding
/// callback when dropped.
///
/// If you need to send the Subscription handle to another thread, you may wish to enable
/// the `sync` feature such that Subscription implements `Send+Sync`.
#[cfg(not(feature = "sync"))]
pub type Subscription = Arc<dyn Drop + 'static>;

#[cfg(test)]
mod test {
    use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
    use std::sync::Arc;

    use crate::observer::Observer;

    #[test]
    fn subscription() {
        let o: Observer<Box<dyn Fn(&u32) + Send + Sync + 'static>> = Observer::new();
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

        // subscriptions were dropped, we don't expect updates to be propagated

        o.trigger(|fun| fun(&3));
        assert_eq!(s1_state.load(Ordering::Acquire), 2);
        assert_eq!(s2_state.load(Ordering::Acquire), 4);
    }

    #[test]
    fn subscribers_predicate() {
        let o: Observer<Box<dyn Fn(&u32) + Send + Sync + 'static>> = Observer::new();
        assert!(!o.has_subscribers());

        let _sub = o.subscribe(Box::new(move |_| {}));
        assert!(o.has_subscribers());

        drop(_sub);
        o.clean();

        assert!(!o.has_subscribers());
    }

    #[test]
    #[cfg(feature = "sync")]
    fn multi_threading() {
        let o: Observer<Box<dyn Fn(u32) + Send + Sync + 'static>> = Observer::new();

        let s1_state = Arc::new(AtomicU32::new(0));
        let a = s1_state.clone();
        let sub1 = o.subscribe(Box::new(move |v| a.store(v, Ordering::Release)));

        let s2_state = Arc::new(AtomicU32::new(0));
        let b = s2_state.clone();
        let sub2 = o.subscribe(Box::new(move |v| b.store(v, Ordering::Release)));

        let handle = std::thread::spawn(move || {
            o.trigger(|fun| fun(1));
            drop(sub1);
            drop(sub2);
        });

        handle.join().unwrap();

        assert_eq!(s1_state.load(Ordering::Acquire), 1);
        assert_eq!(s2_state.load(Ordering::Acquire), 1);
    }

    #[test]
    fn subscribe_with_replaced_old_callback() {
        let (tx, rx) = std::sync::mpsc::channel();
        let o: Observer<Box<dyn Fn(u32) + Send + Sync + 'static>> = Observer::new();
        let ta = tx.clone();
        let _a = o.subscribe_with(
            123.into(),
            Box::new(move |i| ta.send(format!("a-{i}")).unwrap()),
        );
        o.trigger(|fun| fun(1));
        assert_eq!(rx.try_recv().unwrap(), "a-1");

        // override the callback with the same key
        let _b = o.subscribe_with(
            123.into(),
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
        let o: Observer<DropCounter> = Observer::new();
        for _ in 0..100 {
            assert_eq!(counter.load(Ordering::SeqCst), 0);
            let _sub = o.subscribe(DropCounter::new(counter.clone()));
            assert_eq!(counter.load(Ordering::SeqCst), 1);
            // drop subscription
        }
    }

    #[test]
    fn drop_subscription2() {
        let counter = Arc::new(AtomicI32::new(0));
        let o: Observer<DropCounter> = Observer::new();
        let mut subscriptions = Vec::new();
        for _ in 0..100 {
            let sub = o.subscribe(DropCounter::new(counter.clone()));
            subscriptions.push(sub);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 100);
        drop(subscriptions);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn unsubscribe() {
        let counter = Arc::new(AtomicI32::new(0));
        let o: Observer<DropCounter> = Observer::new();
        for i in 0..100 {
            assert_eq!(counter.load(Ordering::SeqCst), 0);

            o.subscribe_with(i.into(), DropCounter::new(counter.clone()));

            assert_eq!(counter.load(Ordering::SeqCst), 1);

            let unsubscribed = o.unsubscribe(&i.into());
            assert!(unsubscribed, "unsubscribe failed for {}", i);
        }
    }

    #[test]
    fn unsubscribe2() {
        let counter = Arc::new(AtomicI32::new(0));
        let o: Observer<DropCounter> = Observer::new();
        for i in 0..100 {
            o.subscribe_with(i.into(), DropCounter::new(counter.clone()));
        }

        assert_eq!(counter.load(Ordering::SeqCst), 100);

        for i in 0..100 {
            let unsubscribed = o.unsubscribe(&i.into());
            assert!(unsubscribed, "unsubscribe failed for {}", i);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn clean() {
        let counter = Arc::new(AtomicI32::new(0));
        let o: Observer<DropCounter> = Observer::new();
        let mut subscriptions = Vec::new();
        for _ in 0..100 {
            let sub = o.subscribe(DropCounter::new(counter.clone()));
            subscriptions.push(sub);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 100);
        o.clean();
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
}
