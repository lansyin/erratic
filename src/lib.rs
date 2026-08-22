//! This crate provides `Error<S = Stateless>`, an error type with typed state,
//! enabling applications to handle errors uniformly across different scenarios.
//!
//! ![splash](https://raw.githubusercontent.com/lansyin/erratic/c384432ef0b892c98d3c303d0fc6c6322a8a8389/splash.svg)
//!
//! # Quick Start
//!
//! In most cases, `Error` can serve as a drop-in replacement for `Box<dyn Error>`.
//! Compared to the latter, it occupies only 1 usize and eliminates allocations
//! altogether when constructed from a literal string or a small state.
//!
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
//! When constructing an error, you can attach a context to it. All helper macros support
//! constructing context from a format string.
//!
//! ```
//! # use std::env;
//! # const MAX_CHUNK_SIZE: usize = 1024;
//! # type Reader = ();
//! # type Writer = ();
//! # mod rb { pub fn ringbuf(_: usize) -> ((), ()) { unimplemented!() }}
//! # mod metrics { pub fn attach<T>(_: T) -> erratic::Result<T> { unimplemented!() } }
//! use erratic::*;
//!
//! fn ringbuf(size: usize) -> Result<(Writer, Reader)> {
//!     mksure!(size > MAX_CHUNK_SIZE)?; // Prints the operand values on failure.
//!
//!     let pair = metrics::attach(rb::ringbuf(size))
//!         .with_context("failed to attatch metrics")?;
//!     Ok(pair)
//! }
//! ```
//!
//! # Binding State
//!
//! When propagating domain errors, you can attach a state to it. A small state
//! with no other components incurs no heap allocation.
//!
//! ```
//! # struct Writer { id: usize }
//! # impl Writer {
//! #     fn write(&mut self, _: &[u8]) -> Result<()> { unimplemented!() }
//! #     fn reserve_chunk(&self, _: usize) -> Result<()> { unimplemented!() }
//! # }
//! use erratic::*;
//!
//! #[derive(Debug)]
//! enum State { RetryLater } // Smaller than 1 usize.
//!
//! fn try_write(w: &mut Writer, chunk: &[u8]) -> Result<(), Error<State>> {
//!     w.reserve_chunk(chunk.len())
//!         .ok()
//!         .with_state(State::RetryLater)?; // No alloc.
//!     w.write(chunk)
//!         .with_context(mkctx!("failed to write to {}", w.id))?;
//!     Ok(())
//! }
//! ```
//!
//! When no runtime state is actually stored, errors can be cheaply converted between different state types.
//! This allows infrastructure errors to cross any number of layers with no extra allocation, domain errors
//! avoid the heap entirely, and both share the same `Error<S>` type.
//!
//! ```
//! # use std::{thread, result};
//! # use erratic::*;
//! # #[derive(Debug)]
//! # enum State { RetryLater }
//! # struct Writer;
//! # fn try_write(_: &mut Writer, chunk: &[u8]) -> Result<(), Error<State>> { unimplemented!() }
//! fn write(w: &mut Writer, chunk: &[u8]) -> Result<()> {
//!     while let Err((state, _)) = try_write(w, chunk).extract_state()? {
//!         // Handle domain errors.                                  ^ Bubble up infra errors.
//!         match state {
//!             State::RetryLater => thread::yield_now(),
//!             // ..
//!         }
//!     }
//!     Ok(())
//! }
//! ```
//!
//! The `?` operator covers the most common cases, regardless of whether the return type carries state:
//!
//! | Source Type        | Return Type   | Explanation                                           |
//! | :----------------- | :------------ | :---------------------------------------------------- |
//! | `impl Error`       | `Error<_>`    | Wraps any standard error type.                        |
//! | `Builder<..>`      | `Error<_>`    | Builds an error from state, context, and/or source.   |
//! | `Error<Stateless>` | `Error<S>`    | Cheaply converts a stateless error to a stateful one. |
//!
//! States are meant to be handled explicitly. Several utility methods are provided:
//!
//! | Method          | Conversion                                    | Explanation                                      |
//! | :-------------- | :-------------------------------------------- | :----------------------------------------------- |
//! | `extract_state` | `Error<S>` -> `Result<(S, Vacant<S>), Error>` | Takes the state out, or propagate the error.     |
//! | `map_state`     | `Error<S>` -> `Error<S2>`                     | Transforms the state with a closure.             |
//! | `lift_state`    | `Error<S>` -> `Error<S2>` where `S2: From<S>` | Transforms the state via `From`.                 |
//! | `erase_state`   | `Error<S>` -> `Error<Stateless>`              | Erases the state, keeping the message unchanged. |
//!
//! # Formatting
//!
//! If the error has a state and/or a context, it builds error messages from them. Otherwise, it acts as an error container,
//! inheriting the message from its source. When wrapped, the container itself will not be added as another source layer,
//! preventing duplicate messages in the chain.
//!
//! ```text
//! <error> ::= <source>
//!           | <state>": "<context>
//!           | <context>
//!           | <state>
//! <chain> ::= <error>
//!           | <error>"\n  -> "<chain>
//! ```
//!
//! By default, only the top-level error is shown during formatting. To print the full error chain,
//! format with alternate or debug specifiers.
//!
//! | Specifier | Explanation                                               |
//! | :-------- | :-------------------------------------------------------- |
//! | `{}`      | Prints only the top-level error.                          |
//! | `{:#}`    | Prints the full error chain.                              |
//! | `{:?}`    | Prints the full error chain with backtrace, if captured.  |
//! | `{:#?}`   | Prints all information in a struct-like format.           |
//!
//! # Backtrace
//!
//! When the `backtrace` feature is enabled and backtrace capture is configured via
//! [environment variables][backtrace-conf], `Error<S>` automatically captures a backtrace if there isn't
//! one already in the source chain. The backtrace will be appended after the error chain during debug
//! formatting, unless the minus sign, e.g. `{:-?}`, is specified to suppress it.
//!
//! [backtrace-conf]: https://doc.rust-lang.org/std/backtrace/index.html#environment-variables
//!
//! # Representation
//!
//! Type-wise, `Error<S>` is an internally tagged union, and it requires pointers to be aligned to 4 bytes,
//! freeing up the lower 2 bits to encode its discriminant. Pointer tagging in this crate fully follows
//! [strict provenance][strict-provenance], and is verified by Miri.
//!
//! [strict-provenance]: https://doc.rust-lang.org/1.89.0/std/ptr/index.html#strict-provenance
//!
//! The error has three possible layouts. When constructed from a literal, it stores a pointer to the literal.
//! When constructed from a small state, it stores the state inline. Otherwise, it points to a heap-allocated object
//! containing a vtable and potentially a state, source, and/or context.
//!
//! ```plaintext
//! ┌Error<State>─────────╎───┐   ┌ConstBody─────┐
//! │ Align4Ref<ConstBody>╎00 ├───┤ &'static str │
//! └─────────────────────╎───┘   └──────────────┘
//! ┌Error<State>─────────╎───┐   ┌BoxedBody─────────────┬────────────────────┬────────┬─────────┐
//! │ Align4Own<BoxedBody>╎01 ├───┤ Align4Ref<VTable>╎ST │ MaybeUninit<State> │ Source │ Context │
//! └─────────────────────╎───┘   └─────────┼──────────┼─┴──────────────┼─────┴────────┴─────────┘
//! ┌Error<State>─┬───────╎───┐   ┌VTable───┴──┬─╌     │╭ ST=00:Uninit ╮│
//! │    State    │ 000000╎10 │   │ Drop::drop │       ╰┤ ST=01:Active ├╯
//! └─────────────┴───────╎───┘   └────────────┴─╌      ╰ ST=10:Erased ╯
//! ```
//!
#![no_std]
#![allow(clippy::type_complexity)]
#![allow(clippy::collapsible_if)] // Suggested by Rust 1.96 clippy, but our MSRV is 1.89.
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

