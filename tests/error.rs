use core::{cell::Cell, error};

#[cfg(test)]
use erratic::test_fixtures::*;
use erratic::{builder::Builder, *};
use std::{
    assert_matches,
    io::{self, ErrorKind},
    mem,
};

#[test]
fn from_error_round_trip() {
    let err: Error = mkerr!(error = TestError::FOO);
    let (context, source) = err.into_parts::<TestMessage, TestError>();
    assert_matches!(source, Some(TestError::FOO));
    assert!(context.is_none());
}

#[test]
fn builder_with_error_builds_correctly() {
    let err: Error = mkerr!(error = TestError::FOO, context = TestMessage::HOGE,);
    let (context, source) = err.into_parts::<TestMessage, TestError>();
    assert_matches!(context, Some(TestMessage::HOGE));
    assert_matches!(source, Some(TestError::FOO));
}

#[test]
fn builder_case1() {
    // error only (fast path)
    {
        let err: Error = Builder::with_error(TestError::FOO).into();
        assert_eq!(err.chain().count(), 1);
        let (context, source) = err.into_parts::<TestMessage, TestError>();
        assert!(context.is_none());
        assert_matches!(source, Some(TestError::FOO));
    }
    // state only (fast path)
    {
        let err: Error<TestState> = Builder::with_state(TestState::AppleNotFound).into();
        assert_eq!(err.chain().count(), 1);
        assert_eq!(err.state().unwrap(), &TestState::AppleNotFound);
        assert!(err.into_source().is_none());
    }
    // context only (fast path)
    {
        let err: Error = Builder::with_context(TestMessage::HOGE).into();
        assert_eq!(err.chain().count(), 1);
        let (context, source) = err.into_parts::<TestMessage, TestError>();
        assert_matches!(context, Some(TestMessage::HOGE));
        assert!(source.is_none());
    }
    // all present (no data loss)
    {
        let err: Error<TestState> = mkerr!(
            state = TestState::AppleNotFound,
            context = TestMessage::HOGE,
            error = TestError::FOO,
        );
        let (state, context, source) = err.into_parts::<TestMessage, TestError>();
        assert_eq!(state, Some(TestState::AppleNotFound));
        assert_matches!(context, Some(TestMessage::HOGE));
        assert_matches!(source, Some(TestError::FOO));
    }
}

#[test]
fn builder_case2() {
    // Note: case2 was removed as it has no meaningful use case.
}

#[test]
fn builder_case3() {
    // error only -> state (fast path)
    {
        let err: Error<TestState> = Builder::with_error(TestError::FOO).into();
        assert_eq!(err.chain().count(), 1);
        let (state, context, source) = err.into_parts::<TestMessage, TestError>();
        assert!(state.is_none());
        assert!(context.is_none());
        assert_matches!(source, Some(TestError::FOO));
    }
    // context only -> state (fast path)
    {
        let err: Error<TestState> = Builder::with_context(TestMessage::HOGE).into();
        let (state, context, source) = err.into_parts::<TestMessage, TestError>();
        assert!(state.is_none());
        assert_eq!(context, Some(TestMessage::HOGE));
        assert!(source.is_none());
    }
    // error + context -> state (no fast path)
    {
        let err: Error<TestState> = Builder::with_error(TestError::FOO)
            .with_context(TestMessage::HOGE)
            .into();
        let (state, context, source) = err.into_parts::<TestMessage, TestError>();
        assert!(state.is_none());
        assert_eq!(context, Some(TestMessage::HOGE));
        assert_matches!(source, Some(TestError::FOO));
    }
}

