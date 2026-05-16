//! Payload placeholder for [`Builder`][crate::Builder].
use std::fmt::{self, Display};

/// A zero-sized payload placeholder for [Error][crate::Error].
#[derive(Debug)]
pub struct Empty;

impl Display for Empty {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

/// A trait for types that can [produce a payload][crate::BuilderExt::with_payload_fn].
pub trait PayloadFn {
    type Output;

    fn load(self) -> Self::Output;
}

impl<T, R> PayloadFn for T
where
    T: FnOnce() -> R,
{
    type Output = R;

    fn load(self) -> Self::Output {
        self()
    }
}

/// A wrapper that can be used as [`PayloadFn`].
pub struct Payload<P>(pub P);

impl<T> PayloadFn for Payload<T> {
    type Output = T;

    fn load(self) -> Self::Output {
        self.0
    }
}
