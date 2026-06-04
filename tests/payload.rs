mod common;

use common::{TestError, TestMessage};
use erratic::*;
use std::assert_matches;

#[test]
fn from_payload_creates_boxed() {
    let err = mkerr!(payload = TestMessage("hello")).stateless();
    let parts = err.into_parts::<TestMessage, TestError>();
    assert!(parts.0.is_none());
    assert_matches!(parts.1, Some(TestMessage("hello")));
}

#[test]
fn downcast_payload_ok() {
    let err = mkerr!(payload = TestMessage("hello")).stateless();
    assert!(err.has_payload_of::<TestMessage>());
    assert_eq!(
        err.downcast_payload_ref::<TestMessage>().unwrap().0,
        "hello"
    );
}