#[test]
fn builder_case4() {
    // erratic state -> same state (fast path)
    {
        let inner: Error<TestState> =
            mkerr!(error = TestError::BAR, state = TestState::AppleNotFound);
        let outer: Error<TestState> = Builder::with_error(inner).into();
        assert_eq!(outer.chain().count(), 2);
        let (state, context, source) = outer.into_parts::<TestMessage, TestError>();
        assert_eq!(state, Some(TestState::AppleNotFound));
        assert!(context.is_none());
        assert_matches!(source, Some(TestError::BAR));
    }
    // erratic source + context (no fast path)
    {
        let inner: Error<TestState> =
            mkerr!(error = TestError::BAR, state = TestState::AppleNotFound);
        let outer: Error<TestState> = Builder::with_error(inner)
            .with_context(TestMessage::PIYO)
            .into();
        assert!(outer.find::<TestError>().is_some());
        let (state, context, _source) = outer.into_parts::<TestMessage, TestError>();
        assert_eq!(state, Some(TestState::AppleNotFound));
        assert_matches!(context, Some(TestMessage::PIYO));
    }
}

#[test]
fn builder_case4_regression_260806() {
    // Regression: When the builder had a context and the wrapped error did not have a state,
    // the builder would just call `with_phantom_state` without attaching the builder's context,
    // resulting in the context being lost.

    let inner: Error<TestState> = TestError::FOO.into();
    assert!(inner.state().is_none());

    let outer: Error<TestState> = Builder::with_error(inner)
        .with_context(TestMessage::HOGE)
        .into();

    assert_eq!(outer.context().unwrap().to_string(), "hoge");
    assert!(outer.find::<TestError>().is_some());
    let (state, context, _source) = outer.into_parts::<TestMessage, TestError>();
    assert!(state.is_none());
    assert_matches!(context, Some(TestMessage::HOGE));
}

#[test]
fn builder_case5() {
    // Note: case5 was removed as it has no meaningful use case.
}

#[test]
fn builder_case6() {
    // erratic stateless -> state (fast path)
    {
        let inner: Error = mkerr!(error = TestError::BAR);
        let outer: Error<TestState> = Builder::with_error(inner).into();
        assert_eq!(outer.chain().count(), 1);
        let (state, context, source) = outer.into_parts::<TestMessage, TestError>();
        assert!(state.is_none());
        assert!(context.is_none());
        assert_matches!(source, Some(TestError::BAR));
    }
    // erratic source + context -> state (no fast path)
    {
        let inner: Error = mkerr!(error = TestError::BAR);
        let outer: Error<TestState> = Builder::with_error(inner)
            .with_context(TestMessage::FUGA)
            .into();
        let (state, context, _source) = outer.into_parts::<TestMessage, TestError>();
        assert!(state.is_none());
        assert_matches!(context, Some(TestMessage::FUGA));
    }
}

#[test]
fn builder_case7() {
    let inner: Error = mkerr!(error = TestError::BAR);
    let _: Error<TestState> = Builder::with_error(inner)
        .with_context(TestMessage::HOGE)
        .into();
}

#[test]
fn downcast_source_ok() {
    let err: Error = mkerr!(error = TestError::FOO);
    assert!(err.has_source_of::<TestError>());
    assert_matches!(
        err.downcast_source_ref::<TestError>(),
        Some(&TestError::FOO)
    );
}

#[test]
fn downcast_source_wrong_type() {
    let err: Error = mkerr!(error = TestError::FOO);
    assert!(!err.has_source_of::<std::io::Error>());
}

#[test]
fn downcast_source_mut_ok() {
    let mut err: Error = mkerr!(error = TestError::FOO);
    let source = err.downcast_source_mut::<TestError>().unwrap();
    assert_matches!(*source, TestError::FOO);
    *source = TestError::BAR;
    assert_eq!(
        err.downcast_source_ref::<TestError>().unwrap(),
        &TestError::BAR
    );
}

#[test]
fn downcast_source_mut_wrong_type() {
    let mut err: Error = mkerr!(error = TestError::FOO);
    assert!(err.downcast_source_mut::<std::io::Error>().is_none());
}

#[test]
fn erase_makes_opaque() {
    let err: Error = mkerr!(error = TestError::FOO);
    assert_eq!(err.erase_state().to_string(), "foo");
}

#[test]
fn into_source_returns_boxed_source() {
    let err: Error = mkerr!(error = TestError::FOO);
    assert_eq!(err.into_source().unwrap().to_string(), "foo");
}

#[test]
fn into_source_const_is_none() {
    let err: Error = mkerr!("test");
    assert!(err.into_source().is_none());
}

