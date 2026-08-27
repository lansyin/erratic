use core::assert_matches;
use erratic::test_fixtures::*;
use erratic::*;

#[test]
fn from_context_creates_const() {
    let mut err: Error = mkerr!("oops");

    assert_matches!(err.downcast_context_ref::<&'static str>(), Some(&"oops"));
    if cfg!(not(feature = "backtrace")) {
        assert_matches!(err.downcast_context_mut::<&'static str>(), None);
    }
}

#[test]
fn context_methods_boxed_context() {
    let mut err: Error = mkerr!(context = TestMessage::HOGE);

    // `context()` is present and displays correctly.
    assert_eq!(err.context().unwrap().to_string(), "hoge");

    // `has_context_of` matches the exact type only.
    assert!(err.has_context_of::<TestMessage>());
    assert!(!err.has_context_of::<TestError>());

    // Downcast by shared reference.
    assert_matches!(
        err.downcast_context_ref::<TestMessage>(),
        Some(&TestMessage::HOGE)
    );
    assert!(err.downcast_context_ref::<TestError>().is_none());

    // Boxed errors are heap-allocated, so mutable downcast works.
    {
        let ctx = err.downcast_context_mut::<TestMessage>().unwrap();
        assert_eq!(ctx.0, "hoge");
        ctx.0 = "mutated";
    }
    assert_eq!(err.context().unwrap().to_string(), "mutated");
    assert!(err.downcast_context_mut::<TestError>().is_none());
}

#[test]
fn context_methods_literal_context() {
    let mut err: Error = mkerr!("oops");

    assert_eq!(err.context().unwrap().to_string(), "oops");
    assert!(err.has_context_of::<&'static str>());
    assert!(!err.has_context_of::<TestMessage>());
    assert_matches!(err.downcast_context_ref::<&'static str>(), Some(&"oops"));
    assert!(err.downcast_context_ref::<TestMessage>().is_none());

    // Allocation-free const errors cannot be downcast mutably.
    if cfg!(not(feature = "backtrace")) {
        assert!(err.downcast_context_mut::<&'static str>().is_none());
    }
}

#[test]
fn context_methods_without_context() {
    // Source-only error: no context.
    let mut err: Error = mkerr!(error = TestError::FOO);
    assert!(err.context().is_none());
    assert!(!err.has_context_of::<TestMessage>());
    assert!(!err.has_context_of::<&'static str>());
    assert!(err.downcast_context_ref::<TestMessage>().is_none());
    assert!(err.downcast_context_mut::<TestMessage>().is_none());

    // State-only inline error: no context.
    let mut err: Error<i8> = mkerr!(state = 42i8);
    assert!(err.context().is_none());
    assert!(!err.has_context_of::<TestMessage>());
    assert!(err.downcast_context_ref::<TestMessage>().is_none());
    assert!(err.downcast_context_mut::<TestMessage>().is_none());
}
