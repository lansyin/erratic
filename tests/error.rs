use core::error;

#[cfg(test)]
use erratic::test_artifacts::*;
use erratic::{builder::Builder, *};
use std::{assert_matches, mem, result};

#[test]
fn from_error_round_trip() {
    let err = mkerr!(error = TestError::FOO).stateless();
    let (context, source) = err.into_parts::<TestMessage, TestError>();
    assert_matches!(source, Some(TestError::FOO));
    assert!(context.is_none());
}

#[test]
fn builder_with_error_builds_correctly() {
    let err = mkerr!(error = TestError::FOO, context = TestMessage::HOGE,).stateless();
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
        let err: Error = Builder::with_context(mkctx!("context only")).into();
        assert_eq!(err.chain().count(), 1);
        let (context, source) = err.into_parts::<&'static str, TestError>();
        assert_matches!(context, Some("context only"));
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
        let err: Error<TestState> = Builder::with_context("context only").into();
        let (state, context, source) = err.into_parts::<&str, TestError>();
        assert!(state.is_none());
        assert_eq!(context, Some("context only"));
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
fn builder_case5() {
    // Note: case5 was removed as it has no meaningful use case.
}

#[test]
fn builder_case6() {
    // erratic stateless -> state (fast path)
    {
        let inner = mkerr!(error = TestError::BAR).stateless();
        let outer: Error<TestState> = Builder::with_error(inner).into();
        assert_eq!(outer.chain().count(), 1);
        let (state, context, source) = outer.into_parts::<TestMessage, TestError>();
        assert!(state.is_none());
        assert!(context.is_none());
        assert_matches!(source, Some(TestError::BAR));
    }
    // erratic source + context -> state (no fast path)
    {
        let inner = mkerr!(error = TestError::BAR).stateless();
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
    let inner = mkerr!(error = TestError::BAR).stateless();
    let _: Error<TestState> = Builder::with_error(inner)
        .with_context(TestMessage::HOGE)
        .into();
}

#[test]
fn downcast_source_ok() {
    let err = mkerr!(error = TestError::FOO).stateless();
    assert!(err.has_source_of::<TestError>());
    assert_eq!(err.downcast_source_ref::<TestError>().unwrap().0, "foo");
}

#[test]
fn downcast_source_wrong_type() {
    let err = mkerr!(error = TestError::FOO).stateless();
    assert!(!err.has_source_of::<std::io::Error>());
}

#[test]
fn downcast_source_mut_ok() {
    let mut err = mkerr!(error = TestError::FOO).stateless();
    let source = err.downcast_source_mut::<TestError>().unwrap();
    assert_eq!(source.0, "foo");
    source.0 = "modified";
    assert_eq!(
        err.downcast_source_ref::<TestError>().unwrap().0,
        "modified"
    );
}

#[test]
fn downcast_source_mut_wrong_type() {
    let mut err = mkerr!(error = TestError::FOO).stateless();
    assert!(err.downcast_source_mut::<std::io::Error>().is_none());
}

#[test]
fn erase_makes_opaque() {
    let err = mkerr!(error = TestError::FOO).stateless();
    assert_eq!(format!("{}", err.erase()), "foo");
}

#[test]
fn erase_ref_lifetime() {
    let err = mkerr!(error = TestError::FOO).stateless();
    let opaque: &(dyn std::error::Error + Send + Sync + 'static) = err.erase_ref();
    assert_eq!(format!("{opaque}"), "foo");
}

#[test]
fn into_source_returns_boxed_source() {
    let err = mkerr!(error = TestError::FOO).stateless();
    assert_eq!(err.into_source().unwrap().to_string(), "foo");
}

#[test]
fn into_source_const_is_none() {
    let err = mkerr!(context = "test").stateless();
    assert!(err.into_source().is_none());
}

#[test]
fn chain_wraps_source() {
    let inner = mkerr!(error = TestError::BAR).stateless();
    let outer = mkerr!(error = inner.erase()).stateless();
    let mut chain = outer.chain();
    assert_eq!(chain.next().unwrap().to_string(), "bar");
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
    let inner = mkerr!(error = TestError::BAR).stateless();
    let outer: Error = inner.erase().into();
    assert_eq!(outer.into_source().unwrap().to_string(), "bar");
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
    let err = mkerr!(error = TestError::FOO).stateless();
    let boxed: Box<dyn std::error::Error + Send + Sync + 'static> = err.into();
    assert_eq!(format!("{boxed}"), "foo");
}

#[test]
fn wrap_self() {
    let _: Error = mkerr!(context = "while testing wrap_self");
    let _: Error = mkerr!(context = "while testing wrap_self");
    let _: Error = mkerr!(
        error = mkerr!(context = "while testing wrap_self").stateless(),
        context = "while testing wrap_self with nested error",
    );
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

#[test]
fn dedup_repeated_message_in_chain() {
    {
        let inner = TestError::BAR;
        let outer: Error = Error::from_error(inner);
        assert_eq!(format!("{}", outer), "bar");
        assert_eq!(format!("{}", outer.source().unwrap()), "bar");
        assert_eq!(format!("{:#}", outer), "bar");
    }

    {
        let inner = TestError::BAR;
        let outer = mkerr!(error = inner, "outer").stateless();
        assert_eq!(format!("{:#}", outer), "outer\n  -> bar");
    }

    {
        let inner = TestError::BAR;
        let mid = mkerr!(error = inner).stateless();
        let outer = mkerr!(error = mid, "outer").stateless();
        assert_eq!(format!("{}", outer), "outer");
        assert_eq!(format!("{}", outer.source().unwrap()), "bar");
        assert_eq!(format!("{}", outer.chain().last().unwrap()), "bar");
        assert_eq!(format!("{:#}", outer), "outer\n  -> bar");
    }

    {
        let inner = TestError::BAR;
        let mid = mkerr!(error = inner, "mid").stateless();
        let outer = mkerr!(error = mid, "outer").stateless();
        assert_eq!(format!("{:#}", outer), "outer\n  -> mid\n  -> bar");
    }
}

#[test]
fn eliminate_alloc() {
    {
        let inner = TestError::BAR;
        let mid = mkerr!(error = inner, state = TestState::AppleNotFound);
        let outer = mkerr!(error = mid) as Error<TestState>;
        assert_eq!(outer.chain().count(), 2);
    }
    {
        let inner = TestError::BAR;
        let mid = mkerr!(error = inner).stateless();
        let outer = mkerr!(error = mid) as Error<TestState>;
        assert_eq!(outer.chain().count(), 1);
    }
    {
        let inner = TestError::BAR;
        let mid = mkerr!(error = inner, state = TestState::AppleNotFound).erase();
        let outer = mkerr!(error = mid) as Error<TestState>;
        assert_eq!(outer.chain().count(), 2);
    }
    {
        let inner = TestError::BAR;
        let mid = mkerr!(error = inner).stateless().erase();
        let outer = mkerr!(error = mid) as Error<TestState>;
        assert_eq!(outer.chain().count(), 1);
    }
    {
        let inner = TestError::BAR;
        let mid = mkerr!(error = inner).stateless().erase();
        let outer = Builder::with_error(mid).build_error();
        assert_eq!(outer.chain().count(), 1);
    }
}

#[test]
fn deref_and_deref_mut() {
    let mut err = mkerr!("oops").stateless();
    let _: &dyn error::Error = &*err;
    let _: &mut dyn error::Error = &mut *err;
}

#[cfg(feature = "backtrace")]
#[test]
fn backtrace_captures_from_first_layer() {
    fn inner_most() -> Error {
        mkerr!(error = TestError::BAZ).stateless()
    }
    fn middle() -> Error {
        mkerr!(error = inner_most(), "middle layer").stateless()
    }
    fn outer_most() -> Error {
        mkerr!(error = middle(), "outer layer").stateless()
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

#[test]
fn root_finds_deepest_source() {
    fn inner_most() -> Error {
        mkerr!(error = TestError::BAZ).stateless()
    }
    fn middle() -> Error {
        mkerr!(error = inner_most(), "middle layer").stateless()
    }
    fn outer_most() -> Error {
        mkerr!(error = middle(), "outer layer").stateless()
    }

    let err = outer_most();
    let root = err.root().expect("root should be found");
    assert_eq!(root.to_string(), "baz");
    assert!(root.downcast_ref::<TestError>().is_some());
}

#[test]
fn find_looks_up_error_chain() {
    fn inner_most() -> Error {
        mkerr!(error = TestError::BAZ).stateless()
    }
    fn middle() -> Error {
        mkerr!(error = inner_most(), "middle layer").stateless()
    }
    fn outer_most() -> Error {
        mkerr!(error = middle(), "outer layer").stateless()
    }

    let err = outer_most();

    // Should find TestError (deepest)
    let found = err.find::<TestError>();
    assert!(found.is_some());
    assert_eq!(found.unwrap().0, "baz");

    // Should not find a type not in the chain
    assert!(err.find::<core::fmt::Error>().is_none());
}
