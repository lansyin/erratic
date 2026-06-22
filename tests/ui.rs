use core::fmt::Debug;
use std::fmt;

use erratic::fmt::Formatter;
use erratic::state::FormatWith;
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
    let source_1 = mkerr!("no such fruit").stateless().erase();
    let source_2 = mkerr!(error = source_1).stateless().erase();
    let source_3 = mkerr!(error = source_2, "failed to forage for food")
        .stateless()
        .erase();
    mkerr!(
        error = source_3,
        state = TestState::AppleNotFound,
        context = TestMessage::HOGE,
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

// --- Custom formatter tests ---

/// A custom formatter that prints the error chain with `  └─ ` prefix for each level.
struct Arrow;

impl Formatter for Arrow {
    fn format_debug(
        f: &mut core::fmt::Formatter<'_>,
        context: Option<impl fmt::Debug + fmt::Display>,
        mut source: Option<&(dyn core::error::Error + 'static)>,
        _backtrace: Option<impl fmt::Debug + fmt::Display>,
    ) -> core::fmt::Result {
        let mut errs = context
            .map(|e| format!("{e}"))
            .into_iter()
            .collect::<Vec<_>>();
        while let Some(err) = source {
            errs.push(format!("{}", err));
            source = err.source();
        }
        Debug::fmt(&errs, f)
    }

    fn format_display(
        f: &mut fmt::Formatter<'_>,
        context: Option<impl fmt::Debug + fmt::Display>,
        mut source: Option<&(dyn std::error::Error + 'static)>,
        _backtrace: Option<impl fmt::Debug + fmt::Display>,
    ) -> fmt::Result {
        if let Some(ctx) = context {
            write!(f, "{ctx}")?;
        } else if let Some(err) = source {
            write!(f, "{err}")?;
            source = err.source();
        }
        if f.alternate() {
            while let Some(err) = source {
                source = err.source();
                if source.is_some() {
                    write!(f, "\n├─▶ {err}")?;
                } else {
                    write!(f, "\n└─▶ {err}")?;
                }
            }
        }
        Ok(())
    }
}

#[test]
fn debug_custom_triple() {
    assert_eq!(
        format!(
            "{:?}",
            Error::<FormatWith<Arrow>>::from(generate_triple().erase())
        ),
        include_str!("ui/debug_custom_triple.stderr")
    );
}

#[test]
fn display_custom_triple() {
    assert_eq!(
        format!(
            "{:#}",
            Error::<FormatWith<Arrow>>::from(generate_triple().erase())
        ),
        include_str!("ui/display_custom_triple.stderr")
    );
}
