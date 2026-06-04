mod common;

use common::{TestError, TestMessage, TestState};
use erratic::*;

#[test]
fn builder_with_state_inlines_when_no_source() {
    let err: Error<TestState> = Error::with_state(TestState::FileNotFound).build_error();
    assert!(matches!(err.state(), Some(TestState::FileNotFound)));
}

#[test]
fn builder_with_state_and_payload_boxes() {
    let err: Error<TestState> = Error::with_state(TestState::FileNotFound)
        .with_payload(TestMessage("data"))
        .build_error();
    assert!(matches!(err.state(), Some(TestState::FileNotFound)));
    let (_, _, payload, _) = err.into_parts::<TestMessage, TestError>();
    assert!(matches!(payload, Some(TestMessage("data"))));
}

#[test]
fn reconstruct_error_from_vacant() {
    let err: Error<TestState> = mkerr!(
        state = TestState::FileNotFound,
        payload = TestMessage("data"),
    );

    let (state, vacant) = err.extract_state().unwrap();
    assert_eq!(state, TestState::FileNotFound);

    let err = vacant.with_state(TestState::PermissionDenied);
    assert!(matches!(err.state(), Some(TestState::PermissionDenied)));

    let (state, _, payload, _) = err.into_parts::<TestMessage, TestError>();
    assert_eq!(state, Some(TestState::PermissionDenied));
    assert!(matches!(payload, Some(TestMessage("data"))));
}

#[test]
fn reconstruct_with_error_source_from_vacant() {
    let err: Error<TestState> = mkerr!(
        state = TestState::FileNotFound,
        payload = TestMessage("data"),
        error = TestError("oops"),
    );

    let (state, vacant) = err.extract_state().unwrap();
    assert_eq!(state, TestState::FileNotFound);

    let err = vacant.with_state(TestState::PermissionDenied);
    assert!(matches!(err.state(), Some(TestState::PermissionDenied)));

    let (state, _, payload, source) = err.into_parts::<TestMessage, TestError>();
    assert_eq!(state, Some(TestState::PermissionDenied));
    assert!(matches!(payload, Some(TestMessage("data"))));
    assert!(matches!(source, Some(TestError("oops"))));
}

#[test]
fn extract_state_from_inline_and_reconstruct() {
    let err: Error<TestState> = mkerr!(state = TestState::FileNotFound);

    let (state, vacant) = err.extract_state().unwrap();
    assert_eq!(state, TestState::FileNotFound);

    let err = vacant.with_state(TestState::PermissionDenied);
    assert!(matches!(err.state(), Some(TestState::PermissionDenied)));
    assert_eq!(err.extract_state().unwrap().0, TestState::PermissionDenied);
}

#[test]
fn vacant_try_into_stateless() {
    let err: Error<TestState> = mkerr!(
        state = TestState::FileNotFound,
        payload = TestMessage("data"),
    );

    let (_state, vacant) = err.extract_state().unwrap();
    let stateless = vacant.try_into_stateless().unwrap();

    let (_, payload, _) = stateless.into_parts::<TestMessage, TestError>();
    assert!(matches!(payload, Some(TestMessage("data"))));
}

#[test]
fn vacant_try_into_stateless_from_inline_returns_none() {
    let err: Error<TestState> = mkerr!(state = TestState::FileNotFound);

    let (_state, vacant) = err.extract_state().unwrap();
    let stateless = vacant.try_into_stateless();
    assert!(stateless.is_err());
}
