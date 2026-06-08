//! Context traits and its placeholder for [`Builder`][crate::Builder].
use core::fmt::{self, Debug, Display};

use alloc::string::String;

/// A displayable value that can be attached to an [`Error`](crate::Error) as context.
pub trait Context: Sized {
    const FALLBACK: Option<&'static str> = None;

    type Repr: Display + Send + Sync + 'static;

    fn try_into_repr(self) -> Option<Self::Repr>;
}

impl<C> Context for C
where
    C: Display + Send + Sync + 'static,
{
    const FALLBACK: Option<&'static str> = None;

    type Repr = C;

    fn try_into_repr(self) -> Option<Self::Repr> {
        Some(self)
    }
}

/// A zero-sized context placeholder for [Builder][crate::Builder].
#[derive(Debug)]
pub struct Contextless {
    _priv: (),
}

impl Contextless {
    pub(crate) const fn new() -> Self {
        Self { _priv: () }
    }
}

impl Context for Contextless {
    type Repr = Blank;

    fn try_into_repr(self) -> Option<Self::Repr> {
        None
    }
}

/// A zero-sized display placeholder for [`Contextless`].
#[derive(Debug)]
pub struct Blank {
    _priv: (),
}

impl Blank {
    pub(crate) const fn new() -> Self {
        Self { _priv: () }
    }
}

impl Display for Blank {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

/// A marker trait for zero-sized types that represent a `&'static str` literal.
pub trait Literal {
    const LITERAL: &'static str;
}

/// A lazily evaluated context produced by [`mkctx!`](crate::mkctx).
pub struct Mkctx<F, L>
where
    F: FnOnce() -> Option<String>,
{
    format: F,
    _literal: L,
}

impl<F, L> Mkctx<F, L>
where
    F: FnOnce() -> Option<String>,
{
    #[doc(hidden)]
    pub const fn __priv_new(format: F, _literal: L) -> Self {
        Self { format, _literal }
    }
}

impl<F, L> Context for Mkctx<F, L>
where
    F: FnOnce() -> Option<String>,
    L: Literal,
{
    const FALLBACK: Option<&'static str> = Some(L::LITERAL);

    type Repr = String;

    fn try_into_repr(self) -> Option<Self::Repr> {
        (self.format)()
    }
}

/// A trait for types that can [produce a context][crate::BuilderExt::with_context_fn].
pub trait IntoContext {
    type Output: Context;

    fn into_context(self) -> Self::Output;
}

impl<T, R> IntoContext for T
where
    T: FnOnce() -> R,
    R: Context,
{
    type Output = R;

    fn into_context(self) -> Self::Output {
        self()
    }
}

/// A wrapper that can be used as [`IntoContext`].
#[derive(Debug)]
pub struct Identity<C>(pub C)
where
    C: Context;

impl<C> IntoContext for Identity<C>
where
    C: Context,
{
    type Output = C;

    fn into_context(self) -> Self::Output {
        self.0
    }
}
