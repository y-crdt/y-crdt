/// Timestamp used by [crate::sync::Awareness] to tag most recent updates.
pub type Timestamp = u64;

/// A clock trait used to obtain the current time.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

impl<F> Clock for F
where
    F: Fn() -> Timestamp + Send + Sync,
{
    #[inline]
    fn now(&self) -> Timestamp {
        self()
    }
}

/// A clock which uses standard (non-monotonic) OS date time.
#[cfg(not(target_family = "wasm"))]
#[derive(Debug, Copy, Clone, Default)]
pub struct SystemClock;

#[cfg(not(target_family = "wasm"))]
impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as Timestamp
    }
}
