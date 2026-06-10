//! This library provides `Error<S = Stateless>`, an error type with **optional** dynamic dispatch,
//! enabling applications to handle errors uniformly across different scenarios.
//!
//! # Quick Start
//!
//! In most cases, `Error` can serve as a drop-in replacement for `Box<dyn Error>`.
//! Compared to the latter, it occupies only 1 usize, making the happy path faster.
//! ```
//! # use std::{fs::File, io::Write};
//! fn say_hi(filename: &str) -> erratic::Result<()> {
//!     File::open(filename)?.write_all(b"Hello, World!")?;
//!     Ok(())
//! }
//! ```
//!
//! # Attaching Context
//!
//! When constructing an error, you can optionally attach a context. If the context is a literal string
//! and it's the only component of the error, no heap allocation occurs.
//!
//! ```
//! # use std::sync::Weak;
//! # struct Reader;
//! # impl Reader {
//! #     fn read(&self, _: &[u8]) -> Result<u64> { unimplemented!() }
//! #     fn id(&self) -> String { unimplemented!() }
//! # }
//! use erratic::*;
//!
//! fn read_weak(r: &mut Weak<Reader>, buf: &mut [u8]) -> Result<u64> {
//!     if buf.is_empty() {
//!         return mkres!("buf must not be empty"); // No alloc so long as no format args.
//!     }
//!     let r = r.upgrade()
//!         .with_context("stream expired")?; // Accepts any value implementing `Display`.
//!     let n = r.read(buf)
//!         .with_context(mkctx!("cannot read {}", r.id()))?; // `mkctx!` evaluates lazily.
//!     //  .with_context_fn(|| format!("cannot read {}", r.id()))?; // Same as the previous line.
//!     Ok(n)
//! }
//! ```
//!
//! # Binding State
//!
//! When propagating an error that requires special handling, you can optionally attach a state to it.
//! If the state is small enough and it's the only component of the error, the state is inlined
//! without any heap allocation.
//!
//! ```
//! # use std::{result::Result};
//! # struct Writer;
//! # impl Writer {
//! #     fn write(&mut self, _: &[u8]) -> erratic::Result<()> { unimplemented!() }
//! #     fn reserve_chunk(&self, _: usize) -> erratic::Result<()> { unimplemented!() }
//! #     fn id(&self) -> String { unimplemented!() }
//! # }
//! use erratic::*;
//!
//! #[derive(Debug)]
//! enum State { RetryLater }
//!
//! fn try_write(w: &mut Writer, data: &[u8; 64]) -> Result<(), Error<State>> {
//!     w.reserve_chunk(64)
//!         .ok()
//!         .with_state(State::RetryLater)?; // No alloc.
//!     w.write(data)
//!         .with_context(mkctx!("failed to write to {}", w.id()))?;
//!     Ok(())
//! }
//! ```
//!
//! When no runtime state is actually stored, errors can be cheaply converted between different state types.
//! This allows infrastructure errors to cross any number of layers with no extra allocation, domain errors
//! avoid the heap entirely, and both share the same `Error<S>` type. All compose orthogonally.
//!
//! ```
//! # use std::{thread, result};
//! # use erratic::*;
//! # #[derive(Debug)]
//! # enum State { RetryLater }
//! # struct Writer;
//! # fn try_write(_: &mut Writer, data: &[u8; 64]) -> result::Result<(), Error<State>> { unimplemented!() }
//! fn write(w: &mut Writer, data: &[u8; 64]) -> Result<()> {
//!     while let Err((state, _)) = try_write(w, data).extract_state()? {
//!         match state {
//!             State::RetryLater => {
//!                 thread::yield_now();
//!             }
//!         }
//!     }
//!     Ok(())
//! }
//! ```
//!
//! The `?` operator covers the most common cases, notably including conversion from `Error` to `Error<S>`:
//!
//! - `impl Error`  -> `Error`
//! - `impl Error`  -> `Error<S>`
//! - `Builder<..>`  -> `Error`
//! - `Builder<..>`  -> `Error<S>`
//! - `Error`       -> `Error<S>`
//!
//! Stateful errors are meant to be handled explicitly. Several utility methods are provided:
//!
//!   - `erase_error()?`:    Propagate the error.
//!   - `extract_state()?`:  Take the state out, or propagate the error.
//!   - `map_state()?`:      Transform the state with a closure.
//!   - `lift_state()?`:     Transform via `From<S>`.
//!
//! # Backtrace
//!
//! When the `backtrace` feature is enabled and backtrace capture is configured via
//! [environment variables][backtrace-conf], `Error<S>` automatically captures a backtrace if there isn't
//! one already in the source chain. The backtrace will be appended after the error chain during debug
//! formatting, unless the minus flag, e.g. `{:-?}`, is specified to suppress it.
//!
//! [backtrace-conf]: https://doc.rust-lang.org/std/backtrace/index.html#environment-variables
//!
//! # Representation
//!
//! If the error contains only a source, the error message is inherited from the source. Otherwise, the
//! error message is constructed from other attached components.
//!
//! ```text
//! <error> ::= <source>
//!           | <state>": "<context>
//!           | <context>
//!           | <state>
//! ```
//!
//! By default, only the top-level error is shown during formatting. To display the full error chain,
//! format with alternate or debug specifiers.
//!
//! - `{}`:       Displays only the top-level error.
//! - `{:#}`:     Displays the full error chain.
//! - `{:?}`:     Displays the full error chain with backtrace (if captured).
//! - `{:#?}`:    Displays all information in a struct-like format.
//!
//! The error chain is defined as follows:
//!
//! ```text
//! <chain> ::= <error>
//!           | <error>"\n  -> "<chain>
//! ```
//!
//! # Layout
//!
//! Type-wise, `Error<S>` is an internally tagged union, and it requires pointers to be aligned to 4 bytes,
//! freeing up the lower 2 bits to encode its discriminant. Pointer tagging in this crate fully follows
//! [strict provenance][strict_provenance], and is verified by Miri.
//!
//! [strict_provenance]: https://doc.rust-lang.org/std/ptr/index.html#strict-provenance
//!
//! ```plaintext
//! (32-bit platform, little-endian)
//! (Context Only)
//! [......00|........|........|........]
//!                                     \
//!                                      `rodata-> [Context]
//! (Allocation Required)
//! [......01|........|........|........]
//!                                     \
//!                                      `heap-> [VTable|State|Error|Context]
//! (Small State Only)
//! [00000010|     ~    State     ~     ]
//! ```
//!
#![no_std]
#![allow(clippy::type_complexity)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

