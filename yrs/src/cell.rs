use std::ops::{Deref, DerefMut};

///! [Cell] is a container type for shared state. It's implementation depends on the `sync` feature
/// flag in a way that uses a `parking_lot::Mutex` for synchronization when the feature is enabled,
/// and a `std::cell::RefCell` when it is not. This allows for both synchronous and asynchronous
/// usage of the shared state.

#[cfg(feature = "sync")]
#[repr(transparent)]
#[derive(Debug)]
pub struct Cell<S> {
    inner: std::sync::Arc<parking_lot::RwLock<S>>,
}

#[cfg(feature = "sync")]
impl<S> PartialEq for Cell<S> {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.inner, &other.inner)
    }
}

#[cfg(feature = "sync")]
#[repr(transparent)]
#[derive(Debug)]
pub struct CellMut<'a, S> {
    inner: parking_lot::RwLockWriteGuard<'a, S>,
}

#[cfg(feature = "sync")]
impl<'a, S> CellMut<'a, S> {
    pub fn new(inner: parking_lot::RwLockWriteGuard<'a, S>) -> Self {
        CellMut { inner: inner }
    }
}

#[cfg(feature = "sync")]
impl<'a, S> Deref for CellMut<'a, S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(feature = "sync")]
impl<'a, S> DerefMut for CellMut<'a, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[cfg(feature = "sync")]
pub struct CellRef<'a, S> {
    inner: parking_lot::RwLockReadGuard<'a, S>,
}

#[cfg(feature = "sync")]
impl<'a, S> CellRef<'a, S> {
    pub fn new(inner: parking_lot::RwLockReadGuard<'a, S>) -> Self {
        CellRef { inner: inner }
    }
}

#[cfg(feature = "sync")]
impl<'a, S> Deref for CellRef<'a, S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(not(feature = "sync"))]
#[repr(transparent)]
#[derive(Debug, PartialEq)]
pub struct Cell<S> {
    inner: std::rc::Rc<std::cell::RefCell<S>>,
}

#[cfg(not(feature = "sync"))]
#[repr(transparent)]
pub struct CellMut<'a, S> {
    inner: std::cell::RefMut<'a, S>,
}

#[cfg(not(feature = "sync"))]
impl<'a, S> CellMut<'a, S> {
    pub fn new(inner: std::cell::RefMut<'a, S>) -> Self {
        CellMut { inner }
    }
}

#[cfg(not(feature = "sync"))]
impl<'a, S> Deref for CellMut<'a, S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(not(feature = "sync"))]
impl<'a, S> DerefMut for CellMut<'a, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[cfg(not(feature = "sync"))]
#[repr(transparent)]
pub struct CellRef<'a, S> {
    inner: std::cell::Ref<'a, S>,
}

#[cfg(not(feature = "sync"))]
impl<'a, S> CellRef<'a, S> {
    pub fn new(inner: std::cell::Ref<'a, S>) -> Self {
        CellRef { inner }
    }
}

#[cfg(not(feature = "sync"))]
impl<'a, S> Deref for CellRef<'a, S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(feature = "sync")]
pub type WeakCell<S> = std::sync::Weak<parking_lot::RwLock<S>>;

#[cfg(not(feature = "sync"))]
pub type WeakCell<S> = std::rc::Weak<std::cell::RefCell<S>>;

#[cfg(feature = "sync")]
impl<S> Cell<S> {
    pub fn new(inner: S) -> Self {
        Cell {
            inner: std::sync::Arc::new(parking_lot::RwLock::new(inner)),
        }
    }

    pub fn borrow(&self) -> CellRef<'_, S> {
        CellRef::new(self.inner.read())
    }

    pub fn borrow_mut(&mut self) -> CellMut<'_, S> {
        CellMut::new(self.inner.write())
    }

    pub fn downgrade(&self) -> WeakCell<S> {
        std::sync::Arc::downgrade(&self.inner)
    }

    pub fn upgrade(weak: &WeakCell<S>) -> Option<Self> {
        weak.upgrade().map(|inner| Cell { inner })
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl<S> From<S> for Cell<S> {
    fn from(inner: S) -> Self {
        Cell::new(inner)
    }
}

impl<S> Clone for Cell<S> {
    fn clone(&self) -> Self {
        Cell {
            inner: self.inner.clone(),
        }
    }
}

#[cfg(not(feature = "sync"))]
impl<S> Cell<S> {
    pub fn new(inner: S) -> Self {
        Cell {
            inner: std::rc::Rc::new(std::cell::RefCell::new(inner)),
        }
    }

    pub fn borrow(&self) -> CellRef<'_, S> {
        CellRef::new(self.inner.borrow())
    }

    pub fn borrow_mut(&mut self) -> CellMut<'_, S> {
        CellMut::new(self.inner.borrow_mut())
    }

    pub fn downgrade(&self) -> WeakCell<S> {
        std::rc::Rc::downgrade(&self.inner)
    }

    pub fn upgrade(weak: &WeakCell<S>) -> Option<Self> {
        weak.upgrade().map(|inner| Cell { inner })
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.inner, &other.inner)
    }
}
