mod common;

use common::{TestError, TestMessage, TestState};
use erratic::{nae::Nae, *};
use std::{assert_matches, mem, result};

#[test]
fn from_error_round_trip() {
    let err = mkerr!(error = TestError("oops")).stateless();
    let (context, payload, source) = err.into_parts::<TestMessage, TestError>();
    assert_matches!(source, Some(TestError("oops")));
    assert!(payload.is_none());
    assert!(context.is_none());
}

#[test]
fn builder_with_error_builds_correctly() {
    let err = mkerr!(
        error = TestError("oops"),
        context = "context",
        payload = TestMessage("payload"),
    )
    .stateless();
    let (context, payload, source) = err.into_parts::<TestMessage, TestError>();
    assert_matches!(context, Some("context"));
    assert_matches!(source, Some(TestError("oops")));
    assert_matches!(payload, Some(TestMessage("payload")));
}

#[test]
fn builder_case1_error_only_shortcut() {
    let err: Error = Error::with_error(TestError("oops")).into();
    let (context, payload, source) = err.into_parts::<TestMessage, TestError>();
    assert!(context.is_none());
    assert!(payload.is_none());
    assert_matches!(source, Some(TestError("oops")));
}

#[test]
fn builder_case1_state_only_shortcut() {
    let err: Error<TestState> = Error::with_state(TestState::FileNotFound).into();
    assert_eq!(err.extract_state().unwrap().0, TestState::FileNotFound);
}

#[test]
fn builder_case1_context_only_shortcut() {
    let err: Error = Error::with_context(literal!("context only")).into();
    let (context, payload, source) = err.into_parts::<TestMessage, TestError>();
    assert_matches!(context, Some("context only"));
    assert!(payload.is_none());
    assert!(source.is_none());
}

#[test]
fn builder_case2_state_only_to_stateless_shortcut() {
    let err: Error = Error::with_state(TestState::FileNotFound).into();
    assert_eq!(format!("{}", err), "FileNotFound");
    assert_eq!(err.chain().count(), 1);
    assert_eq!(err.into_source().unwrap().to_string(), "FileNotFound");
}

#[test]
fn builder_case3_error_only_to_state_shortcut() {
    let err: Error<TestState> = Error::with_error(TestError("oops")).into();
    let (state, context, payload, source) = err.into_parts::<TestMessage, TestError>();
    assert!(state.is_none());
    assert!(context.is_none());
    assert!(payload.is_none());
    assert_matches!(source, Some(TestError("oops")));
}

#[test]
fn builder_case3_context_only_to_state_shortcut() {
    let err: Error<TestState> = Error::with_context(literal!("context only")).into();
    let (state, context, payload, source) = err.into_parts::<TestMessage, TestError>();
    assert!(state.is_none());
    assert_matches!(context, Some("context only"));
    assert!(payload.is_none());
    assert!(source.is_none());
}

#[test]
fn builder_case4_erratic_state_to_state_shortcut() {
    let inner: Error<TestState> =
        mkerr!(error = TestError("inner"), state = TestState::FileNotFound,);
    let outer: Error<TestState> = Error::with_error(inner).into();
    let (state, context, payload, source) = outer.into_parts::<TestMessage, TestError>();
    assert_eq!(state, Some(TestState::FileNotFound));
    assert!(context.is_none());
    assert!(payload.is_none());
    assert_matches!(source, Some(TestError("inner")));
}

#[test]
fn builder_case6_erratic_stateless_to_state_shortcut() {
    let inner = mkerr!(error = TestError("inner")).stateless();
    let outer: Error<TestState> = Error::with_error(inner).into();
    let (state, context, payload, source) = outer.into_parts::<TestMessage, TestError>();
    assert!(state.is_none());
    assert!(context.is_none());
    assert!(payload.is_none());
    assert_matches!(source, Some(TestError("inner")));
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
fn downcast_source_mut_ok() {
    let mut err = mkerr!(error = TestError("oops")).stateless();
    let source = err.downcast_source_mut::<TestError>().unwrap();
    assert_eq!(source.0, "oops");
    source.0 = "modified";
    assert_eq!(
        err.downcast_source_ref::<TestError>().unwrap().0,
        "modified"
    );
}

#[test]
fn downcast_source_mut_wrong_type() {
    let mut err = mkerr!(error = TestError("oops")).stateless();
    assert!(err.downcast_source_mut::<Nae>().is_none());
}

#[test]
fn erase_makes_opaque() {
    let err = mkerr!(error = TestError("oops")).stateless();
    assert_eq!(format!("{}", err.erase()), "oops");
}

#[test]
fn erase_ref_lifetime() {
    let err = mkerr!(error = TestError("oops")).stateless();
    let opaque: &(dyn std::error::Error + Send + Sync + 'static) = err.erase_ref();
    assert_eq!(format!("{opaque}"), "oops");
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
    assert_eq!(format!("{boxed}"), "oops");
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
        let inner = TestError("inner");
        let outer: Error = Error::from_error(inner);
        assert_eq!(format!("{}", outer), "inner");
        assert_eq!(format!("{}", outer.source().unwrap()), "inner");
        assert_eq!(format!("{:#}", outer), "inner");
    }

    {
        let inner = TestError("inner");
        let outer = mkerr!(error = inner, "outer").stateless();
        assert_eq!(format!("{:#}", outer), "outer\n  -> inner");
    }

    {
        let inner = TestError("inner");
        let mid = mkerr!(error = inner).stateless();
        let outer = mkerr!(error = mid, "outer").stateless();
        assert_eq!(format!("{}", outer), "outer");
        assert_eq!(format!("{}", outer.source().unwrap()), "inner");
        assert_eq!(format!("{}", outer.chain().last().unwrap()), "inner");
        assert_eq!(format!("{:#}", outer), "outer\n  -> inner");
    }

    {
        let inner = TestError("inner");
        let mid = mkerr!(error = inner, "mid").stateless();
        let outer = mkerr!(error = mid, "outer").stateless();
        assert_eq!(format!("{:#}", outer), "outer\n  -> mid\n  -> inner");
    }
}

#[test]
fn eliminate_alloc() {
    {
        let inner = TestError("inner");
        let mid = mkerr!(error = inner, state = TestState::FileNotFound);
        let outer = mkerr!(error = mid) as Error<TestState>;
        assert_eq!(outer.chain().count(), 1);
    }
    {
        let inner = TestError("inner");
        let mid = mkerr!(error = inner).stateless();
        let outer = mkerr!(error = mid) as Error<TestState>;
        assert_eq!(outer.chain().count(), 1);
    }
    {
        let inner = TestError("inner");
        let mid = mkerr!(error = inner, state = TestState::FileNotFound).erase();
        let outer = mkerr!(error = mid) as Error<TestState>;
        assert_eq!(outer.chain().count(), 1);
    }
    {
        let inner = TestError("inner");
        let mid = mkerr!(error = inner).stateless().erase();
        let outer = mkerr!(error = mid) as Error<TestState>;
        assert_eq!(outer.chain().count(), 1);
    }
    {
        let inner = TestError("inner");
        let mid = mkerr!(error = inner).stateless().erase();
        let outer = Error::with_error(mid).build_error();
        assert_eq!(outer.chain().count(), 1);
    }
}