#[cfg(feature = "backtrace")]
extern crate std;

mod backtrace;
mod ptr;
mod raw;
mod render;
mod rtti;

#[doc(hidden)]
pub mod macros;

pub mod context;
pub mod nae;
pub mod state;

use alloc::boxed::Box;
use core::{
    error,
    fmt::{self, Debug, Display},
    ops::Deref,
    result,
};

use crate::{
    context::{Blank, Context, Contextless, Identity, IntoContext},
    nae::Nae,
    raw::RawError,
    state::{State, Stateless, Vacant},
};

pub type Result<T> = result::Result<T, Error>;

/// An error type that can carry optional state, source, and context.
#[repr(transparent)]
pub struct Error<S = Stateless>(RawError<S::Repr>)
where
    S: State + ?Sized;

impl Error {
    /// Starts building an `Error` from a source error.
    pub fn with_error<E>(err: E) -> Builder<E, Stateless, Identity<Contextless>> {
        Builder {
            err,
            state: None,
            context_fn: Identity(Contextless::new()),
        }
    }

    /// Starts building an `Error` with a typed state.
    ///
    /// The state is inlined when no source or context is attached.
    pub fn with_state<S>(state: S) -> Builder<Nae, S, Identity<Contextless>>
    where
        S: State,
    {
        Builder {
            err: Nae::new(),
            state: Some(state.into_repr()),
            context_fn: Identity(Contextless::new()),
        }
    }

    /// Starts building an `Error` with a context.
    pub fn with_context<C>(context: C) -> Builder<Nae, Stateless, Identity<C>>
    where
        C: Context,
    {
        Builder {
            err: Nae::new(),
            state: None,
            context_fn: Identity(context),
        }
    }

    /// Starts building an `Error` with a lazily evaluated context.
    ///
    /// The closure `context_fn` is called only when the error is materialized.
    pub fn with_context_fn<F>(context_fn: F) -> Builder<Nae, Stateless, F>
    where
        F: IntoContext,
    {
        Builder {
            err: Nae::new(),
            state: None,
            context_fn,
        }
    }

