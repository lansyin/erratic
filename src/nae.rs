//! Error placeholder for [`Builder`][crate::Builder].
use core::{
    error,
    fmt::{self, Display},
};

/// Not an error, a zero-sized error placeholder for [`Builder`][crate::Builder], also represents
/// the end of source chain in [`Error`][crate::Error]. It is used by [`Builder`][crate::Builder]
/// and will not appear in the iterator returned by [`chain`][crate::Error::chain].
#[derive(Debug)]
pub struct Nae {
    _private: (),
}

impl Nae {
    /// Creates a new [`Nae`] instance.
    pub(crate) fn new() -> Self {
        Nae { _private: () }
    }
}

impl Display for Nae {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

impl error::Error for Nae {}
