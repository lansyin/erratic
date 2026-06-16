mod backtrace;
mod erased;
mod ptr;
mod source;

use alloc::{boxed::Box, format};
use core::{
    any::{Any, TypeId},
    convert::Infallible,
    error,
    fmt::{Debug, Display},
    mem::{self, ManuallyDrop, MaybeUninit},
    ops::{Deref, DerefMut},
    ptr::NonNull,
    result,
};

use crate::{
    context::{self, Context, Empty},
    fmt::{self, DebugDisplay},
    match_else,
    raw::{
        erased::ErasedRawError,
        ptr::{Align4, Align4Own, Align4PtrCompat, Align4Ref, Metadata, Mut, Ref},
        source::{IndirectSource, NoSource, Source, WithBacktraceSource},
    },
    rtti,
};
use backtrace::WithBacktrace;

pub use source::BoxedSource;

/// Triple-state error storage.
///
/// # Safety invariants
///
/// All three union variants share the invariant that the first byte's lowest 2 bits
/// encode the discriminant:
///   - `00`: [`const_body`](RawError::const_body) (static string literal)
///   - `01`: [`boxed_body`](RawError::boxed_body) (heap-allocated [`DynBody`])
///   - `10`: [`inline_body`](RawError::inline_body) (inline [`Align4PtrCompat`])
///
/// The discriminant is written at construction and must never be modified afterward.
#[repr(C)]
pub union RawError<S = Infallible>
where
    S: 'static,
{
    const_body: ManuallyDrop<Align4Ref<'static, ConstBody>>,
    boxed_body: ManuallyDrop<ErasedDynBody>,
    inline_body: ManuallyDrop<Align4PtrCompat<S>>,
}