    /// Extracts the context and source error.
    ///
    /// Returns `None` when the corresponding requested type does not match.
    pub fn into_parts<C, E>(self) -> (Option<C>, Option<E>)
    where
        E: 'static,
        C: 'static,
    {
        let (_state, context, source) = self.0.into_parts::<C, E>();
        (context, source)
    }

    /// Helper for type inference when the state is not needed.
    pub fn stateless(self) -> Self {
        self
    }

    /// Converts to an error of another state without providing the state value.
    pub fn with_phantom_state<S>(self) -> Error<S>
    where
        S: State + ?Sized,
    {
        Error(self.0.with_phantom_state())
    }
}

impl<S> Error<S>
where
    S: State + ?Sized,
{
    /// Returns an opaque [`Error`][error::Error].
    pub fn erase(self) -> impl error::Error + Send + Sync + 'static {
        ImplError::<S>(self.0)
    }

    /// Returns a reference to an opaque [`Error`][error::Error].
    pub fn erase_ref(&self) -> &(impl error::Error + Send + Sync + 'static) {
        &self.0
    }

    /// Creates an `Error` from any [`Error`][core::error::Error].
    pub fn from_error<E>(err: E) -> Self
    where
        E: error::Error + Send + Sync + 'static,
    {
        err.into()
    }

    /// Creates an `Error` from a context.
    pub fn from_context<C>(ctx: C) -> Self
    where
        C: Context,
    {
        Self(RawError::new(None, Nae::new(), ctx))
    }

    /// Creates an `Error` from a boxed error.
    pub fn from_boxed(value: Box<dyn error::Error + Send + Sync + 'static>) -> Self {
        struct BoxError(Box<dyn error::Error + Send + Sync + 'static>);

        impl Debug for BoxError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                Debug::fmt(&*self.0, f)
            }
        }

        impl Display for BoxError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                Display::fmt(&*self.0, f)
            }
        }

        impl error::Error for BoxError {
            fn source(&self) -> Option<&(dyn error::Error + 'static)> {
                self.0.source()
            }
        }

        Self(RawError::new::<_, _>(
            None,
            BoxError(value),
            Contextless::new(),
        ))
    }

    /// Returns `true` if there is a state stored inside the error.
    pub fn has_state(&self) -> bool {
        self.0.state().is_some()
    }

    /// Converts to a stateless error. Returns `Err` when no extra details remain after dropping the state.
    pub fn try_into_stateless(self) -> result::Result<Error, Self> {
        match self.0.extract_state() {
            Err(err) => Ok(Error(err)),
            Ok((state, Some(vac))) => match vac.try_into_stateless() {
                Ok(err) => Ok(Error(err)),
                Err(vac) => Err(Error(
                    vac.try_with_state(state)
                        .expect("try_with_state will not fail with correct state"),
                )),
            },
            Ok((state, None)) => Err(Error(RawError::new(
                Some(state),
                Nae::new(),
                Contextless::new(),
            ))),
        }
    }

    /// Returns a reference to the context, if present.
    pub fn context(&self) -> Option<&(dyn Display + Send + Sync + 'static)> {
        self.0.context()
    }

    /// Returns `true` if the wrapped source error is of type `E`.
    pub fn has_source_of<E>(&self) -> bool
    where
        E: error::Error + 'static,
    {
        self.0.downcast_source_ref::<E>().is_some()
    }

    /// Returns a reference to the source error, if any.
    pub fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.0.source().map(|v| v as _)
    }

    /// Attempts to downcast the wrapped source error to `E` by shared reference.
    pub fn downcast_source_ref<E>(&self) -> Option<&E>
    where
        E: error::Error + 'static,
    {
        self.0.downcast_source_ref::<E>()
    }

    /// Attempts to downcast the wrapped source error to `E` by mutable reference.
    pub fn downcast_source_mut<E>(&mut self) -> Option<&mut E>
    where
        E: error::Error + 'static,
    {
        self.0.downcast_source_mut::<E>()
    }

    /// Returns `true` if the attached context is of type `C`.
    pub fn has_context_of<C>(&self) -> bool
    where
        C: 'static,
    {
        self.0.downcast_context_ref::<C>().is_some()
    }

    /// Attempts to downcast the context to `C` by shared reference.
    pub fn downcast_context_ref<C>(&self) -> Option<&C>
    where
        C: 'static,
    {
        self.0.downcast_context_ref::<C>()
    }

    /// Attempts to downcast the context to `C` by mutable reference.
    ///
    /// # Caveats
    ///
    /// This method returns `None` if the error is allocation-free, i.e. created from a string literal.
    /// Note that when backtrace is enabled via environment variables, all errors are heap allocated.
    pub fn downcast_context_mut<C>(&mut self) -> Option<&mut C>
    where
        C: 'static,
    {
        self.0.downcast_context_mut::<C>()
    }

    /// Consumes `self` and returns the boxed source error, if any.
    pub fn into_source(self) -> Option<Box<dyn error::Error + Send + Sync + 'static>> {
        self.0.into_source()
    }

    /// Iterates over the source error chain, starting from the immediate source.
    pub fn chain(&self) -> impl Iterator<Item = &(dyn error::Error + 'static)> {
        self.0.chain()
    }

    /// Returns the backtrace, if any.
    #[cfg_attr(docsrs, doc(cfg(feature = "backtrace")))]
    #[cfg(feature = "backtrace")]
    pub fn backtrace(&self) -> Option<&std::backtrace::Backtrace> {
        self.0.backtrace()
    }
}

