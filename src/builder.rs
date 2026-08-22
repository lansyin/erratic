//! Builder for constructing errors.
use core::{convert::Infallible, error, fmt::Debug};

use crate::{
    BuilderExt, Error, ErrorExt,
    context::{Context, ContextFn, Contextless, Identity, Value},
    match_else,
    raw::RawError,
    state::{State, Stateless},
};

/// An intermediate builder for constructing an [`Error`].
#[derive(Debug)]
pub struct Builder<E, S, F>
where
    F: ContextFn,
    S: State + ?Sized,
{
    err: Option<E>,
    state: Option<S::Repr>,
    context_fn: F,
}

impl Builder<Infallible, Stateless, Identity<Contextless>> {
    /// Starts building an `Error` from a source error.
    pub fn with_error<E>(err: E) -> Builder<E, Stateless, Identity<Contextless>> {
        Builder {
            err: Some(err),
            state: None,
            context_fn: Identity(Contextless::new()),
        }
    }

    /// Starts building an `Error` with a state.
    pub fn with_state<S>(state: S) -> Builder<Infallible, S, Identity<Contextless>>
    where
        S: State,
    {
        Builder {
            err: None,
            state: Some(state.into_repr()),
            context_fn: Identity(Contextless::new()),
        }
    }

    /// Starts building an `Error` with a context.
    pub fn with_context<C>(context: C) -> Builder<Infallible, Stateless, Identity<C>>
    where
        C: Context,
    {
        Builder {
            err: None,
            state: None,
            context_fn: Identity(context),
        }
    }

    /// Starts building an `Error` with a lazily evaluated context.
    ///
    /// The closure `context_fn` is called only when the error is materialized.
    pub fn with_context_fn<F>(context_fn: F) -> Builder<Infallible, Stateless, F>
    where
        F: ContextFn,
    {
        Builder {
            err: None,
            state: None,
            context_fn,
        }
    }
}

impl<E, F> Builder<E, Stateless, F>
where
    F: ContextFn,
{
    /// Converts to a builder of another state without providing the state value.
    pub(crate) fn with_phantom_state<S>(self) -> Builder<E, S, F>
    where
        S: State + ?Sized,
    {
        Builder {
            err: self.err,
            state: None,
            context_fn: self.context_fn,
        }
    }
}

// Builder Case #1: generic error; state -> state
impl<E, S, F> From<Builder<E, S, F>> for Error<S>
where
    F: ContextFn,
    E: error::Error + Send + Sync + 'static,
    S: State + ?Sized,
{
    fn from(value: Builder<E, S, F>) -> Self {
        match (
            value.state,
            value.err,
            !matches!(F::Output::VALUE, Value::None),
        ) {
            (None, None, false) => unreachable!(),
            (None, Some(err), false) => err.into(),
            (state, err, _) => {
                Error::<S>(RawError::from_error(state, err, value.context_fn.call()))
            }
        }
    }
}

// Builder Case #2: generic error; state -> stateless
// Removed as it has no meaningful use case.
// Signature: impl<E, S, F> From<Builder<E, S, F>> for Error

// Builder Case #3: generic error; stateless -> state
impl<E, S, F> From<Builder<E, Stateless, F>> for Error<S>
where
    F: ContextFn,
    E: error::Error + Send + Sync + 'static,
    S: State,
{
    fn from(value: Builder<E, Stateless, F>) -> Self {
        match (value.err, !matches!(F::Output::VALUE, Value::None)) {
            (None, false) => unreachable!(),
            (Some(err), false) => err.into(),
            (err, _) => Error(RawError::from_error(None, err, value.context_fn.call())),
        }
    }
}

// Builder Case #4: erratic error; state+stateless -> state
impl<S, F> From<Builder<Error<S>, Stateless, F>> for Error<S>
where
    F: ContextFn,
    S: State,
{
    fn from(value: Builder<Error<S>, Stateless, F>) -> Self {
        match (value.err, !matches!(F::Output::VALUE, Value::None)) {
            (None, false) => unreachable!(),
            (Some(err), false) => err,
            (None, true) => Error(RawError::from_error(
                None,
                None::<Infallible>,
                value.context_fn.call(),
            )),
            (Some(err), _) => {
                let Ok((state, vacant)) = match_else!(err.extract_state(), Err(err) => {
                    // No state to extract: still attach the context by wrapping the error as a source.
                    return Error(RawError::from_erased(
                        None,
                        Some(err.0.into_erased()),
                        value.context_fn.call(),
                    ));
                });
                vacant.derive(state, value.context_fn.call())
            }
        }
    }
}

