//! Builder for constructing errors.
use core::{convert::Infallible, error, fmt::Debug, result};

use crate::{
    BuilderExt, Error, ErrorExt,
    context::{Context, ContextFn, Contextless, Identity},
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
    pub err: Option<E>,
    pub state: Option<S::Repr>,
    pub context_fn: F,
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

// Builder Case #1: generic error; state -> state
impl<E, S, F> From<Builder<E, S, F>> for Error<S>
where
    F: ContextFn,
    E: error::Error + Send + Sync + 'static,
    S: State + ?Sized,
{
    fn from(value: Builder<E, S, F>) -> Self {
        match (value.state, value.err, F::Output::is_contextless()) {
            (None, None, true) => unreachable!(),
            (None, Some(err), true) => err.into(),
            (state, err, _) => Error::<S>(RawError::new(state, err, value.context_fn.call())),
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
        match (value.err, F::Output::is_contextless()) {
            (None, true) => unreachable!(),
            (Some(err), true) => err.into(),
            (err, _) => Error(RawError::new(None, err, value.context_fn.call())),
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
        match (value.err, F::Output::is_contextless()) {
            (None, true) => unreachable!(),
            (Some(err), true) => err,
            (None, false) => Error(RawError::new(
                None,
                None::<Infallible>,
                value.context_fn.call(),
            )),
            (Some(err), _) => {
                let Ok((state, vacant)) = match_else!(err.extract_state(), Err(err) => {
                    return err.with_phantom_state();
                });
                vacant.derive(state, value.context_fn.call())
            }
        }
    }
}

// Builder Case #5: erratic error; state -> stateless
// Removed as it has no meaningful use case.
// Signature: impl<S1, S, F, L> From<Builder<Error<S1>, S, F, L>> for Error

// Builder Case #6: erratic error; stateless+stateless -> state
impl<S, F> From<Builder<Error<Stateless>, Stateless, F>> for Error<S>
where
    F: ContextFn,
    S: State + ?Sized,
{
    fn from(value: Builder<Error<Stateless>, Stateless, F>) -> Self {
        match (value.err, F::Output::is_contextless()) {
            (None, true) => unreachable!(),
            (Some(err), true) => err.with_phantom_state(),
            (None, false) => Error(RawError::new(
                None,
                None::<Infallible>,
                value.context_fn.call(),
            )),
            (Some(err), false) => Error(RawError::new(
                None,
                Some(err.erase()),
                value.context_fn.call(),
            )),
        }
    }
}

impl<T, E1> BuilderExt for result::Result<T, E1>
where
    E1: error::Error + Send + Sync + 'static,
{
    type Result<E> = result::Result<T, E>;

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

impl<T, E1, S1, F1> BuilderExt for result::Result<T, Builder<E1, S1, F1>>
where
    F1: ContextFn,
    S1: State + ?Sized,
{
    type Result<E> = result::Result<T, E>;

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
    type Result<E> = result::Result<T, E>;

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

    fn erase_error(self) -> Self::Result<impl error::Error + Send + Sync + 'static> {
        self.build_error().erase()
    }
}

impl<S1, S, F> ErrorExt for Builder<Error<S1>, S, F>
where
    S1: State + ?Sized,
    F: ContextFn,
    S: State + ?Sized,
{
    type Result<E> = E;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        Builder {
            err: self.err.map(|e| e.erase()),
            state: self.state,
            context_fn: self.context_fn,
        }
        .build_error()
    }

    fn erase_error(self) -> Self::Result<impl error::Error + Send + Sync + 'static> {
        self.build_error().erase()
    }
}

impl<T, E1, S, F> ErrorExt for result::Result<T, Builder<E1, S, F>>
where
    E1: error::Error + Send + Sync + 'static,
    F: ContextFn,
    S: State + ?Sized,
{
    type Result<E> = result::Result<T, E>;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.map_err(Error::from)
    }

    fn erase_error(self) -> Self::Result<impl error::Error + Send + Sync + 'static> {
        self.build_error().map_err(|err| err.erase())
    }
}

impl<T, S1, S, F> ErrorExt for result::Result<T, Builder<Error<S1>, S, F>>
where
    S1: State + ?Sized,
    F: ContextFn,
    S: State + ?Sized,
{
    type Result<E> = result::Result<T, E>;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.map_err(|err| {
            Builder {
                err: err.err.map(|err| err.erase()),
                state: err.state,
                context_fn: err.context_fn,
            }
            .build_error()
        })
    }

    fn erase_error(self) -> Self::Result<impl error::Error + Send + Sync + 'static> {
        self.build_error().map_err(|err| err.erase())
    }
}
