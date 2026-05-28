//! This library provides `Error<S = Stateless>`, an **optionally** dynamic dispatched error type,
//! enabling applications to handle errors uniformly across different contexts.
//!
//! # Quick Start
//! In most cases, `Error` can serve as a drop-in replacement for `Box<dyn Error>`.
//! Compared to the latter, it occupies only 1 usize, making the happy path faster.
//! ```
//! # use std::{fs::File, io::Write};
//! use erratic::*;
//!
//! fn write_log(filename: String) -> Result<()> {
//!     File::open(&filename)?.write_all(b"Hello, World!")?;
//!     Ok(())
//! }
//! ```
//!
//! # Attaching Context & Payload
//! When constructing an error, you can optionally attach a static context and/or a dynamic payload.
//! If attached, their memory is merged into a single allocation when the upstream error is erased.
//! If omitted, no extra memory is allocated for them. If only a context is provided, no heap allocation
//! occurs at all.
//!
//! ```
//! # use std::{fs::File, io::Write};
//! use erratic::*;
//!
//! fn write_log(filename: String) -> Result<()> {
//!     File::open(&filename)
//!         .ok()
//!         .with_context(literal!("failed to open the log file"))? // No alloc.
//!         .write_all(b"Hello, World!")
//!         .with_context(literal!("while writing to"))
//!         .with_payload(filename)?; // Alloc once for `io::Error`, `filename`, and `Context`.
//!     Ok(())
//! }
//! ```
//!
//! # Binding State
//! When propagating an error that requires special handling, you can attach a generic state
//! to it. The state is optional and can be cheaply erased or extracted using `extract_state`.
//!
//! When the state is small enough and none of the source error, context, or payload is attached,
//! the state is inlined without any heap allocation.
//!
//! ```
//! # use std::{fs::File, io::Write, fmt::Debug};
//! use erratic::*;
//!
//! #[derive(Debug)]
//! enum WriteLog {
//!     FileNotFound,
//! }
//!
//! fn write_log(filename: String) -> std::result::Result<(), Error<WriteLog>> {
//!     File::open(&filename)
//!         .ok()
//!         .with_state(WriteLog::FileNotFound)? // No alloc.
//!         .write_all(b"Hello, World!")
//!         .with_context(literal!("while writing to"))
//!         .with_payload(filename)?;
//!     Ok(())
//! }
//! ```
//!
//! # Representation
//! Type-wise, `Error<S>` is an internally tagged union, and it requires pointers to constant or
//! heap-allocated data to be aligned to 4 bytes, freeing up the lower 2 bits to encode
//! the discriminant. This design allows heap allocation to be avoided when unnecessary.
//!
//! ```plaintext
//! (32-bit platform, little-endian)
//! (Context Only)
//! [XXXXXX00|XXXXXXXX|XXXXXXXX|XXXXXXXX]
//!                                     \
//!                                     `rodata-> [Context]
//! (State Only)
//! [00000010|     ~    State     ~     ]
//!
//! (Otherwise)
//! [XXXXXX01|XXXXXXXX|XXXXXXXX|XXXXXXXX]
//!                                     \
//!                                     `heap-> [VTable|State|Error|Payload|Context]
//! ```
//!

#![allow(clippy::type_complexity)]

mod ptr;
mod raw;
mod rtti;

#[doc(hidden)]
pub mod macros;

pub mod context;
pub mod nae;
pub mod payload;
pub mod state;

use std::{
    self,
    convert::Infallible,
    error,
    fmt::{self, Debug, Display},
    marker::PhantomData,
    mem, result,
};

use crate::{
    context::{Context, Literal},
    nae::Nae,
    payload::{Immediate, PayloadFn},
    raw::RawError,
    state::{State, Stateless},
};

pub type Result<T> = result::Result<T, Error>;

/// An error type that can carry optional state, source, context, and payload.
#[repr(transparent)]
pub struct Error<S = Stateless>(RawError<S::Repr>)
where
    S: State + ?Sized;

impl Error {
    /// Starts building an `Error` from a source error.
    pub fn with_error<E>(
        err: E,
    ) -> Builder<E, Stateless, Immediate<payload::Empty>, context::Blank> {
        Builder {
            err,
            context: PhantomData,
            state: None,
            payload_fn: Immediate(payload::Empty::new()),
        }
    }

