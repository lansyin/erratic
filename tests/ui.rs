use erratic::*;

use common::*;

mod common;

fn generate_simple() -> Error<TestState> {
    mkerr!(
        error = TestError("no such device"),
        state = TestState::FileNotFound,
        context = "while opening file: ",
        payload = TestMessage("hello.txt")
    )
}

fn generate_triple() -> Error<TestState> {
    let source_1 = mkerr!("no such device").stateless().erase();
    let source_2 = mkerr!(error = source_1).stateless().erase();
    let source_3 = mkerr!(error = source_2, "while invoking copy_context")
        .stateless()
        .erase();
    mkerr!(
        error = source_3,
        state = TestState::FileNotFound,
        context = "while copying file: ",
        payload = TestMessage("hello.txt")
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
