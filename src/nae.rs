use std::{
    error,
    fmt::{self, Display},
};

/// Not an error, a zero-sized error placeholder for [Error][crate::Error], represents the end of the error source chain.
#[derive(Debug)]
pub struct Nae;

impl Display for Nae {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

impl error::Error for Nae {}
