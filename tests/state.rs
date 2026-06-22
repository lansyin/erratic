use erratic::test_fixtures::*;
use erratic::{builder::Builder, *};

use std::assert_matches;

#[test]
fn builder_with_state_inlines_when_no_source() {
    let err: Error<TestState> = Builder::with_state(TestState::AppleNotFound).build_error();
    assert_matches!(err.state(), Some(TestState::AppleNotFound));
}

#[test]
fn builder_with_state_and_context_boxes() {
    let err: Error<TestState> = Builder::with_state(TestState::AppleNotFound)
        .with_context(TestMessage::HOGE)
        .build_error();
    assert_matches!(err.state(), Some(TestState::AppleNotFound));
    let (_, context, _) = err.into_parts::<TestMessage, TestError>();
    assert_matches!(context, Some(TestMessage::HOGE));
}

#[test]
fn reconstruct_error_from_vacant() {
    let err: Error<TestState> = mkerr!(
        state = TestState::AppleNotFound,
        context = TestMessage::HOGE,
    );

    let (state, vacant) = err.extract_state().unwrap();
    assert_eq!(state, TestState::AppleNotFound);

    let err = vacant.with_state(TestState::BananaDenied);
    assert_matches!(err.state(), Some(TestState::BananaDenied));

    let (state, context, _) = err.into_parts::<TestMessage, TestError>();
    assert_eq!(state, Some(TestState::BananaDenied));
    assert_matches!(context, Some(TestMessage::HOGE));
}

#[test]
fn reconstruct_with_error_source_from_vacant() {
    let err: Error<TestState> = mkerr!(
        state = TestState::AppleNotFound,
        context = TestMessage::HOGE,
        error = TestError::FOO,
    );

    let (state, vacant) = err.extract_state().unwrap();
    assert_eq!(state, TestState::AppleNotFound);

    let err = vacant.with_state(TestState::BananaDenied);
    assert_matches!(err.state(), Some(TestState::BananaDenied));

    let (state, context, source) = err.into_parts::<TestMessage, TestError>();
    assert_eq!(state, Some(TestState::BananaDenied));
    assert_matches!(context, Some(TestMessage::HOGE));
    assert_matches!(source, Some(TestError::FOO));
}

#[test]
fn extract_state_from_inline_and_reconstruct() {
    let err: Error<TestState> = mkerr!(state = TestState::AppleNotFound);

    let (state, vacant) = err.extract_state().unwrap();
    assert_eq!(state, TestState::AppleNotFound);

    let err = vacant.with_state(TestState::BananaDenied);
    assert_matches!(err.state(), Some(TestState::BananaDenied));
    assert_eq!(err.extract_state().unwrap().0, TestState::BananaDenied);
}

#[test]
fn vacant_try_into_stateless() {
    let err: Error<TestState> = mkerr!(
        state = TestState::AppleNotFound,
        context = TestMessage::HOGE,
    );

    let (_state, vacant) = err.extract_state().unwrap();
    let stateless = vacant.try_into_stateless().unwrap();

    let (context, _) = stateless.into_parts::<TestMessage, TestError>();
    assert_matches!(context, Some(TestMessage::HOGE));
}

#[test]
fn vacant_try_into_stateless_from_inline_returns_none() {
    let err: Error<TestState> = mkerr!(state = TestState::AppleNotFound);

    let (_state, vacant) = err.extract_state().unwrap();
    let stateless = vacant.try_into_stateless();
    assert!(stateless.is_err());
}