// Builder Case #5: erratic error; state -> stateless
// Removed as it has no meaningful use case.
// Signature: impl<S1, S, F, L> From<Builder<Error<S1>, S, F, L>> for Error

// Builder Case #6: erratic error; stateless+stateless -> state; stateless+stateless -> stateless
impl<S, F> From<Builder<Error<Stateless>, Stateless, F>> for Error<S>
where
    F: ContextFn,
    S: State + ?Sized,
{
    fn from(value: Builder<Error<Stateless>, Stateless, F>) -> Self {
        Error(RawError::from_erased(
            None,
            value.err.map(|e| e.0.into_erased()),
            value.context_fn.call(),
        ))
    }
}

// Builder Case #7: generic error; stateless+state -> state
impl<S, F> From<Builder<Error<Stateless>, S, F>> for Error<S>
where
    F: ContextFn,
    S: State,
{
    fn from(value: Builder<Error<Stateless>, S, F>) -> Self {
        Error(RawError::from_erased(
            value.state,
            value.err.map(|e| e.0.into_erased()),
            value.context_fn.call(),
        ))
    }
}

impl<T, E1> BuilderExt for Result<T, E1>
where
    E1: error::Error + Send + Sync + 'static,
{
    type Result<E> = Result<T, E>;

    type E = E1;
    type S = Stateless;
    type F = Identity<Contextless>;

    fn with_state<S>(self, state: S) -> Self::Result<Builder<Self::E, S, Self::F>>
    where
        S: State + Sized,
    {
        self.map_err(|err| Builder {
            err: Some(err),
            state: Some(state.into_repr()),
            context_fn: Identity(Contextless::new()),
        })
    }

    fn with_state_if<S, F>(self, state: S, f: F) -> Self::Result<Builder<Self::E, S, Self::F>>
    where
        S: State + Sized,
        F: FnOnce(&Self::E) -> bool,
    {
        self.map_err(|err| {
            if f(&err) {
                Builder {
                    err: Some(err),
                    state: Some(S::into_repr(state)),
                    context_fn: Identity(Contextless::new()),
                }
            } else {
                Builder {
                    err: Some(err),
                    state: None,
                    context_fn: Identity(Contextless::new()),
                }
            }
        })
    }

    fn with_context_fn<F>(self, context_fn: F) -> Self::Result<Builder<Self::E, Self::S, F>>
    where
        F: ContextFn,
    {
        self.map_err(|err| Builder {
            err: Some(err),
            state: None,
            context_fn,
        })
    }
}

impl<E1, S1, F1> BuilderExt for Builder<E1, S1, F1>
where
    F1: ContextFn,
    S1: State + ?Sized,
{
    type Result<E> = E;

    type E = E1;
    type S = S1;
    type F = F1;

    fn with_state<S>(self, state: S) -> Self::Result<Builder<Self::E, S, Self::F>>
    where
        S: State,
    {
        Builder {
            state: Some(state.into_repr()),
            err: self.err,
            context_fn: self.context_fn,
        }
    }

    fn with_state_if<S, F>(self, state: S, f: F) -> Self::Result<Builder<Self::E, S, Self::F>>
    where
        S: State + Sized,
        F: FnOnce(&Self::E) -> bool,
    {
        if self.err.as_ref().map(f).unwrap_or_default() {
            Builder {
                err: self.err,
                state: Some(S::into_repr(state)),
                context_fn: self.context_fn,
            }
        } else {
            Builder {
                err: self.err,
                state: None,
                context_fn: self.context_fn,
            }
        }
    }

    fn with_context_fn<F>(self, context_fn: F) -> Self::Result<Builder<Self::E, Self::S, F>>
    where
        F: ContextFn,
    {
        Builder {
            err: self.err,
            state: self.state,
            context_fn,
        }
    }
}

