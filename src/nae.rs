//! Error placeholder for [`Builder`][crate::Builder].
use core::{
    error,
    fmt::{self, Display},
};

/// Not an error, a zero-sized error placeholder for [`Builder`][crate::Builder].
///
/// It is consumed during materialization and never appears in the error chain or source.
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
