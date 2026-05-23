//! Payload placeholder for [`Builder`][crate::Builder].
use std::fmt::{self, Display};

/// A zero-sized payload placeholder for [Error][crate::Error].
#[derive(Debug)]
pub struct Empty {
    _private: (),
}

impl Empty {
    /// Creates a new [`Empty`] instance.
    pub(crate) fn new() -> Self {
        Empty { _private: () }
    }
}

impl Display for Empty {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

/// A trait for types that can [produce a payload][crate::BuilderExt::with_payload_fn].
pub trait PayloadFn {
    type Output: Display + Send + Sync + 'static;

    fn call(self) -> Self::Output;
}

impl<T, R> PayloadFn for T
where
    T: FnOnce() -> R,
    R: Display + Send + Sync + 'static,
{
    type Output = R;

    fn call(self) -> Self::Output {
        self()
    }
}

/// A wrapper that can be used as [`PayloadFn`].
#[derive(Debug)]
pub struct Immediate<P>(pub P)
where
    P: Display + Send + Sync + 'static;

impl<P> PayloadFn for Immediate<P>
where
    P: Display + Send + Sync + 'static,
{
    type Output = P;

    fn call(self) -> Self::Output {
        self.0
    }
}
