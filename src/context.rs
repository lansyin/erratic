//! Context helpers and traits.
use core::{
    any::Any,
    convert::{self, Infallible},
    fmt::{Debug, Display},
    marker::PhantomData,
    result,
};

use alloc::string::String;

pub enum Value<Repr, Src = Repr> {
    None,
    Literal(&'static str),
    Lazy(fn(Src) -> Repr),
}

/// A trait for types that can be used as an error context.
///
/// Most types implement `Context::Repr = Self` via blanket impl.
pub trait Context: Sized {
    type Alt: Context;
    type Repr: Debug + Display + Send + Sync + 'static;

    const VALUE: Value<Self::Repr, Self>;

    fn try_into_alt(self) -> result::Result<Self::Alt, Self> {
        Err(self)
    }
}

impl<C> Context for C
where
    C: Debug + Display + Send + Sync + 'static,
{
    type Alt = Infallible;
    type Repr = Self;

    const VALUE: Value<Self::Repr> = Value::Lazy(convert::identity);
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
    type Alt = Infallible;
    type Repr = Infallible;

    const VALUE: Value<Self::Repr, Self> = Value::None;
}

/// A trait for types representing string literals.
pub trait Literal {
    const LITERAL: &'static str;
}

/// A lazily evaluated context produced by [`mkctx!`](crate::mkctx).
pub struct Mkctx<L, F = ()> {
    format: F,
    _literal: PhantomData<L>,
}

impl<L, F> Mkctx<L, F> {
    #[doc(hidden)]
    pub const fn __priv_new(format: F) -> Self {
        Self {
            format,
            _literal: PhantomData,
        }
    }
}

impl<L> Mkctx<L> {
    #[doc(hidden)]
    pub const fn __priv_new_const() -> Self {
        Self {
            format: (),
            _literal: PhantomData,
        }
    }
}

impl<L> Context for Mkctx<L>
where
    L: Literal,
{
    type Alt = Infallible;
    type Repr = Infallible;

    const VALUE: Value<Self::Repr, Self> = Value::Literal(L::LITERAL);
}

impl<L, F> Context for Mkctx<L, F>
where
    F: Fn(&mut dyn Any),
    L: Literal,
{
    type Alt = Mkctx<L>;
    type Repr = String;

    const VALUE: Value<Self::Repr, Self> = Value::Lazy(|this| {
        let mut s = String::new();
        (this.format)(&mut s);
        s
    });

    fn try_into_alt(self) -> result::Result<Self::Alt, Self> {
        let mut is_literal = false;
        (self.format)(&mut is_literal);

        if is_literal {
            Ok(Self::Alt {
                format: (),
                _literal: PhantomData,
            })
        } else {
            Err(self)
        }
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

/// A value implementing both `Debug` and `Display`.
pub trait Printable: Debug + Display {}

impl<T> Printable for T where T: Debug + Display {}
