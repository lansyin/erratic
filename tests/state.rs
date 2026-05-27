mod common;

use common::{TestError, TestMessage, TestState};
use erratic::*;

#[test]
fn builder_with_state_inlines_when_no_source() {
    let err: Error<TestState> = Error::with_state(TestState::FileNotFound).build();
    assert!(matches!(err.state(), Some(TestState::FileNotFound)));
}

#[test]
fn builder_with_state_and_payload_boxes() {
    let err: Error<TestState> = Error::with_state(TestState::FileNotFound)
        .with_payload(TestMessage("data".into()))
        .build();
    assert!(matches!(err.state(), Some(TestState::FileNotFound)));
    let (_, _, payload, _) = err.into_parts::<TestMessage, TestError>();
    assert!(payload.is_some());
    assert_eq!(payload.unwrap().0, "data");
}
