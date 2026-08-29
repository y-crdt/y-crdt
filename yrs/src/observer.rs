use std::collections::HashMap;

use crate::Origin;

#[cfg(feature = "sync")]
type Shared<T> = std::sync::Arc<T>;
#[cfg(not(feature = "sync"))]
type Shared<T> = std::rc::Rc<T>;

#[cfg(feature = "sync")]
type SharedWeak<T> = std::sync::Weak<T>;
#[cfg(not(feature = "sync"))]
type SharedWeak<T> = std::rc::Weak<T>;

#[cfg(feature = "sync")]
type Lock<T> = std::sync::Mutex<T>;
#[cfg(not(feature = "sync"))]
type Lock<T> = std::cell::RefCell<T>;

#[cfg(feature = "sync")]
type Counter = std::sync::atomic::AtomicUsize;
#[cfg(not(feature = "sync"))]
type Counter = std::cell::Cell<usize>;

#[cfg(feature = "sync")]
trait Cancel: Send + Sync {
    fn cancel(&self);
}
#[cfg(not(feature = "sync"))]
trait Cancel {
    fn cancel(&self);
}

#[cfg(feature = "sync")]
type CancelWeak = SharedWeak<dyn Cancel + Send + Sync>;
#[cfg(not(feature = "sync"))]
type CancelWeak = SharedWeak<dyn Cancel>;

/// Subscription handle returned by [Observer::subscribe], which will unsubscribe the
/// corresponding callback when dropped.
///
/// Dropping a subscription guarantees that the callback will never start again. If the callback
/// is already running on another thread, its captures are released when that call finishes.
/// Blocking until an in-flight call completes would deadlock a callback that drops its own
/// subscription.
///
/// The callback destructor runs on the thread that drops this handle, except when the callback
/// is cancelled while it is running: then it runs on the dispatch thread after that call returns.
/// Other callbacks in the same [Observer::trigger] that have already been cancelled are skipped.
#[must_use = "dropping this subscription immediately unsubscribes the callback"]
pub struct Subscription(CancelWeak);

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(callback) = self.0.upgrade() {
            callback.cancel();
        }
    }
}

enum CallbackState<F> {
    Ready(F),
    Running,
    Cancelled,
}

struct Callback<F> {
    live: Shared<Counter>,
    state: Lock<CallbackState<F>>,
}

impl<F> Callback<F> {
    fn new(live: Shared<Counter>, callback: F) -> Shared<Self> {
        live_inc(&live);
        Shared::new(Callback {
            live,
            state: Lock::new(CallbackState::Ready(callback)),
        })
    }

    fn is_active(&self) -> bool {
        !matches!(*lock(&self.state), CallbackState::Cancelled)
    }

    /// Cancels the callback and releases it immediately unless it is currently running.
    /// The live counter is decremented exactly once, on the transition to [CallbackState::Cancelled].
    fn release(&self) {
        let callback = {
            let mut state = lock(&self.state);
            match std::mem::replace(&mut *state, CallbackState::Cancelled) {
                CallbackState::Ready(callback) => {
                    live_dec(&self.live);
                    Some(callback)
                }
                CallbackState::Running => {
                    live_dec(&self.live);
                    None
                }
                CallbackState::Cancelled => None,
            }
        };
        // Drop F outside the slot lock. User destructors may cancel other slots
        // and mutate `live` while this method is running; that is safe because
        // `release` never touches `slots` or `named`. Holding the lock across this
        // drop would deadlock a Mutex / panic a RefCell if F owns its Subscription.
        drop(callback);
    }

    fn call<E: FnMut(&mut F)>(&self, each: &mut E) {
        let callback = {
            let mut state = lock(&self.state);
            match std::mem::replace(&mut *state, CallbackState::Running) {
                CallbackState::Ready(callback) => Some(callback),
                CallbackState::Running => None,
                CallbackState::Cancelled => {
                    *state = CallbackState::Cancelled;
                    None
                }
            }
        };

        if let Some(callback) = callback {
            let mut dispatch = Dispatch {
                owner: self,
                callback: Some(callback),
            };
            each(dispatch.callback.as_mut().unwrap());
        }
    }
}

impl<F> Drop for Callback<F> {
    fn drop(&mut self) {
        // Observer teardown and a failed `Shared::new` skip an explicit `release`.
        // `release` is idempotent if the slot was already cancelled.
        self.release();
    }
}

