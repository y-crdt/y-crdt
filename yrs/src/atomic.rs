//! `atomic` module is a home for [AtomicRef] cell-like struct, used to perform thread-safe
//! operations using underlying hardware intristics.

use std::fmt::Formatter;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

/// Atomic reference holding a value, that's supposed to be shared - potentially between multiple
/// threads. Internally this value is hidden behind [Arc] reference, which is returned during
/// [AtomicRef::get] method. This cell doesn't allow to return &mut references to stored object.
/// Instead updates can be performed as lock-free operation mutating function
/// passed over during [AtomicRef::update] call.
///
/// Example:
/// ```rust
/// use yrs::atomic::AtomicRef;
///
/// let atom = AtomicRef::new(vec!["John"]);
/// atom.update(|users| {
///     let mut users_copy = users.cloned().unwrap_or_else(Vec::default);
///     users_copy.push("Susan");
///     users_copy
/// });
/// let users = atom.get(); // John, Susan
/// ```
/// **Important note**: since [AtomicRef::update] may call provided function multiple times (in
/// scenarios, when another thread intercepted update with its own update call), provided function
/// should be idempotent and preferably quick to execute.
#[repr(transparent)]
pub struct AtomicRef<T>(AtomicPtr<T>);

unsafe impl<T> Send for AtomicRef<T> {}
unsafe impl<T> Sync for AtomicRef<T> {}

impl<T> AtomicRef<T> {
    /// Creates a new instance of [AtomicRef]. This call boxes provided `value` and allocates it
    /// on a heap.
    pub fn new(value: T) -> Self {
        let arc = Arc::new(value);
        let ptr = Arc::into_raw(arc) as *mut _;
        AtomicRef(AtomicPtr::new(ptr))
    }

    /// Returns a reference to current state hold by the [AtomicRef]. Keep in mind that after
    /// acquiring it, it may not present the current view of the state, but instead be changed by
    /// the concurrent [AtomicRef::update] call.
    pub fn get(&self) -> Option<Arc<T>> {
        let ptr = self.0.load(Ordering::SeqCst);
        if ptr.is_null() {
            None
        } else {
            let arc = unsafe { Arc::from_raw(ptr) };
            let result = arc.clone();
            std::mem::forget(arc);
            Some(result)
        }
    }

    /// Atomically replaces currently stored value with a new one, returning the last stored value.
    pub fn swap(&self, value: T) -> Option<Arc<T>> {
        let new_ptr = Arc::into_raw(Arc::new(value)) as *mut _;
        let prev = self.0.swap(new_ptr, Ordering::Release);
        if prev.is_null() {
            None
        } else {
            let arc = unsafe { Arc::from_raw(prev) };
            Some(arc)
        }
    }

    /// Atomically replaces currently stored value with a null, returning the last stored value.
    pub fn take(&self) -> Option<Arc<T>> {
        let prev = self.0.swap(null_mut(), Ordering::Release);
        if prev.is_null() {
            None
        } else {
            let arc = unsafe { Arc::from_raw(prev) };
            Some(arc)
        }
    }

    /// Updates stored value in place using provided function `f`, which takes read-only refrence
    /// to the most recently known state and producing new state in the result.
    ///
    /// **Important note**: since [AtomicRef::update] may call provided function multiple times (in
    /// scenarios, when another thread intercepted update with its own update call), provided
    /// function should be idempotent and preferably quick to execute.
    pub fn update<F>(&self, f: F)
    where
        F: Fn(Option<&T>) -> T,
    {
        loop {
            let old_ptr = self.0.load(Ordering::SeqCst);
            let old_value = unsafe { old_ptr.as_ref() };

            // modify copied value
            let new_value = f(old_value);

            let new_ptr = Arc::into_raw(Arc::new(new_value)) as *mut _;

            let swapped =
                self.0
                    .compare_exchange(old_ptr, new_ptr, Ordering::AcqRel, Ordering::Relaxed);

            match swapped {
                Ok(old) => {
                    if !old.is_null() {
                        unsafe { Arc::decrement_strong_count(old) }; // drop reference to old
                    }
                    break; // we succeeded
                }
                Err(new) => {
                    if !new.is_null() {
                        unsafe { Arc::decrement_strong_count(new) }; // drop reference to new and retry
                    }
                }
            }
        }
    }
}

impl<T: Copy> AtomicRef<T> {
    /// Returns a current state copy hold by the [AtomicRef]. Keep in mind that after
    /// acquiring it, it may not present the current view of the state, but instead be changed by
    /// the concurrent [AtomicRef::update] call.
    pub fn get_owned(&self) -> Option<T> {
        let ptr = self.0.load(Ordering::SeqCst);
        if ptr.is_null() {
            None
        } else {
            let arc = unsafe { Arc::from_raw(ptr) };
            let result = *arc;
            std::mem::forget(arc);
            Some(result)
        }
    }
}

impl<T> Drop for AtomicRef<T> {
    fn drop(&mut self) {
        unsafe {
            let ptr = self.0.load(Ordering::Acquire);
            if !ptr.is_null() {
                Arc::decrement_strong_count(ptr);
            }
        }
    }
}

impl<T> PartialEq for AtomicRef<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        let a = self.0.load(Ordering::Acquire);
        let b = other.0.load(Ordering::Acquire);
        if std::ptr::eq(a, b) {
            true
        } else {
            unsafe { a.as_ref() == b.as_ref() }
        }
    }
}

impl<T> Eq for AtomicRef<T> where T: Eq {}

impl<T: std::fmt::Debug> std::fmt::Debug for AtomicRef<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = self.get();
        write!(f, "AtomicRef({:?})", value.as_deref())
    }
}

impl<T> Default for AtomicRef<T> {
    fn default() -> Self {
        AtomicRef(AtomicPtr::new(null_mut()))
    }
}

#[cfg(test)]
mod test {
    use crate::atomic::AtomicRef;

    #[test]
    fn init_get() {
        let atom = AtomicRef::new(1);
        let value = atom.get();
        assert_eq!(value.as_deref().cloned(), Some(1));
    }

    #[test]
    fn update() {
        let atom = AtomicRef::new(vec!["John"]);
        let old_users = atom.get().unwrap();
        let actual: &[&str] = &old_users;
        assert_eq!(actual, &["John"]);

        atom.update(|users| {
            let mut users_copy = users.cloned().unwrap_or_else(Vec::default);
            users_copy.push("Susan");
            users_copy
        });

        // after update new Arc ptr data returns updated content
        let new_users = atom.get().unwrap();
        let actual: &[&str] = &new_users;
        assert_eq!(actual, &["John", "Susan"]);

        // old Arc ptr data is unchanged
        let actual: &[&str] = &old_users;
        assert_eq!(actual, &["John"]);
    }
}
