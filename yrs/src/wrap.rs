///! [Wrap] is a container type for shared state. It's implementation depends on the `sync` feature
/// flag in a way that uses a `parking_lot::Mutex` for synchronization when the feature is enabled,
/// and a `std::cell::RefCell` when it is not. This allows for both synchronous and asynchronous
/// usage of the shared state.

#[cfg(feature = "sync")]
#[repr(transparent)]
#[derive(Debug)]
pub struct Wrap<S> {
    inner: std::sync::Arc<parking_lot::Mutex<S>>,
}

#[cfg(not(feature = "sync"))]
#[repr(transparent)]
pub struct Wrap<S> {
    inner: std::rc::Rc<std::cell::RefCell<S>>,
}

#[cfg(feature = "sync")]
pub type WeakWrap<S> = std::sync::Weak<parking_lot::Mutex<S>>;

#[cfg(not(feature = "sync"))]
pub type WeakWrap<S> = std::rc::Weak<std::cell::RefCell<S>>;

#[cfg(feature = "sync")]
impl<S> Wrap<S> {
    pub fn new(inner: S) -> Self {
        Wrap {
            inner: std::sync::Arc::new(parking_lot::Mutex::new(inner)),
        }
    }

    pub fn borrow(&self) -> parking_lot::MutexGuard<'_, S> {
        self.inner.try_lock().unwrap()
    }

    pub fn borrow_mut(&mut self) -> parking_lot::MutexGuard<'_, S> {
        self.inner.try_lock().unwrap()
    }

    pub fn downgrade(&self) -> WeakWrap<S> {
        std::sync::Arc::downgrade(&self.inner)
    }

    pub fn upgrade(weak: &WeakWrap<S>) -> Option<Self> {
        weak.upgrade().map(|inner| Wrap { inner })
    }
}

impl<S> From<S> for Wrap<S> {
    fn from(inner: S) -> Self {
        Wrap::new(inner)
    }
}

impl<S> Clone for Wrap<S> {
    fn clone(&self) -> Self {
        Wrap {
            inner: self.inner.clone(),
        }
    }
}

#[cfg(not(feature = "sync"))]
impl<S> Wrap<S> {
    pub fn new(inner: S) -> Self {
        Wrap {
            inner: std::rc::Rc::new(std::cell::RefCell::new(inner)),
        }
    }

    pub fn borrow(&self) -> std::cell::Ref<'_, S> {
        self.inner.borrow()
    }

    pub fn borrow_mut(&mut self) -> std::cell::RefMut<'_, S> {
        self.inner.borrow_mut()
    }

    pub fn downgrade(&self) -> WeakWrap<S> {
        std::rc::Rc::downgrade(&self.inner)
    }

    pub fn upgrade(weak: &WeakWrap<S>) -> Option<Self> {
        weak.upgrade().map(|inner| Wrap { inner })
    }
}
