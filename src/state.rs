//! State traits and the [`Stateless`] marker.
use core::{convert::Infallible, fmt::Debug};

use crate::{Error, backtrace::WithBacktrace, render};

/// Associates an error state type with its stored representation.
///
/// Most types implement `State::Repr = Self` via blanket impl.
pub trait State: Debug + Send + Sync + 'static {
    /// The type used to store the state inside [`Error`](crate::Error).
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
#[derive(Debug)]
pub struct Stateless(#[allow(unused)] [()]);

impl State for Stateless {
    type Repr = Infallible;
}

/// An [`Error<S>`] with its state temporarily extracted. It maintains a compatible
/// storage layout to support reattachment.
pub struct Vacant<S>(Option<Error<S>>)
where
    S: State;

impl<S> Vacant<S>
where
    S: State,
{
    pub(crate) fn new(err: Option<Error<S>>) -> Self {
        Self(err)
    }

    /// Restores the original error by reattaching the extracted state.
    pub fn with_state(self, state: S) -> Error<S> {
        let Some(mut err) = self.0 else {
            return Error::from_state(state);
        };
        err.0
            .try_set_state(State::into_repr(state))
            .expect("Vacant must be created with correct state storage type");

        err
    }

    /// Converts into a stateless error. Returns `None` if no error details remain.
    pub fn try_into_stateless(self) -> Option<Error> {
        self.0.map(|s| {
            s.try_into_stateless()
                .expect("Vacant must not be created with an empty Error")
        })
    }
}

impl<S> Debug for Vacant<S>
where
    S: State,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Some(err) = &self.0 {
            render::format_debug_struct(
                f,
                "Vacant",
                err.state(),
                err.context(),
                err.payload(),
                err.source(),
                WithBacktrace::search_debug(err.erase_ref()),
            )
        } else {
            write!(f, "Vacant")
        }
    }
}
