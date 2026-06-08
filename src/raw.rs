use alloc::{boxed::Box, format};
use core::{
    any::TypeId,
    convert::Infallible,
    error::{self, Error},
    fmt::{self, Debug, Display},
    mem::{self, ManuallyDrop, MaybeUninit},
    ptr::NonNull,
    result,
};

use crate::{
    backtrace::WithBacktrace,
    context::{self, Blank, Context},
    match_else,
    nae::Nae,
    ptr::{Align4, Align4Own, Align4PtrCompat, Align4Ref, Metadata, Mut, Ref},
    render, rtti,
};

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
pub union RawError<S>
where
    S: 'static,
{
    const_body: ManuallyDrop<Align4Ref<'static, ConstBody>>,
    boxed_body: ManuallyDrop<Align4Own<DynBody>>,
    inline_body: ManuallyDrop<Align4PtrCompat<S>>,
}

enum SelectRef<'a, S>
where
    S: 'static,
{
    Const(&'a Align4Ref<'static, ConstBody>),
    Boxed(&'a Align4Own<DynBody>),
    Inline(&'a Align4PtrCompat<S>),
}

enum SelectMut<'a, S>
where
    S: 'static,
{
    Const(&'a mut Align4Ref<'static, ConstBody>),
    Boxed(&'a mut Align4Own<DynBody>),
    Inline(&'a mut Align4PtrCompat<S>),
}

enum SelectOwn<S>
where
    S: 'static,
{
    Const(Align4Ref<'static, ConstBody>),
    Boxed(ManuallyDrop<Align4Own<DynBody>>),
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
                Self::KIND_BOXED => {
                    SelectOwn::Boxed(ManuallyDrop::new(ManuallyDrop::take(&mut this.boxed_body)))
                }
                Self::KIND_INLINE => SelectOwn::Inline(ManuallyDrop::take(&mut this.inline_body)),
                _ => unreachable!(),
            }
        }
    }
}

impl RawError<Infallible> {
    /// Constructs a const-variant [`RawError`] from a typed literal.
    fn try_new_const<C>() -> Option<Self>
    where
        C: Context,
    {
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
            SelectOwn::Boxed(body) => RawError { boxed_body: body },
        }
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

    /// Constructs a [`RawError`].
    pub fn new<E, C>(state: Option<S>, source: E, context: C) -> Self
    where
        S: Debug + Send + Sync + 'static,
        E: error::Error + Send + Sync + 'static,
        C: context::Context,
    {
        let context = context.try_into_repr();
        let context_fallback = C::FALLBACK;

        match WithBacktrace::try_attach(source) {
            Ok(source) => match (context, context_fallback) {
                (Some(context), _) => Self::new_boxed(state, source, context),
                (None, Some(context)) => Self::new_boxed(state, source, context),
                (None, None) => Self::new_boxed(state, source, Blank::new()),
            },
            Err(source) => {
                let mut state = state;
                let has_state = state.is_some();
                let has_source = !rtti::is_same_ty::<E, Nae>();
                let has_context = !rtti::is_same_ty::<C::Repr, Blank>();

                match (has_state, has_context, has_source) {
                    (true, false, false) => {
                        let state_value = state.take().unwrap();
                        let Err(state_value) = match_else!(Self::try_new_inline(state_value), Ok(this) => {
                            return this;
                        });
                        state = Some(state_value);
                    }
                    (false, true, false) if context.is_none() && context_fallback.is_some() => {
                        if let Some(this) = RawError::try_new_const::<C>() {
                            return this.with_phantom_state();
                        }
                    }
                    _ => {}
                }

                match (context, context_fallback) {
                    (Some(context), _) => Self::new_boxed(state, source, context),
                    (None, Some(context)) => Self::new_boxed(state, source, context),
                    (None, None) => Self::new_boxed(state, source, Blank::new()),
                }
            }
        }
    }

    fn new_boxed<E, C>(state: Option<S>, source: E, context: C) -> Self
    where
        S: Debug + Send + Sync + 'static,
        E: error::Error + Send + Sync + 'static,
        C: Display + Send + Sync + 'static,
    {
        let (vtable, state) = DynBody::<S, E, C>::vtable_from_state(state);

        // # Safety
        //
        // The `Align4Own` pointer is cast to `DynBody<Infallible, (), ()>` for uniform storage.
        // This is valid because all monomorphizations of `DynBody<S, E, C::Repr>` share
        // the same vtable pointer, and the concrete `S`, `E`, `C` are erased.
        // The cast only changes the type parameter defaults — it does not violate the layout
        // because `()` is a ZST.
        // Due to `Align4Own`assumes a correct layout, and that's not true after the cast,
        // we need to put it in a `ManuallyDrop` and drop it via vtable instead.
        RawError::<S> {
            boxed_body: unsafe {
                Align4Own::from_boxed(
                    Box::new(Align4(DynBody::<S, E, C> {
                        vtable,
                        state,
                        source,
                        context,
                    })),
                    RawError::<S>::KIND_BOXED,
                )
                .cast::<DynBody>()
            },
        }
    }

    /// Returns a reference to the displayable context.
    pub fn context(&self) -> Option<&'_ (dyn Display + Send + Sync + 'static)> {
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
                (vtable.context)(body.borrow())
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
                    NonNull::from_mut(&mut result).cast(),
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
                    NonNull::from_mut(&mut result).cast(),
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
                    NonNull::from_mut(&mut state).cast(),
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
                    TypeId::of::<E>(),
                    NonNull::from_mut(&mut err).cast(),
                    TypeId::of::<C>(),
                    NonNull::from_mut(&mut context).cast(),
                    TypeId::of::<S>(),
                    NonNull::from_mut(&mut state).cast(),
                );

                (state, context, err)
            },
        }
    }

    pub fn extract_state(self) -> result::Result<(S, Option<RawVacant>), RawError<Infallible>> {
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
                    let re = (vt.extract_state)(
                        body,
                        TypeId::of::<S>(),
                        NonNull::from_mut(&mut state_dst).cast(),
                    );

                    match (state_dst, re) {
                        (Some(state), Ok(vacant)) => Ok((state, Some(vacant))),
                        (None, Err(body)) => Err(RawError { boxed_body: body }),
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
                    (vtable.try_set_state)(
                        body.borrow_mut(),
                        TypeId::of::<S>(),
                        NonNull::from_mut(&mut state).cast(),
                    )
                };

                if let Some(state) = state {
                    Err(state)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Iterates over the source error chain, starting from the immediate source.
    pub fn chain(&self) -> impl Iterator<Item = &(dyn error::Error + 'static)> {
        struct Chain<'a>(Option<&'a (dyn error::Error + 'static)>);

        impl<'a> Iterator for Chain<'a> {
            type Item = &'a (dyn error::Error + 'static);

            fn next(&mut self) -> Option<Self::Item> {
                let next = self.0.and_then(|err| err.source());

                mem::replace(&mut self.0, next)
            }
        }

        Chain(
            self.source()
                .map(|err| err as &(dyn error::Error + 'static)),
        )
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
            Self::KIND_BOXED => {
                unsafe {
                    let vtable = DynBody::vtable(self.boxed_body.borrow());
                    // Safety: The body pointer is confirmed valid.
                    (vtable.drop)(ManuallyDrop::new(ManuallyDrop::take(&mut self.boxed_body)));
                }
            }
            _ => unreachable!(),
        }
    }
}

impl<S> Debug for RawError<S>
where
    S: Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        render::format_debug(
            f,
            self.state(),
            self.context().map(|v| v as _),
            self.source().map(|v| v as _),
            WithBacktrace::search_debug(|| self.source().map(|v| v as _)),
        )
    }
}

