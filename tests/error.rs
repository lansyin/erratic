mod common;

use common::{TestError, TestMessage, TestState};
use erratic::{nae::Nae, *};
use std::{mem, result};

#[test]
fn from_error_round_trip() {
    let err = mkerr!(error = TestError("oops")).stateless();
    let (context, payload, source) = err.into_parts::<TestMessage, TestError>();
    assert!(source.is_some());
    assert_eq!(source.unwrap().0, "oops");
    assert!(payload.is_none());
    assert!(context.is_none());
}

#[test]
fn builder_with_error_builds_correctly() {
    let err: Error = Error::with_error(TestError("oops"))
        .with_context(literal!("context"))
        .with_payload(TestMessage("payload".into()))
        .build();
    let (context, payload, source) = err.into_parts::<TestMessage, TestError>();
    assert!(matches!(context, Some("context")));
    assert!(source.is_some());
    assert_eq!(source.as_ref().unwrap().0, "oops");
    assert!(payload.is_some());
    assert_eq!(payload.unwrap().0, "payload");
}

#[test]
fn downcast_source_ok() {
    let err = mkerr!(error = TestError("oops")).stateless();
    assert!(err.has_source_of::<TestError>());
    assert_eq!(err.downcast_source_ref::<TestError>().unwrap().0, "oops");
}

#[test]
fn downcast_source_wrong_type() {
    let err = mkerr!(error = TestError("oops")).stateless();
    assert!(!err.has_source_of::<Nae>());
}

#[test]
fn erase_makes_opaque() {
    let err = mkerr!(error = TestError("oops")).stateless();
    assert_eq!(format!("{:-}", err.erase()), "oops");
}

#[test]
fn erase_ref_lifetime() {
    let err = mkerr!(error = TestError("oops")).stateless();
    let opaque: &(dyn std::error::Error + Send + Sync + 'static) = err.erase_ref();
    assert_eq!(format!("{opaque:-}"), "oops");
}

#[test]
fn into_source_returns_boxed_source() {
    let err = mkerr!(error = TestError("oops")).stateless();
    assert_eq!(err.into_source().unwrap().to_string(), "oops");
}

#[test]
fn into_source_const_is_none() {
    let err = mkerr!(context = "test").stateless();
    assert!(err.into_source().is_none());
}

#[test]
fn chain_wraps_source() {
    let inner = mkerr!(error = TestError("inner")).stateless();
    let outer = mkerr!(error = inner.erase()).stateless();
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
    let inner = mkerr!(error = TestError("inner")).stateless();
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

#[test]
fn into_boxed_error() {
    let err = mkerr!(error = TestError("oops")).stateless();
    let boxed: Box<dyn std::error::Error + Send + Sync + 'static> = err.into();
    assert_eq!(format!("{boxed:-}"), "oops");
}

#[test]
fn wrap_self() -> Result<()> {
    Ok::<(), Error>(()).with_context(literal!("while testing wrap_self"))?;
    Ok::<(), Error>(())
        .with_context(literal!("while testing wrap_self"))
        .build_error()?;
    Ok::<(), Error>(())
        .with_context(literal!("while testing wrap_self"))
        .build_error()
        .with_context(literal!("while testing wrap_self with nested error"))?;

    Ok(())
}

#[test]
fn extract_state() -> Result<()> {
    let re: result::Result<(), _> = mkres!(state = 12);
    let re = re.extract_state()?;
    match re.unwrap_err() {
        (12, ..) => Ok(()),
        _ => unreachable!(),
    }
}