impl<S> Error<S>
where
    S: State,
{
    /// Creates an `Error` from a state value.
    pub fn from_state(state: S) -> Self {
        Error(RawError::new(
            Some(S::into_repr(state)),
            Nae::new(),
            Contextless::new(),
        ))
    }

    /// Returns a reference to the attached state.
    pub fn state(&self) -> Option<&S> {
        self.0.state().map(S::from_repr_ref)
    }

    /// Consumes `self` and returns the state, context, and error.
    ///
    /// Returns `None` when the requested types do not match.
    pub fn into_parts<C, E>(self) -> (Option<S>, Option<C>, Option<E>)
    where
        E: 'static,
        C: 'static,
    {
        let (state, context, error) = self.0.into_parts::<C, E>();
        (state.map(S::from_repr), context, error)
    }

    /// Attempts to extract the state.  
    pub fn extract_state(self) -> result::Result<(S, Vacant<S>), Error> {
        match self.0.extract_state() {
            Ok((s, o)) => Ok((S::from_repr(s), Vacant::new(o))),
            Err(e) => Err(Error(e)),
        }
    }

    pub fn map_state<F, S2>(self, f: F) -> Error<S2>
    where
        F: FnOnce(S) -> S2,
        S2: State,
    {
        let Ok((state, vacant)) = match_else!(self.extract_state(), Err(err) => {
            return err.with_phantom_state();
        });
        let state = f(state);
        let Err(vacant) = match_else!(rtti::concretize::<_, Vacant<S2>>(vacant), Ok(vacant) => {
            return vacant.with_state(state);
        });
        vacant.derive(state, Contextless::new())
    }

    pub fn lift_state<S2>(self) -> Error<S2>
    where
        S2: From<S> + State,
    {
        self.map_state(S2::from)
    }
}