#[cfg(feature = "backtrace")]
extern crate std;

mod raw;
mod render;
mod rtti;

#[doc(hidden)]
pub mod macros;
#[doc(hidden)]
pub mod test_fixtures;

pub mod builder;
pub mod context;
pub mod state;

use alloc::boxed::Box;
use core::{
    convert::Infallible,
    error,
    fmt::{self, Debug, Display},
    ops::{Deref, DerefMut},
};

use crate::{
    builder::Builder,
    context::{Context, ContextFn, Contextless, Identity, Printable},
    raw::{ErasedRawError, RawError},
    state::{State, Stateless, Vacant},
};

pub type Result<T, E = Error> = core::result::Result<T, E>;

/// An error type that can carry optional state, source, and context.
#[repr(transparent)]
pub struct Error<S = Stateless>(RawError<S::Repr>)
where
    S: State + ?Sized;

impl<S> Error<S>
where
    S: State + ?Sized,
{
    /// Creates an `Error` from a context.
    pub fn from_context<C>(ctx: C) -> Self
    where
        C: Context,
    {
        Self(RawError::from_error(None, None::<Infallible>, ctx))
    }

    /// Creates an `Error` from any [`Error`][core::error::Error].
    pub fn from_error<E>(err: E) -> Self
    where
        E: error::Error + Send + Sync + 'static,
    {
        err.into()
    }

    /// Creates an `Error` from a boxed error.
    pub fn from_boxed(boxed: Box<dyn error::Error + Send + Sync + 'static>) -> Self {
        Self(RawError::from_boxed(None, boxed, Contextless::new()))
    }

    /// Returns `true` if there is a state stored inside the error.
    pub fn has_state(&self) -> bool {
        self.0.state().is_some()
    }

    /// Erases the state type, while keeping the error message unchanged.
    pub fn erase_state(self) -> Error {
        Error(self.0.erase_state())
    }

    /// Returns a reference to the context, if present.
    pub fn context(&self) -> Option<&(dyn Printable + Send + Sync + 'static)> {
        self.0.context()
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

    /// Returns a reference to the source error, if any.
    pub fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.0.source().map(|v| v as _)
    }

    /// Consumes `self` and returns the boxed source error, if any.
    pub fn into_source(self) -> Option<Box<dyn error::Error + Send + Sync + 'static>> {
        self.0.into_source()
    }

    /// Returns `true` if the wrapped source error is of type `E`.
    pub fn has_source_of<E>(&self) -> bool
    where
        E: error::Error + 'static,
    {
        self.0.downcast_source_ref::<E>().is_some()
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

    /// Returns the root cause of the error.
    pub fn root(&self) -> Option<&(dyn error::Error + 'static)> {
        self.chain().last()
    }

    /// Attempts to find the first error of the given type in the error chain.
    pub fn find<E>(&self) -> Option<&E>
    where
        E: error::Error + 'static,
    {
        for err in self.chain() {
            if let Some(err) = err.downcast_ref::<E>() {
                return Some(err);
            }
        }
        None
    }

    /// Iterates over the error chain.
    ///
    /// If this error has its own context or state, it appears first;
    /// otherwise the chain starts from the source.
    pub fn chain(&self) -> impl Iterator<Item = &(dyn error::Error + 'static)> {
        self.0.chain()
    }

    /// Checks if the state type is small enough to be stored inline.
    ///
    /// Note: Allocations can still occur even when this function returns `true`, in the following cases:
    ///
    /// - The `backtrace` feature is enabled and runtime backtrace capture is active.
    /// - The error contains additional context or a source error.
    pub const fn is_state_inlinable() -> bool {
        RawError::<S::Repr>::is_state_inlinable()
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
        Error(RawError::from_error(
            Some(S::into_repr(state)),
            None::<Infallible>,
            Contextless::new(),
        ))
    }

    /// Returns a reference to the attached state.
    pub fn state(&self) -> Option<&S> {
        self.0.state().map(S::from_repr_ref)
    }

    /// Attempts to extract the state.  
    pub fn extract_state(self) -> Result<(S, Vacant<S>), Error> {
        match self.0.extract_state() {
            Ok((s, o)) => Ok((S::from_repr(s), Vacant::new(o))),
            Err(e) => Err(Error(e)),
        }
    }

    /// Converts to another state via a closure.
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

    /// Converts to another state via the `From` trait.
    pub fn lift_state<S2>(self) -> Error<S2>
    where
        S2: From<S> + State,
    {
        self.map_state(S2::from)
    }

    /// Consumes `self` and returns the state, context, and error.
    ///
    /// Returns `None` when the requested types do not match.
    pub fn into_parts<C, E>(self) -> (Option<S>, Option<C>, Option<E>)
    where
        E: 'static,
        C: 'static,
    {
        let (state, context, err) = self.into_parts_mapped::<C, E>();
        (state.map(S::from_repr), context, err)
    }
}

