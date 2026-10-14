//! `atomic` module is a home for [AtomicRef] cell-like struct, used to perform thread-safe
//! operations using underlying hardware intristics.

use arc_swap::ArcSwapOption;
use std::fmt::Formatter;
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
pub struct AtomicRef<T>(ArcSwapOption<T>);

impl<T> AtomicRef<T> {
    /// Creates a new instance of [AtomicRef]. This call boxes provided `value` and allocates it
    /// on a heap.
    pub fn new(value: T) -> Self {
        AtomicRef(ArcSwapOption::from_pointee(value))
    }

    /// Returns a reference to current state hold by the [AtomicRef]. Keep in mind that after
    /// acquiring it, it may not present the current view of the state, but instead be changed by
    /// the concurrent [AtomicRef::update] call.
    pub fn get(&self) -> Option<Arc<T>> {
        self.0.load_full()
    }

    /// Atomically replaces currently stored value with a new one, returning the last stored value.
    pub fn swap(&self, value: T) -> Option<Arc<T>> {
        self.0.swap(Some(Arc::new(value)))
    }

    /// Atomically replaces currently stored value with a null, returning the last stored value.
    pub fn take(&self) -> Option<Arc<T>> {
        self.0.swap(None)
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
        self.0.rcu(|old| Some(Arc::new(f(old.as_deref()))));
    }
}

impl<T: Copy> AtomicRef<T> {
    /// Returns a current state copy hold by the [AtomicRef]. Keep in mind that after
    /// acquiring it, it may not present the current view of the state, but instead be changed by
    /// the concurrent [AtomicRef::update] call.
    pub fn get_owned(&self) -> Option<T> {
        self.0.load().as_deref().copied()
    }
}

impl<T> PartialEq for AtomicRef<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        let a = self.0.load();
        let b = other.0.load();
        match (a.as_ref(), b.as_ref()) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
            (None, None) => true,
            _ => false,
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
        AtomicRef(ArcSwapOption::empty())
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

    #[test]
    fn concurrent_get_and_swap() {
        use std::sync::Arc;

        let atom = Arc::new(AtomicRef::new(vec![0usize; 16]));
        let writers: Vec<_> = (0..4)
            .map(|i| {
                let atom = atom.clone();
                std::thread::spawn(move || {
                    for j in 0..10_000 {
                        if j % 2 == 0 {
                            atom.swap(vec![i; 16]);
                        } else {
                            atom.update(|v| v.cloned().unwrap_or_default());
                        }
                    }
                })
            })
            .collect();
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let atom = atom.clone();
                std::thread::spawn(move || {
                    for _ in 0..10_000 {
                        // a value read must never be torn or freed under us
                        let v = atom.get().unwrap();
                        assert!(v.iter().all(|x| *x == v[0]));
                    }
                })
            })
            .collect();
        for h in writers.into_iter().chain(readers) {
            h.join().unwrap();
        }
        assert_eq!(atom.get().unwrap().len(), 16);
    }
}