#[cfg(feature = "sync")]
impl<F: Send> Cancel for Callback<F> {
    fn cancel(&self) {
        self.release();
    }
}

#[cfg(not(feature = "sync"))]
impl<F> Cancel for Callback<F> {
    fn cancel(&self) {
        self.release();
    }
}

/// Restores a callback after dispatch, or releases it if its subscription was cancelled.
struct Dispatch<'a, F> {
    owner: &'a Callback<F>,
    callback: Option<F>,
}

impl<F> Drop for Dispatch<'_, F> {
    fn drop(&mut self) {
        if let Some(callback) = self.callback.take() {
            let callback = {
                let mut state = lock(&self.owner.state);
                if matches!(*state, CallbackState::Running) {
                    *state = CallbackState::Ready(callback);
                    None
                } else {
                    Some(callback)
                }
            };
            // Same as `Callback::release`: drop F after releasing the slot lock.
            drop(callback);
        }
    }
}

/// Data structure used to handle publish/subscribe callbacks of a specific type.
///
/// Dropping a [Subscription] releases the callback immediately. Empty slot entries may remain
/// until the next [Observer::subscribe], [Observer::subscribe_with], [Observer::unsubscribe],
/// or [Observer::trigger].
pub struct Observer<F> {
    slots: Vec<Shared<Callback<F>>>,
    named: HashMap<Origin, Shared<Callback<F>>>,
    live: Shared<Counter>,
}

impl<F> Observer<F> {
    /// Creates a new [Observer] with no active callbacks.
    pub fn new() -> Self {
        Observer {
            slots: Vec::new(),
            named: HashMap::new(),
            live: Shared::new(Counter::new(0)),
        }
    }

    /// Returns `true` if this observer has any active subscribers.
    ///
    /// This becomes `false` as soon as the last [Subscription] is dropped, without waiting for
    /// a later [Observer::trigger].
    pub fn has_subscribers(&self) -> bool {
        live_load(&self.live) > 0
    }

    /// Subscribes a callback to this observer with an auto-generated key.
    /// Returns a [Subscription] which will unsubscribe the callback when dropped.
    #[cfg(feature = "sync")]
    pub fn subscribe(&mut self, callback: F) -> Subscription
    where
        F: Send + 'static,
    {
        Self::subscription(self.insert_slot(callback))
    }

    /// Subscribes a callback to this observer with an auto-generated key.
    /// Returns a [Subscription] which will unsubscribe the callback when dropped.
    #[cfg(not(feature = "sync"))]
    pub fn subscribe(&mut self, callback: F) -> Subscription
    where
        F: 'static,
    {
        Self::subscription(self.insert_slot(callback))
    }

    /// Subscribes a callback with a specific key. If a callback with the same key already
    /// exists, it will be replaced.
    pub fn subscribe_with(&mut self, id: impl Into<Origin>, callback: F) {
        let callback = self.insert_slot(callback);
        let previous = self.named.insert(id.into(), callback);
        if let Some(previous) = previous {
            previous.release();
            self.compact();
        }
    }

    /// Removes a callback by its key. Returns `true` if the callback was found and removed.
    pub fn unsubscribe(&mut self, id: &Origin) -> bool {
        self.assert_named_active();
        let callback = self.named.remove(id);
        if let Some(callback) = callback {
            callback.release();
            self.compact();
            true
        } else {
            false
        }
    }

    /// Calls `each` for every registered callback, giving mutable access to the callback.
    ///
    /// Callbacks cancelled before they are reached, including ones cancelled by an earlier
    /// callback in this dispatch, are skipped.
    pub fn trigger<E: FnMut(&mut F)>(&mut self, mut each: E) {
        self.compact();
        // Holding `&Callback` across user code is sound because neither collection
        // has interior mutability, and every route to `&mut Observer` is already
        // gated by a lock held during dispatch. A future `&self` trigger would
        // turn this walk into aliasing UB.
        for callback in &self.slots {
            callback.call(&mut each);
        }
    }

    fn insert_slot(&mut self, callback: F) -> Shared<Callback<F>> {
        self.assert_named_active();
        self.compact();
        let callback = Callback::new(self.live.clone(), callback);
        self.slots.push(Shared::clone(&callback));
        callback
    }

    fn subscription(callback: Shared<Callback<F>>) -> Subscription
    where
        Callback<F>: Cancel + 'static,
    {
        let weak = Shared::downgrade(&callback);
        let weak: CancelWeak = weak;
        Subscription(weak)
    }

