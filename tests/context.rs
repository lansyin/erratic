use erratic::*;
use std::error::Error as _;

#[test]
fn from_context_creates_const() {
    let err = mkerr!("file not found").stateless();
    assert!(err.erase_ref().source().is_none());
}
