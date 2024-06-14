/// Timestamp used by [crate::sync::Awareness] to tag most recent updates.
pub type Timestamp = u64;

/// A clock trait used to obtain the current time.
pub trait Clock {
    fn now(&self) -> Timestamp;
}

impl<F> Clock for F
where
    F: Fn() -> Timestamp,
{
    #[inline]
    fn now(&self) -> Timestamp {
        self()
    }
}

/// A clock which uses standard (non-monotonic) OS date time.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Copy, Clone, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as Timestamp
    }
}
