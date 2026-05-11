use std::fmt::{self, Debug, Display};

/// A string literal identified by a zero-sized type.
pub trait Literal: 'static {
    const LITERAL: &'static str;
}

/// A zero-sized context placeholder for [Error][crate::Error].
#[derive(Debug)]
pub struct Blank(#[allow(unused)] [()]);

impl Literal for Blank {
    const LITERAL: &'static str = "";
}

/// Maps a [`Literal`] type to a concrete value and its associated display type.
pub trait Context: Literal {
    type Repr: Display + Debug + Send + Sync + 'static;

    fn new_context() -> Self::Repr;
}

impl<L> Context for L
where
    L: Literal,
{
    type Repr = &'static str;

    fn new_context() -> Self::Repr {
        L::LITERAL
    }
}

impl Context for Blank {
    type Repr = Unit;

    fn new_context() -> Self::Repr {
        Unit
    }
}

/// A zero-sized displayable type used as the [`Context::Repr`] for [`Blank`].
#[derive(Debug)]
pub struct Unit;

impl Display for Unit {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}