#[test]
fn chain_wraps_source() {
    let inner: Error = mkerr!(error = TestError::BAR);
    let outer: Error = mkerr!(error = inner.erase_state());
    let mut chain = outer.chain();
    assert_eq!(chain.next().unwrap().to_string(), "bar");
    assert!(chain.next().is_none());
}

#[test]
fn from_std_error_via_into() {
    let io_err = io::Error::new(ErrorKind::NotFound, "file missing");
    let err: Error = io_err.into();
    assert!(err.into_source().is_some());
}

#[test]
fn from_same_type_id_does_not_double_wrap() {
    let inner: Error = mkerr!(error = TestError::BAR);
    let outer: Error = inner.erase_state().into();
    assert_eq!(outer.into_source().unwrap().to_string(), "bar",);
}

#[test]
fn into_parts_stateful_recovers_nested_erratic() {
    // A nested erratic error stored as the source must be recoverable through
    // `Error::<S>::into_parts` (the stateful variant) as a full `Error`.
    let inner: Error = mkerr!(error = TestError::FOO, context = TestMessage::HOGE);
    let outer: Error<TestState> = Builder::with_error(inner)
        .with_state(TestState::AppleNotFound)
        .with_context(TestMessage::PIYO)
        .into();

    let (state, context, source) = outer.into_parts::<TestMessage, Error>();
    assert_eq!(state, Some(TestState::AppleNotFound));
    assert_matches!(context, Some(TestMessage::PIYO));
    assert_matches!(
        source.as_ref().map(|e| e.to_string()),
        Some(s) if s == "hoge"
    );

    // The recovered nested error is a real `Error` that still carries its own parts.
    let nested = source.expect("nested erratic error should be recoverable");
    let (nested_context, nested_source) = nested.into_parts::<TestMessage, TestError>();
    assert_matches!(nested_context, Some(TestMessage::HOGE));
    assert_matches!(nested_source, Some(TestError::FOO));
}

#[test]
fn into_parts_stateless_recovers_nested_erratic() {
    // Same as above, through the stateless `Error::into_parts` variant.
    let inner: Error = mkerr!(error = TestError::FOO, context = TestMessage::HOGE);
    let outer: Error = Builder::with_error(inner)
        .with_context(TestMessage::PIYO)
        .into();

    let (context, source) = outer.into_parts::<TestMessage, Error>();
    assert_matches!(context, Some(TestMessage::PIYO));
    assert_matches!(
        source.as_ref().map(|e| e.to_string()),
        Some(s) if s == "hoge"
    );

    let nested = source.expect("nested erratic error should be recoverable");
    let (nested_context, nested_source) = nested.into_parts::<TestMessage, TestError>();
    assert_matches!(nested_context, Some(TestMessage::HOGE));
    assert_matches!(nested_source, Some(TestError::FOO));
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
    let err: Error = mkerr!(error = TestError::FOO);
    let boxed: Box<dyn std::error::Error + Send + Sync + 'static> = err.into();
    assert_eq!(boxed.to_string(), "foo");
}

#[test]
fn wrap_self() {
    let _: Error = mkerr!(context = TestMessage::HOGE);
    let _: Error = mkerr!(context = TestMessage::HOGE);
    let _: Error = mkerr!(
        error = mkerr!(context = TestMessage::HOGE).stateless(),
        context = TestMessage::FUGA,
    );
}

#[test]
fn extract_state() -> Result<()> {
    let re: Result<(), _> = mkres!(state = 12);
    let re = re.extract_state()?;
    match re.unwrap_err() {
        (12, ..) => Ok(()),
        _ => unreachable!(),
    }
}