    fn compact(&mut self) {
        if live_load(&self.live) != self.slots.len() {
            // Tombstones already had their payload taken at cancellation, so
            // retain cannot run user destructors. Anonymous drops still leave
            // shells here until the next compacting `&mut self` method.
            self.slots.retain(|callback| callback.is_active());
        }
    }

    fn assert_named_active(&self) {
        debug_assert!(self.named.values().all(|callback| callback.is_active()));
    }
}

impl<F> Default for Observer<F> {
    fn default() -> Self {
        Self::new()
    }
}

fn live_inc(live: &Counter) {
    #[cfg(feature = "sync")]
    live.fetch_add(1, std::sync::atomic::Ordering::Release);
    #[cfg(not(feature = "sync"))]
    live.set(live.get() + 1);
}

fn live_dec(live: &Counter) {
    debug_assert!(live_load(live) > 0);
    #[cfg(feature = "sync")]
    live.fetch_sub(1, std::sync::atomic::Ordering::Release);
    #[cfg(not(feature = "sync"))]
    live.set(live.get() - 1);
}

fn live_load(live: &Counter) -> usize {
    #[cfg(feature = "sync")]
    {
        live.load(std::sync::atomic::Ordering::Acquire)
    }
    #[cfg(not(feature = "sync"))]
    {
        live.get()
    }
}

#[cfg(feature = "sync")]
fn lock<T>(lock: &Lock<T>) -> std::sync::MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(not(feature = "sync"))]
fn lock<T>(lock: &Lock<T>) -> std::cell::RefMut<'_, T> {
    lock.borrow_mut()
}

#[cfg(test)]
mod test {
    use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::observer::{Observer, Subscription};
    use crate::Origin;

    #[cfg(feature = "sync")]
    type TestFn = Box<dyn FnMut() + Send + Sync>;
    #[cfg(not(feature = "sync"))]
    type TestFn = Box<dyn FnMut()>;

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

