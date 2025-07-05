use std::ops::{Deref, DerefMut};

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

#[cfg(feature = "sync")]
#[repr(transparent)]
#[derive(Debug)]
pub struct WrapMut<'a, S> {
    inner: parking_lot::MutexGuard<'a, S>,
}

#[cfg(feature = "sync")]
impl<'a, S> WrapMut<'a, S> {
    pub fn new(inner: parking_lot::MutexGuard<'a, S>) -> Self {
        WrapMut { inner: inner }
    }
}

#[cfg(feature = "sync")]
impl<'a, S> Deref for WrapMut<'a, S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(feature = "sync")]
impl<'a, S> DerefMut for WrapMut<'a, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[cfg(feature = "sync")]
pub type WrapRef<'a, S> = WrapMut<'a, S>;

#[cfg(not(feature = "sync"))]
#[repr(transparent)]
pub struct Wrap<S> {
    inner: std::rc::Rc<std::cell::RefCell<S>>,
}

#[cfg(not(feature = "sync"))]
#[repr(transparent)]
pub struct WrapMut<'a, S> {
    inner: std::cell::RefMut<'a, S>,
}

#[cfg(not(feature = "sync"))]
impl<'a, S> WrapMut<'a, S> {
    pub fn new(inner: std::cell::RefMut<'a, S>) -> Self {
        WrapMut { inner }
    }
}

#[cfg(not(feature = "sync"))]
impl<'a, S> Deref for WrapMut<'a, S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(not(feature = "sync"))]
impl<'a, S> DerefMut for WrapMut<'a, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[cfg(not(feature = "sync"))]
#[repr(transparent)]
pub struct WrapRef<'a, S> {
    inner: std::cell::Ref<'a, S>,
}

#[cfg(not(feature = "sync"))]
impl<'a, S> WrapRef<'a, S> {
    pub fn new(inner: std::cell::Ref<'a, S>) -> Self {
        WrapRef { inner }
    }
}

#[cfg(not(feature = "sync"))]
impl<'a, S> Deref for WrapRef<'a, S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
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

    pub fn borrow(&self) -> WrapRef<'_, S> {
        WrapRef::new(self.inner.try_lock().unwrap())
    }

    pub fn borrow_mut(&mut self) -> WrapMut<'_, S> {
        WrapMut::new(self.inner.try_lock().unwrap())
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

    pub fn borrow(&self) -> WrapRef<'_, S> {
        WrapRef::new(self.inner.borrow())
    }

    pub fn borrow_mut(&mut self) -> WrapMut<'_, S> {
        WrapMut::new(self.inner.borrow_mut())
    }

    pub fn downgrade(&self) -> WeakWrap<S> {
        std::rc::Rc::downgrade(&self.inner)
    }

    pub fn upgrade(weak: &WeakWrap<S>) -> Option<Self> {
        weak.upgrade().map(|inner| Wrap { inner })
    }
}
