use erratic::*;

use common::*;

mod common;

fn generate_simple() -> Error<TestState> {
    Error::with_error(TestError("no such device"))
        .with_state(TestState::FileNotFound)
        .with_context(literal!("while opening file"))
        .with_payload(TestMessage("hello.txt".to_owned()))
        .build()
}

fn generate_triple() -> Error<TestState> {
    let source_1 = mkerr!("no such device").stateless().erase();
    let source_2 = Error::with_error(source_1)
        .with_payload("while invoking open")
        .erase_error();
    let source_3 = Error::with_error(source_2)
        .with_payload("while invoking copy_context")
        .erase_error();
    Error::with_error(source_3)
        .with_state(TestState::FileNotFound)
        .with_context(literal!("while copying file"))
        .with_payload(TestMessage("hello.txt".to_owned()))
        .build()
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
        format!("{:?}", generate_simple()),
        include_str!("ui/debug_simple.stderr")
    );
}

#[test]
fn debug_triple() {
    assert_eq!(
        format!("{:?}", generate_triple()),
        include_str!("ui/debug_triple.stderr")
    );
}

#[test]
fn debug_alt_simple() {
    assert_eq!(
        format!("{:#?}", generate_simple()),
        include_str!("ui/debug_alt_simple.stderr")
    );
}

#[test]
fn debug_alt_triple() {
    assert_eq!(
        format!("{:#?}", generate_triple()),
        include_str!("ui/debug_alt_triple.stderr")
    );
}
