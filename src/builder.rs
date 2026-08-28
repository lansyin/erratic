//! Auxiliary types and traits for combinators.
//!
//! Disclaimer: Types in this module should not be used directly. They are auxiliary and may change
//! without a major version bump.
use core::{convert::Infallible, error, fmt::Debug, marker::PhantomData};

use crate::{
    BuilderExt, DeriveExt, Error, ErrorExt,
    context::{self, Context, ContextFn, Contextless, Value},
    match_else,
    raw::RawError,
    state::{self, Derive, Lazy, State, StateFn, Stateless},
};

/// A placeholder type for builders without a source error.
///
// Note: `Infallible` was used to denote the absence of a source error, since it implements
// `error::Error` and thus satisfies the generic `Builder`'s `E: Error` bound. But with the
// introduction of `DeriveExt::with_state_derived`, it became possible to create an empty builder:
// ```
// let _empty = None::<()>.with_state(0).with_state_derived(|_| None);
// ```
// To prevent that, we introduced `Errorless`, which deliberately excludes errorless builders
// from implementing `DeriveExt`.
pub enum Errorless {}

/// An intermediate builder for constructing an [`Error`].
#[derive(Debug)]
pub struct Builder<E, Sf, S, C>
where
    C: ContextFn,
    Sf: StateFn<E, S>,
    S: State + ?Sized,
{
    err: Option<E>,
    state: Sf,
    _state_ty: PhantomData<S>,
    context_fn: C,
}

impl Builder<Errorless, state::Identity<Stateless>, Stateless, context::Identity<Contextless>> {
    /// Starts building an `Error` from a source error.
    pub fn with_error<E>(
        err: E,
    ) -> Builder<E, state::Identity<Stateless>, Stateless, context::Identity<Contextless>> {
        Builder {
            err: Some(err),
            state: state::Identity::phantom(),
            _state_ty: PhantomData,
            context_fn: context::Identity::contextless(),
        }
    }

    /// Starts building an `Error` with a state.
    pub fn with_state<S>(
        state: S,
    ) -> Builder<Errorless, state::Identity<S>, S, context::Identity<Contextless>>
    where
        S: State,
    {
        Builder {
            err: None,
            state: state::Identity::new(state),
            _state_ty: PhantomData,
            context_fn: context::Identity::contextless(),
        }
    }

    pub fn with_state_fn<F, S>(
        f: F,
    ) -> Builder<Errorless, Lazy<F, S>, S, context::Identity<Contextless>>
    where
        S: State,
        F: FnOnce() -> S,
    {
        Builder {
            err: None,
            state: Lazy::new(f),
            _state_ty: PhantomData,
            context_fn: context::Identity::contextless(),
        }
    }

    /// Starts building an `Error` with a context.
    pub fn with_context<C>(
        context: C,
    ) -> Builder<Errorless, state::Identity<Stateless>, Stateless, context::Identity<C>>
    where
        C: Context,
    {
        Builder {
            err: None,
            state: state::Identity::phantom(),
            _state_ty: PhantomData,
            context_fn: context::Identity::new(context),
        }
    }

    /// Starts building an `Error` with a lazily-evaluated context.
    ///
    /// The closure `context_fn` is called only when the error is materialized.
    pub fn with_context_fn<F>(
        context_fn: F,
    ) -> Builder<Errorless, state::Identity<Stateless>, Stateless, F>
    where
        F: ContextFn,
    {
        Builder {
            err: None,
            state: state::Identity::phantom(),
            _state_ty: PhantomData,
            context_fn,
        }
    }
}