    /// Starts building an `Error` with a typed state.
    ///
    /// The state is inlined when no source or payload is attached.
    pub fn with_state<S>(state: S) -> Builder<Nae, S, Immediate<payload::Empty>, context::Blank>
    where
        S: State,
    {
        Builder {
            err: Nae::new(),
            context: PhantomData,
            state: Some(state.into_repr()),
            payload_fn: Immediate(payload::Empty::new()),
        }
    }

    /// Starts building an `Error` with a payload.
    pub fn with_payload<P>(payload: P) -> Builder<Nae, Stateless, Immediate<P>, context::Blank>
    where
        P: Display + Send + Sync + 'static,
    {
        Builder {
            err: Nae::new(),
            context: PhantomData,
            state: None,
            payload_fn: Immediate(payload),
        }
    }

    /// Starts building an `Error` with a lazily-evaluated payload.
    ///
    /// The closure `payload_fn` is called only when the error is materialized.
    pub fn with_payload_fn<F>(payload_fn: F) -> Builder<Nae, Stateless, F, context::Blank>
    where
        F: PayloadFn,
    {
        Builder {
            err: Nae::new(),
            context: PhantomData,
            state: None,
            payload_fn,
        }
    }

    /// Starts building an `Error` with a typed literal context.
    ///
    /// For dynamic content, use [`with_payload`][Self::with_payload] instead.
    pub fn with_context<L>(_ty: L) -> Builder<Nae, Stateless, Immediate<payload::Empty>, L>
    where
        L: Context,
    {
        Self::with_context_ty::<L>()
    }

    /// Starts building an `Error` with a literal context type, inferred at the call site.
    pub fn with_context_ty<L>() -> Builder<Nae, Stateless, Immediate<payload::Empty>, L> {
        Builder {
            err: Nae::new(),
            context: PhantomData,
            state: None,
            payload_fn: Immediate(payload::Empty::new()),
        }
    }

    /// Extracts the source error and payload by type.
    ///
    /// Returns `None` when the corresponding requested type do not match.
    pub fn into_parts<P, E>(self) -> (Option<&'static str>, Option<P>, Option<E>)
    where
        E: 'static,
        P: 'static,
    {
        let (_state, context, payload, source) = self.0.into_parts::<P, E>();
        (context, payload, source)
    }

    /// Helper for type inference when the state is not needed.
    pub fn stateless(self) -> Self {
        self
    }

    /// Converts to a state-tagged error without storing any runtime state.
    pub fn with_phantom_state<S>(self) -> Error<S>
    where
        S: State + ?Sized,
    {
        Error(self.0.with_phantom_state::<S::Repr>())
    }
}

impl<S> Error<S>
where
    S: State + ?Sized,
{
    /// Erases the state type, returning an opaque [`Error`][error::Error].
    ///
    /// The returned error implements `std::error::Error`, `Send`, and `Sync`,
    /// making it suitable for propagation through `?` or storage in `Box<dyn Error>`.
    pub fn erase(self) -> impl error::Error + Send + Sync + 'static {
        ImplError::<S>(self.0)
    }

    /// Returns a reference to an opaque [`Error`][error::Error] without consuming `self`.
    pub fn erase_ref(&self) -> &(impl error::Error + Send + Sync + 'static) {
        // Safety: `ImplError<S>` is `#[repr(transparent)]` over `RawError<S::Repr>`,
        // so `&RawError<S::Repr>` and `&ImplError<S>` have identical layout.
        unsafe { mem::transmute::<&RawError<S::Repr>, &ImplError<S>>(&self.0) }
    }

    /// Creates an `Error` from any [`Error`][std::error::Error].
    ///
    /// Equivalent to `E::into()` via the blanket `From` impl.
    pub fn from_error<E>(err: E) -> Self
    where
        E: error::Error + Send + Sync + 'static,
    {
        err.into()
    }

    /// Creates an `Error` from a typed literal context.
    pub fn from_context<L>(_ty: L) -> Self
    where
        L: Literal,
    {
        Self::from_context_ty::<L>()
    }

