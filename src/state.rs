use std::fmt::Debug;

/// Associates an error state type with its stored representation.
///
/// Most types implement `State::Repr = Self` via blanket impl.
pub trait State: 'static {
    /// The type used to store the state inside [`Error`](crate::Error).
    type Repr: 'static;

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
    T: Debug + 'static,
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
///
/// Maps to `()` as the stored representation.
#[derive(Debug)]
pub struct Stateless(#[allow(unused)] [()]);

impl State for Stateless {
    type Repr = ();
}