enum SelectRef<'a, S>
where
    S: 'static,
{
    Const(&'a Align4Ref<'static, ConstBody>),
    Boxed(&'a ErasedDynBody),
    Inline(&'a Align4PtrCompat<S>),
}

enum SelectMut<'a, S>
where
    S: 'static,
{
    Const(&'a mut Align4Ref<'static, ConstBody>),
    Boxed(&'a mut ErasedDynBody),
    Inline(&'a mut Align4PtrCompat<S>),
}

enum SelectOwn<S>
where
    S: 'static,
{
    Const(Align4Ref<'static, ConstBody>),
    Boxed(ErasedDynBody),
    Inline(Align4PtrCompat<S>),
}

impl<S> RawError<S> {
    const KIND_CONST: Metadata = Metadata::_0;
    const KIND_BOXED: Metadata = Metadata::_1;
    const KIND_INLINE: Metadata = Metadata::_2;

    /// Reads the 2-bit discriminant from the first byte of the union.
    fn kind(&self) -> Metadata {
        // # Safety: All three union variants have `repr(C)` layout, and their first field
        // is `Align4Ptr`/`Align4PtrCompat`, all of which store the metadata at offset 0.
        unsafe { Metadata((&raw const (*self) as *const u8).read() & Metadata::MASK) }
    }

    /// Selects a shared reference to the active union variant.
    fn select_ref(&self) -> SelectRef<'_, S> {
        // # Safety: `RawError::kind` always returns a valid discriminant.
        unsafe {
            match self.kind() {
                Self::KIND_CONST => SelectRef::Const(&self.const_body),
                Self::KIND_BOXED => SelectRef::Boxed(&self.boxed_body),
                Self::KIND_INLINE => SelectRef::Inline(&self.inline_body),
                _ => unreachable!(),
            }
        }
    }

    /// Selects a mutable reference to the active union variant.
    fn select_mut(&mut self) -> SelectMut<'_, S> {
        // # Safety: `RawError::kind` always returns a valid discriminant.
        unsafe {
            match self.kind() {
                Self::KIND_CONST => SelectMut::Const(&mut self.const_body),
                Self::KIND_BOXED => SelectMut::Boxed(&mut self.boxed_body),
                Self::KIND_INLINE => SelectMut::Inline(&mut self.inline_body),
                _ => unreachable!(),
            }
        }
    }

    /// Takes ownership of the active union variant.
    fn select_own(self) -> SelectOwn<S> {
        let kind = self.kind();
        let mut this = ManuallyDrop::new(self);

        // # Safety: `RawError::kind` always returns a valid discriminant.
        unsafe {
            match kind {
                Self::KIND_CONST => SelectOwn::Const(ManuallyDrop::take(&mut this.const_body)),
                Self::KIND_BOXED => SelectOwn::Boxed(ManuallyDrop::take(&mut this.boxed_body)),
                Self::KIND_INLINE => SelectOwn::Inline(ManuallyDrop::take(&mut this.inline_body)),
                _ => unreachable!(),
            }
        }
    }
}

impl RawError {
    /// Constructs a const-variant [`RawError`] from a typed literal.
    fn try_new_const<C>() -> Option<Self>
    where
        C: Context,
    {
        // Note: Explicitly check the fallback context first as we CANNOT return in the const block.
        #[allow(clippy::question_mark)]
        if C::FALLBACK.is_none() {
            return None;
        }
        // Note: Relies on const promotion to produce a new constant.
        let body: &'static Align4<ConstBody> = &const {
            let literal = match C::FALLBACK {
                Some(v) => v, // Note: As of Rust 1.96, unwrap_or_default is unavailable in const blocks.
                None => "", // Note: This branch is never taken; it only exists to keep rustc happy.
            };
            Align4(ConstBody { context: literal })
        };
        Some(Self {
            const_body: ManuallyDrop::new(Align4Ref::new(body, Self::KIND_CONST)),
        })
    }
}

impl<S> RawError<S> {
    /// Constructs an inline-variant [`RawError`] with `state` stored directly.
    fn try_new_inline(state: S) -> result::Result<Self, S>
    where
        S: Debug + Send + Sync + 'static,
    {
        Ok(Self {
            inline_body: ManuallyDrop::new(Align4PtrCompat::new(Self::KIND_INLINE, state)?),
        })
    }

    /// Check if the [`RawError`] contains only a source.
    fn is_source_only(&self) -> bool {
        match self.select_ref() {
            SelectRef::Const(_) | SelectRef::Inline(_) => false,
            SelectRef::Boxed(body) => {
                let vt = DynBody::vtable(body.borrow());
                let has_state = unsafe { (vt.has_state)(body.borrow()) };
                let has_context = unsafe { (vt.context)(body.borrow()).is_some() };
                let has_source = unsafe { (vt.source)(body.borrow()).is_some() };

                matches!((has_state, has_context, has_source), (false, false, true))
            }
        }
    }

    fn new_boxed<E, C>(state: Option<S>, source: E, context: C) -> Self
    where
        S: Debug + Send + Sync + 'static,
        E: Source + Send + Sync + 'static,
        C: Debug + Display + Send + Sync + 'static,
    {
        let (vtable, state) = DynBody::<S, E, C>::vtable_from_state(state);

        // # Safety
        //
        // The `Align4Own` pointer is cast to `DynBody<Infallible, (), ()>` for uniform storage.
        // This is valid because all monomorphizations of `DynBody<S, E, C::Repr>` share
        // the same vtable pointer, and the concrete `S`, `E`, `C` are erased.
        // The cast only changes the type parameter defaults — it does not violate the layout
        // because `()` is a ZST.
        RawError::<S> {
            boxed_body: ManuallyDrop::new(ErasedDynBody::from_typed(Align4Own::from_boxed(
                Box::new(Align4(DynBody::<S, E, C> {
                    vtable,
                    state,
                    source,
                    context: helper::Exclude::new(context),
                })),
                RawError::<S>::KIND_BOXED,
            ))),
        }
    }
}

impl RawError {
    /// Converts to a state-tagged error without storing any runtime state.
    pub fn with_phantom_state<S>(self) -> RawError<S>
    where
        S: 'static,
    {
        match self.select_own() {
            SelectOwn::Const(body) => RawError {
                const_body: ManuallyDrop::new(body),
            },
            SelectOwn::Inline(_body) => unreachable!(),
            SelectOwn::Boxed(body) => RawError {
                boxed_body: ManuallyDrop::new(body),
            },
        }
    }
}

impl<S> RawError<S> {
    /// Constructs a [`RawError`].
    pub fn new<E, C>(state: Option<S>, source: Option<E>, context: C) -> Self
    where
        S: Debug + Send + Sync + 'static,
        E: Source + Send + Sync + 'static,
        C: context::Context,
    {
        fn new_1<S, E, C>(state: Option<S>, source: E, context: C) -> RawError<S>
        where
            S: Debug + Send + Sync + 'static,
            E: Source + Send + Sync + 'static,
            C: context::Context,
        {
            match WithBacktrace::try_attach(source) {
                Ok(source) => new_2(state, source, context),
                Err(source) => {
                    let has_source = source.error_ref().is_some();
                    let has_context = !C::is_contextless();

                    match (state, has_context, has_source) {
                        (Some(state), false, false) => {
                            let Err(state) = match_else!(RawError::try_new_inline(state), Ok(this) => {
                                return this;
                            });
                            new_2(Some(state), source, context)
                        }
                        (None, true, false) => match context.try_into_repr() {
                            None => match RawError::try_new_const::<C>() {
                                Some(raw) => raw.with_phantom_state(),
                                None => new_2(None, source, Empty::new()),
                            },
                            Some(context) => new_2(None, source, context),
                        },
                        (None, false, true) => {
                            match rtti::concretize::<_, ErasedRawError>(source) {
                                Ok(erased) => match erased.try_into_stateless() {
                                    Ok(stateless) => stateless.with_phantom_state(),
                                    Err(erased) => new_2(None, erased, context),
                                },
                                Err(source) => new_2(None, source, context),
                            }
                        }
                        (state, _, _) => new_2(state, source, context),
                    }
                }
            }
        }

        fn new_2<S, E, C>(state: Option<S>, source: E, context: C) -> RawError<S>
        where
            S: Debug + Send + Sync + 'static,
            E: Source + Send + Sync + 'static,
            C: context::Context,
        {
            let context = context.try_into_repr();
            let context_fallback = C::FALLBACK;

            match (context, context_fallback) {
                (Some(context), _) => new_3(state, source, context),
                (None, Some(context)) => new_3(state, source, context),
                (None, None) => new_3(state, source, Empty::new()),
            }
        }

        fn new_3<S, E, C>(state: Option<S>, source: E, context: C) -> RawError<S>
        where
            S: Debug + Send + Sync + 'static,
            E: Source + Send + Sync + 'static,
            C: Debug + Display + Send + Sync + 'static,
        {
            let Err(source) = match_else!(rtti::concretize::<_, ErasedRawError>(source), Ok(erased) => {
                match erased.try_into_stateless() {
                    Ok(stateless) => match IndirectSource::try_new(stateless) {
                        Ok(source) => return RawError::new_boxed(state, source, context),
                        Err(stateless) => return RawError::new_boxed(state, stateless.erase(), context),
                    },
                    Err(erased) => {
                        return RawError::new_boxed(state, erased, context)
                    }
                }
            });

            RawError::new_boxed(state, source, context)
        }

        let Some(source) = source else {
            return new_1(state, NoSource, context);
        };
        let Err(source) = match_else!(rtti::concretize::<_, BoxedSource>(source), Ok(BoxedSource(source)) => {
            let Err(source) = match_else!(source.downcast::<ErasedRawError>(), Ok(erased) => {
                return new_1(state, *erased, context);
            });
            let Err(source) = match_else!(source.downcast::<Align4<DynBody<Infallible, BoxedSource, Empty>>>(), Ok(boxed) => {
                let Align4(body) = *boxed;
                let (_, source, _) = body.destruct();
                return new_1(state, source, context);
            });

            return new_1(state, BoxedSource(source), context);
        });

        new_1(state, source, context)
    }

    /// Returns a reference to the displayable context.
    pub fn context(&self) -> Option<&'_ (dyn DebugDisplay + Send + Sync + 'static)> {
        match self.select_ref() {
            // Safety: Projection from `ConstBody` to `ConstBody::context` is safe.
            SelectRef::Const(body) => unsafe {
                Some(
                    body.borrow()
                        .project(|body| &raw const (*body).context)
                        .deref(),
                )
            },
            SelectRef::Boxed(body) => unsafe {
                let vtable = DynBody::vtable(body.borrow());
                // Safety: The body pointer is confirmed valid.
                (vtable.context)(body.borrow()).map(|c| c as _)
            },
            SelectRef::Inline(_body) => None,
        }
    }

    /// Returns a reference to the wrapped source error, if present.
    pub fn source(&self) -> Option<&(dyn error::Error + Send + Sync + 'static)> {
        match self.select_ref() {
            SelectRef::Const(_body) => None,
            SelectRef::Inline(_body) => None,
            SelectRef::Boxed(body) => unsafe {
                let vtable = DynBody::vtable(body.borrow());
                // Safety: The body pointer is confirmed valid.
                (vtable.source)(body.borrow())
            },
        }
    }

    /// Returns a mutable reference to the wrapped source error, if present.
    pub fn source_mut(&mut self) -> Option<&mut (dyn error::Error + Send + Sync + 'static)> {
        match self.select_mut() {
            SelectMut::Const(_body) => None,
            SelectMut::Inline(_body) => None,
            SelectMut::Boxed(body) => unsafe {
                let vtable = DynBody::vtable(body.borrow());
                // Safety: The body pointer is confirmed valid.
                (vtable.source_mut)(body.borrow_mut())
            },
        }
    }

    /// Attempts to downcast the stored source error to `E`.
    ///
    /// Returns `None` if the source is not of type `E` or does not exist.
    pub fn downcast_source_ref<E>(&self) -> Option<&E>
    where
        E: error::Error + 'static,
    {
        self.source()?.downcast_ref::<E>()
    }

    /// Attempts to downcast the stored source error to `E`.
    ///
    /// Returns `None` if the source is not of type `E` or does not exist.
    pub fn downcast_source_mut<E>(&mut self) -> Option<&mut E>
    where
        E: error::Error + 'static,
    {
        self.source_mut()?.downcast_mut::<E>()
    }

    /// Attempts to downcast the stored context to `C`.
    pub fn downcast_context_ref<C>(&self) -> Option<&C>
    where
        C: 'static,
    {
        match self.select_ref() {
            SelectRef::Const(body) => {
                rtti::concretize_ref::<_, C>(&body.borrow().deref().context).ok()
            }
            SelectRef::Inline(_body) => None,
            SelectRef::Boxed(body) => unsafe {
                let vtable = DynBody::vtable(body.borrow());
                let mut result = None::<&C>;
                // Safety: The body pointer is confirmed valid.
                (vtable.downcast_context_ref)(
                    body.borrow(),
                    TypeId::of::<C>(),
                    NonNull::from(&mut result).cast(),
                );
                result
            },
        }
    }

    /// Attempts to downcast the stored context to `C` by mutable reference.
    pub fn downcast_context_mut<C>(&mut self) -> Option<&mut C>
    where
        C: 'static,
    {
        match self.select_mut() {
            SelectMut::Const(_body) => None,
            SelectMut::Inline(_body) => None,
            SelectMut::Boxed(body) => unsafe {
                let vtable = DynBody::vtable(body.borrow());
                let mut result = None::<&mut C>;
                // Safety: The body pointer is confirmed valid.
                (vtable.downcast_context_mut)(
                    body.borrow_mut(),
                    TypeId::of::<C>(),
                    NonNull::from(&mut result).cast(),
                );
                result
            },
        }
    }

    /// Returns a shared reference to the stored state.
    pub fn state(&self) -> Option<&S> {
        match self.select_ref() {
            SelectRef::Const(_body) => None,
            SelectRef::Inline(_body) => unsafe {
                // Safety: Access `InlineBody::value` is safe.
                Some(self.inline_body.borrow_value())
            },
            SelectRef::Boxed(body) => unsafe {
                let vtable = DynBody::vtable(body.borrow());
                let mut state = None::<&S>;
                // Safety: The body and state pointers are confirmed valid.
                (vtable.state)(
                    body.borrow(),
                    TypeId::of::<S>(),
                    NonNull::from(&mut state).cast(),
                );

                state
            },
        }
    }

    /// Consumes `self` and returns the boxed source error, if any.
    pub fn into_source(self) -> Option<Box<dyn error::Error + Send + Sync + 'static>> {
        match self.select_own() {
            SelectOwn::Const(_body) => None,
            SelectOwn::Inline(_body) => None,
            SelectOwn::Boxed(body) => unsafe {
                let vtable = DynBody::vtable(body.borrow());
                // Safety: The body pointer is confirmed valid.
                (vtable.into_source)(body)
            },
        }
    }

    /// Consumes `self` and extracts the source error and paylaad by type.
    ///
    /// Returns `None` if the types do not match or no source/paylaad exists.
    ///
    /// # Existence Guarantee
    /// It's guaranteed that at least one components exist.
    pub fn into_parts<C, E>(self) -> (Option<S>, Option<C>, Option<E>)
    where
        E: 'static,
        C: 'static,
    {
        match self.select_own() {
            SelectOwn::Const(body) => {
                // Safety: The project to context is inbound.
                let context =
                    unsafe { body.borrow().project(|v| &raw const (*v).context).copied() };
                let context = rtti::concretize::<_, C>(context).ok();
                (None, context, None)
            }
            SelectOwn::Inline(body) => (Some(body.into_value()), None, None),
            SelectOwn::Boxed(body) => unsafe {
                let vtable = DynBody::vtable(body.borrow());
                let mut state: Option<S> = None;
                let mut context: Option<C> = None;
                let mut err: Option<E> = None;

                // Safety: The body, state, context, and error pointers are confirmed valid.
                (vtable.into_parts)(
                    body,
                    &mut err as &mut dyn Any,
                    &mut context as &mut dyn Any,
                    &mut state as &mut dyn Any,
                );

                (state, context, err)
            },
        }
    }

    pub fn extract_state(self) -> result::Result<(S, Option<RawVacant>), RawError> {
        match self.select_own() {
            SelectOwn::Const(body) => Err(RawError {
                const_body: ManuallyDrop::new(body),
            }),
            SelectOwn::Inline(body) => Ok((body.into_value(), None)),
            SelectOwn::Boxed(body) => {
                unsafe {
                    let vt = DynBody::vtable(body.borrow());
                    let mut state_dst = None::<S>;
                    // Safety: The body, state pointers are confirmed valid.
                    let re = (vt.extract_state)(body, &mut state_dst as &mut dyn Any);

                    match (state_dst, re) {
                        (Some(state), Ok(vacant)) => Ok((state, Some(vacant))),
                        (None, Err(body)) => Err(RawError {
                            boxed_body: ManuallyDrop::new(body),
                        }),
                        (None, Ok(_)) | (Some(_), Err(_)) => {
                            unreachable!() // Note: `state_dst` becomes `Some` iff `extract_state` returns `Ok`. 
                        }
                    }
                }
            }
        }
    }

    // Sets the state if the underlying storage type is compatible.
    #[allow(dead_code)]
    pub fn try_set_state(&mut self, state: S) -> result::Result<(), S> {
        match self.select_mut() {
            SelectMut::Const(_body) => Err(state),
            SelectMut::Inline(body) => {
                body.replace_value(state);
                Ok(())
            }
            SelectMut::Boxed(body) => {
                let vtable = DynBody::vtable(body.borrow());
                let mut state = Some(state);

                unsafe {
                    if (vtable.try_set_state)(body.borrow_mut(), &mut state as &mut dyn Any) {
                        return Ok(());
                    }
                }

                Err(state.unwrap())
            }
        }
    }

    /// Iterates over the error chain. If this error has its own context or state, it appears first;
    /// otherwise the chain starts from the source.
    pub fn chain(&self) -> impl Iterator<Item = &(dyn error::Error + 'static)>
    where
        S: Debug,
    {
        struct Chain<'a>(Option<&'a (dyn error::Error + 'static)>);

        impl<'a> Iterator for Chain<'a> {
            type Item = &'a (dyn error::Error + 'static);

            fn next(&mut self) -> Option<Self::Item> {
                let next = self.0.and_then(|err| err.source());

                mem::replace(&mut self.0, next)
            }
        }

        if self.is_source_only() {
            Chain(
                self.source()
                    .map(|err| err as &(dyn error::Error + 'static)),
            )
        } else {
            Chain(Some(self))
        }
    }

    /// Convert into a boxed error without reallocation if already boxed, otherwise box the error.
    pub fn into_boxed_error(self) -> Box<dyn error::Error + Send + Sync + 'static>
    where
        S: Debug,
    {
        match self.select_own() {
            SelectOwn::Const(body) => unsafe {
                // Safety: Projection from `ConstBody` to `ConstBody::context` is safe.
                let context = body
                    .borrow()
                    .project(|body| &raw const (*body).context)
                    .deref();
                (*context).into()
            },
            SelectOwn::Inline(body) => format!("{:?}", body.borrow_value()).into(),
            SelectOwn::Boxed(body) => unsafe {
                let vtable = DynBody::vtable(body.borrow());
                // Safety: The body pointer is confirmed valid.
                (vtable.into_boxed_error)(body)
            },
        }
    }

    pub fn erase(self) -> impl error::Error + Send + Sync + 'static
    where
        S: Debug + Send + Sync + 'static,
    {
        ErasedRawError::from_typed(self)
    }

    pub fn backtrace_opaque(&self) -> Option<&dyn DebugDisplay> {
        #[cfg(feature = "backtrace")]
        {
            WithBacktrace::search(|| self.source().map(|v| v as _)).map(|v| v as _)
        }
        #[cfg(not(feature = "backtrace"))]
        None
    }

    #[cfg(feature = "backtrace")]
    pub fn backtrace(&self) -> Option<&std::backtrace::Backtrace> {
        WithBacktrace::search(|| self.source().map(|v| v as _))
    }
}

impl<S> Drop for RawError<S> {
    fn drop(&mut self) {
        match self.kind() {
            Self::KIND_CONST => {}
            Self::KIND_INLINE => unsafe {
                // Safety: The variant hasn't been moved out, as `self` remains owned and not forgotten.
                ManuallyDrop::drop(&mut self.inline_body);
            },
            Self::KIND_BOXED => unsafe {
                let _body = ManuallyDrop::take(&mut self.boxed_body);
            },
            _ => unreachable!(),
        }
    }
}

impl<S> Debug for RawError<S>
where
    S: Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.select_ref() {
            SelectRef::Const(body) => fmt::format_debug(
                f,
                None::<&()>,
                Some(body.borrow().deref().context),
                None,
                None::<&Infallible>,
            ),
            SelectRef::Inline(_) => {
                fmt::format_debug(f, self.state(), None::<&str>, None, None::<&Infallible>)
            }
            SelectRef::Boxed(body) => {
                let vtable = DynBody::vtable(body.borrow());
                unsafe { (vtable.debug)(body.borrow(), f) }
            }
        }
    }
}

impl<S> Display for RawError<S>
where
    S: Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.select_ref() {
            SelectRef::Const(_) | SelectRef::Inline(_) => {
                fmt::format_display(f, self.state(), self.context(), None, None::<&Infallible>)
            }
            SelectRef::Boxed(body) => {
                let vtable = DynBody::vtable(body.borrow());
                unsafe { (vtable.display)(body.borrow(), f) }
            }
        }
    }
}

