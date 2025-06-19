use std::cell::UnsafeCell;
use std::ops::Deref;

pub(crate) struct Lazy<T, F> {
    state: UnsafeCell<State<T, F>>,
}

enum State<T, F> {
    Uninitialized(F),
    Initializing,
    Initialized(T),
}

impl<T, F> Lazy<T, F>
where
    F: Once<Output = T>,
{
    pub fn new(f: F) -> Self {
        Self {
            state: UnsafeCell::new(State::Uninitialized(f)),
        }
    }

    pub fn force(&self) -> &T {
        unsafe {
            let state = &mut *self.state.get();
            match state {
                State::Uninitialized(_) => {
                    let State::Uninitialized(f) = std::mem::replace(state, State::Initializing)
                    else {
                        unreachable!()
                    };
                    let value = f.call();
                    *state = State::Initialized(value);
                }
                State::Initializing => panic!("Lazy is already being initialized"),
                State::Initialized(value) => return value,
            }
            match &*self.state.get() {
                State::Initialized(value) => value,
                _ => unreachable!(),
            }
        }
    }
}

impl<T, F> Deref for Lazy<T, F>
where
    F: Once<Output = T>,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.force()
    }
}

pub trait Once {
    type Output;

    fn call(self) -> Self::Output;
}