impl<S> Deref for Error<S>
where
    S: State + ?Sized,
{
    type Target = dyn error::Error + Send + Sync + 'static;

    fn deref(&self) -> &Self::Target {
        &self.0
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
        let Err(err) = match_else!(rtti::concretize::<E, ImplError<Stateless>>(err),
            Ok(err_unit) => return Error(err_unit.0.with_phantom_state()),
        );

        Error(RawError::new(None, err, Contextless::new()))
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

impl<S> From<Error<S>> for Box<dyn error::Error + 'static>
where
    S: State + ?Sized,
{
    fn from(value: Error<S>) -> Self {
        value.0.into_boxed_error()
    }
}

impl<S> From<Error<S>> for Box<dyn error::Error + Send + 'static>
where
    S: State + ?Sized,
{
    fn from(value: Error<S>) -> Self {
        value.0.into_boxed_error()
    }
}

impl<S> From<Error<S>> for Box<dyn error::Error + Sync + 'static>
where
    S: State + ?Sized,
{
    fn from(value: Error<S>) -> Self {
        value.0.into_boxed_error()
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
        self.0.source().map(|s| s as _)
    }
}

/// An intermediate builder for constructing an [`Error`].
#[derive(Debug)]
pub struct Builder<E, S, F>
where
    F: IntoContext,
    S: State + ?Sized,
{
    err: E,
    state: Option<S::Repr>,
    context_fn: F,
}

// Builder Case #1: generic error; state -> state
impl<E, S, F> From<Builder<E, S, F>> for Error<S>
where
    F: IntoContext,
    E: error::Error + Send + Sync + 'static,
    S: State + ?Sized,
{
    fn from(value: Builder<E, S, F>) -> Self {
        let has_state = !rtti::is_same_ty::<S, Stateless>();
        let has_error = !rtti::is_same_ty::<E, Nae>();
        let has_context = !rtti::is_same_ty::<<F::Output as Context>::Repr, Blank>();

        match (has_state, has_error, has_context) {
            (false, false, false) => unreachable!(),
            (false, true, false) => value.err.into(),
            _ => Error::<S>(RawError::new(
                value.state,
                value.err,
                value.context_fn.into_context(),
            )),
        }
    }
}

// Builder Case #2: generic error; state -> stateless
// Removed as it has no meaningful use case.
// Signature: impl<E, S, F> From<Builder<E, S, F>> for Error

// Builder Case #3: generic error; stateless -> state
impl<E, S, F> From<Builder<E, Stateless, F>> for Error<S>
where
    F: IntoContext,
    E: error::Error + Send + Sync + 'static,
    S: State,
{
    fn from(value: Builder<E, Stateless, F>) -> Self {
        let has_error = !rtti::is_same_ty::<E, Nae>();
        let has_context = !rtti::is_same_ty::<<F::Output as Context>::Repr, Blank>();

        match (has_error, has_context) {
            (false, false) => unreachable!(),
            (true, false) => value.err.into(),
            _ => Error(RawError::new(
                None,
                value.err,
                value.context_fn.into_context(),
            )),
        }
    }
}

// Builder Case #4: erratic error; state+stateless -> state
impl<S, F> From<Builder<Error<S>, Stateless, F>> for Error<S>
where
    F: IntoContext,
    S: State,
{
    fn from(value: Builder<Error<S>, Stateless, F>) -> Self {
        let has_context = !rtti::is_same_ty::<<F::Output as Context>::Repr, Blank>();

        if !has_context {
            return value.err;
        }

        let Ok((state, vacant)) = match_else!(value.err.extract_state(), Err(err) => {
            return err.with_phantom_state();
        });

        vacant.derive(state, value.context_fn.into_context())
    }
}

// Builder Case #5: erratic error; state -> stateless
// Removed as it has no meaningful use case.
// Signature: impl<S1, S, F, L> From<Builder<Error<S1>, S, F, L>> for Error

// Builder Case #6: erratic error; stateless+stateless -> state
impl<S, F> From<Builder<Error<Stateless>, Stateless, F>> for Error<S>
where
    F: IntoContext,
    S: State + ?Sized,
{
    fn from(value: Builder<Error<Stateless>, Stateless, F>) -> Self {
        let has_context = !rtti::is_same_ty::<<F::Output as Context>::Repr, Blank>();

        match has_context {
            false => value.err.with_phantom_state(),
            _ => Error(RawError::new(
                None,
                value.err.erase(),
                value.context_fn.into_context(),
            )),
        }
    }
}

/// Extension trait for working with the state.
pub trait StateExt {
    type T;
    type S: State;
    type Result<T, E>;

    ///  Extracts the state if it has been set.
    fn extract_state(
        self,
    ) -> result::Result<Self::Result<Self::T, (Self::S, Vacant<Self::S>)>, Error>
    where
        Self::S: Sized;

    fn map_state<F, S>(self, f: F) -> Self::Result<Self::T, Error<S>>
    where
        F: FnOnce(Self::S) -> S,
        S: State;

    fn lift_state<S>(self) -> Self::Result<Self::T, Error<S>>
    where
        S: State,
        S: From<Self::S>,
        Self: Sized,
    {
        self.map_state(S::from)
    }
}

impl<S1> StateExt for Error<S1>
where
    S1: State,
{
    type T = ();
    type S = S1;
    type Result<T, E> = E;

    fn extract_state(
        self,
    ) -> result::Result<Self::Result<Self::T, (Self::S, Vacant<Self::S>)>, Error>
    where
        Self::S: Sized,
    {
        self.extract_state()
    }

    fn map_state<F, S>(self, f: F) -> Self::Result<Self::T, Error<S>>
    where
        F: FnOnce(Self::S) -> S,
        S: State,
    {
        self.map_state(f)
    }
}

impl<T1, S> StateExt for result::Result<T1, Error<S>>
where
    S: State,
{
    type T = T1;
    type S = S;
    type Result<T, E> = result::Result<T, E>;

    fn extract_state(
        self,
    ) -> result::Result<Self::Result<Self::T, (Self::S, Vacant<Self::S>)>, Error>
    where
        Self::S: Sized,
    {
        match self {
            Ok(v) => Ok(Ok(v)),
            Err(err) => err.build_error().extract_state().map(|err| Err(err)),
        }
    }

    fn map_state<F, S2>(self, f: F) -> Self::Result<Self::T, Error<S2>>
    where
        F: FnOnce(Self::S) -> S2,
        S2: State,
    {
        self.map_err(|err| err.map_state(f))
    }
}

/// Extension trait for attaching context and state to an existing error.
pub trait BuilderExt: Sized {
    type Result<E>;

    type E;
    type S: State + ?Sized;
    type F: IntoContext;

    /// Attaches any value that implements [`Display`] as the error context.
    ///
    /// # Examples
    ///
    /// ```
    /// # use erratic::*;
    /// # fn foo() -> Result<()> {
    /// # let foo = || -> std::result::Result<(), std::io::Error> { unimplemented!() };
    /// # let stream_id = 1;
    /// # let filename = "";
    /// foo().with_context("file not found")?;
    /// foo().with_context(mkctx!("file not found"))?;
    /// foo().with_context(filename.to_string())?;
    /// foo().with_context(mkctx!("failed to read from stream {stream_id}"))?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    fn with_context<C>(self, context: C) -> Self::Result<Builder<Self::E, Self::S, Identity<C>>>
    where
        C: Context,
    {
        self.with_context_fn(Identity(context))
    }

    /// Attaches a typed state.
    fn with_state<S>(self, state: S) -> Self::Result<Builder<Self::E, S, Self::F>>
    where
        S: State + Sized;

    /// Attaches a lazily-evaluated context.
    fn with_context_fn<F>(self, context_fn: F) -> Self::Result<Builder<Self::E, Self::S, F>>
    where
        F: IntoContext;
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
            err,
            state: Some(state.into_repr()),
            context_fn: Identity(Contextless::new()),
        })
    }

    fn with_context_fn<F>(self, context_fn: F) -> Self::Result<Builder<Self::E, Self::S, F>>
    where
        F: IntoContext,
    {
        self.map_err(|err| Builder {
            err,
            state: None,
            context_fn,
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
    type F = Identity<Contextless>;

    fn with_state<S>(self, state: S) -> Self::Result<Builder<Self::E, S, Self::F>>
    where
        S: State,
    {
        self.map_err(|err| Builder {
            err,
            state: Some(state.into_repr()),
            context_fn: Identity(Contextless::new()),
        })
    }

    fn with_context_fn<F>(self, context_fn: F) -> Self::Result<Builder<Self::E, Self::S, F>>
    where
        F: IntoContext,
    {
        self.map_err(|err| Builder {
            err,
            state: None,
            context_fn,
        })
    }
}

impl<E1, S1, F1> BuilderExt for Builder<E1, S1, F1>
where
    F1: IntoContext,
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
        F: IntoContext,
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
    F1: IntoContext,
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
        F: IntoContext,
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

    type E = Nae;
    type S = Stateless;
    type F = Identity<Contextless>;

    fn with_state<S>(self, state: S) -> Self::Result<Builder<Self::E, S, Self::F>>
    where
        S: State,
    {
        self.ok_or(Builder {
            state: Some(state.into_repr()),
            err: Nae::new(),
            context_fn: Identity(Contextless::new()),
        })
    }

    fn with_context_fn<F>(self, context_fn: F) -> Self::Result<Builder<Self::E, Self::S, F>>
    where
        F: IntoContext,
    {
        self.ok_or(Builder {
            err: Nae::new(),
            state: None,
            context_fn,
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

impl<E1, S, F> ErrorExt for Builder<E1, S, F>
where
    E1: error::Error + Send + Sync + 'static,
    F: IntoContext,
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
    F: IntoContext,
    S: State + ?Sized,
{
    type Result<E> = E;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        Builder {
            err: self.err.erase(),
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
    F: IntoContext,
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
    F: IntoContext,
    S: State + ?Sized,
{
    type Result<E> = result::Result<T, E>;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.map_err(|err| {
            Builder {
                err: err.err.erase(),
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
    S: State + ?Sized,
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
