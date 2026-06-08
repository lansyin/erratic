use core::assert_matches;
use erratic::*;

#[test]
fn from_context_creates_const() {
    let mut err = mkerr!("oops").stateless();

    assert_matches!(err.downcast_context_ref::<&'static str>(), Some(&"oops"));
    if cfg!(not(feature = "backtrace")) {
        assert_matches!(err.downcast_context_mut::<&'static str>(), None);
    }
}