impl Error {
    /// Converts to an error of another state without providing the state value.
    pub fn with_phantom_state<S>(self) -> Error<S>
    where
        S: State + ?Sized,
    {
        Error(self.0.with_phantom_state())
    }

    /// Extracts the context and source error.
    ///
    /// Returns `None` when the corresponding requested type does not match.
    pub fn into_parts<C, E>(self) -> (Option<C>, Option<E>)
    where
        E: 'static,
        C: 'static,
    {
        let (_state, context, err) = self.into_parts_mapped::<C, E>();
        (context, err)
    }

    /// Identity helper to aid type inference.
    pub fn stateless(self) -> Self {
        self
    }
}

impl<S> Error<S>
where
    S: State + ?Sized,
{
    fn into_parts_mapped<C, E>(self) -> (Option<S::Repr>, Option<C>, Option<E>)
    where
        E: 'static,
        C: 'static,
    {
        if rtti::is_same_ty::<E, Error>() {
            let (state, context, err) = self.0.into_parts::<C, ErasedRawError>();
            let err = err
                .map(|erased| match erased.try_into_stateless() {
                    Ok(err) => Error::<Stateless>(err),
                    Err(erased) => Error(RawError::from_erased(
                        None,
                        Some(erased),
                        Contextless::new(),
                    )),
                })
                .and_then(|err| rtti::concretize::<_, E>(err).ok());
            (state, context, err)
        } else {
            self.0.into_parts::<C, E>()
        }
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

impl<S> DerefMut for Error<S>
where
    S: State + ?Sized,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<E, S> From<E> for Error<S>
where
    E: error::Error + Send + Sync + 'static,
    S: State + ?Sized,
{
    fn from(err: E) -> Self {
        Error(RawError::from_error(None, Some(err), Contextless::new()))
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
    S: State,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(&self.0, f)
    }
}

impl<S> Display for Error<S>
where
    S: State,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl Debug for Error<Stateless> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(&self.0, f)
    }
}

