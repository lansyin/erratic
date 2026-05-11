use std::fmt::{self, Display};

/// A zero-sized payload placeholder for [Error][crate::Error].
#[derive(Debug)]
pub struct Empty;

impl Display for Empty {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}
