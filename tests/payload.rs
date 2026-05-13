mod common;

use common::{TestError, TestMessage};
use erratic::*;

#[test]
fn from_payload_creates_boxed() {
    let err = mkerr!(TestMessage("hello".into())).stateless();
    let parts = err.into_parts::<TestError, TestMessage>();
    assert!(parts.0.is_none());
    assert!(parts.1.is_some());
    assert_eq!(parts.1.unwrap().0, "hello");
}

#[test]
fn downcast_payload_ok() {
    let err = mkerr!(TestMessage("hello".into())).stateless();
    assert!(err.has_payload_of::<TestMessage>());
    assert_eq!(
        err.downcast_payload_ref::<TestMessage>().unwrap().0,
        "hello"
    );
}
