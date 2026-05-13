mod common;

use common::{TestError, TestMessage, TestState};
use erratic::*;
use std::mem;

#[test]
fn from_error_round_trip() {
    let err = mkerr!(TestError("oops")).stateless();
    let parts = err.into_parts::<TestError, TestMessage>();
    assert!(parts.0.is_some());
    assert_eq!(parts.0.unwrap().0, "oops");
    assert!(parts.1.is_none());
}

#[test]
fn builder_with_error_builds_correctly() {
    let err: Error = Error::with_error(TestError("oops"))
        .with_context(literal!("context"))
        .with_payload(|| TestMessage("payload".into()))
        .build();
    let (source, payload) = err.into_parts::<TestError, TestMessage>();
    assert!(source.is_some());
    assert_eq!(source.as_ref().unwrap().0, "oops");
    assert!(payload.is_some());
    assert_eq!(payload.unwrap().0, "payload");
}

#[test]
fn downcast_source_ok() {
    let err = mkerr!(TestError("oops")).stateless();
    assert!(err.has_source_of::<TestError>());
    assert_eq!(err.downcast_source_ref::<TestError>().unwrap().0, "oops");
}

#[test]
fn downcast_source_wrong_type() {
    let err = mkerr!(TestError("oops")).stateless();
    assert!(!err.has_source_of::<String>());
}

#[test]
fn erase_makes_opaque() {
    let err = mkerr!(TestError("oops")).stateless();
    assert_eq!(err.erase().to_string(), "oops");
}

#[test]
fn erase_ref_lifetime() {
    let err = mkerr!(TestError("oops")).stateless();
    let opaque: &(dyn std::error::Error + Send + Sync + 'static) = err.erase_ref();
    assert_eq!(opaque.to_string(), "oops");
}

#[test]
fn into_source_returns_boxed_source() {
    let err = mkerr!(TestError("oops")).stateless();
    assert_eq!(err.into_source().unwrap().to_string(), "oops");
}

#[test]
fn into_source_const_is_none() {
    let err = mkerr!("test").stateless();
    assert!(err.into_source().is_none());
}

#[test]
fn chain_wraps_source() {
    let inner = mkerr!(TestError("inner")).stateless();
    let outer = mkerr!(inner.erase()).stateless();
    let mut chain = outer.chain();
    assert_eq!(chain.next().unwrap().to_string(), "inner");
    assert!(chain.next().is_none());
}

#[test]
fn from_std_error_via_into() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
    let err: Error = io_err.into();
    assert!(err.into_source().is_some());
}

#[test]
fn from_same_type_id_does_not_double_wrap() {
    let inner = mkerr!(TestError("inner")).stateless();
    let outer: Error = inner.erase().into();
    assert_eq!(outer.into_source().unwrap().to_string(), "inner");
}

#[test]
fn error_is_one_usize() {
    assert_eq!(mem::size_of::<Error>(), mem::size_of::<usize>());
    assert_eq!(mem::size_of::<Error<TestState>>(), mem::size_of::<usize>());
}

#[test]
fn error_is_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<Error>();
    assert_sync::<Error>();
    assert_send::<Error<TestState>>();
    assert_sync::<Error<TestState>>();
}
