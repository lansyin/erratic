use erratic::*;
use std::error::Error as _;

#[test]
fn from_context_creates_const() {
    let err = Error::from_context(literal!("file not found"));
    assert!(err.erase_ref().source().is_none());
}