impl<S> Display for RawError<S>
where
    S: Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        render::format_display(
            f,
            self.state(),
            self.context().map(|v| v as _),
            self.source().map(|v| v as _),
            WithBacktrace::search_display(|| self.source().map(|v| v as _)),
        )
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
    context: C,
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
    into_source: unsafe fn(
        ManuallyDrop<Align4Own<DynBody>>,
    ) -> Option<Box<dyn error::Error + Send + Sync + 'static>>,
    /// See [DynBody::into_backtrace].
    into_backtrace: unsafe fn(ManuallyDrop<Align4Own<DynBody>>) -> Option<WithBacktrace>,
    /// See [DynBody::into_parts].
    into_parts: unsafe fn(
        ManuallyDrop<Align4Own<DynBody>>,
        TypeId,
        NonNull<()>,
        TypeId,
        NonNull<()>,
        TypeId,
        NonNull<()>,
    ),
    /// See [DynBody::extract_state].
    extract_state: unsafe fn(
        ManuallyDrop<Align4Own<DynBody>>,
        TypeId,
        NonNull<()>,
    ) -> result::Result<RawVacant, ManuallyDrop<Align4Own<DynBody>>>,
    /// See [DynBody::into_boxed_error].
    into_boxed_error: unsafe fn(
        ManuallyDrop<Align4Own<DynBody>>,
    ) -> Box<dyn error::Error + Send + Sync + 'static>,
    /// See [DynBody::try_set_state].
    try_set_state: unsafe fn(Mut<DynBody>, TypeId, NonNull<()>) -> bool,
    /// See [DynBody::source].
    source: unsafe fn(Ref<'_, DynBody>) -> Option<&(dyn error::Error + Send + Sync + 'static)>,
    /// See [DynBody::source_mut].
    source_mut:
        unsafe fn(Mut<'_, DynBody>) -> Option<&mut (dyn error::Error + Send + Sync + 'static)>,
    /// See [DynBody::state].
    state: unsafe fn(Ref<'_, DynBody>, TypeId, NonNull<()>),
    /// See [DynBody::context].
    context: unsafe fn(Ref<'_, DynBody>) -> Option<&(dyn Display + Send + Sync + 'static)>,
    /// See [DynBody::downcast_context_ref].
    downcast_context_ref: unsafe fn(Ref<'_, DynBody>, TypeId, NonNull<()>),
    /// See [DynBody::downcast_context_mut].
    downcast_context_mut: unsafe fn(Mut<'_, DynBody>, TypeId, NonNull<()>),
}

impl DynBodyVTable {
    const fn new<S, E, C>() -> Self
    where
        S: Debug + Send + Sync + 'static,
        E: error::Error + Send + Sync + 'static,
        C: Display + Send + Sync + 'static,
    {
        DynBodyVTable {
            drop: DynBody::<S, E, C>::drop,
            into_source: DynBody::<S, E, C>::into_source,
            into_backtrace: DynBody::<S, E, C>::into_backtrace,
            into_parts: DynBody::<S, E, C>::into_parts,
            extract_state: DynBody::<S, E, C>::extract_state,
            into_boxed_error: DynBody::<S, E, C>::into_boxed_error,
            try_set_state: DynBody::<S, E, C>::try_set_state,
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
    E: error::Error + Send + Sync + 'static,
    C: Display + Send + Sync + 'static,
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
    fn has_state(&self) -> bool {
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
        self.has_state()
            .then(|| unsafe { self.state.assume_init_ref() })
    }

    /// Replaces the stored state with a new value. Returns the old one, if any.
    fn replace_state(&mut self, state: Option<S>) -> Option<S> {
        unsafe {
            let (has_state, old_state) = match (self.has_state(), state) {
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
    fn destruct(self) -> (Option<S>, E, C) {
        let has_state = self.has_state();
        let mut this = MaybeUninit::new(self);
        let this = this.as_mut_ptr();
        unsafe {
            let state = has_state.then(|| (&raw mut (*this).state).read().assume_init());
            let source = (&raw mut (*this).source).read();
            let context = (&raw mut (*this).context).read();
            (state, source, context)
        }
    }
}

impl<S, E, C> DynBody<S, E, C>
where
    S: Debug + Send + Sync + 'static,
    E: error::Error + Send + Sync + 'static,
    C: Display + Send + Sync + 'static,
{
    /// Drops the boxed body.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to a heap-allocated `DynBody<S, E, C>`.
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
    /// Same as [`into_state`](DynBody::into_state). Returns `None` if `E` is [`Nae`].
    unsafe fn into_source(
        mut this: ManuallyDrop<Align4Own<DynBody>>,
    ) -> Option<Box<dyn error::Error + Send + Sync + 'static>> {
        let Align4(this) = *unsafe {
            ManuallyDrop::into_inner(ManuallyDrop::take(&mut this).cast::<Self>()).into_boxed()
        };
        if rtti::is_same_ty::<E, Nae>() {
            return None;
        };

        let (_, source, ..) = this.destruct();

        match rtti::concretize::<_, WithBacktrace>(source) {
            Ok(with_backtrace) => with_backtrace.into_source(),
            Err(source) => Some(Box::new(source)),
        }
    }

    /// Extracts the source error as a trait object from the boxed body.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to `DynBody<S, E, C>`.
    unsafe fn into_backtrace(mut this: ManuallyDrop<Align4Own<DynBody>>) -> Option<WithBacktrace> {
        let Align4(this) = *unsafe {
            ManuallyDrop::into_inner(ManuallyDrop::take(&mut this).cast::<Self>()).into_boxed()
        };

        if rtti::is_same_ty::<E, Nae>() {
            return None;
        };

        let (_, source, ..) = this.destruct();

        match rtti::concretize::<_, WithBacktrace>(source) {
            Ok(with_backtrace) => Some(with_backtrace),
            Err(_source) => None,
        }
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
        mut this: ManuallyDrop<Align4Own<DynBody>>,
        source_ty: TypeId,
        source_dst: NonNull<()>,
        context_ty: TypeId,
        context_dst: NonNull<()>,
        state_ty: TypeId,
        state_dst: NonNull<()>,
    ) {
        let Align4(this) = *unsafe {
            ManuallyDrop::into_inner(ManuallyDrop::take(&mut this).cast::<Self>()).into_boxed()
        };
        let (state, source, context) = this.destruct();

        if !rtti::is_same_ty::<E, Nae>() {
            match rtti::concretize::<_, WithBacktrace>(source) {
                Ok(with_backtrace) => unsafe {
                    with_backtrace.take_source(source_ty, source_dst);
                },
                Err(source) if TypeId::of::<E>() == source_ty => {
                    // Safety: The caller guarantees `source_dst` points to a valid `Option<E>`.
                    let dst = unsafe { source_dst.cast::<Option<E>>().as_mut() };
                    dst.replace(source);
                }
                _ => {}
            }
        }
        if !rtti::is_same_ty::<C, Blank>() && TypeId::of::<C>() == context_ty {
            // Safety: The caller guarantees `context_dst` points to a valid `Option<C>`.
            let dst = unsafe { context_dst.cast::<Option<C>>().as_mut() };
            dst.replace(context);
        }
        if TypeId::of::<S>() == state_ty {
            let dst = unsafe { state_dst.cast::<Option<S>>().as_mut() };
            *dst = state;
        }
    }

    /// Extracts the state from the boxed body, `state_dst` becomes `Some` iff it succeeds and returns `Ok`.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to `DynBody<S, E, C>`.
    /// - `state_dst` must be a valid, aligned, mutable pointer to `Option<StateTy>`.
    unsafe fn extract_state(
        mut this: ManuallyDrop<Align4Own<DynBody>>,
        state_ty: TypeId,
        state_dst: NonNull<()>,
    ) -> result::Result<RawVacant, ManuallyDrop<Align4Own<DynBody>>> {
        let mut this =
            unsafe { ManuallyDrop::into_inner(ManuallyDrop::take(&mut this).cast::<Self>()) };

        if TypeId::of::<S>() == state_ty {
            let dst = unsafe { state_dst.cast::<Option<S>>().as_mut() };
            *dst = this.borrow_mut().deref_mut().replace_state(None);

            if dst.is_some() {
                return Ok(RawVacant(unsafe { this.cast() }));
            }
        }

        Err(unsafe { this.cast() })
    }

    /// Convert the thin `DynBody` pointer to `Box<Error>` without reallocation.
    ///
    /// # Safety
    ///
    /// Same as [`into_parts`](DynBody::into_parts).
    unsafe fn into_boxed_error(
        mut this: ManuallyDrop<Align4Own<DynBody>>,
    ) -> Box<dyn error::Error + Send + Sync + 'static> {
        unsafe {
            ManuallyDrop::into_inner(ManuallyDrop::take(&mut this).cast::<Self>()).into_boxed()
        }
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
    unsafe fn try_set_state(
        this: Mut<'_, DynBody>,
        state_ty: TypeId,
        state_src: NonNull<()>,
    ) -> bool {
        let this = unsafe { this.cast::<Self>().deref_mut() };

        if TypeId::of::<S>() == state_ty {
            let state_src = unsafe { state_src.cast::<Option<S>>().as_mut() };
            let Some(state_src) = state_src.take() else {
                panic!("try_set_state: state_src must be `Some`");
            };
            this.replace_state(Some(state_src));
            true
        } else {
            false
        }
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

        if rtti::is_same_ty::<E, Nae>() {
            return None;
        }

        let source = rtti::concretize_ref::<_, WithBacktrace>(&this.source).ok();

        match (source, WithBacktrace::searching()) {
            (Some(source), false) => source.source(),
            (Some(_), true) | (None, _) => Some(&this.source as _),
        }
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

        if rtti::is_same_ty::<E, Nae>() {
            return None;
        }

        // Note: Check the type first. As of Rust 2024 doing the same thing in the `Err` case of
        // `concretize_mut` will run into NCC problem case #3:
        // https://smallcultfollowing.com/babysteps/blog/2016/04/27/non-lexical-lifetimes-introduction/
        if !rtti::is_same_ty::<E, WithBacktrace>() || WithBacktrace::searching() {
            return Some(&mut this.source as _);
        }

        // Note: This unwrap will never panic as we checked the type first.
        rtti::concretize_mut::<_, WithBacktrace>(&mut this.source)
            .unwrap()
            .source_mut()
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

    /// Returns a reference to the displayable context.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, C>`.
    unsafe fn context(this: Ref<'_, DynBody>) -> Option<&(dyn Display + Send + Sync + 'static)> {
        let this = unsafe { this.cast::<Self>().deref() };

        if rtti::is_same_ty::<C, Blank>() {
            None
        } else {
            Some(&this.context as &(dyn Display + Send + Sync + 'static))
        }
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

        if !rtti::is_same_ty::<C, Blank>() && TypeId::of::<C>() == ty {
            let dst = unsafe { dst.cast::<Option<&C>>().as_mut() };
            *dst = Some(&this.context);
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

        if !rtti::is_same_ty::<C, Blank>() && TypeId::of::<C>() == ty {
            let dst = unsafe { dst.cast::<Option<&mut C>>().as_mut() };
            *dst = Some(&mut this.context);
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

impl<S, E, C> fmt::Debug for DynBody<S, E, C>
where
    S: Debug + Send + Sync + 'static,
    E: error::Error + Send + Sync + 'static,
    C: Display + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        render::format_debug(
            f,
            self.try_get_state(),
            (!rtti::is_same_ty::<C, Blank>()).then_some(&self.context),
            self.source(),
            WithBacktrace::search_debug(|| self.source()),
        )
    }
}

impl<S, E, C> fmt::Display for DynBody<S, E, C>
where
    S: Debug + Send + Sync + 'static,
    E: error::Error + Send + Sync + 'static,
    C: Display + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        render::format_display(
            f,
            self.try_get_state(),
            (!rtti::is_same_ty::<C, Blank>()).then_some(&self.context),
            self.source(),
            WithBacktrace::search_display(|| self.source()),
        )
    }
}

impl<S, E, C> error::Error for DynBody<S, E, C>
where
    S: Debug + Send + Sync + 'static,
    E: error::Error + Send + Sync + 'static,
    C: Display + Send + Sync + 'static,
{
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        if rtti::is_same_ty::<E, Nae>() {
            return None;
        }
        match (
            rtti::is_same_ty::<E, WithBacktrace>(),
            WithBacktrace::searching(),
        ) {
            (true, false) => self.source.source(),
            (true, true) | (false, _) => Some(&self.source as _),
        }
    }
}

pub struct RawVacant(ManuallyDrop<Align4Own<DynBody>>);

impl RawVacant {
    pub fn try_with_state<S>(self, state: S) -> result::Result<RawError<S>, (Self, S)> {
        let mut this = ManuallyDrop::new(self);
        let mut body = unsafe { ManuallyDrop::new(ManuallyDrop::take(&mut this.0)) };

        unsafe {
            let vt = DynBody::vtable(body.borrow());
            let mut state_src = Some(state);

            (vt.try_set_state)(
                body.borrow_mut(),
                TypeId::of::<S>(),
                NonNull::from_mut(&mut state_src).cast(),
            );

            if let Some(state) = state_src {
                Err((Self(body), state))
            } else {
                Ok(RawError { boxed_body: body })
            }
        }
    }

    pub fn try_into_stateless(self) -> result::Result<RawError<Infallible>, Self> {
        let mut this = ManuallyDrop::new(self);
        let body = unsafe { ManuallyDrop::new(ManuallyDrop::take(&mut this.0)) };
        let vt = DynBody::vtable(body.borrow());

        unsafe {
            let body_ref = body.borrow();
            match ((vt.context)(body_ref), (vt.source)(body_ref)) {
                (None, None) => Err(RawVacant(body)),
                _ => Ok(RawError { boxed_body: body }),
            }
        }
    }

    pub fn inherit_self<S, C>(self, state: Option<S>, context: C) -> RawError<S>
    where
        S: Debug + Send + Sync + 'static,
        C: context::Context,
    {
        let mut this = ManuallyDrop::new(self);
        let body = unsafe { ManuallyDrop::new(ManuallyDrop::take(&mut this.0)) };
        let vt = DynBody::vtable(body.borrow());

        unsafe {
            let body_ref = body.borrow();
            match ((vt.context)(body_ref), (vt.source)(body_ref)) {
                (None, None) => match (vt.into_backtrace)(body) {
                    Some(backtrace) => RawError::new(state, backtrace, context),
                    None => RawError::new(state, Nae::new(), context),
                },
                _ => RawError::new(state, RawError::<Infallible> { boxed_body: body }, context),
            }
        }
    }
}

impl Debug for RawVacant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

            render::format_debug_struct::<Infallible>(
                f,
                "Vacant",
                None,
                context.map(|v| v as _),
                source.map(|v| v as _),
                backtrace,
            )
        }
    }
}

impl Drop for RawVacant {
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
    use core::{
        assert_matches,
        convert::Infallible,
        error,
        fmt::{self, Display},
        mem,
    };

    use super::*;
    use crate::{context::Contextless, nae::Nae};

    // --- Test helpers ---

    /// A custom source error for testing.
    #[derive(Debug)]
    struct TestError(&'static str);

    impl Display for TestError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    impl error::Error for TestError {}

    /// A typed literal for testing.
    #[derive(Debug)]
    struct TestContext;

    impl Context for TestContext {
        type Repr = Infallible;

        fn try_into_repr(self) -> Option<Self::Repr> {
            None
        }

        const FALLBACK: Option<&'static str> = Some("test context");
    }

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
        let err = RawError::new_boxed(None::<Infallible>, TestError("oops"), Blank::new());
        assert_eq!(err.kind(), RawError::<()>::KIND_BOXED);
    }

    // --- Const variant ---

    #[test]
    fn const_variant_context() {
        let err = RawError::try_new_const::<TestContext>().unwrap();
        let ctx = err.context();
        assert_eq!(ctx.unwrap().to_string(), "test context");
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
        let err = RawError::new_boxed(None::<Infallible>, TestError("oops"), Blank::new());
        let src = err.source();
        assert_eq!(src.unwrap().to_string(), "oops");
    }

    #[test]
    fn boxed_variant_downcast_source() {
        let err = RawError::new_boxed(None::<Infallible>, TestError("oops"), Blank::new());
        let downcasted = err.downcast_source_ref::<TestError>();
        assert_matches!(downcasted, Some(TestError("oops")));
    }

    #[test]
    fn boxed_variant_downcast_source_wrong_type() {
        let err = RawError::new_boxed(None::<Infallible>, TestError("oops"), Blank::new());
        let downcasted = err.downcast_source_ref::<Nae>();
        assert!(downcasted.is_none());
    }

    #[test]
    fn boxed_variant_context() {
        let err = RawError::new_boxed(
            None::<Infallible>,
            TestError("oops"),
            TestContext::FALLBACK.unwrap(),
        );
        let ctx = err.context();
        assert_eq!(ctx.unwrap().to_string(), "test context");
    }

    #[test]
    fn boxed_variant_nae_source_is_none() {
        // When source is `Nae`, `.source()` should return `None`.
        let err = RawError::new_boxed(Some(42u32), Nae::new(), Blank::new());
        assert!(err.source().is_none());
        assert_matches!(err.state(), Some(42));
    }

    // --- into_source ---

    #[test]
    fn boxed_variant_into_source_returns_boxed_error() {
        let err = RawError::new_boxed(None::<Infallible>, TestError("oops"), Blank::new());
        let src = err.into_source();
        assert_eq!(src.unwrap().to_string(), "oops");
    }

    #[test]
    fn boxed_variant_into_source_nae_returns_none() {
        let err = RawError::new_boxed(None::<Infallible>, Nae::new(), Blank::new());
        assert!(err.into_source().is_none());
    }

    // --- into_parts ---

    #[test]
    fn boxed_variant_into_parts_matches_types() {
        let err = RawError::new_boxed(
            Some("state"),
            TestError("oops"),
            TestContext::FALLBACK.unwrap(),
        );
        let (state, context, source) = err.into_parts::<&str, TestError>();
        assert_matches!(state, Some("state"));
        assert_matches!(source, Some(TestError("oops")));
        assert_eq!(context, TestContext::FALLBACK);
    }

    #[test]
    fn boxed_variant_into_parts_context_downcasts() {
        let err = RawError::new_boxed(None::<Infallible>, TestError("oops"), Blank::new());
        let (_, context, _) = err.into_parts::<Blank, String>();
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
        let (state, context, source) = err.into_parts::<Blank, TestError>();
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
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "")
            }
        }

        impl error::Error for DropWatch {}

        {
            let _err = RawError::new_boxed(None::<Infallible>, DropWatch, Blank::new());
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
        assert_eq!(
            mem::size_of::<RawError<Infallible>>(),
            mem::size_of::<usize>()
        );
    }

    // --- State extraction ---

    #[test]
    fn state_extraction() {
        {
            let err = RawError::new(Some(42u8), Nae::new(), Contextless::new());
            if cfg!(feature = "backtrace") {
                assert_matches!(err.extract_state(), Ok((42, Some(_) | None)));
            } else {
                assert_matches!(err.extract_state(), Ok((42, None)));
            }
        }
        {
            let err = RawError::new(Some(42u128), Nae::new(), Contextless::new());
            assert_matches!(err.extract_state(), Ok((42, Some(_))));
        }
        {
            let err = RawError::new_boxed(None::<Infallible>, Nae::new(), format!("oops"));
            assert_matches!(err.extract_state(), Err(err) if format!("{err}") == "oops");
        }
        {
            let err = RawError::new_boxed(Some(42i32), Nae::new(), format!("oops"));
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
}