    /// Creates an `Error` from a literal context type, inferred at the call site.
    pub fn from_context_ty<L>() -> Self
    where
        L: Literal,
    {
        if let Ok(err) =
            rtti::concretize::<Error<Stateless>, Error<S>>(Error(RawError::new_const::<L>()))
        {
            return err;
        }

        Self(RawError::new_boxed::<_, _, context::Blank>(
            None,
            Nae::new(),
            payload::Empty::new(),
        ))
    }

    /// Creates an `Error` with a dynamic payload.
    pub fn from_payload<P>(payload: P) -> Self
    where
        P: Display + Send + Sync + 'static,
    {
        Self(RawError::new_boxed::<_, P, context::Blank>(
            None,
            Nae::new(),
            payload,
        ))
    }

    /// Returns a reference to the context, if present.
    pub fn context(&self) -> Option<&(dyn Display + Send + Sync + 'static)> {
        self.0.context()
    }

    /// Returns a reference to the displayable payload, if present.
    pub fn payload(&self) -> Option<&(dyn Display + Send + Sync + 'static)> {
        self.0.payload()
    }

    /// Returns `true` if the wrapped source error is of type `E`.
    pub fn has_source_of<E>(&self) -> bool
    where
        E: 'static,
    {
        self.0.downcast_source_ref::<E>().is_some()
    }

    /// Attempts to downcast the wrapped source error to `E` by shared reference.
    pub fn downcast_source_ref<E>(&self) -> Option<&E>
    where
        E: 'static,
    {
        self.0.downcast_source_ref::<E>()
    }

    /// Attempts to downcast the wrapped source error to `E` by mutable reference.
    pub fn downcast_source_mut<E>(&mut self) -> Option<&mut E>
    where
        E: 'static,
    {
        self.0.downcast_source_mut::<E>()
    }

    /// Returns `true` if the attached payload is of type `P`.
    pub fn has_payload_of<P>(&self) -> bool
    where
        P: 'static,
    {
        self.0.downcast_payload_ref::<P>().is_some()
    }

    /// Attempts to downcast the payload to `P` by shared reference.
    pub fn downcast_payload_ref<P>(&self) -> Option<&P>
    where
        P: 'static,
    {
        self.0.downcast_payload_ref::<P>()
    }

    /// Attempts to downcast the payload to `P` by mutable reference.
    pub fn downcast_payload_mut<P>(&mut self) -> Option<&mut P>
    where
        P: 'static,
    {
        self.0.downcast_payload_mut::<P>()
    }

    /// Consumes `self` and returns the boxed source error, if any.
    pub fn into_source(self) -> Option<Box<dyn error::Error + Send + Sync + 'static>> {
        self.0.into_source()
    }

    /// Iterates over the source error chain, starting from the immediate source.
    pub fn chain(&self) -> impl Iterator<Item = &(dyn error::Error + 'static)> {
        self.0.chain()
    }
}

impl<S> Error<S>
where
    S: State,
{
    /// Creates an `Error` with `state` inlined.
    pub fn from_state(state: S) -> Self {
        Error(RawError::new_inline_or_boxed(S::into_repr(state)))
    }

    /// Returns a reference to the attached state.
    pub fn state(&self) -> Option<&S> {
        self.0.state().map(S::from_repr_ref)
    }

    /// Consumes `self` and returns the state, context, payload, and error.
    ///
    /// Returns `None` when the requested types do not match.
    pub fn into_parts<P, E>(self) -> (Option<S>, Option<&'static str>, Option<P>, Option<E>)
    where
        E: 'static,
        P: 'static,
    {
        let (state, context, payload, error) = self.0.into_parts::<P, E>();
        (state.map(S::from_repr), context, payload, error)
    }

    /// Consumes `self` and returns the attached state.
    pub fn into_state(self) -> Option<S> {
        self.0.into_state().map(S::from_repr)
    }

    /// Preserves the state; retains the error if additional info is present.
    pub fn extract_state(self) -> result::Result<(S, Option<Error>), Error> {
        match self.0.extract_state() {
            Ok((s, o)) => Ok((S::from_repr(s), o.map(Error))),
            Err(e) => Err(Error(e)),
        }
    }
}

impl<S> From<Error<S>> for Box<dyn error::Error + Send + Sync + 'static>
where
    S: State + ?Sized,
{
    fn from(value: Error<S>) -> Self {
        value.0.into_boxed_error()
    }
}

impl<S> Debug for Error<S>
where
    S: State + ?Sized,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(&self.0, f)
    }
}