// Builder Case #1: generic error; state -> state
impl<E, Sf, S, C> From<Builder<E, Sf, S, C>> for Error<S>
where
    E: error::Error + Send + Sync + 'static,
    C: ContextFn,
    Sf: StateFn<E, S>,
    S: State + ?Sized,
{
    fn from(value: Builder<E, Sf, S, C>) -> Self {
        let state = value.state.call(value.err.as_ref());
        match (state, value.err, !matches!(C::Output::VALUE, Value::None)) {
            (None, None, false) => unreachable!(),
            (None, Some(err), false) => err.into(),
            (state, err, _) => {
                Error::<S>(RawError::from_error(state, err, value.context_fn.call()))
            }
        }
    }
}

// Companion impl for Builder Case #1.
impl<Sf, S, C> From<Builder<Errorless, Sf, S, C>> for Error<S>
where
    C: ContextFn,
    Sf: StateFn<Errorless, S>,
    S: State + ?Sized,
{
    fn from(value: Builder<Errorless, Sf, S, C>) -> Self {
        let state = value.state.call(value.err.as_ref());
        match (state, value.err, !matches!(C::Output::VALUE, Value::None)) {
            (None, None, false) => unreachable!(),
            (state, None, _) => Error::<S>(RawError::from_error(
                state,
                None::<Infallible>,
                value.context_fn.call(),
            )),
        }
    }
}

// Builder Case #2: generic error; state -> stateless
// Removed as it has no meaningful use case.
// Signature: impl<E, S, C> From<Builder<E, S, C>> for Error

// Builder Case #3: generic error; stateless -> state
impl<E, S, C> From<Builder<E, state::Identity<Stateless>, Stateless, C>> for Error<S>
where
    C: ContextFn,
    E: error::Error + Send + Sync + 'static,
    S: State,
{
    fn from(value: Builder<E, state::Identity<Stateless>, Stateless, C>) -> Self {
        match (value.err, !matches!(C::Output::VALUE, Value::None)) {
            (None, false) => unreachable!(),
            (Some(err), false) => err.into(),
            (err, _) => Error(RawError::from_error(None, err, value.context_fn.call())),
        }
    }
}

// Companion impl for Builder Case #3.
impl<S, C> From<Builder<Errorless, state::Identity<Stateless>, Stateless, C>> for Error<S>
where
    C: ContextFn,
    S: State,
{
    fn from(value: Builder<Errorless, state::Identity<Stateless>, Stateless, C>) -> Self {
        match (value.err, !matches!(C::Output::VALUE, Value::None)) {
            (None, false) => unreachable!(),
            (None, true) => Error(RawError::from_error(
                None,
                None::<Infallible>,
                value.context_fn.call(),
            )),
        }
    }
}

