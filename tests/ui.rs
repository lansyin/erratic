use erratic::test_fixtures::*;
use erratic::*;

fn generate_simple() -> Error<TestState> {
    mkerr!(
        error = TestError::FOO,
        state = TestState::AppleNotFound,
        context = TestMessage::HOGE,
    )
}

fn generate_triple() -> Error<TestState> {
    let source_1 = TestError::FOO;
    let source_2 = mkerr!(error = source_1).stateless().erase_state();
    let source_3 = mkerr!(error = source_2, context = TestMessage::HOGE)
        .stateless()
        .erase_state();
    mkerr!(
        error = source_3,
        state = TestState::AppleNotFound,
        context = TestMessage::FUGA,
    )
}

#[test]
fn display_simple() {
    assert_eq!(
        format!("{}", generate_simple()),
        include_str!("ui/display_simple.stderr")
    );
}

#[test]
fn display_triple() {
    assert_eq!(
        format!("{}", generate_triple()),
        include_str!("ui/display_triple.stderr")
    );
}

#[test]
fn display_alt_simple() {
    assert_eq!(
        format!("{:#}", generate_simple()),
        include_str!("ui/display_alt_simple.stderr")
    );
}

#[test]
fn display_alt_triple() {
    assert_eq!(
        format!("{:#}", generate_triple()),
        include_str!("ui/display_alt_triple.stderr")
    );
}

#[test]
fn debug_simple() {
    assert_eq!(
        format!("{:-?}", generate_simple()),
        include_str!("ui/debug_simple.stderr")
    );
}

#[test]
fn debug_triple() {
    assert_eq!(
        format!("{:-?}", generate_triple()),
        include_str!("ui/debug_triple.stderr")
    );
}

#[test]
fn debug_alt_simple() {
    assert_eq!(
        format!("{:-#?}", generate_simple()),
        include_str!("ui/debug_alt_simple.stderr")
    );
}

#[test]
fn debug_alt_triple() {
    assert_eq!(
        format!("{:-#?}", generate_triple()),
        include_str!("ui/debug_alt_triple.stderr")
    );
}