impl<S> Display for Error<S>
where
    S: State + ?Sized,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

#[repr(transparent)]
struct ImplError<S = Stateless>(RawError<S::Repr>)
where
    S: State + ?Sized;

impl<S> Debug for ImplError<S>
where
    S: State + ?Sized,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(&self.0, f)
    }
}

impl<S> Display for ImplError<S>
where
    S: State + ?Sized,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl<S> error::Error for ImplError<S>
where
    S: State + ?Sized,
{
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.0
            .source()
            .map(|src| src as &(dyn error::Error + 'static))
    }
}

impl<E, S> From<E> for Error<S>
where
    E: error::Error + Send + Sync + 'static,
    S: State + ?Sized,
{
    fn from(err: E) -> Self {
        let Err(err) = match_else!(rtti::concretize::<E, ImplError<S>>(err),
            Ok(err_unit) => return Error(err_unit.0),
        );

        Error(RawError::new_boxed::<_, _, context::Blank>(
            None,
            err,
            payload::Empty::new(),
        ))
    }
}

impl<S> From<Error<S>> for Error
where
    S: State,
{
    fn from(err: Error<S>) -> Self {
        if err.state().is_none() {
            return err.extract_state().unwrap_err();
        }
        Error(RawError::new_boxed::<_, _, context::Blank>(
            None,
            err.erase(),
            payload::Empty::new(),
        ))
    }
}

impl<S> From<Error> for Error<S>
where
    S: State,
{
    fn from(value: Error) -> Self {
        value.with_phantom_state()
    }
}

/// An intermediate builder for constructing an [`Error`].
///
/// `Builder` accumulates the error source, state, payload, and context,
/// then materializes them into an `Error<S>` via [`build`](Builder::build) or [`Into`].
#[derive(Debug)]
pub struct Builder<E, S, F, L>
where
    F: PayloadFn,
    S: State + ?Sized,
    L: ?Sized,
{
    err: E,
    state: Option<S::Repr>,
    payload_fn: F,
    context: PhantomData<L>,
}

impl<E, S, F, L> Builder<E, S, F, L>
where
    F: PayloadFn,
    E: error::Error + Send + Sync + 'static,
    S: State + ?Sized,
    L: Context + ?Sized,
{
    /// Materializes the builder into an [`Error<S>`].
    pub fn build(self) -> Error<S> {
        self.into()
    }
}

impl<E, S, F, L> From<Builder<E, S, F, L>> for Error<S>
where
    F: PayloadFn,
    E: error::Error + Send + Sync + 'static,
    S: State + ?Sized,
    L: Context + ?Sized,
{
    fn from(value: Builder<E, S, F, L>) -> Self {
        let has_state = !rtti::is_same_ty::<S::Repr, Infallible>();
        let has_context = !rtti::is_same_ty::<L, context::Blank>();
        let has_error = !rtti::is_same_ty::<E, Nae>();
        let has_payload = !rtti::is_same_ty::<F::Output, payload::Empty>();

        match (has_state, has_context, has_error, has_payload) {
            (false, false, false, false) => unreachable!(),
            (true, false, false, false) => Error::<S>(RawError::new_inline_or_boxed(
                value.state.expect("checked by has_state"),
            )),
            (false, true, false, false) => {
                let Ok(body) = match_else!(rtti::concretize::<_, RawError<S::Repr>>(RawError::new_const::<L>()),
                    Err(_) => unreachable!(),
                );
                Error(body)
            }
            _ => Error::<S>(RawError::new_boxed::<_, _, L>(
                value.state,
                value.err,
                value.payload_fn.call(),
            )),
        }
    }
}

impl<E, S, F, L> From<Builder<E, S, F, L>> for Error
where
    F: PayloadFn,
    E: error::Error + Send + Sync + 'static,
    S: State,
    L: Context + ?Sized,
{
    fn from(value: Builder<E, S, F, L>) -> Self {
        let has_state = !rtti::is_same_ty::<S::Repr, Infallible>();
        let has_context = !rtti::is_same_ty::<L, context::Blank>();
        let has_error = !rtti::is_same_ty::<E, Nae>();
        let has_payload = !rtti::is_same_ty::<F::Output, payload::Empty>();

        match (has_state, has_context, has_error, has_payload) {
            (false, false, false, false) => unreachable!(),
            (true, false, false, false) => Error(RawError::new_boxed::<_, _, L>(
                None,
                ImplError::<S>(RawError::new_inline_or_boxed(
                    value.state.expect("checked by has_state"),
                )),
                payload::Empty::new(),
            )),
            (false, true, false, false) => Error(RawError::new_const::<L>()),
            (false, _, _, _) => Error(RawError::new_boxed::<_, _, L>(
                None,
                value.err,
                value.payload_fn.call(),
            )),
            _ => Error(RawError::new_boxed::<_, _, context::Blank>(
                None,
                ImplError::<S>(RawError::new_boxed::<_, _, L>(
                    value.state,
                    value.err,
                    value.payload_fn.call(),
                )),
                payload::Empty::new(),
            )),
        }
    }
}

impl<E, S, F, L> From<Builder<E, Stateless, F, L>> for Error<S>
where
    F: PayloadFn,
    E: error::Error + Send + Sync + 'static,
    S: State,
    L: Context + ?Sized,
{
    fn from(value: Builder<E, Stateless, F, L>) -> Self {
        let has_state = !rtti::is_same_ty::<S::Repr, Infallible>();
        let has_context = !rtti::is_same_ty::<L, context::Blank>();
        let has_error = !rtti::is_same_ty::<E, Nae>();
        let has_payload = !rtti::is_same_ty::<F::Output, payload::Empty>();

        match (has_state, has_context, has_error, has_payload) {
            (true, false, false, false) => {
                Error(RawError::<S::Repr>::new_boxed::<_, _, context::Blank>(
                    None,
                    Nae::new(),
                    payload::Empty::new(),
                ))
            }
            (false, true, false, false) => {
                let Ok(body) = match_else!(rtti::concretize::<_, RawError<S::Repr>>(RawError::new_const::<L>()),
                    Err(_) => unreachable!(),
                );
                Error(body)
            }
            _ => Error(RawError::new_boxed::<_, _, L>(
                None,
                value.err,
                value.payload_fn.call(),
            )),
        }
    }
}

impl<S1, S, F, L> From<Builder<Error<S1>, S, F, L>> for Error<S>
where
    S1: State + ?Sized,
    F: PayloadFn,
    S: State + ?Sized,
    L: Context + ?Sized,
{
    fn from(value: Builder<Error<S1>, S, F, L>) -> Self {
        Error(RawError::new_boxed::<_, _, L>(
            value.state,
            value.err.erase(),
            value.payload_fn.call(),
        ))
    }
}

/// Extension trait for extracting the state to a separate layer.
pub trait StateExt {
    type T;
    type S;
    type Result<T, E>;

    ///  Extracts the state if it has been set.
    fn extract_state(
        self,
    ) -> result::Result<Self::Result<Self::T, (Self::S, Option<Error>)>, Error>
    where
        Self::S: Sized;
}

impl<S1> StateExt for Error<S1>
where
    S1: State,
{
    type T = ();
    type S = S1;
    type Result<T, E> = E;

    fn extract_state(self) -> result::Result<Self::Result<Self::T, (Self::S, Option<Error>)>, Error>
    where
        Self::S: Sized,
    {
        self.extract_state()
    }
}

impl<T1, S> StateExt for result::Result<T1, Error<S>>
where
    S: State,
{
    type T = T1;
    type S = S;
    type Result<T, E> = result::Result<T, E>;

    fn extract_state(self) -> result::Result<Self::Result<Self::T, (Self::S, Option<Error>)>, Error>
    where
        Self::S: Sized,
    {
        match self {
            Ok(v) => Ok(Ok(v)),
            Err(err) => err.build_error().extract_state().map(|err| Err(err)),
        }
    }
}

impl<E1, S, F, L> StateExt for Builder<E1, S, F, L>
where
    E1: error::Error + Send + Sync + 'static,
    F: PayloadFn,
    S: State,
    L: Context + ?Sized,
{
    type T = ();
    type Result<T, E> = E;
    type S = S;

    fn extract_state(self) -> result::Result<Self::Result<Self::T, (Self::S, Option<Error>)>, Error>
    where
        Self::S: Sized,
    {
        self.build_error().extract_state()
    }
}

impl<S1, S, F, L> StateExt for Builder<Error<S1>, S, F, L>
where
    S1: State + ?Sized,
    F: PayloadFn,
    S: State,
    L: Context + ?Sized,
{
    type T = ();
    type S = S;
    type Result<T, E> = E;

    fn extract_state(self) -> result::Result<Self::Result<Self::T, (Self::S, Option<Error>)>, Error>
    where
        Self::S: Sized,
    {
        self.build_error().extract_state()
    }
}

impl<T1, E1, S, F, L> StateExt for result::Result<T1, Builder<E1, S, F, L>>
where
    E1: error::Error + Send + Sync + 'static,
    F: PayloadFn,
    S: State,
    L: Context + ?Sized,
{
    type T = T1;
    type S = S;
    type Result<T, E> = result::Result<T, E>;

    fn extract_state(self) -> result::Result<Self::Result<Self::T, (Self::S, Option<Error>)>, Error>
    where
        Self::S: Sized,
    {
        match self {
            Ok(v) => Ok(Ok(v)),
            Err(err) => err.build_error().extract_state().map(|err| Err(err)),
        }
    }
}

impl<T1, S1, S, F, L> StateExt for result::Result<T1, Builder<Error<S1>, S, F, L>>
where
    S1: State + ?Sized,
    F: PayloadFn,
    S: State,
    L: Context + ?Sized,
{
    type T = T1;
    type S = S;
    type Result<T, E> = result::Result<T, E>;

    fn extract_state(self) -> result::Result<Self::Result<Self::T, (Self::S, Option<Error>)>, Error>
    where
        Self::S: Sized,
    {
        match self {
            Ok(v) => Ok(Ok(v)),
            Err(err) => err.build_error().extract_state().map(|err| Err(err)),
        }
    }
}

/// Extension trait for attaching context, state, or payload to an existing error.
pub trait BuilderExt: Sized {
    type Result<E>;

    type E;
    type S: State + ?Sized;
    type F: PayloadFn;
    type L: Literal + ?Sized;

    /// Attaches a literal context identified by its type.
    fn with_context_ty<L>(self) -> Self::Result<Builder<Self::E, Self::S, Self::F, L>>
    where
        L: Context;

    /// Attaches a static literal context.
    ///
    /// For dynamic content, use [`with_payload`][Self::with_payload] instead.
    fn with_context<L>(self, _ty: L) -> Self::Result<Builder<Self::E, Self::S, Self::F, L>>
    where
        L: Context,
    {
        self.with_context_ty::<L>()
    }

    /// Attaches a typed state.
    fn with_state<S>(self, state: S) -> Self::Result<Builder<Self::E, S, Self::F, Self::L>>
    where
        S: State + Sized;

    /// Attaches a lazily-evaluated payload.
    fn with_payload_fn<F>(
        self,
        payload_fn: F,
    ) -> Self::Result<Builder<Self::E, Self::S, F, Self::L>>
    where
        F: PayloadFn;

    /// Attaches a displayable payload.
    fn with_payload<P>(
        self,
        payload: P,
    ) -> Self::Result<Builder<Self::E, Self::S, Immediate<P>, Self::L>>
    where
        P: Display + Send + Sync + 'static,
    {
        self.with_payload_fn(Immediate(payload))
    }
}

impl<T, E1> BuilderExt for result::Result<T, E1>
where
    E1: error::Error + Send + Sync + 'static,
{
    type Result<E> = result::Result<T, E>;

    type E = E1;
    type S = Stateless;
    type F = Immediate<payload::Empty>;
    type L = context::Blank;

    fn with_context_ty<L>(self) -> Self::Result<Builder<Self::E, Self::S, Self::F, L>>
    where
        L: Context,
    {
        self.map_err(|err| Builder {
            err,
            context: PhantomData,
            state: None,
            payload_fn: Immediate(payload::Empty::new()),
        })
    }

    fn with_state<S>(self, state: S) -> Self::Result<Builder<Self::E, S, Self::F, Self::L>>
    where
        S: State + Sized,
    {
        self.map_err(|err| Builder {
            err,
            context: PhantomData,
            state: Some(state.into_repr()),
            payload_fn: Immediate(payload::Empty::new()),
        })
    }

    fn with_payload_fn<F>(
        self,
        payload_fn: F,
    ) -> Self::Result<Builder<Self::E, Self::S, F, Self::L>>
    where
        F: PayloadFn,
    {
        self.map_err(|err| Builder {
            err,
            context: PhantomData,
            state: None,
            payload_fn,
        })
    }
}

impl<T, S1> BuilderExt for result::Result<T, Error<S1>>
where
    S1: State + ?Sized,
{
    type Result<E> = result::Result<T, E>;

    type E = Error<S1>;
    type S = Stateless;
    type F = Immediate<payload::Empty>;
    type L = context::Blank;

    fn with_context_ty<L>(self) -> Self::Result<Builder<Self::E, Self::S, Self::F, L>>
    where
        L: Context,
    {
        self.map_err(|err| Builder {
            err,
            context: PhantomData,
            state: None,
            payload_fn: Immediate(payload::Empty::new()),
        })
    }

    fn with_state<S>(self, state: S) -> Self::Result<Builder<Self::E, S, Self::F, Self::L>>
    where
        S: State,
    {
        self.map_err(|err| Builder {
            err,
            context: PhantomData,
            state: Some(state.into_repr()),
            payload_fn: Immediate(payload::Empty::new()),
        })
    }

    fn with_payload_fn<F>(
        self,
        payload_fn: F,
    ) -> Self::Result<Builder<Self::E, Self::S, F, Self::L>>
    where
        F: PayloadFn,
    {
        self.map_err(|err| Builder {
            err,
            context: PhantomData,
            state: None,
            payload_fn,
        })
    }
}

impl<E1, S1, F1, L1> BuilderExt for Builder<E1, S1, F1, L1>
where
    F1: PayloadFn,
    S1: State + ?Sized,
    L1: Literal + ?Sized,
{
    type Result<E> = E;

    type E = E1;
    type S = S1;
    type F = F1;
    type L = L1;

    fn with_context_ty<L>(self) -> Self::Result<Builder<Self::E, Self::S, Self::F, L>>
    where
        L: Context,
    {
        Builder {
            err: self.err,
            context: PhantomData,
            state: self.state,
            payload_fn: self.payload_fn,
        }
    }

    fn with_state<S>(self, state: S) -> Self::Result<Builder<Self::E, S, Self::F, Self::L>>
    where
        S: State,
    {
        Builder {
            state: Some(state.into_repr()),
            err: self.err,
            context: self.context,
            payload_fn: self.payload_fn,
        }
    }

    fn with_payload_fn<F>(
        self,
        payload_fn: F,
    ) -> Self::Result<Builder<Self::E, Self::S, F, Self::L>>
    where
        F: PayloadFn,
    {
        Builder {
            err: self.err,
            context: self.context,
            state: self.state,
            payload_fn,
        }
    }
}

impl<T, E1, S1, F1, L1> BuilderExt for result::Result<T, Builder<E1, S1, F1, L1>>
where
    F1: PayloadFn,
    S1: State + ?Sized,
    L1: Literal + ?Sized,
{
    type Result<E> = result::Result<T, E>;

    type E = E1;
    type S = S1;
    type F = F1;
    type L = L1;

    fn with_context_ty<L>(self) -> Self::Result<Builder<Self::E, Self::S, Self::F, L>>
    where
        L: Context,
    {
        self.map_err(|err| Builder {
            err: err.err,
            context: PhantomData,
            state: err.state,
            payload_fn: err.payload_fn,
        })
    }

    fn with_state<S>(self, state: S) -> Self::Result<Builder<Self::E, S, Self::F, Self::L>>
    where
        S: State,
    {
        self.map_err(|err| Builder {
            state: Some(state.into_repr()),
            err: err.err,
            context: err.context,
            payload_fn: err.payload_fn,
        })
    }

    fn with_payload_fn<F>(
        self,
        payload_fn: F,
    ) -> Self::Result<Builder<Self::E, Self::S, F, Self::L>>
    where
        F: PayloadFn,
    {
        self.map_err(|err| Builder {
            err: err.err,
            context: err.context,
            state: err.state,
            payload_fn,
        })
    }
}

impl<T> BuilderExt for Option<T> {
    type Result<E> = result::Result<T, E>;

    type E = Nae;
    type S = Stateless;
    type F = Immediate<payload::Empty>;
    type L = context::Blank;

    fn with_context_ty<L>(self) -> Self::Result<Builder<Self::E, Self::S, Self::F, L>>
    where
        L: Context,
    {
        self.ok_or(Builder {
            err: Nae::new(),
            context: PhantomData,
            state: None,
            payload_fn: Immediate(payload::Empty::new()),
        })
    }

    fn with_state<S>(self, state: S) -> Self::Result<Builder<Self::E, S, Self::F, Self::L>>
    where
        S: State,
    {
        self.ok_or(Builder {
            state: Some(state.into_repr()),
            err: Nae::new(),
            context: PhantomData,
            payload_fn: Immediate(payload::Empty::new()),
        })
    }

    fn with_payload_fn<F>(
        self,
        payload_fn: F,
    ) -> Self::Result<Builder<Self::E, Self::S, F, Self::L>>
    where
        F: PayloadFn,
    {
        self.ok_or(Builder {
            err: Nae::new(),
            context: PhantomData,
            state: None,
            payload_fn,
        })
    }
}

/// Extension trait for materializing or erasing an error.
pub trait ErrorExt: Sized {
    type Result<E>;
    type S: State + ?Sized;

    /// Materializes the final [`Error<Self::S>`].
    fn build_error(self) -> Self::Result<Error<Self::S>>;

    /// Materializes and then erases the state, returning an opaque `impl Error`.
    fn erase_error(self) -> Self::Result<impl error::Error + Send + Sync + 'static>;
}

impl<S> ErrorExt for Error<S>
where
    S: State + ?Sized,
{
    type Result<E> = E;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self
    }

    fn erase_error(self) -> Self::Result<impl error::Error + Send + Sync + 'static> {
        self.erase()
    }
}