impl Display for Error<Stateless> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

/// Extension trait for working with the state.
pub trait StateExt {
    type T;
    type S: State;
    type Result<T, E>;

    ///  Extracts the state if it has been set.
    fn extract_state(self) -> Result<Self::Result<Self::T, (Self::S, Vacant<Self::S>)>, Error>
    where
        Self::S: Sized;

    /// Converts to another state via a closure.
    fn map_state<F, S>(self, f: F) -> Self::Result<Self::T, Error<S>>
    where
        F: FnOnce(Self::S) -> S,
        S: State;

    /// Converts to another state via the `From` trait.
    fn lift_state<S>(self) -> Self::Result<Self::T, Error<S>>
    where
        S: State,
        S: From<Self::S>,
        Self: Sized,
    {
        self.map_state(S::from)
    }

    /// Erases the state type, while keeping the error message unchanged.
    fn erase_state(self) -> Self::Result<Self::T, Error>;
}

impl<S1> StateExt for Error<S1>
where
    S1: State,
{
    type T = ();
    type S = S1;
    type Result<T, E> = E;

    fn extract_state(self) -> Result<Self::Result<Self::T, (Self::S, Vacant<Self::S>)>, Error>
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

    fn erase_state(self) -> Self::Result<Self::T, Error> {
        self.erase_state()
    }
}