#[test]
fn dedup_repeated_message_in_chain() {
    {
        let inner = TestError::BAR;
        let outer: Error = Error::from_error(inner);
        assert_eq!(outer.to_string(), "bar");
        assert_eq!(outer.source().unwrap().to_string(), "bar");
        assert_eq!(format!("{:#}", outer), "bar");
    }

    {
        let inner = TestError::BAR;
        let outer: Error = mkerr!(error = inner, "outer");
        assert_eq!(format!("{:#}", outer), "outer\n  -> bar");
    }

    {
        let inner = TestError::BAR;
        let mid = mkerr!(error = inner).stateless();
        let outer: Error = mkerr!(error = mid, "outer");
        assert_eq!(format!("{}", outer), "outer");
        assert_eq!(format!("{}", outer.source().unwrap()), "bar");
        assert_eq!(format!("{}", outer.chain().last().unwrap()), "bar");
        assert_eq!(format!("{:#}", outer), "outer\n  -> bar");
    }

    {
        let inner = TestError::BAR;
        let mid: Error = mkerr!(error = inner, "mid");
        let outer: Error = mkerr!(error = mid, "outer");
        assert_eq!(format!("{:#}", outer), "outer\n  -> mid\n  -> bar");
    }
}

#[test]
fn eliminate_alloc() {
    {
        let inner = TestError::BAR;
        let mid = mkerr!(error = inner, state = TestState::AppleNotFound);
        let outer: Error = mkerr!(error = mid.erase_state());
        assert_eq!(outer.chain().count(), 2);
    }
    {
        let inner = TestError::BAR;
        let mid: Error = mkerr!(error = inner);
        let outer: Error = mkerr!(error = mid.erase_state());
        assert_eq!(outer.chain().count(), 1);
    }
    {
        let inner = TestError::BAR;
        let mid = mkerr!(error = inner, state = TestState::AppleNotFound).erase_state();
        let outer: Error = mkerr!(error = mid);
        assert_eq!(outer.chain().count(), 2);
    }
    {
        let inner = TestError::BAR;
        let mid = mkerr!(error = inner).stateless().erase_state();
        let outer: Error = mkerr!(error = mid);
        assert_eq!(outer.chain().count(), 1);
    }
    {
        let inner = TestError::BAR;
        let mid = mkerr!(error = inner).stateless().erase_state();
        let outer = Builder::with_error(mid).build_error();
        assert_eq!(outer.chain().count(), 1);
    }
}

#[test]
fn deref_and_deref_mut() {
    let mut err: Error = mkerr!("oops");
    let _: &dyn error::Error = &*err;
    let _: &mut dyn error::Error = &mut *err;
}

#[cfg(feature = "backtrace")]
#[test]
fn backtrace_captures_from_first_layer() {
    fn inner_most() -> Error {
        mkerr!(error = TestError::BAZ)
    }
    fn middle() -> Error {
        mkerr!(error = inner_most(), "middle layer")
    }
    fn outer_most() -> Error {
        mkerr!(error = middle(), "outer layer")
    }

    let err = outer_most();
    let Some(bt) = err.backtrace() else {
        return;
    };
    let bt_str = format!("{bt:#?}");

    assert!(
        bt_str.contains("outer_most"),
        "backtrace should contain the outermost function name 'outer_most', got: {bt_str}"
    );
    assert!(
        bt_str.contains("inner_most"),
        "backtrace should contain the innermost function name 'inner_most', got: {bt_str}"
    );
}

#[cfg(feature = "backtrace")]
#[test]
fn backtrace_only_at_captured_level_in_debug_alt() {
    fn inner_most() -> Error {
        mkerr!(error = TestError::BAZ)
    }
    fn middle() -> Error {
        mkerr!(error = inner_most(), "middle layer")
    }
    fn outer_most() -> Error {
        mkerr!(error = middle(), "outer layer")
    }

    let err = outer_most();
    let Some(_bt) = err.backtrace() else {
        return;
    };

    let formatted = format!("{err:#?}");
    let count = formatted.matches("backtrace: Backtrace").count();
    assert_eq!(
        count, 1,
        "`backtrace:` should appear exactly once (only at the captured inner level) \
         in `{{:#?}}`, got {count} occurrences:\n{formatted}"
    );
}

#[test]
fn root_finds_deepest_source() {
    fn inner_most() -> Error {
        mkerr!(error = TestError::BAZ)
    }
    fn middle() -> Error {
        mkerr!(error = inner_most(), "middle layer")
    }
    fn outer_most() -> Error {
        mkerr!(error = middle(), "outer layer")
    }

    let err = outer_most();
    let root = err.root().expect("root should be found");
    assert_eq!(root.to_string(), "baz");
    assert!(root.downcast_ref::<TestError>().is_some());
}