// Builder Case #4: erratic error; state+stateless -> state
impl<S, C> From<Builder<Error<S>, state::Identity<Stateless>, Stateless, C>> for Error<S>
where
    C: ContextFn,
    S: State,
{
    fn from(value: Builder<Error<S>, state::Identity<Stateless>, Stateless, C>) -> Self {
        match (value.err, !matches!(C::Output::VALUE, Value::None)) {
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
// Signature: impl<S1, S, C, L> From<Builder<Error<S1>, S, C, L>> for Error

// Builder Case #6: erratic error; stateless+stateless -> state; stateless+stateless -> stateless
impl<S, C> From<Builder<Error, state::Identity<Stateless>, Stateless, C>> for Error<S>
where
    C: ContextFn,
    S: State + ?Sized,
{
    fn from(value: Builder<Error, state::Identity<Stateless>, Stateless, C>) -> Self {
        Error(RawError::from_erased(
            None,
            value.err.map(|e| e.0.into_erased()),
            value.context_fn.call(),
        ))
    }
}

// Builder Case #7: erratic error; stateless+state -> state
impl<Sf, S, C> From<Builder<Error, Sf, S, C>> for Error<S>
where
    C: ContextFn,
    Sf: StateFn<Error, S>,
    S: State,
{
    fn from(value: Builder<Error, Sf, S, C>) -> Self {
        let state = value.state.call(value.err.as_ref());
        Error(RawError::from_erased(
            state,
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
    type Sf = state::Identity<Stateless>;
    type S = Stateless;
    type C = context::Identity<Contextless>;

    fn with_context_fn<F>(
        self,
        context_fn: F,
    ) -> Self::Result<Builder<Self::E, Self::Sf, Self::S, F>>
    where
        F: ContextFn,
    {
        self.map_err(|err| Builder {
            err: Some(err),
            state: state::Identity::phantom(),
            _state_ty: PhantomData,
            context_fn,
        })
    }

    fn with_state<S>(
        self,
        state: S,
    ) -> Self::Result<Builder<Self::E, state::Identity<S>, S, Self::C>>
    where
        S: State,
    {
        self.map_err(|err| Builder {
            err: Some(err),
            state: state::Identity::new(state),
            _state_ty: PhantomData,
            context_fn: context::Identity::contextless(),
        })
    }

    fn with_state_fn<F, S>(self, f: F) -> Self::Result<Builder<Self::E, Lazy<F, S>, S, Self::C>>
    where
        F: FnOnce() -> S,
        S: State + Sized,
    {
        self.map_err(|err| Builder {
            err: Some(err),
            state: Lazy::new(f),
            _state_ty: PhantomData,
            context_fn: context::Identity::contextless(),
        })
    }
}

impl<E1, Sf, S1, C1> BuilderExt for Builder<E1, Sf, S1, C1>
where
    C1: ContextFn,
    Sf: StateFn<E1, S1>,
    S1: State + ?Sized,
{
    type Result<E> = E;

    type E = E1;
    type Sf = Sf;
    type S = S1;
    type C = C1;

    fn with_context_fn<F>(
        self,
        context_fn: F,
    ) -> Self::Result<Builder<Self::E, Self::Sf, Self::S, F>>
    where
        F: ContextFn,
    {
        Builder {
            err: self.err,
            state: self.state,
            _state_ty: self._state_ty,
            context_fn,
        }
    }

    fn with_state<S>(
        self,
        state: S,
    ) -> Self::Result<Builder<Self::E, state::Identity<S>, S, Self::C>>
    where
        S: State,
    {
        Builder {
            state: state::Identity::new(state),
            _state_ty: PhantomData,
            err: self.err,
            context_fn: self.context_fn,
        }
    }

    fn with_state_fn<F, S>(self, f: F) -> Self::Result<Builder<Self::E, Lazy<F, S>, S, Self::C>>
    where
        F: FnOnce() -> S,
        S: State + Sized,
    {
        Builder {
            state: Lazy::new(f),
            _state_ty: PhantomData,
            err: self.err,
            context_fn: self.context_fn,
        }
    }
}

impl<T, E1, Sf, S1, C1> BuilderExt for Result<T, Builder<E1, Sf, S1, C1>>
where
    C1: ContextFn,
    Sf: StateFn<E1, S1>,
    S1: State + ?Sized,
{
    type Result<E> = Result<T, E>;

    type E = E1;
    type Sf = Sf;
    type S = S1;
    type C = C1;

    fn with_context_fn<F>(
        self,
        context_fn: F,
    ) -> Self::Result<Builder<Self::E, Self::Sf, Self::S, F>>
    where
        F: ContextFn,
    {
        self.map_err(|builder| Builder {
            err: builder.err,
            state: builder.state,
            _state_ty: builder._state_ty,
            context_fn,
        })
    }

    fn with_state<S>(
        self,
        state: S,
    ) -> Self::Result<Builder<Self::E, state::Identity<S>, S, Self::C>>
    where
        S: State,
    {
        self.map_err(|builder| Builder {
            state: state::Identity::new(state),
            _state_ty: PhantomData,
            err: builder.err,
            context_fn: builder.context_fn,
        })
    }

    fn with_state_fn<F, S>(self, f: F) -> Self::Result<Builder<Self::E, Lazy<F, S>, S, Self::C>>
    where
        F: FnOnce() -> S,
        S: State + Sized,
    {
        self.map_err(|builder| Builder {
            state: Lazy::new(f),
            _state_ty: PhantomData,
            err: builder.err,
            context_fn: builder.context_fn,
        })
    }
}

impl<T> BuilderExt for Option<T> {
    type Result<E> = Result<T, E>;

    type E = Errorless;
    type Sf = state::Identity<Stateless>;
    type S = Stateless;
    type C = context::Identity<Contextless>;

    fn with_context_fn<F>(
        self,
        context_fn: F,
    ) -> Self::Result<Builder<Self::E, Self::Sf, Self::S, F>>
    where
        F: ContextFn,
    {
        self.ok_or(Builder {
            err: None,
            state: state::Identity::phantom(),
            _state_ty: PhantomData,
            context_fn,
        })
    }

    fn with_state<S>(
        self,
        state: S,
    ) -> Self::Result<Builder<Self::E, state::Identity<S>, S, Self::C>>
    where
        S: State,
    {
        self.ok_or(Builder {
            state: state::Identity::new(state),
            _state_ty: PhantomData,
            err: None,
            context_fn: context::Identity::contextless(),
        })
    }

    fn with_state_fn<F, S>(self, f: F) -> Self::Result<Builder<Self::E, Lazy<F, S>, S, Self::C>>
    where
        F: FnOnce() -> S,
        S: State + Sized,
    {
        self.ok_or(Builder {
            state: Lazy::new(f),
            _state_ty: PhantomData,
            err: None,
            context_fn: context::Identity::contextless(),
        })
    }
}

impl<T, E1> DeriveExt for Result<T, E1>
where
    E1: error::Error + Send + Sync + 'static,
{
    type Result<E> = Result<T, E>;

    type E = E1;
    type C = context::Identity<Contextless>;

    fn with_state_derived<F, S>(
        self,
        f: F,
    ) -> Self::Result<Builder<Self::E, crate::state::Derive<F, Self::E, S>, S, Self::C>>
    where
        S: State,
        F: FnOnce(&Self::E) -> Option<S>,
    {
        self.map_err(|err| Builder {
            err: Some(err),
            state: Derive::new(f),
            _state_ty: PhantomData,
            context_fn: context::Identity::contextless(),
        })
    }
}

impl<E1, Sf, S1, C1> DeriveExt for Builder<E1, Sf, S1, C1>
where
    E1: error::Error,
    C1: ContextFn,
    Sf: StateFn<E1, S1>,
    S1: State + ?Sized,
{
    type Result<E> = E;

    type E = E1;
    type C = C1;

    fn with_state_derived<F, S>(
        self,
        f: F,
    ) -> Self::Result<Builder<Self::E, Derive<F, Self::E, S>, S, Self::C>>
    where
        S: State,
        F: FnOnce(&Self::E) -> Option<S>,
    {
        Builder {
            err: self.err,
            state: Derive::new(f),
            _state_ty: PhantomData,
            context_fn: self.context_fn,
        }
    }
}

impl<T, E1, Sf, S1, C1> DeriveExt for Result<T, Builder<E1, Sf, S1, C1>>
where
    E1: error::Error,
    C1: ContextFn,
    Sf: StateFn<E1, S1>,
    S1: State + ?Sized,
{
    type Result<E> = Result<T, E>;

    type E = E1;
    type C = C1;

    fn with_state_derived<F, S>(
        self,
        f: F,
    ) -> Self::Result<Builder<Self::E, Derive<F, Self::E, S>, S, Self::C>>
    where
        S: State,
        F: FnOnce(&Self::E) -> Option<S>,
    {
        self.map_err(|builder| Builder {
            err: builder.err,
            state: Derive::new(f),
            _state_ty: PhantomData,
            context_fn: builder.context_fn,
        })
    }
}

impl<E1, Sf, S, C> ErrorExt for Builder<E1, Sf, S, C>
where
    E1: error::Error + Send + Sync + 'static,
    C: ContextFn,
    Sf: StateFn<E1, S>,
    S: State + ?Sized,
{
    type Result<E> = E;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.into()
    }
}

impl<Sf, S, C> ErrorExt for Builder<Errorless, Sf, S, C>
where
    C: ContextFn,
    Sf: StateFn<Errorless, S>,
    S: State + ?Sized,
{
    type Result<E> = E;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.into()
    }
}

impl<Sf, S, C> ErrorExt for Builder<Error, Sf, S, C>
where
    C: ContextFn,
    Sf: StateFn<Error, S>,
    S: State,
{
    type Result<E> = E;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.into()
    }
}

impl<C> ErrorExt for Builder<Error, state::Identity<Stateless>, Stateless, C>
where
    C: ContextFn,
{
    type Result<E> = E;
    type S = Stateless;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.into()
    }
}

impl<T, E1, Sf, S, C> ErrorExt for Result<T, Builder<E1, Sf, S, C>>
where
    E1: error::Error + Send + Sync + 'static,
    C: ContextFn,
    Sf: StateFn<E1, S>,
    S: State + ?Sized,
{
    type Result<E> = Result<T, E>;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.map_err(Error::from)
    }
}

impl<T, Sf, S, C> ErrorExt for Result<T, Builder<Errorless, Sf, S, C>>
where
    C: ContextFn,
    Sf: StateFn<Errorless, S>,
    S: State + ?Sized,
{
    type Result<E> = Result<T, E>;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.map_err(Error::from)
    }
}

