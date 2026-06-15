//! State helpers and traits.
use core::{convert::Infallible, fmt::Debug, marker::PhantomData, result};

use crate::{
    Error,
    context::Context,
    raw::{RawError, RawVacant},
};

mod sealed {
    pub trait Sealed {}
}

/// Associates an error state type with its stored representation.
///
/// Most types implement `State::Repr = Self` via blanket impl.
pub trait State: sealed::Sealed + Debug + Send + Sync + 'static {
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

impl<T> sealed::Sealed for T where T: Debug + Send + Sync + 'static {}

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
#[derive(Debug)]
pub struct Stateless(#[allow(unused)] [()]);

impl sealed::Sealed for Stateless {}

impl State for Stateless {
    type Repr = Infallible;
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

    /// Converts into a stateless error. Returns `Err` if no error details remain.
    pub fn try_into_stateless(self) -> result::Result<Error, Self> {
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
            return Error(RawError::new(
                Some(S2::into_repr(state)),
                None::<Infallible>,
                context,
            ));
        };
        Error(vacant.derive(Some(S2::into_repr(state)), context))
    }

    /// Derives a stateless error from this vacant.
    pub fn derive_stateless<C>(self, context: C) -> Error
    where
        C: Context,
    {
        let Some(vacant) = self.inner else {
            return Error(RawError::new(None, None::<Infallible>, context));
        };
        Error(vacant.derive(None, context))
    }
}

impl<S> Debug for Vacant<S>
where
    S: State,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let Some(vacant) = &self.inner else {
            return write!(f, "Vacant");
        };
        Debug::fmt(vacant, f)
    }
}
