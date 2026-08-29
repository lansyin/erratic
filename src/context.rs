//! Auxiliary types and traits for context.
//!
//! You rarely need to use any type in this module directly: to attach a context to an error,
//! just use the following extension methods:
//!
//! - [`with_context`][crate::BuilderExt::with_context]
//! - [`with_context_fn`][crate::BuilderExt::with_context_fn]
use core::{
    convert::{self, Infallible},
    fmt::{Debug, Display},
    marker::PhantomData,
    result,
};

use alloc::{borrow::ToOwned, string::String};

/// A compile-time context recipe.
///
/// See [`Context`].
pub enum Value<C: Context> {
    None,
    Literal(&'static str),
    Lazy(fn(C) -> C::Repr),
}

/// A trait for types that can be used as an error context.
///
/// Most types implement `Context::Repr = Self` via the built-in impl.
///
/// # See also
///
/// - [`with_context`][crate::BuilderExt::with_context]
/// - [`with_context_fn`][crate::BuilderExt::with_context_fn]
pub trait Context: Sized {
    type Alt: Context<Alt = Infallible>;
    type Repr: Debug + Display + Send + Sync + 'static;

    const VALUE: Value<Self>;

    fn select(self) -> result::Result<Self::Alt, Unique<Self>> {
        Err(Unique(self))
    }
}

/// A context with its alternatives explicitly excluded.
///
/// See [`Context`].
pub struct Unique<C: Context>(C);

impl<C> Context for Unique<C>
where
    C: Context,
{
    type Alt = Infallible;

    type Repr = C::Repr;

    const VALUE: Value<Self> = match C::VALUE {
        Value::None => Value::None,
        Value::Literal(context) => Value::Literal(context),
        Value::Lazy(_f) => Value::Lazy(|context: Self| -> Self::Repr {
            // Note: Re-read the value as we cannot capture `_f` in const context.
            let Value::Lazy(f) = C::VALUE else {
                unreachable!()
            };
            f(context.0)
        }),
    };
}

impl<C> Context for C
where
    C: Debug + Display + Send + Sync + 'static,
{
    type Alt = Infallible;
    type Repr = Self;

    const VALUE: Value<Self> = Value::Lazy(convert::identity);
}

/// A zero-sized context placeholder.
///
/// See [`BuilderExt`][crate::BuilderExt].
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

    const VALUE: Value<Self> = Value::None;
}

/// A trait for types representing a string literal.
///
/// # See also
/// - [`mkctx!`](crate::mkctx)
/// - [`mkerr!`](crate::mkerr)
/// - [`mkres!`](crate::mkres)
/// - [`mksure!`](crate::mksure)
pub trait Literal {
    const LITERAL: &'static str;
}

/// A lazily-evaluated and literal-aware context.
///
/// See [`mkctx!`](crate::mkctx).
pub struct Mkctx<L, F = ()> {
    context: MaybeEvaluated<F>,
    _literal: PhantomData<L>,
}

enum MaybeEvaluated<F> {
    Lazy(F),
    Evaluated(Option<String>),
}

impl<L, F> Mkctx<L, F> {
    #[doc(hidden)]
    pub const fn __priv_new(format: F) -> Self {
        Self {
            context: MaybeEvaluated::Lazy(format),
            _literal: PhantomData,
        }
    }
}

impl<L> Mkctx<L> {
    #[doc(hidden)]
    pub const fn __priv_new_const() -> Self {
        Self {
            context: MaybeEvaluated::Evaluated(None),
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

    const VALUE: Value<Self> = Value::Literal(L::LITERAL);
}

impl<L, F> Context for Mkctx<L, F>
where
    F: FnOnce() -> Option<String>,
    L: Literal,
{
    type Alt = Mkctx<L>;
    type Repr = String;

    const VALUE: Value<Self> = Value::Lazy(|this| {
        match this.context {
            MaybeEvaluated::Lazy(format) => {
                if let Some(context) = format() {
                    return context;
                }
            }
            MaybeEvaluated::Evaluated(evaluated) => {
                if let Some(context) = evaluated {
                    return context;
                }
            }
        }
        match Self::Alt::VALUE {
            Value::Literal(context) => context.to_owned(),
            Value::Lazy(_) | Value::None => unreachable!(),
        }
    });

    fn select(mut self) -> result::Result<Self::Alt, Unique<Self>> {
        self.context = match self.context {
            context @ MaybeEvaluated::Evaluated(_) => context,
            MaybeEvaluated::Lazy(format) => MaybeEvaluated::Evaluated(format()),
        };

        if matches!(self.context, MaybeEvaluated::Evaluated(None)) {
            Ok(Self::Alt {
                context: MaybeEvaluated::Evaluated(None),
                _literal: PhantomData,
            })
        } else {
            Err(Unique(self))
        }
    }
}

/// A potentially lazily-evaluated context.
///
/// See [`with_context_fn`][crate::BuilderExt::with_context_fn].
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
///
/// See [`with_context`][crate::BuilderExt::with_context].
#[derive(Debug)]
pub struct Identity<C>(C)
where
    C: Context;

impl<C> Identity<C>
where
    C: Context,
{
    pub(crate) fn new(context: C) -> Self {
        Self(context)
    }
}

impl Identity<Contextless> {
    pub(crate) fn contextless() -> Self {
        Self(Contextless::new())
    }
}

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
///
/// See [`context`][crate::Error::context].
pub trait Printable: Debug + Display {}

impl<T> Printable for T where T: Debug + Display {}