impl<T, E1, S1, F1> BuilderExt for Result<T, Builder<E1, S1, F1>>
where
    F1: ContextFn,
    S1: State + ?Sized,
{
    type Result<E> = Result<T, E>;

    type E = E1;
    type S = S1;
    type F = F1;

    fn with_state<S>(self, state: S) -> Self::Result<Builder<Self::E, S, Self::F>>
    where
        S: State,
    {
        self.map_err(|err| Builder {
            state: Some(state.into_repr()),
            err: err.err,
            context_fn: err.context_fn,
        })
    }

    fn with_state_if<S, F>(self, state: S, f: F) -> Self::Result<Builder<Self::E, S, Self::F>>
    where
        S: State + Sized,
        F: FnOnce(&Self::E) -> bool,
    {
        self.map_err(|builder| {
            if builder.err.as_ref().map(f).unwrap_or_default() {
                Builder {
                    err: builder.err,
                    state: Some(S::into_repr(state)),
                    context_fn: builder.context_fn,
                }
            } else {
                Builder {
                    err: builder.err,
                    state: None,
                    context_fn: builder.context_fn,
                }
            }
        })
    }

    fn with_context_fn<F>(self, context_fn: F) -> Self::Result<Builder<Self::E, Self::S, F>>
    where
        F: ContextFn,
    {
        self.map_err(|err| Builder {
            err: err.err,
            state: err.state,
            context_fn,
        })
    }
}

impl<T> BuilderExt for Option<T> {
    type Result<E> = Result<T, E>;

    type E = Infallible;
    type S = Stateless;
    type F = Identity<Contextless>;

    fn with_state<S>(self, state: S) -> Self::Result<Builder<Self::E, S, Self::F>>
    where
        S: State,
    {
        self.ok_or(Builder {
            state: Some(state.into_repr()),
            err: None,
            context_fn: Identity(Contextless::new()),
        })
    }

    /// Note: It's hard to define the semantics of this method on an `Option`, use `with_state` instead.
    fn with_state_if<S, F>(self, state: S, _f: F) -> Self::Result<Builder<Self::E, S, Self::F>>
    where
        S: State + Sized,
        F: FnOnce(&Self::E) -> bool,
    {
        self.ok_or(Builder {
            err: None,
            state: Some(S::into_repr(state)),
            context_fn: Identity(Contextless::new()),
        })
    }

    fn with_context_fn<F>(self, context_fn: F) -> Self::Result<Builder<Self::E, Self::S, F>>
    where
        F: ContextFn,
    {
        self.ok_or(Builder {
            err: None,
            state: None,
            context_fn,
        })
    }
}

impl<E1, S, F> ErrorExt for Builder<E1, S, F>
where
    E1: error::Error + Send + Sync + 'static,
    F: ContextFn,
    S: State + ?Sized,
{
    type Result<E> = E;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.into()
    }
}

impl<S, F> ErrorExt for Builder<Error<Stateless>, S, F>
where
    F: ContextFn,
    S: State,
{
    type Result<E> = E;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.into()
    }
}

impl<F> ErrorExt for Builder<Error<Stateless>, Stateless, F>
where
    F: ContextFn,
{
    type Result<E> = E;
    type S = Stateless;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.into()
    }
}

impl<T, E1, S, F> ErrorExt for Result<T, Builder<E1, S, F>>
where
    E1: error::Error + Send + Sync + 'static,
    F: ContextFn,
    S: State + ?Sized,
{
    type Result<E> = Result<T, E>;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.map_err(Error::from)
    }
}

impl<T, S, F> ErrorExt for Result<T, Builder<Error<Stateless>, S, F>>
where
    F: ContextFn,
    S: State,
{
    type Result<E> = Result<T, E>;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.map_err(|err| err.build_error())
    }
}

impl<T, F> ErrorExt for Result<T, Builder<Error<Stateless>, Stateless, F>>
where
    F: ContextFn,
{
    type Result<E> = Result<T, E>;
    type S = Stateless;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.map_err(|err| err.build_error())
    }
}