impl<S> error::Error for RawError<S>
where
    S: Debug,
{
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.source()
            .map(|err| err as &(dyn error::Error + 'static))
    }
}

#[repr(C)]
pub struct ConstBody {
    context: &'static str,
}

/// Heap-allocated error body with type-erased source and context.
///
/// The concrete types `E`, `C` are only known at construction time and at
/// the monomorphized vtable function sites. The `RawError` stores the body as
/// `DynBody<S, (), ()>`. The `S` can also be erased when the state is
/// extracted.
///
/// # Safety
///
/// The `vtable` pointer must point to a `DynBodyVTable` that was monomorphized
/// for the same `S`, `E`, `C` as the stored data.
#[repr(C)]
struct DynBody<S = Infallible, E = (), C = ()>
where
    S: 'static,
    E: 'static,
    C: 'static,
{
    vtable: Align4Ref<'static, DynBodyVTable>, // Note: The vtable must be the first field as the other fields may be erased.
    state: MaybeUninit<S>,
    source: E,
    context: helper::Exclude<C, Empty>,
}

mod helper {
    use core::marker::PhantomData;

    use crate::rtti;

    pub struct Exclude<T, X> {
        value: T,
        _marker: PhantomData<X>,
    }

    impl<T, X> Exclude<T, X>
    where
        T: 'static,
        X: 'static,
    {
        pub fn new(value: T) -> Self {
            Self {
                value,
                _marker: PhantomData,
            }
        }

        pub fn get(&self) -> Option<&T> {
            if rtti::is_same_ty::<T, X>() {
                None
            } else {
                Some(&self.value)
            }
        }

        pub fn get_mut(&mut self) -> Option<&mut T> {
            if rtti::is_same_ty::<T, X>() {
                None
            } else {
                Some(&mut self.value)
            }
        }

        pub fn into_inner(self) -> Option<T> {
            if rtti::is_same_ty::<T, X>() {
                None
            } else {
                Some(self.value)
            }
        }
    }
}

/// Virtual function table for type-erased operations on [`DynBody`].
///
/// Each function pointer is monomorphized for the concrete `S`, `E`, `C`.
///
/// # Safety
///
/// All function pointers must be valid for the concrete types stored in the `DynBody`.
/// The `Ref`/`Mut`/`Align4Own` arguments must point to a `DynBody` whose type parameters
/// match the monomorphization that produced the function pointer.
struct DynBodyVTable {
    /// See [DynBody::drop].
    drop: unsafe fn(ManuallyDrop<Align4Own<DynBody>>),
    /// See [DynBody::into_source].
    into_source: unsafe fn(ErasedDynBody) -> Option<Box<dyn error::Error + Send + Sync + 'static>>,
    /// See [DynBody::into_backtrace].
    into_backtrace: unsafe fn(ErasedDynBody) -> Option<WithBacktrace>,
    /// See [DynBody::into_parts].
    into_parts: unsafe fn(ErasedDynBody, &mut dyn Any, &mut dyn Any, &mut dyn Any),
    /// See [DynBody::extract_state].
    extract_state:
        unsafe fn(ErasedDynBody, &mut dyn Any) -> result::Result<RawVacant, ErasedDynBody>,
    /// See [DynBody::into_boxed_error].
    into_boxed_error: unsafe fn(ErasedDynBody) -> Box<dyn error::Error + Send + Sync + 'static>,
    /// See [DynBody::debug].
    debug: unsafe fn(Ref<'_, DynBody>, &mut core::fmt::Formatter<'_>) -> core::fmt::Result,
    /// See [DynBody::display].
    display: unsafe fn(Ref<'_, DynBody>, &mut core::fmt::Formatter<'_>) -> core::fmt::Result,
    /// See [DynBody::try_set_state].
    try_set_state: unsafe fn(Mut<DynBody>, &mut dyn Any) -> bool,
    /// See [DynBody::has_state].
    has_state: unsafe fn(Ref<'_, DynBody>) -> bool,
    /// See [DynBody::source].
    source: unsafe fn(Ref<'_, DynBody>) -> Option<&(dyn error::Error + Send + Sync + 'static)>,
    /// See [DynBody::source_mut].
    source_mut:
        unsafe fn(Mut<'_, DynBody>) -> Option<&mut (dyn error::Error + Send + Sync + 'static)>,
    /// See [DynBody::state].
    state: unsafe fn(Ref<'_, DynBody>, TypeId, NonNull<()>),
    /// See [DynBody::context].
    context: unsafe fn(Ref<'_, DynBody>) -> Option<&(dyn DebugDisplay + Send + Sync + 'static)>,
    /// See [DynBody::downcast_context_ref].
    downcast_context_ref: unsafe fn(Ref<'_, DynBody>, TypeId, NonNull<()>),
    /// See [DynBody::downcast_context_mut].
    downcast_context_mut: unsafe fn(Mut<'_, DynBody>, TypeId, NonNull<()>),
}

impl DynBodyVTable {
    const fn new<S, E, C>() -> Self
    where
        S: Debug + Send + Sync + 'static,
        E: Source + Send + Sync + 'static,
        C: Debug + Display + Send + Sync + 'static,
    {
        DynBodyVTable {
            drop: DynBody::<S, E, C>::drop,
            into_source: DynBody::<S, E, C>::into_source,
            into_backtrace: DynBody::<S, E, C>::into_backtrace,
            into_parts: DynBody::<S, E, C>::into_parts,
            extract_state: DynBody::<S, E, C>::extract_state,
            into_boxed_error: DynBody::<S, E, C>::into_boxed_error,
            debug: DynBody::<S, E, C>::debug,
            display: DynBody::<S, E, C>::display,
            try_set_state: DynBody::<S, E, C>::try_set_state,
            has_state: DynBody::<S, E, C>::has_state,
            source: DynBody::<S, E, C>::source,
            source_mut: DynBody::<S, E, C>::source_mut,
            state: DynBody::<S, E, C>::state,
            context: DynBody::<S, E, C>::context,
            downcast_context_ref: DynBody::<S, E, C>::downcast_context_ref,
            downcast_context_mut: DynBody::<S, E, C>::downcast_context_mut,
        }
    }
}

impl<S, E, C> DynBody<S, E, C> {
    const NO_STATE: Metadata = Metadata::_0;
    const HAS_STATE: Metadata = Metadata::_1;

    /// Returns a static shared reference to the vtable.
    fn vtable(this: Ref<'_, DynBody<S, E, C>>) -> &'static DynBodyVTable {
        unsafe {
            this.project(|body| &raw const (*body).vtable)
                .deref()
                .borrow()
                .deref()
        }
    }
}

impl<S, E, C> DynBody<S, E, C>
where
    S: Debug + Send + Sync + 'static,
    E: Source + Send + Sync + 'static,
    C: Debug + Display + Send + Sync + 'static,
{
    fn vtable_from_state(state: Option<S>) -> (Align4Ref<'static, DynBodyVTable>, MaybeUninit<S>) {
        (
            Align4Ref::new(
                &const { Align4(DynBodyVTable::new::<S, E, C>()) },
                match state {
                    Some(_) => Self::HAS_STATE,
                    None => Self::NO_STATE,
                },
            ),
            match state {
                Some(state) => MaybeUninit::new(state),
                None => MaybeUninit::uninit(),
            },
        )
    }

    /// Check if the state exisis.
    fn has_state_bit_set(&self) -> bool {
        unsafe {
            // # Safety: `Align4Ref` is `repr(C)` and stores the metadata at offset 0.
            match Metadata((&raw const (self.vtable) as *const u8).read() & Metadata::MASK) {
                Self::NO_STATE => false,
                Self::HAS_STATE => true,
                _ => unreachable!(),
            }
        }
    }

    /// Returns a shared reference to the state, if any.
    fn try_get_state(&self) -> Option<&S> {
        self.has_state_bit_set()
            .then(|| unsafe { self.state.assume_init_ref() })
    }

    /// Replaces the stored state with a new value. Returns the old one, if any.
    fn replace_state(&mut self, state: Option<S>) -> Option<S> {
        unsafe {
            let (has_state, old_state) = match (self.has_state_bit_set(), state) {
                (false, None) => (false, None),
                (false, Some(state)) => {
                    self.state.write(state);
                    (true, None)
                }
                (true, None) => (false, Some(self.state.assume_init_read())),
                (true, Some(state)) => {
                    let old_state = self.state.assume_init_read();
                    self.state.write(state);
                    (true, Some(old_state))
                }
            };
            let pvt = self.vtable.borrow_raw().deref();
            self.vtable = Align4Ref::new(
                pvt,
                match has_state {
                    false => Self::NO_STATE,
                    true => Self::HAS_STATE,
                },
            );

            old_state
        }
    }

    /// Consumes `self` and decomposes into its raw components:
    /// `(state, source, context)`.
    fn destruct(mut self) -> (Option<S>, E, Option<C>) {
        let state = self.replace_state(None);

        let mut this = MaybeUninit::new(self);
        let this = this.as_mut_ptr();

        let context = unsafe { (&raw mut (*this).context).read() }.into_inner();
        let source = unsafe { (&raw mut (*this).source).read() };

        (state, source, context)
    }
}

impl<S, E, C> DynBody<S, E, C>
where
    S: Debug + Send + Sync + 'static,
    E: Source + Send + Sync + 'static,
    C: Debug + Display + Send + Sync + 'static,
{
    /// Drops the boxed body.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to `DynBody<S, E, C>`.
    unsafe fn drop(mut this: ManuallyDrop<Align4Own<DynBody>>) {
        unsafe {
            let this = ManuallyDrop::take(&mut this).cast::<Self>();

            let _ = ManuallyDrop::into_inner(this).into_boxed();
        }
    }

    /// Extracts the source error as a trait object from the boxed body.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to `DynBody<S, E, C>`.
    unsafe fn into_source(
        this: ErasedDynBody,
    ) -> Option<Box<dyn error::Error + Send + Sync + 'static>> {
        let Align4(this) = unsafe { *ErasedDynBody::into_inner::<S, E, C>(this).into_boxed() };

        let (_, source, ..) = this.destruct();

        source.into_boxed()
    }

    /// Extracts the source error as a trait object from the boxed body.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to `DynBody<S, E, C>`.
    unsafe fn into_backtrace(this: ErasedDynBody) -> Option<WithBacktrace> {
        let Align4(this) = unsafe { *ErasedDynBody::into_inner::<S, E, C>(this).into_boxed() };

        let (_, source, ..) = this.destruct();

        source.into_backtrace()
    }

    /// Decomposes the boxed body: extracts source and context into caller-provided
    /// `Option`s (if the `TypeId` matches), and returns the state.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to `DynBody<S, E, C>`.
    /// - `source_dst`, `context_dst`, `state_dst` must be valid, aligned, mutable
    ///   pointers to `Option<E>`, `Option<C>` and `Option<S>` respectively.
    #[allow(clippy::too_many_arguments)]
    unsafe fn into_parts(
        this: ErasedDynBody,
        source_dst: &mut dyn Any,
        context_dst: &mut dyn Any,
        state_dst: &mut dyn Any,
    ) {
        let Align4(this) = unsafe { *ErasedDynBody::into_inner::<S, E, C>(this).into_boxed() };
        let (state, source, context) = this.destruct();

        if let Some(state) = state {
            if let Some(dst) = state_dst.downcast_mut::<Option<S>>() {
                dst.replace(state);
            }
        }
        if let Some(context) = context {
            if let Some(dst) = context_dst.downcast_mut::<Option<C>>() {
                dst.replace(context);
            }
        }

        source.downcast_container(source_dst).ok();
    }

    /// Extracts the state from the boxed body, `state_dst` becomes `Some` iff it succeeds and returns `Ok`.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to `DynBody<S, E, C>`.
    /// - `state_dst` must be a valid, aligned, mutable pointer to `Option<StateTy>`.
    unsafe fn extract_state(
        this: ErasedDynBody,
        state_dst: &mut dyn Any,
    ) -> result::Result<RawVacant, ErasedDynBody> {
        let mut this = unsafe { ErasedDynBody::into_inner::<S, E, C>(this) };

        if let Some(dst) = state_dst.downcast_mut::<Option<S>>() {
            *dst = this.borrow_mut().deref_mut().replace_state(None);

            if dst.is_some() {
                return Ok(RawVacant(ErasedDynBody::from_typed(this)));
            }
        }

        Err(ErasedDynBody::from_typed(this))
    }

    /// Convert the thin `DynBody` pointer to `Box<Error>` without reallocation.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to `DynBody<S, E, C>`.
    unsafe fn into_boxed_error(
        this: ErasedDynBody,
    ) -> Box<dyn error::Error + Send + Sync + 'static> {
        unsafe {
            let this = ErasedDynBody::into_inner::<S, E, C>(this);

            this.into_boxed()
        }
    }

    /// Formats the boxed underlying body using the `Debug` trait.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Mut` pointing to `DynBody<S, E, C>`.
    unsafe fn debug(this: Ref<'_, DynBody>, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let this = unsafe { this.cast::<Self>().deref() };
        <Self as Debug>::fmt(this, f)
    }

    /// Formats the boxed underlying body using the `Display` trait.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Mut` pointing to `DynBody<S, E, C>`.
    unsafe fn display(
        this: Ref<'_, DynBody>,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        let this = unsafe { this.cast::<Self>().deref() };
        <Self as Display>::fmt(this, f)
    }

    /// Replace the state if type matches. `state_src` becomes `None` iff it succeeds and returns true.
    ///
    /// # Panics
    ///
    /// If the state_src is `None`, it panics.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Mut` pointing to `DynBody<S, E, C>`.
    /// - `state_src` must be a valid, aligned, mutable pointer to `Option<S>`.
    unsafe fn try_set_state(this: Mut<'_, DynBody>, state_src: &mut dyn Any) -> bool {
        let this = unsafe { this.cast::<Self>().deref_mut() };

        if let Some(state_src) = state_src.downcast_mut::<Option<S>>() {
            let Some(state_src) = state_src.take() else {
                panic!("try_set_state: state_src must be `Some`");
            };
            this.replace_state(Some(state_src));
            true
        } else {
            false
        }
    }

    /// Check if there is a state in the body.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, C>`.
    unsafe fn has_state(this: Ref<'_, DynBody>) -> bool {
        let this = unsafe { this.cast::<Self>().deref() };

        this.has_state_bit_set()
    }

    /// Returns a reference to the source error.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, C>`.
    unsafe fn source(
        this: Ref<'_, DynBody>,
    ) -> Option<&(dyn error::Error + Send + Sync + 'static)> {
        let this = unsafe { this.cast::<Self>().deref() };

        this.source.error_ref()
    }

    /// Returns a mutable reference to the source error.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, C>`.
    unsafe fn source_mut(
        this: Mut<'_, DynBody>,
    ) -> Option<&mut (dyn error::Error + Send + Sync + 'static)> {
        let this = unsafe { this.cast::<Self>().deref_mut() };

        this.source.error_mut()
    }

    /// Returns a reference to the state.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, C>`.
    /// - `dst` must be a valid, aligned, mutable pointer to `Option<&StateTy>`.
    unsafe fn state(this: Ref<'_, DynBody>, state_ty: TypeId, state_dst: NonNull<()>) {
        let this = unsafe { this.cast::<Self>().deref() };

        if TypeId::of::<S>() == state_ty {
            let dst = unsafe { state_dst.cast::<Option<&S>>().as_mut() };

            if let Some(state) = this.try_get_state() {
                dst.replace(state);
            }
        }
    }

    /// Returns a displayable reference to the context.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, C>`.
    unsafe fn context(
        this: Ref<'_, DynBody>,
    ) -> Option<&(dyn DebugDisplay + Send + Sync + 'static)> {
        let this = unsafe { this.cast::<Self>().deref() };

        this.context.get().map(|c| c as _)
    }

    /// Attempts to downcast the context field to the requested type `C`.
    ///
    /// Writes `Some(&C)` into `dst` if the type matches, otherwise does nothing.
    ///
    /// # Safety
    ///
    /// Same as [`downcast_source_ref`](DynBody::downcast_source_ref) for the context field.
    /// - `dst` must be a valid, aligned, mutable pointer to `Option<&Ty>`.
    unsafe fn downcast_context_ref(this: Ref<'_, DynBody>, ty: TypeId, dst: NonNull<()>) {
        let this = unsafe { this.cast::<Self>().deref() };

        if let Some(context) = this.context.get() {
            if TypeId::of::<C>() == ty {
                let dst = unsafe { dst.cast::<Option<&C>>().as_mut() };
                *dst = Some(context);
            }
        }
    }

    /// Attempts to downcast the context field to the requested type `C` (mutable).
    ///
    /// Writes `Some(&mut C)` into `dst` if the type matches, otherwise does nothing.
    ///
    /// # Safety
    ///
    /// Same as [`downcast_context_ref`](DynBody::downcast_context_ref) with mutable access.
    /// - `dst` must be a valid, aligned, mutable pointer to `Option<&mut Ty>`.
    unsafe fn downcast_context_mut(this: Mut<'_, DynBody>, ty: TypeId, dst: NonNull<()>) {
        let this = unsafe { this.cast::<Self>().deref_mut() };

        if let Some(context) = this.context.get_mut() {
            if TypeId::of::<C>() == ty {
                let dst = unsafe { dst.cast::<Option<&mut C>>().as_mut() };
                *dst = Some(context);
            }
        }
    }
}

impl<S, E, C> Drop for DynBody<S, E, C> {
    fn drop(&mut self) {
        unsafe {
            match Metadata((&raw const (self.vtable) as *const u8).read() & Metadata::MASK) {
                Self::HAS_STATE => {
                    MaybeUninit::assume_init_drop(&mut self.state);
                }
                Self::NO_STATE => {}
                _ => unreachable!(),
            }
        }
    }
}

impl<S, E, C> core::fmt::Debug for DynBody<S, E, C>
where
    S: Debug + Send + Sync + 'static,
    E: Source + Send + Sync + 'static,
    C: Debug + Display + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        fmt::format_debug(
            f,
            self.try_get_state(),
            self.context.get(),
            self.source.error_ref().map(|e| e as _),
            WithBacktrace::search_debug(|| self.source.error_ref().map(|e| e as _)),
        )
    }
}

impl<S, E, C> core::fmt::Display for DynBody<S, E, C>
where
    S: Debug + Send + Sync + 'static,
    E: Source + Send + Sync + 'static,
    C: Debug + Display + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        fmt::format_display(
            f,
            self.try_get_state(),
            self.context.get(),
            self.source.error_ref().map(|e| e as _),
            WithBacktrace::search_display(|| self.source.error_ref().map(|e| e as _)),
        )
    }
}