impl<T, Sf, S, C> ErrorExt for Result<T, Builder<Error, Sf, S, C>>
where
    C: ContextFn,
    Sf: StateFn<Error, S>,
    S: State,
{
    type Result<E> = Result<T, E>;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.map_err(|err| err.build_error())
    }
}

impl<T, C> ErrorExt for Result<T, Builder<Error, state::Identity<Stateless>, Stateless, C>>
where
    C: ContextFn,
{
    type Result<E> = Result<T, E>;
    type S = Stateless;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.map_err(|err| err.build_error())
    }
}

#[cfg(test)]
mod _builder_cases_check {
    use core::error;

    use crate::{
        Error,
        builder::Builder,
        context::{self, Contextless},
        state::{self, State, StateFn, Stateless},
    };

    #[allow(dead_code)]
    fn check_builder_cases<E, Sf, S>()
    where
        E: error::Error + Send + Sync + 'static,
        Sf: StateFn<E, S>,
        S: State,
    {
        // 1. From<Builder<E, Sf, S, C>> for Error<S>
        let _: Error<S> = From::from(builder_fixture::<E, Sf, S>());
        // 2. From<Builder<E, S, C>> for Error
        let _removed = ();
        // 3. From<Builder<E, Identity<Stateless>, C>> for Error<S>
        let _: Error = From::from(builder_fixture::<E, state::Identity<Stateless>, Stateless>());
        // 4. From<Builder<Error<S>, Identity<Stateless>, C>> for Error<S>
        let _: Error<S> = From::from(builder_fixture::<
            Error<S>,
            state::Identity<Stateless>,
            Stateless,
        >());
        // 5. From<Builder<Error<S1>, S, C, L>> for Error
        let _removed = ();
        // 6. From<Builder<Error, Identity<Stateless>, C>> for Error<S>
        let _: Error<S> = From::from(builder_fixture::<
            Error,
            state::Identity<Stateless>,
            Stateless,
        >());
        // 7. From<Builder<Error, S, C>> for Error<S>
        let _: Error<S> = From::from(builder_fixture::<Error, state::Identity<S>, S>());
    }

    fn builder_fixture<E, Sf, S>() -> Builder<E, Sf, S, context::Identity<Contextless>>
    where
        Sf: StateFn<E, S>,
        S: State + ?Sized,
    {
        unimplemented!()
    }
}