impl<T1, S> StateExt for Result<T1, Error<S>>
where
    S: State,
{
    type T = T1;
    type S = S;
    type Result<T, E> = Result<T, E>;

    fn extract_state(self) -> Result<Self::Result<Self::T, (Self::S, Vacant<Self::S>)>, Error>
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

    fn erase_state(self) -> Self::Result<Self::T, Error> {
        self.map_err(|err| err.erase_state())
    }
}

/// Extension trait for attaching context and state to an existing error.
pub trait BuilderExt: Sized {
    type Result<E>;

    type E;
    type S: State + ?Sized;
    type F: ContextFn;

    /// Attaches any value that implements [`Context`] as the error context.
    ///
    /// # Examples
    ///
    /// ```
    /// # use erratic::*;
    /// # fn foo() -> Result<()> {
    /// # let foo = || -> Result<(), std::io::Error> { unimplemented!() };
    /// # let stream_id = 1;
    /// # let filename = "";
    /// foo().with_context("file not found")?;
    /// foo().with_context(filename.to_string())?;
    /// foo().with_context(mkctx!("cannot read {stream_id}"))?;
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

    /// Attaches a typed state if the previous error meets the given condition.
    fn with_state_if<S, F>(self, state: S, f: F) -> Self::Result<Builder<Self::E, S, Self::F>>
    where
        S: State + Sized,
        F: FnOnce(&Self::E) -> bool;

    /// Attaches a lazily-evaluated context.
    fn with_context_fn<F>(self, context_fn: F) -> Self::Result<Builder<Self::E, Self::S, F>>
    where
        F: ContextFn;
}

impl<T> BuilderExt for Result<T, Error<Stateless>> {
    type Result<E> = Result<T, E>;

    type E = Error;
    type S = Stateless;
    type F = Identity<Contextless>;

    fn with_state<S>(self, state: S) -> Self::Result<Builder<Self::E, S, Self::F>>
    where
        S: State,
    {
        self.map_err(|err| Builder::with_error(err).with_state(state))
    }

    fn with_state_if<S, F>(self, state: S, f: F) -> Self::Result<Builder<Self::E, S, Self::F>>
    where
        S: State + Sized,
        F: FnOnce(&Self::E) -> bool,
    {
        self.map_err(|err| {
            if f(&err) {
                Builder::with_error(err).with_state(state)
            } else {
                Builder::with_error(err).with_phantom_state()
            }
        })
    }

    fn with_context_fn<F>(self, context_fn: F) -> Self::Result<Builder<Self::E, Self::S, F>>
    where
        F: ContextFn,
    {
        self.map_err(|err| Builder::with_error(err).with_context_fn(context_fn))
    }
}

/// Extension trait for materializing or erasing an error.
pub trait ErrorExt: Sized {
    type Result<E>;
    type S: State + ?Sized;

    /// Materializes the final [`Error<Self::S>`].
    fn build_error(self) -> Self::Result<Error<Self::S>>;
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
}

impl<T, E1> ErrorExt for Result<T, E1>
where
    E1: error::Error + Send + Sync + 'static,
{
    type Result<E> = Result<T, E>;
    type S = Stateless;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self.map_err(Error::from)
    }
}

impl<T, S> ErrorExt for Result<T, Error<S>>
where
    S: State + ?Sized,
{
    type Result<E> = Result<T, E>;
    type S = S;

    fn build_error(self) -> Self::Result<Error<Self::S>> {
        self
    }
}