        // subscriptions were dropped, releasing their callbacks
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
        assert!(!o.has_subscribers());
    }

    #[test]
    fn subscribe_with_has_subscribers() {
        let mut o: Observer<TestFn> = Observer::new();
        assert!(!o.has_subscribers());

        o.subscribe_with(1usize, Box::new(|| {}));
        assert!(o.has_subscribers());

        assert!(o.unsubscribe(&Origin::from(1usize)));
        assert!(!o.has_subscribers());
    }

    #[test]
    fn dropping_subscription_releases_callback() {
        let value = Arc::new(());
        let weak = Arc::downgrade(&value);
        let mut observer: Observer<TestFn> = Observer::new();
        let subscription = observer.subscribe(Box::new({
            let value = value.clone();
            move || {
                let _ = &value;
            }
        }));

        drop(value);
        assert!(weak.upgrade().is_some());

        drop(subscription);
        assert!(
            weak.upgrade().is_none(),
            "dropping Subscription retained its callback"
        );
        assert!(!observer.has_subscribers());
    }

    #[test]
    fn dropping_subscription_during_callback_is_safe() {
        let value = Arc::new(());
        let weak = Arc::downgrade(&value);
        let subscription = Arc::new(Mutex::new(None::<Subscription>));
        let mut observer: Observer<TestFn> = Observer::new();
        let subscription_ref = subscription.clone();
        let sub = observer.subscribe(Box::new({
            let value = value.clone();
            move || {
                let _ = &value;
                subscription_ref.lock().unwrap().take();
            }
        }));
        *subscription.lock().unwrap() = Some(sub);
        drop(value);

        observer.trigger(|callback| callback());

        assert!(!observer.has_subscribers());
        assert!(
            weak.upgrade().is_none(),
            "Subscription dropped during its own callback retained it"
        );

        observer.trigger(|callback| callback());
    }

    #[test]
    fn dropping_subscription_during_dispatch_skips_callback() {
        let subscriptions = Arc::new(Mutex::new(vec![None::<Subscription>, None]));
        let calls = Arc::new(AtomicU32::new(0));
        let mut observer: Observer<TestFn> = Observer::new();

        for id in 0..2 {
            let subscription = observer.subscribe(Box::new({
                let subscriptions = subscriptions.clone();
                let calls = calls.clone();
                move || {
                    subscriptions.lock().unwrap()[1 - id].take();
                    calls.fetch_add(1, Ordering::SeqCst);
                }
            }));
            subscriptions.lock().unwrap()[id] = Some(subscription);
        }

        observer.trigger(|callback| callback());

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "callback cancelled during dispatch was still called"
        );
    }

    #[cfg(panic = "unwind")]
    #[test]
    fn panicking_callback_stays_registered() {
        let calls = Arc::new(AtomicU32::new(0));
        let mut observer: Observer<TestFn> = Observer::new();
        let _subscription = observer.subscribe(Box::new({
            let calls = calls.clone();
            move || {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    panic!("callback failed");
                }
            }
        }));

        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            observer.trigger(|callback| callback());
        }));
        std::panic::set_hook(hook);
        assert!(panicked.is_err());

        assert!(observer.has_subscribers());
        observer.trigger(|callback| callback());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
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

    #[test]
    fn subscribe_with_replace_releases_old_callback() {
        let counter = Arc::new(AtomicI32::new(0));
        let mut o: Observer<DropCounter> = Observer::new();
        o.subscribe_with(1usize, DropCounter::new(counter.clone()));
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        o.subscribe_with(1usize, DropCounter::new(counter.clone()));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(o.has_subscribers());
        assert_eq!(o.slots.len(), 1, "replace left a cancelled slot");

        assert!(o.unsubscribe(&Origin::from(1usize)));
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert!(!o.has_subscribers());
        assert!(o.slots.is_empty(), "unsubscribe left cancelled slots");
    }

    #[test]
    fn subscribe_drop_then_subscribe_with_still_fires() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut o: Observer<TestFn> = Observer::new();
        let sub = o.subscribe(Box::new(|| panic!("dropped subscription was called")));
        drop(sub);

        o.subscribe_with(1usize, Box::new(move || tx.send(()).unwrap()));
        o.trigger(|callback| callback());
        assert_eq!(rx.try_recv(), Ok(()));
    }

    #[test]
    fn dropping_observer_releases_self_owned_handle() {
        let value = Arc::new(());
        let weak = Arc::downgrade(&value);
        let holder = Arc::new(Mutex::new(None::<Subscription>));
        let mut observer: Observer<TestFn> = Observer::new();
        let sub = observer.subscribe(Box::new({
            let value = value.clone();
            let holder = holder.clone();
            move || {
                let _ = (&value, &holder);
            }
        }));
        *holder.lock().unwrap() = Some(sub);
        drop(value);
        drop(holder);
        drop(observer);
        assert!(
            weak.upgrade().is_none(),
            "observer drop retained a self-owned callback"
        );
    }

    #[test]
    fn dropping_observer_releases_mutual_handle_cycle() {
        let a_value = Arc::new(());
        let b_value = Arc::new(());
        let a_weak = Arc::downgrade(&a_value);
        let b_weak = Arc::downgrade(&b_value);
        let holder_a = Arc::new(Mutex::new(None::<Subscription>));
        let holder_b = Arc::new(Mutex::new(None::<Subscription>));
        let mut observer: Observer<TestFn> = Observer::new();

        let sub_a = observer.subscribe(Box::new({
            let a_value = a_value.clone();
            let holder_b = holder_b.clone();
            move || {
                let _ = (&a_value, &holder_b);
            }
        }));
        let sub_b = observer.subscribe(Box::new({
            let b_value = b_value.clone();
            let holder_a = holder_a.clone();
            move || {
                let _ = (&b_value, &holder_a);
            }
        }));
        *holder_a.lock().unwrap() = Some(sub_a);
        *holder_b.lock().unwrap() = Some(sub_b);
        drop(a_value);
        drop(b_value);
        drop(holder_a);
        drop(holder_b);
        drop(observer);
        assert!(
            a_weak.upgrade().is_none() && b_weak.upgrade().is_none(),
            "observer drop retained a mutual-handle cycle"
        );
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
            assert_eq!(counter.load(Ordering::SeqCst), 0);
            assert!(
                o.slots.len() <= 1,
                "subscribe/drop left {} tombstones",
                o.slots.len()
            );
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
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        o.trigger(|_| {});
        assert!(
            o.slots.is_empty(),
            "trigger left {} cancelled slots",
            o.slots.len()
        );
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
            assert!(
                o.slots.is_empty(),
                "unsubscribe left {} cancelled slots",
                o.slots.len()
            );
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
        assert!(
            o.slots.is_empty(),
            "unsubscribe left {} cancelled slots",
            o.slots.len()
        );
    }
}