impl<E1, S, F, L> ErrorExt for Builder<E1, S, F, L>
where
    E1: error::Error + Send + Sync + 'static,
    F: PayloadFn,
    S: State + ?Sized,
    L: Context + ?Sized,
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

impl<S1, S, F, L> ErrorExt for Builder<Error<S1>, S, F, L>
where
    S1: State + ?Sized,
    F: PayloadFn,
    S: State + ?Sized,
    L: Context + ?Sized,
{
    type Result<E> = E;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        Builder {
            err: self.err.erase(),
            state: self.state,
            payload_fn: self.payload_fn,
            context: self.context,
        }
        .build_error()
    }

    fn erase_error(self) -> Self::Result<impl error::Error + Send + Sync + 'static> {
        self.build_error().erase()
    }
}

impl<T, E1, S, F, L> ErrorExt for result::Result<T, Builder<E1, S, F, L>>
where
    E1: error::Error + Send + Sync + 'static,
    F: PayloadFn,
    S: State + ?Sized,
    L: Context + ?Sized,
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

impl<T, S1, S, F, L> ErrorExt for result::Result<T, Builder<Error<S1>, S, F, L>>
where
    S1: State + ?Sized,
    F: PayloadFn,
    S: State + ?Sized,
    L: Context + ?Sized,
{
    type Result<E> = result::Result<T, E>;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.map_err(|err| {
            Builder {
                err: err.err.erase(),
                state: err.state,
                payload_fn: err.payload_fn,
                context: err.context,
            }
            .build_error()
        })
    }

    fn erase_error(self) -> Self::Result<impl error::Error + Send + Sync + 'static> {
        self.build_error().map_err(|err| err.erase())
    }
}

impl<T, E1> ErrorExt for result::Result<T, E1>
where
    E1: error::Error + Send + Sync + 'static,
{
    type Result<E> = result::Result<T, E>;
    type S = Stateless;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.map_err(Error::from)
    }

    fn erase_error(self) -> Self::Result<impl error::Error + Send + Sync + 'static> {
        self.build_error().map_err(|err| err.erase())
    }
}

impl<T, S> ErrorExt for result::Result<T, Error<S>>
where
    S: State,
{
    type Result<E> = result::Result<T, E>;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self
    }

    fn erase_error(self) -> Self::Result<impl error::Error + Send + Sync + 'static> {
        self.map_err(|err| err.erase())
    }
}