#[test]
fn find_looks_up_error_chain() {
    fn inner_most() -> Error {
        mkerr!(error = TestError::BAZ)
    }
    fn middle() -> Error {
        mkerr!(error = inner_most(), "middle layer")
    }
    fn outer_most() -> Error {
        mkerr!(error = middle(), "outer layer")
    }

    let err = outer_most();

    // Should find TestError (deepest)
    let found = err.find::<TestError>();
    assert!(found.is_some());
    assert_eq!(found.unwrap().0, "baz");

    // Should not find a type not in the chain
    assert!(err.find::<core::fmt::Error>().is_none());
}

#[derive(Debug, PartialEq)]
enum IoState {
    NotFound,
}

#[test]
fn with_state_fn_attaches_state() {
    let res: Result<(), io::Error> = Err(io::Error::new(ErrorKind::Other, "boom"));
    let built: Result<(), Error<IoState>> = res.with_state_fn(|| IoState::NotFound).build_error();
    let err = built.unwrap_err();
    assert_eq!(err.state(), Some(&IoState::NotFound));
    assert!(err.find::<io::Error>().is_some());
}

#[test]
fn with_state_fn_evaluates_lazily() {
    let calls = Cell::new(0u32);
    let res: Result<(), io::Error> = Err(io::Error::new(ErrorKind::Other, "boom"));
    let builder = res.with_state_fn(|| {
        calls.set(calls.get() + 1);
        IoState::NotFound
    });
    assert_eq!(calls.get(), 0);
    let built: Result<(), Error<IoState>> = builder.build_error();
    assert_eq!(calls.get(), 1);
    built.unwrap_err();
}

#[test]
fn with_state_fn_ok_short_circuits() {
    let res: Result<u32, io::Error> = Ok(42);
    let built: Result<u32, Error<IoState>> = res.with_state_fn(|| IoState::NotFound).build_error();
    assert_eq!(built.unwrap(), 42);
}

#[test]
fn with_state_fn_keeps_context() {
    let res: Result<(), io::Error> = Err(io::Error::new(ErrorKind::Other, "boom"));
    let built: Result<(), Error<IoState>> = res
        .with_state_fn(|| IoState::NotFound)
        .with_context(TestMessage::HOGE)
        .build_error();
    let err = built.unwrap_err();
    assert_eq!(err.state(), Some(&IoState::NotFound));
    let (_, context, _) = err.into_parts::<TestMessage, TestError>();
    assert_matches!(context, Some(TestMessage::HOGE));
}

fn derive_io_state(err: &io::Error) -> Option<IoState> {
    match err.kind() {
        ErrorKind::NotFound => Some(IoState::NotFound),
        _ => None,
    }
}

#[test]
fn with_state_derived_maps_io_not_found() {
    let res: Result<(), io::Error> = Err(io::Error::new(ErrorKind::NotFound, "no such file"));
    let built: Result<(), Error<IoState>> = res.with_state_derived(derive_io_state).build_error();
    let err = built.unwrap_err();
    assert_eq!(err.state(), Some(&IoState::NotFound));
    assert!(err.find::<std::io::Error>().is_some());
}

#[test]
fn with_state_derived_none_leaves_state_empty() {
    let res: Result<(), io::Error> = Err(io::Error::new(ErrorKind::PermissionDenied, "denied"));
    let built: Result<(), Error<IoState>> = res.with_state_derived(derive_io_state).build_error();
    let err = built.unwrap_err();
    assert!(err.state().is_none());
    assert!(err.find::<std::io::Error>().is_some());
}

#[test]
fn with_state_derived_ok_short_circuits() {
    let res: Result<u32, io::Error> = Ok(7);
    let built: Result<u32, Error<IoState>> = res
        .with_state_derived(|_| Some(IoState::NotFound))
        .build_error();
    assert_eq!(built.unwrap(), 7);
}