impl<S, E, C> error::Error for DynBody<S, E, C>
where
    S: Debug + Send + Sync + 'static,
    E: Source + Send + Sync + 'static,
    C: Debug + Display + Send + Sync + 'static,
{
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.source.error_ref().map(|e| e as _)
    }
}

pub struct RawVacant(ErasedDynBody);

impl RawVacant {
    pub fn try_with_state<S>(mut self, state: S) -> result::Result<RawError<S>, (Self, S)> {
        unsafe {
            let vt = DynBody::vtable(self.0.borrow());
            let mut state_src = Some(state);

            if (vt.try_set_state)(self.0.borrow_mut(), &mut state_src as &mut dyn Any) {
                Ok(RawError {
                    boxed_body: ManuallyDrop::new(self.0),
                })
            } else {
                // Note: This `unwrap` will not panic as `state_src` is `None` iff try_set_state returns true.
                Err((self, state_src.unwrap()))
            }
        }
    }

    pub fn try_into_stateless(self) -> result::Result<RawError, Self> {
        let vt = DynBody::vtable(self.0.borrow());

        unsafe {
            let body_ref = self.0.borrow();
            let has_context = (vt.context)(body_ref);
            let has_source = (vt.source)(body_ref);
            match (has_context, has_source) {
                (None, None) => Err(self),
                _ => Ok(RawError {
                    boxed_body: ManuallyDrop::new(self.0),
                }),
            }
        }
    }

