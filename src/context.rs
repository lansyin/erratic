//! Context helpers and traits.
use core::fmt::{self, Debug, Display};

use alloc::string::String;

use crate::rtti;

mod sealed {
    pub trait Sealed {}
}

/// A trait for types that can be used as an error context.
///
/// Most types implement `Context::Repr = Self` via blanket impl.
pub trait Context: sealed::Sealed + Sized {
    const FALLBACK: Option<&'static str> = None;

    type Repr: Debug + Display + Send + Sync + 'static;

    fn try_into_repr(self) -> Option<Self::Repr>;

    fn is_contextless() -> bool
    where
        Self: Sized,
    {
        rtti::is_same_ty::<Self::Repr, Empty>()
    }
}

impl<C> sealed::Sealed for C where C: Debug + Display + Send + Sync + 'static {}

impl<C> Context for C
where
    C: Debug + Display + Send + Sync + 'static,
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

impl sealed::Sealed for Contextless {}

impl Context for Contextless {
    type Repr = Empty;

    fn try_into_repr(self) -> Option<Self::Repr> {
        None
    }
}

/// A zero-sized type used as the context storage for [`Contextless`].
#[derive(Debug)]
pub struct Empty {
    _priv: (),
}

impl Empty {
    pub(crate) const fn new() -> Self {
        Self { _priv: () }
    }
}

impl Display for Empty {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

/// A trait for types representing string literals.
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

impl<F, L> sealed::Sealed for Mkctx<F, L>
where
    F: FnOnce() -> Option<String>,
    L: Literal,
{
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
pub trait ContextFn {
    type Output: Context;

    fn call(self) -> Self::Output;
}

impl<T, R> ContextFn for T
where
    T: FnOnce() -> R,
    R: Context,
{
    type Output = R;

    fn call(self) -> Self::Output {
        self()
    }
}

/// A wrapper that wraps values as [`ContextFn`].
#[derive(Debug)]
pub struct Identity<C>(pub C)
where
    C: Context;

impl<C> ContextFn for Identity<C>
where
    C: Context,
{
    type Output = C;

    fn call(self) -> Self::Output {
        self.0
    }
}
