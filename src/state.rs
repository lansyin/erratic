//! State helpers and traits.
use core::{
    convert::Infallible,
    fmt::{self, Debug},
    marker::PhantomData,
};

use crate::{
    Error,
    context::{Context, Contextless},
    match_else,
    raw::{RawError, RawVacant},
};

/// Associates an error state type with its stored representation.
///
/// Most types implement `State::Repr = Self` via blanket impl.
pub trait State: Debug + Send + Sync + 'static {
    /// The type used to store the state inside [`Error`].
    type Repr: Debug + Send + Sync + 'static;

    /// Converts `self` into its stored representation.
    fn into_repr(self) -> Self::Repr
    where
        Self: Sized;

    /// Recovers the state from its stored representation.
    fn from_repr(state: Self::Repr) -> Self
    where
        Self: Sized;

    /// Recovers a reference to the state from a reference to its stored representation.
    fn from_repr_ref(state: &Self::Repr) -> &Self
    where
        Self: Sized;
}

impl<T> State for T
where
    T: Debug + Send + Sync + 'static,
{
    type Repr = T;

    fn into_repr(self) -> Self::Repr {
        self
    }

    fn from_repr(this: Self::Repr) -> Self
    where
        Self: Sized,
    {
        this
    }

    fn from_repr_ref(this: &Self::Repr) -> &Self
    where
        Self: Sized,
    {
        this
    }
}

/// Marker type indicating no meaningful state.
pub struct Stateless(#[allow(unused)] [()]);

impl Debug for Stateless {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Stateless").finish()
    }
}

impl State for Stateless {
    type Repr = Infallible;
}

/// A potentially lazily evaluated or derived state.
pub trait StateFn<E, S>
where
    S: State + ?Sized,
{
    fn call(self, derive_from: Option<&E>) -> Option<<S as State>::Repr>;
}

/// A wrapper that wraps values as [`StateFn`]. See [`with_state`][crate::BuilderExt::with_state].
pub struct Identity<S>(Option<S::Repr>)
where
    S: State + ?Sized;

impl<S> Identity<S>
where
    S: State + ?Sized,
{
    pub(crate) fn phantom() -> Self {
        Self(None)
    }
}

impl<S> Identity<S>
where
    S: State,
{
    pub(crate) fn new(state: S) -> Self {
        Self(Some(S::into_repr(state)))
    }
}

impl<E, S> StateFn<E, S> for Identity<S>
where
    S: State + ?Sized,
{
    fn call(self, _derive_from: Option<&E>) -> Option<S::Repr> {
        self.0
    }
}

/// A lazily evaluated state. See [`with_state_fn`][crate::BuilderExt::with_state_fn].
pub struct Lazy<F, S>
where
    S: State,
    F: FnOnce() -> S,
{
    f: F,
}

impl<F, S> Lazy<F, S>
where
    S: State,
    F: FnOnce() -> S,
{
    pub(crate) fn new(f: F) -> Self {
        Self { f }
    }
}

impl<F, E, S> StateFn<E, S> for Lazy<F, S>
where
    S: State,
    F: FnOnce() -> S,
{
    fn call(self, _derive_from: Option<&E>) -> Option<S::Repr> {
        Some(S::into_repr((self.f)()))
    }
}

/// Derives a state from an error, if any. See [`with_state_derived`][crate::DeriveExt::with_state_derived].
pub struct Derive<F, E, S>
where
    S: State,
    F: FnOnce(&E) -> Option<S>,
{
    f: F,
    _marker: PhantomData<fn(E)>,
}

impl<F, E, S> Derive<F, E, S>
where
    S: State,
    F: FnOnce(&E) -> Option<S>,
{
    pub(crate) fn new(f: F) -> Self {
        Self {
            f,
            _marker: PhantomData,
        }
    }
}

impl<F, E, S> StateFn<E, S> for Derive<F, E, S>
where
    S: State,
    F: FnOnce(&E) -> Option<S>,
{
    fn call(self, derive_from: Option<&E>) -> Option<S::Repr> {
        Some(S::into_repr((self.f)(derive_from?)?))
    }
}

/// An [`Error<S>`] with its state temporarily extracted.
///
/// It maintains a compatible storage layout to support reattachment.
pub struct Vacant<S>
where
    S: State,
{
    inner: Option<RawVacant>,
    _marker: PhantomData<S>,
}

impl<S> Vacant<S>
where
    S: State,
{
    pub(crate) fn new(vacant: Option<RawVacant>) -> Self {
        Self {
            inner: vacant,
            _marker: PhantomData,
        }
    }

    /// Restores the original error by reattaching the extracted state.
    pub fn with_state(self, state: S) -> Error<S> {
        let Some(vacant) = self.inner else {
            return Error::from_state(state);
        };

        let err = vacant
            .try_with_state(S::into_repr(state))
            .expect("Vacant must be created with correct state storage type");

        Error(err)
    }

    /// Tries to store a state of a different type, reusing the existing error storage.
    ///
    /// It's guaranteed that reuse will succeed when the types are identical, or when both the
    /// original and target types are at most `usize` in size (assuming the alignment also fits).
    pub fn try_with_state<S2>(self, state: S2) -> Result<Error<S2>, (Self, S2)>
    where
        S2: State,
    {
        let Some(vacant) = self.inner else {
            return Err((Self::new(None), state));
        };
        let state = S2::into_repr(state);
        let Ok(err) = match_else!(vacant.try_with_state(state), Err((vacant, state)) => {
            return Err((Self::new(Some(vacant)), S2::from_repr(state)));
        });
        Ok(Error(err))
    }

    /// Converts into a stateless error. Returns `Err` if no error details remain.
    pub fn try_into_stateless(self) -> Result<Error, Self> {
        let Some(vacant) = self.inner else {
            return Err(Self::new(None));
        };

        match vacant.try_into_stateless() {
            Ok(err) => Ok(Error(err)),
            Err(err) => Err(Self::new(Some(err))),
        }
    }

    /// Derives an error from this vacant.
    pub fn derive<S2, C>(self, state: S2, context: C) -> Error<S2>
    where
        S2: State,
        C: Context,
    {
        let Some(vacant) = self.inner else {
            return Error(RawError::from_error(
                Some(S2::into_repr(state)),
                None::<Infallible>,
                context,
            ));
        };
        Error(vacant.derive(Some(S2::into_repr(state)), context))
    }

    /// Derives a contextless error from this vacant.
    pub fn derive_with_state<S2>(self, state: S2) -> Error<S2>
    where
        S2: State,
    {
        let Some(vacant) = self.inner else {
            return Error(RawError::from_error(
                Some(S2::into_repr(state)),
                None::<Infallible>,
                Contextless::new(),
            ));
        };
        Error(vacant.derive(Some(S2::into_repr(state)), Contextless::new()))
    }

    /// Derives a stateless error from this vacant.
    pub fn derive_with_context<C>(self, context: C) -> Error
    where
        C: Context,
    {
        let Some(vacant) = self.inner else {
            return Error(RawError::from_error(None, None::<Infallible>, context));
        };
        Error(vacant.derive(None, context))
    }
}

impl<S> Debug for Vacant<S>
where
    S: State,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(vacant) = &self.inner else {
            return write!(f, "Vacant");
        };
        Debug::fmt(vacant, f)
    }
}