    /// Derives a new error from this vacant while preserving the backtrace. This is the only way to
    /// turn a vacant into an error when no state, context, or source is left to wrap.
    pub fn derive<S, C>(self, state: Option<S>, context: C) -> RawError<S>
    where
        S: Debug + Send + Sync + 'static,
        C: context::Context,
    {
        let vt = DynBody::vtable(self.0.borrow());

        unsafe {
            let body_ref = self.0.borrow();
            let has_context = (vt.context)(body_ref).is_some();
            let has_source = (vt.source)(body_ref).is_some();
            match (has_context, has_source) {
                (false, false) => match (vt.into_backtrace)(self.0) {
                    Some(backtrace) => {
                        RawError::new(state, Some(WithBacktraceSource(backtrace)), context)
                    }
                    None => RawError::new(state, None::<Infallible>, context),
                },
                _ => RawError::new(
                    state,
                    Some(
                        RawError::<Infallible> {
                            boxed_body: ManuallyDrop::new(self.0),
                        }
                        .erase(),
                    ),
                    context,
                ),
            }
        }
    }
}

impl Debug for RawVacant {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let body_ref = self.0.borrow();
        let vt = DynBody::vtable(body_ref);

        unsafe {
            let context = (vt.context)(body_ref);
            let source = (vt.source)(body_ref);
            let backtrace = (!f.sign_minus())
                .then(|| {
                    WithBacktrace::search_debug(|| {
                        (vt.source)(body_ref).map(|v| v as &(dyn error::Error + 'static))
                    })
                })
                .flatten();

            fmt::format_debug_struct::<Infallible>(
                f,
                "Vacant",
                None,
                context,
                source.map(|v| v as _),
                backtrace,
            )
        }
    }
}

struct ErasedDynBody(ManuallyDrop<Align4Own<DynBody>>);

impl ErasedDynBody {
    fn from_typed<S, E, C>(body: Align4Own<DynBody<S, E, C>>) -> Self {
        Self(unsafe { body.cast::<DynBody>() })
    }
    /// Restore the original `Align4Own<DynBody<S, E, C>>` from this `ErasedDynBody`.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `ErasedDynBody` pointing to `DynBody<S, E, C>`.
    unsafe fn into_inner<S, E, C>(this: Self) -> Align4Own<DynBody<S, E, C>> {
        let mut this = ManuallyDrop::new(this);
        unsafe {
            let mut this: ManuallyDrop<Align4Own<DynBody<S, E, C>>> =
                ManuallyDrop::take(&mut this.0).cast();

            ManuallyDrop::take(&mut this)
        }
    }
}

impl Deref for ErasedDynBody {
    type Target = Align4Own<DynBody>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ErasedDynBody {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for ErasedDynBody {
    fn drop(&mut self) {
        let vtable = DynBody::vtable(self.0.borrow());
        unsafe {
            // Safety: The body pointer is confirmed valid.
            (vtable.drop)(ManuallyDrop::new(ManuallyDrop::take(&mut self.0)));
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::{
        format,
        string::{String, ToString},
    };
    use core::{assert_matches, convert::Infallible, fmt::Display, mem};

    use super::*;
    use crate::context::{Contextless, Literal, Mkctx};

    // --- Test helpers ---

    /// A custom source error for testing.
    #[derive(Debug)]
    struct TestError(&'static str);

    impl Display for TestError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    impl error::Error for TestError {}

    /// A typed literal for testing.
    #[derive(Debug)]
    struct TestContextLiteral;

    impl Literal for TestContextLiteral {
        const LITERAL: &'static str = "test context";
    }

    type TestContext = Mkctx<fn() -> Option<String>, TestContextLiteral>;

    // --- RawError kind() discrimination ---

    #[cfg(not(feature = "backtrace"))]
    #[test]
    fn kind_discriminates_const() {
        let err = RawError::try_new_const::<TestContext>().unwrap();
        assert_eq!(err.kind(), RawError::<()>::KIND_CONST);
    }

    #[cfg(not(feature = "backtrace"))]
    #[test]
    fn kind_discriminates_inline() {
        let err = RawError::try_new_inline(42u8).unwrap();
        assert_eq!(err.kind(), RawError::<u16>::KIND_INLINE);
    }

    #[cfg(not(feature = "backtrace"))]
    #[test]
    fn kind_discriminates_boxed() {
        let err = RawError::new(
            None::<Infallible>,
            Some(TestError("oops")),
            Contextless::new(),
        );
        assert_eq!(err.kind(), RawError::<()>::KIND_BOXED);
    }

    // --- Const variant ---

    #[test]
    fn const_variant_context() {
        let err = RawError::try_new_const::<TestContext>().unwrap();
        let ctx = err.context();
        assert_eq!(ctx.unwrap().to_string(), TestContextLiteral::LITERAL);
    }

    #[test]
    fn const_variant_source_is_none() {
        let err = RawError::try_new_const::<TestContext>().unwrap();
        assert!(err.source().is_none());
    }

    // --- Inline variant ---

    #[test]
    fn inline_variant_state() {
        let err = RawError::try_new_inline(42u16).unwrap();
        assert_matches!(err.state(), Some(42));
    }

    #[test]
    fn inline_variant_context_is_none() {
        let err = RawError::try_new_inline(42u16).unwrap();
        assert!(err.context().is_none());
    }

    #[test]
    fn inline_variant_source_is_none() {
        let err = RawError::try_new_inline(42u16).unwrap();
        assert!(err.source().is_none());
    }

    // Boxed variant ---

    #[test]
    fn boxed_variant_source() {
        let err = RawError::new(
            None::<Infallible>,
            Some(TestError("oops")),
            Contextless::new(),
        );
        let src = err.source();
        assert_eq!(src.unwrap().to_string(), "oops");
    }

    #[test]
    fn boxed_variant_downcast_source() {
        let err = RawError::new(
            None::<Infallible>,
            Some(TestError("oops")),
            Contextless::new(),
        );
        let downcasted = err.downcast_source_ref::<TestError>();
        assert_matches!(downcasted, Some(TestError("oops")));
    }

    #[test]
    fn boxed_variant_downcast_source_wrong_type() {
        let err = RawError::new(
            None::<Infallible>,
            Some(TestError("oops")),
            Contextless::new(),
        );
        let downcasted = err.downcast_source_ref::<core::fmt::Error>();
        assert!(downcasted.is_none());
    }

    #[test]
    fn boxed_variant_context() {
        let err = RawError::new(
            None::<Infallible>,
            Some(TestError("oops")),
            TestContextLiteral::LITERAL,
        );
        let ctx = err.context();
        assert_eq!(ctx.unwrap().to_string(), TestContextLiteral::LITERAL);
    }

    #[test]
    fn boxed_variant_nae_source_is_none() {
        // When source is `Nae`, `.source()` should return `None`.
        let err = RawError::new(Some(42u32), None::<Infallible>, Contextless::new());
        assert!(err.source().is_none());
        assert_matches!(err.state(), Some(42));
    }

    // --- into_source ---

    #[test]
    fn boxed_variant_into_source_returns_boxed_error() {
        let err = RawError::new(
            None::<Infallible>,
            Some(TestError("oops")),
            Contextless::new(),
        );
        let src = err.into_source();
        assert_eq!(src.unwrap().to_string(), "oops");
    }

    #[test]
    fn boxed_variant_into_source_nae_returns_none() {
        let err = RawError::new(None::<Infallible>, None::<Infallible>, Contextless::new());
        assert!(err.into_source().is_none());
    }

    // --- into_parts ---

    #[test]
    fn boxed_variant_into_parts_matches_types() {
        let err = RawError::new(
            Some("state"),
            Some(TestError("oops")),
            TestContextLiteral::LITERAL,
        );
        let (state, context, source) = err.into_parts::<&str, TestError>();
        assert_matches!(state, Some("state"));
        assert_matches!(source, Some(TestError("oops")));
        assert_eq!(context, TestContext::FALLBACK);
    }

    #[test]
    fn boxed_variant_into_parts_context_downcasts() {
        let err = RawError::new(
            None::<Infallible>,
            Some(TestError("oops")),
            Contextless::new(),
        );
        let (_, context, _) = err.into_parts::<Empty, String>();
        assert!(context.is_none());
    }

    #[test]
    fn const_variant_into_parts() {
        let err = RawError::try_new_const::<TestContext>().unwrap();
        let (state, context, source) = err.into_parts::<&str, TestError>();
        assert!(source.is_none());
        assert_eq!(context, TestContext::FALLBACK);
        assert_eq!(state, None);
    }

    #[test]
    fn inline_variant_into_parts() {
        let err = RawError::try_new_inline(42u16).unwrap();
        let (state, context, source) = err.into_parts::<Empty, TestError>();
        assert!(source.is_none());
        assert!(context.is_none());
        assert_matches!(state, Some(42));
    }

    // --- Drop safety (checked via sanitizer / basic leak check) ---

    /// Allocate a boxed variant and ensure it can be observed to drop.
    #[test]
    fn boxed_variant_drop_does_not_leak() {
        use core::sync::atomic::{AtomicBool, Ordering};

        static DROPPED: AtomicBool = AtomicBool::new(false);

        #[derive(Debug)]
        struct DropWatch;

        impl Drop for DropWatch {
            fn drop(&mut self) {
                DROPPED.store(true, Ordering::SeqCst);
            }
        }

        impl Display for DropWatch {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "")
            }
        }

        impl error::Error for DropWatch {}

        {
            let _err = RawError::new(None::<Infallible>, Some(DropWatch), Contextless::new());
        } // drop here
        assert!(DROPPED.load(Ordering::SeqCst));
    }

    // --- State round-trip for const variant (S = ()) ---

    #[test]
    fn const_variant_state_is_none() {
        let err = RawError::try_new_const::<TestContext>().unwrap();
        assert!(err.state().is_none());
    }

    // --- Size checks ---

    #[test]
    fn raw_error_size() {
        assert_eq!(mem::size_of::<RawError<()>>(), mem::size_of::<usize>());
        assert_eq!(mem::size_of::<RawError<u128>>(), mem::size_of::<usize>());
        assert_eq!(mem::size_of::<RawError>(), mem::size_of::<usize>());
    }

    // --- State extraction ---

    #[test]
    fn state_extraction() {
        {
            let err = RawError::new(Some(42u8), None::<Infallible>, Contextless::new());
            if cfg!(feature = "backtrace") {
                assert_matches!(err.extract_state(), Ok((42, Some(_) | None)));
            } else {
                assert_matches!(err.extract_state(), Ok((42, None)));
            }
        }
        {
            let err = RawError::new(Some(42u128), None::<Infallible>, Contextless::new());
            assert_matches!(err.extract_state(), Ok((42, Some(_))));
        }
        {
            let err = RawError::new(None::<Infallible>, None::<Infallible>, format!("oops"));
            assert_matches!(err.extract_state(), Err(err) if format!("{err}") == "oops");
        }
        {
            let err = RawError::new(Some(42i32), None::<Infallible>, format!("oops"));
            match err.extract_state() {
                Ok((state, Some(vacant))) if state == 42 => {
                    let err = vacant.try_with_state(state).unwrap();
                    assert_eq!(err.state(), Some(&42));
                    assert_eq!(err.context().unwrap().to_string(), "oops");
                }
                _ => panic!("extract should not fail"),
            }
        }
    }

    // --- Layer elimination ---

    #[test]
    fn new_eliminates_erased_layer() {
        // Build a source-only RawError: (RawError -> TestError)
        let inner = RawError::new(
            None::<Infallible>,
            Some(TestError("root")),
            Contextless::new(),
        );
        // Erase the type → ErasedRawError -> TestError
        let erased = ErasedRawError::from_typed(inner);
        // Re-wrap: this should eliminate the ErasedRawError layer since it carries no extra info
        let err = RawError::new(None::<Infallible>, Some(erased), Contextless::new());
        // Chain should still be 1: RawError -> TestError
        assert_eq!(err.chain().count(), 1);
        assert!(
            err.downcast_source_ref::<TestError>().is_some(),
            "TestError should be reachable directly"
        );
    }

    #[test]
    fn new_eliminates_boxed_erased_layer() {
        // Build a source-only RawError: (RawError -> TestError)
        let inner = RawError::new(
            None::<Infallible>,
            Some(TestError("root")),
            Contextless::new(),
        );
        // Erase the type → ErasedRawError
        let erased = ErasedRawError::from_typed(inner);
        // Box the erased error → Box<dyn Error> -> ErasedRawError -> TestError
        let boxed: Box<dyn error::Error + Send + Sync + 'static> = Box::new(erased);
        // Re-wrap: this should eliminate both Box and ErasedRawError layers
        let err = RawError::new(
            None::<Infallible>,
            Some(BoxedSource(boxed)),
            Contextless::new(),
        );
        // Chain should still be 1: RawError -> TestError
        assert_eq!(err.chain().count(), 1);
        assert!(
            err.downcast_source_ref::<TestError>().is_some(),
            "TestError should be reachable directly"
        );
    }

    #[test]
    fn round_trip_repeatedly_keeps_single_boxed_layer() {
        // Start with a source-only RawError: (RawError -> TestError)
        let mut err = RawError::new(
            None::<Infallible>,
            Some(TestError("root")),
            Contextless::new(),
        );
        assert_eq!(err.chain().count(), 1);

        // Round-trip through Box<dyn Error> multiple times.
        // Each `into_boxed_error` extracts the raw TestError,
        // and `RawError::new` re-wraps it as a single layer.
        for _ in 0..5 {
            let boxed: Box<dyn error::Error + Send + Sync + 'static> = err.into_boxed_error();
            err = RawError::new(
                None::<Infallible>,
                Some(BoxedSource(boxed)),
                Contextless::new(),
            );
            assert_eq!(err.chain().count(), 2, "chain length should always be 2.");
            assert!(
                err.chain().last().unwrap().is::<TestError>(),
                "TestError should always be reachable"
            );
        }
    }
}
