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
    context, match_else,
    nae::Nae,
    payload,
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
    pub fn new_const<L>() -> Self
    where
        L: context::Context + ?Sized,
    {
        #[cfg(feature = "backtrace")]
        if let Ok(source) = WithBacktrace::try_attach(Nae::new()) {
            return RawError::<Infallible>::new_boxed_::<_, _, L>(
                None,
                source,
                payload::Empty::new(),
            );
        }

        // Note: Relies on const promotion to produce a new constant.
        let body: &'static Align4<ConstBody> = &const {
            Align4(ConstBody {
                context: L::LITERAL,
            })
        };
        Self {
            const_body: ManuallyDrop::new(Align4Ref::new(body, Self::KIND_CONST)),
        }
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
    pub fn try_new_inline(state: S) -> result::Result<Self, S>
    where
        S: Debug + Send + Sync + 'static,
    {
        #[cfg(feature = "backtrace")]
        if let Ok(source) = WithBacktrace::try_attach(Nae::new()) {
            return Ok(Self::new_boxed_::<_, _, context::Blank>(
                Some(state),
                source,
                payload::Empty::new(),
            ));
        }

        Ok(Self {
            inline_body: ManuallyDrop::new(Align4PtrCompat::new(Self::KIND_INLINE, state)?),
        })
    }

    /// Constructs a [RawError], storing the given state inline when the state fits within
    /// the inline storage.
    pub fn new_inline_or_boxed(state: S) -> Self
    where
        S: Debug + Send + Sync + 'static,
    {
        let Err(state) = match_else!(Self::try_new_inline(state),
            Ok(this) => return this,
        );
        Self::new_boxed::<Nae, payload::Empty, context::Blank>(
            Some(state),
            Nae::new(),
            payload::Empty::new(),
        )
    }

    /// Constructs a boxed-variant [`RawError`] containing source, payload, and context.
    ///
    /// The source, payload, and context are packed into a single heap allocation
    /// alongside a vtable for type-erased access.
    pub fn new_boxed<E, P, L>(state: Option<S>, source: E, payload: P) -> Self
    where
        S: Debug + Send + Sync + 'static,
        E: error::Error + Send + Sync + 'static,
        P: Display + Send + Sync + 'static,
        L: context::Context + ?Sized,
    {
        match WithBacktrace::try_attach(source) {
            Ok(source) => Self::new_boxed_::<_, _, L>(state, source, payload),
            Err(source) => Self::new_boxed_::<_, _, L>(state, source, payload),
        }
    }

    fn new_boxed_<E, P, L>(state: Option<S>, source: E, payload: P) -> Self
    where
        S: Debug + Send + Sync + 'static,
        E: error::Error + Send + Sync + 'static,
        P: Display + Send + Sync + 'static,
        L: context::Context + ?Sized,
    {
        let (vtable, state) = DynBody::<S, E, P, L::Repr>::vtable_from_state(state);

        // # Safety
        //
        // The `Align4Own` pointer is cast to `DynBody<Infallible, (), (), ()>` for uniform storage.
        // This is valid because all monomorphizations of `DynBody<S, E, P, L::Repr>` share
        // the same vtable pointer, and the concrete `S`, `E`, `P`, `L` are erased.
        // The cast only changes the type parameter defaults — it does not violate the layout
        // because `()` is a ZST.
        // Due to `Align4Own`assumes a correct layout, and that's not true after the cast,
        // we need to put it in a `ManuallyDrop` and drop it via vtable instead.
        Self {
            boxed_body: unsafe {
                Align4Own::from_boxed(
                    Box::new(Align4(DynBody::<S, E, P, L::Repr> {
                        vtable,
                        state,
                        source,
                        payload,
                        context: L::new_context(),
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

    /// Returns a reference to the displayable payload, if present.
    pub fn payload(&self) -> Option<&'_ (dyn Display + Send + Sync + 'static)> {
        match self.select_ref() {
            SelectRef::Inline(_body) => None,
            SelectRef::Const(_body) => None,
            SelectRef::Boxed(body) => unsafe {
                let vtable = DynBody::vtable(body.borrow());
                // Safety: The body pointer is confirmed valid.
                (vtable.payload)(body.borrow())
            },
        }
    }

    /// Returns a reference to the wrapped source error, if present.
    pub fn source(&self) -> Option<&(dyn error::Error + 'static)> {
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

    /// Attempts to downcast the stored source error to `E`.
    ///
    /// Returns `None` if the source is not of type `E` or does not exist.
    pub fn downcast_source_ref<E>(&self) -> Option<&E>
    where
        E: error::Error + 'static,
    {
        self.source()?.downcast_ref::<E>()
    }

    /// Attempts to downcast the stored payload to `P`.
    pub fn downcast_payload_ref<P>(&self) -> Option<&P>
    where
        P: 'static,
    {
        match self.select_ref() {
            SelectRef::Const(_body) => None,
            SelectRef::Inline(_body) => None,
            SelectRef::Boxed(body) => unsafe {
                let vtable = DynBody::vtable(body.borrow());
                let mut result = None::<&P>;
                // Safety: The body pointer is confirmed valid.
                (vtable.downcast_payload_ref)(
                    body.borrow(),
                    TypeId::of::<P>(),
                    NonNull::from_mut(&mut result).cast(),
                );
                result
            },
        }
    }

    /// Attempts to downcast the stored payload to `P` by mutable reference.
    pub fn downcast_payload_mut<P>(&mut self) -> Option<&mut P>
    where
        P: 'static,
    {
        match self.select_mut() {
            SelectMut::Const(_body) => None,
            SelectMut::Inline(_body) => None,
            SelectMut::Boxed(body) => unsafe {
                let vtable = DynBody::vtable(body.borrow());
                let mut result = None::<&mut P>;
                // Safety: The body pointer is confirmed valid.
                (vtable.downcast_payload_mut)(
                    body.borrow_mut(),
                    TypeId::of::<P>(),
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

    /// Consumes `self` and returns the stored state.
    pub fn into_state(self) -> Option<S> {
        match self.select_own() {
            SelectOwn::Const(_body) => None,
            SelectOwn::Inline(body) => Some(body.into_value()),
            SelectOwn::Boxed(body) => {
                unsafe {
                    let vtable = DynBody::vtable(body.borrow());
                    let mut state = None::<S>;

                    // Safety: The body and state pointers are confirmed valid.
                    (vtable.into_state)(
                        body,
                        TypeId::of::<S>(),
                        NonNull::from_mut(&mut state).cast(),
                    );
                    state
                }
            }
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

    /// Consumes `self` and extracts the source error and payload by type.
    ///
    /// Returns `None` if the types do not match or no source/payload exists.
    ///
    /// # Existence Guarantee
    /// It's guaranteed that at least one components exist.
    pub fn into_parts<P, E>(self) -> (Option<S>, Option<&'static str>, Option<P>, Option<E>)
    where
        E: 'static,
        P: 'static,
    {
        match self.select_own() {
            SelectOwn::Const(body) => (
                None,
                Some(
                    // Safety: The project to context is inbound.
                    unsafe { body.borrow().project(|v| &raw const (*v).context).copied() },
                ),
                None,
                None,
            ),
            SelectOwn::Inline(body) => (Some(body.into_value()), None, None, None),
            SelectOwn::Boxed(body) => unsafe {
                let vtable = DynBody::vtable(body.borrow());
                let mut state: Option<S> = None;
                let mut context: Option<&'static str> = None;
                let mut payload: Option<P> = None;
                let mut err: Option<E> = None;

                // Safety: The body, state, context, payload, and error pointers are confirmed valid.
                (vtable.into_parts)(
                    body,
                    TypeId::of::<E>(),
                    NonNull::from_mut(&mut err).cast(),
                    TypeId::of::<P>(),
                    NonNull::from_mut(&mut payload).cast(),
                    NonNull::from_mut(&mut context).cast(),
                    TypeId::of::<S>(),
                    NonNull::from_mut(&mut state).cast(),
                );

                (state, context, payload, err)
            },
        }
    }

    pub fn extract_state(
        self,
    ) -> result::Result<(S, Option<RawError<Infallible>>), RawError<Infallible>> {
        match self.select_own() {
            SelectOwn::Const(body) => Err(RawError {
                const_body: ManuallyDrop::new(body),
            }),
            SelectOwn::Inline(body) => Ok((body.into_value(), None)),
            SelectOwn::Boxed(body) => {
                unsafe {
                    let vtable = DynBody::vtable(body.borrow());
                    let mut state_dst = None::<S>;
                    let mut error_dst = None::<ManuallyDrop<Align4Own<DynBody>>>;
                    // Safety: The body, state, error pointers are confirmed valid.
                    (vtable.extract_state)(
                        body,
                        TypeId::of::<S>(),
                        NonNull::from_mut(&mut state_dst).cast(),
                        NonNull::from_mut(&mut error_dst).cast(),
                    );

                    match (state_dst, error_dst) {
                        (Some(state), Some(body)) => {
                            Ok((state, Some(RawError { boxed_body: body })))
                        }
                        (Some(state), None) => Ok((state, None)),
                        (None, Some(body)) => Err(RawError { boxed_body: body }),
                        (None, None) => unreachable!(),
                    }
                }
            }
        }
    }

    // Sets the state if the underlying storage type is compatible.
    pub fn try_set_state(&mut self, state: S) -> result::Result<(), S> {
        match self.select_mut() {
            SelectMut::Const(_body) => Err(state),
            SelectMut::Inline(body) => {
                body.set_value(state);
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
            "Error",
            self.state(),
            self.context(),
            self.payload(),
            self.source(),
            WithBacktrace::search_debug(self),
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
            self.payload().map(|v| v as _),
            self.source(),
            WithBacktrace::search_display(self),
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

/// Heap-allocated error body with type-erased source, payload, and context.
///
/// The concrete types `E`, `P`, `L` are only known at construction time and at
/// the monomorphized vtable function sites. The `RawError` stores the body as
/// `DynBody<S, (), (), ()>`. The `S` can also be erased when the state is
/// extracted.
///
/// # Safety
///
/// The `vtable` pointer must point to a `DynBodyVTable` that was monomorphized
/// for the same `S`, `E`, `P`, `L` as the stored data.
#[repr(C)]
struct DynBody<S = Infallible, E = (), P = (), L = ()>
where
    S: 'static,
    E: 'static,
    P: 'static,
    L: 'static,
{
    vtable: Align4Ref<'static, DynBodyVTable>, // Note: The vtable must be the first field as the other fields may be erased.
    state: MaybeUninit<S>,
    source: E,
    payload: P,
    context: L,
}

/// Virtual function table for type-erased operations on [`DynBody`].
///
/// Each function pointer is monomorphized for the concrete `S`, `E`, `P`, `L`.
///
/// # Safety
///
/// All function pointers must be valid for the concrete types stored in the `DynBody`.
/// The `Ref`/`Mut`/`Align4Own` arguments must point to a `DynBody` whose type parameters
/// match the monomorphization that produced the function pointer.
struct DynBodyVTable {
    /// See [DynBody::drop].
    drop: unsafe fn(ManuallyDrop<Align4Own<DynBody>>),
    /// See [DynBody::into_state].
    into_state: unsafe fn(ManuallyDrop<Align4Own<DynBody>>, TypeId, NonNull<()>),
    /// See [DynBody::into_source].
    into_source: unsafe fn(
        ManuallyDrop<Align4Own<DynBody>>,
    ) -> Option<Box<dyn error::Error + Send + Sync + 'static>>,
    /// See [DynBody::into_parts].
    into_parts: unsafe fn(
        ManuallyDrop<Align4Own<DynBody>>,
        TypeId,
        NonNull<()>,
        TypeId,
        NonNull<()>,
        NonNull<()>,
        TypeId,
        NonNull<()>,
    ),
    /// See [DynBody::extract_state].
    extract_state: unsafe fn(ManuallyDrop<Align4Own<DynBody>>, TypeId, NonNull<()>, NonNull<()>),
    /// See [DynBody::into_boxed_error].
    into_boxed_error: unsafe fn(
        ManuallyDrop<Align4Own<DynBody>>,
    ) -> Box<dyn error::Error + Send + Sync + 'static>,
    /// See [DynBody::try_set_state].
    try_set_state: unsafe fn(Mut<DynBody>, TypeId, NonNull<()>) -> bool,
    /// See [DynBody::source].
    source: unsafe fn(Ref<'_, DynBody>) -> Option<&(dyn error::Error + 'static)>,
    /// See [DynBody::state].
    state: unsafe fn(Ref<'_, DynBody>, TypeId, NonNull<()>),
    /// See [DynBody::payload].
    payload: unsafe fn(Ref<'_, DynBody>) -> Option<&(dyn Display + Send + Sync + 'static)>,
    /// See [DynBody::context].
    context: unsafe fn(Ref<'_, DynBody>) -> Option<&(dyn Display + Send + Sync + 'static)>,
    /// See [DynBody::downcast_payload_ref].
    downcast_payload_ref: unsafe fn(Ref<'_, DynBody>, TypeId, NonNull<()>),
    /// See [DynBody::downcast_payload_mut].
    downcast_payload_mut: unsafe fn(Mut<'_, DynBody>, TypeId, NonNull<()>),
}

impl DynBodyVTable {
    const fn new<S, E, P, L>() -> Self
    where
        S: Debug + Send + Sync + 'static,
        E: error::Error + Send + Sync + 'static,
        L: Display + Send + Sync + 'static,
        P: Display + Send + Sync + 'static,
    {
        DynBodyVTable {
            drop: DynBody::<S, E, P, L>::drop,
            into_state: DynBody::<S, E, P, L>::into_state,
            into_source: DynBody::<S, E, P, L>::into_source,
            into_parts: DynBody::<S, E, P, L>::into_parts,
            extract_state: DynBody::<S, E, P, L>::extract_state,
            into_boxed_error: DynBody::<S, E, P, L>::into_boxed_error,
            try_set_state: DynBody::<S, E, P, L>::try_set_state,
            source: DynBody::<S, E, P, L>::source,
            state: DynBody::<S, E, P, L>::state,
            payload: DynBody::<S, E, P, L>::payload,
            context: DynBody::<S, E, P, L>::context,
            downcast_payload_ref: DynBody::<S, E, P, L>::downcast_payload_ref,
            downcast_payload_mut: DynBody::<S, E, P, L>::downcast_payload_mut,
        }
    }
}

impl<S, E, P, L> DynBody<S, E, P, L> {
    const NO_STATE: Metadata = Metadata::_0;
    const HAS_STATE: Metadata = Metadata::_1;

    /// Returns a static shared reference to the vtable.
    fn vtable(this: Ref<'_, DynBody<S, E, P, L>>) -> &'static DynBodyVTable {
        unsafe {
            this.project(|body| &raw const (*body).vtable)
                .deref()
                .borrow()
                .deref()
        }
    }
}

impl<S, E, P, L> DynBody<S, E, P, L>
where
    S: Debug + Send + Sync + 'static,
    E: error::Error + Send + Sync + 'static,
    P: Display + Send + Sync + 'static,
    L: Display + Send + Sync + 'static,
{
    fn vtable_from_state(state: Option<S>) -> (Align4Ref<'static, DynBodyVTable>, MaybeUninit<S>) {
        (
            Align4Ref::new(
                &const { Align4(DynBodyVTable::new::<S, E, P, L>()) },
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
    /// `(state, source, payload, context)`.
    fn destruct(self) -> (Option<S>, E, P, L) {
        let has_state = self.has_state();
        let mut this = MaybeUninit::new(self);
        let this = this.as_mut_ptr();
        unsafe {
            let state = has_state.then(|| (&raw mut (*this).state).read().assume_init());
            let source = (&raw mut (*this).source).read();
            let payload = (&raw mut (*this).payload).read();
            let context = (&raw mut (*this).context).read();
            (state, source, payload, context)
        }
    }
}

impl<S, E, P, L> DynBody<S, E, P, L>
where
    S: Debug + Send + Sync + 'static,
    E: error::Error + Send + Sync + 'static,
    P: Display + Send + Sync + 'static,
    L: Display + Send + Sync + 'static,
{
    /// Drops the boxed body.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to a heap-allocated `DynBody<S, E, P, L>`.
    unsafe fn drop(mut this: ManuallyDrop<Align4Own<DynBody>>) {
        unsafe {
            let this = ManuallyDrop::take(&mut this).cast::<Self>();

            let _ = ManuallyDrop::into_inner(this).into_boxed();
        }
    }

    /// Extracts `state` from the boxed body and drops the allocation.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to a heap-allocated `DynBody<S, E, P, L>`.
    /// - `state_dst` must be a valid, aligned, mutable pointer to `Option<S>`.
    unsafe fn into_state(
        mut this: ManuallyDrop<Align4Own<DynBody>>,
        state_ty: TypeId,
        state_dst: NonNull<()>,
    ) {
        let Align4(mut this) = *unsafe {
            ManuallyDrop::into_inner(ManuallyDrop::take(&mut this).cast::<Self>()).into_boxed()
        };

        if TypeId::of::<S>() == state_ty {
            // Safety: The caller guarantees `state_dst` points to a valid `Option<S>`.
            let dst = unsafe { state_dst.cast::<Option<S>>().as_mut() };
            *dst = this.replace_state(None);
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

    /// Decomposes the boxed body: extracts source and payload into caller-provided
    /// `Option`s (if the `TypeId` matches), and returns the state.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to `DynBody<S, E, P, L>`.
    /// - `source_dst`, `payload_dst`,`context_dst`, `state_dst` must be valid, aligned, mutable
    ///   pointers to `Option<E>`, `Option<P>`, `Option<&'static str>` and `Option<S>` respectively.
    #[allow(clippy::too_many_arguments)]
    unsafe fn into_parts(
        mut this: ManuallyDrop<Align4Own<DynBody>>,
        source_ty: TypeId,
        source_dst: NonNull<()>,
        payload_ty: TypeId,
        payload_dst: NonNull<()>,
        context_dst: NonNull<()>,
        state_ty: TypeId,
        state_dst: NonNull<()>,
    ) {
        let Align4(this) = *unsafe {
            ManuallyDrop::into_inner(ManuallyDrop::take(&mut this).cast::<Self>()).into_boxed()
        };
        let (state, source, payload, context) = this.destruct();

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
        if !rtti::is_same_ty::<P, payload::Empty>() && TypeId::of::<P>() == payload_ty {
            // Safety: The caller guarantees `payload_dst` points to a valid `Option<P>`.
            let dst = unsafe { payload_dst.cast::<Option<P>>().as_mut() };
            dst.replace(payload);
        }
        if !rtti::is_same_ty::<L, context::Unit>() && rtti::is_same_ty::<L, &'static str>() {
            // Safety: The caller guarantees `context_dst` points to a valid `Option<&'static str>`.
            let dst = unsafe { context_dst.cast::<Option<L>>().as_mut() };
            dst.replace(context);
        }
        if TypeId::of::<S>() == state_ty {
            let dst = unsafe { state_dst.cast::<Option<S>>().as_mut() };
            *dst = state;
        }
    }

    /// Extracts the state from the boxed body. This function guarantees at least one of the `Option` will be filled.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to `DynBody<S, E, P, L>`.
    /// - `state_dst` must be a valid, aligned, mutable pointer to `Option<StateTy>`.
    /// - `error_dst` must be a valid, aligned, mutable pointer to `Option<ManuallyDrop<Align4Own<DynBody>>>`
    unsafe fn extract_state(
        mut this: ManuallyDrop<Align4Own<DynBody>>,
        state_ty: TypeId,
        state_dst: NonNull<()>,
        error_dst: NonNull<()>,
    ) {
        let mut this =
            unsafe { ManuallyDrop::into_inner(ManuallyDrop::take(&mut this).cast::<Self>()) };

        if TypeId::of::<S>() == state_ty {
            let dst = unsafe { state_dst.cast::<Option<S>>().as_mut() };
            *dst = this.borrow_mut().deref_mut().replace_state(None);
        }

        let has_context = !rtti::is_same_ty::<L, context::Unit>();
        let has_payload = !rtti::is_same_ty::<P, payload::Empty>();
        let has_source = !{
            // TODO: This will discard the backtrace. It's ideal to keep
            // the backtrace even if the error becomes empty.
            (rtti::is_same_ty::<E, Nae>())
                || (rtti::is_same_ty::<E, WithBacktrace>()
                    && this.borrow().deref().source.source().is_none())
        };

        match (has_context, has_payload, has_source) {
            (false, false, false) => {
                mem::drop(this.into_boxed());
            }
            _ => {
                // Safety:
                // Erase the remaining fields to ZSTs is dangerous as the destructor assumes a correct layout.
                // So we wrap it in `ManuallyDrop` to prevent the destructor from being automatically called.
                let error_dst = unsafe {
                    error_dst
                        .cast::<Option<ManuallyDrop<Align4Own<DynBody>>>>()
                        .as_mut()
                };
                error_dst.replace(unsafe { this.cast::<DynBody>() });
            }
        };
    }

    /// Convert the thin `DynBody` pointer to `Box<Error>` without reallocation.
    ///
    /// # Safety
    ///
    /// Same as [`into_parts`](DynBody::into_parts).
    unsafe fn into_boxed_error(
        mut this: ManuallyDrop<Align4Own<DynBody>>,
    ) -> Box<dyn error::Error + Send + Sync + 'static> {
        let this = unsafe {
            ManuallyDrop::into_inner(ManuallyDrop::take(&mut this).cast::<Self>()).into_boxed()
        };

        this
    }

    /// Replace the state if type matches.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Mut` pointing to `DynBody<S, E, P, L>`.
    /// - `state_src` must be a valid, aligned, mutable pointer to `Option<StateTy>`.
    unsafe fn try_set_state(
        this: Mut<'_, DynBody>,
        state_ty: TypeId,
        state_src: NonNull<()>,
    ) -> bool {
        let this = unsafe { this.cast::<Self>().deref_mut() };

        if TypeId::of::<S>() == state_ty {
            let state_src = unsafe { state_src.cast::<Option<S>>().as_mut() };
            let Some(state_src) = state_src.take() else {
                return false;
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
    /// - `this` must point to a valid `DynBody<S, E, P, L>`.
    unsafe fn source(this: Ref<'_, DynBody>) -> Option<&(dyn error::Error + 'static)> {
        let this = unsafe { this.cast::<Self>().deref() };

        if rtti::is_same_ty::<E, Nae>() {
            return None;
        }

        match (
            rtti::is_same_ty::<E, WithBacktrace>(),
            WithBacktrace::searching(),
        ) {
            (true, false) => this.source.source(),
            (true, true) | (false, _) => Some(&this.source as _),
        }
    }

    /// Returns a reference to the state.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, P, L>`.
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

    /// Returns a reference to the displayable payload.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, P, L>`.
    unsafe fn payload(this: Ref<'_, DynBody>) -> Option<&(dyn Display + Send + Sync + 'static)> {
        let this = unsafe { this.cast::<Self>().deref() };

        if rtti::is_same_ty::<P, payload::Empty>() {
            None
        } else {
            Some(&this.payload as &(dyn Display + Send + Sync + 'static))
        }
    }

    /// Returns a reference to the displayable context.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, P, L>`.
    unsafe fn context(this: Ref<'_, DynBody>) -> Option<&(dyn Display + Send + Sync + 'static)> {
        let this = unsafe { this.cast::<Self>().deref() };

        if rtti::is_same_ty::<L, context::Unit>() {
            None
        } else {
            Some(&this.context as &(dyn Display + Send + Sync + 'static))
        }
    }

    /// Attempts to downcast the payload field to the requested type `P`.
    ///
    /// Writes `Some(&P)` into `dst` if the type matches, otherwise does nothing.
    ///
    /// # Safety
    ///
    /// Same as [`downcast_source_ref`](DynBody::downcast_source_ref) for the payload field.
    /// - `dst` must be a valid, aligned, mutable pointer to `Option<&Ty>`.
    unsafe fn downcast_payload_ref(this: Ref<'_, DynBody>, ty: TypeId, dst: NonNull<()>) {
        let this = unsafe { this.cast::<Self>().deref() };

        if !rtti::is_same_ty::<P, payload::Empty>() && TypeId::of::<P>() == ty {
            let dst = unsafe { dst.cast::<Option<&P>>().as_mut() };
            *dst = Some(&this.payload);
        }
    }

    /// Attempts to downcast the payload field to the requested type `P` (mutable).
    ///
    /// Writes `Some(&mut P)` into `dst` if the type matches, otherwise does nothing.
    ///
    /// # Safety
    ///
    /// Same as [`downcast_payload_ref`](DynBody::downcast_payload_ref) with mutable access.
    /// - `dst` must be a valid, aligned, mutable pointer to `Option<&mut Ty>`.
    unsafe fn downcast_payload_mut(this: Mut<'_, DynBody>, ty: TypeId, dst: NonNull<()>) {
        let this = unsafe { this.cast::<Self>().deref_mut() };

        if !rtti::is_same_ty::<P, payload::Empty>() && TypeId::of::<P>() == ty {
            let dst = unsafe { dst.cast::<Option<&mut P>>().as_mut() };
            *dst = Some(&mut this.payload);
        }
    }
}

impl<S, E, P, L> Drop for DynBody<S, E, P, L> {
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

impl<S, E, P, L> fmt::Debug for DynBody<S, E, P, L>
where
    S: Debug + Send + Sync + 'static,
    E: error::Error + Send + Sync + 'static,
    P: Display + Send + Sync + 'static,
    L: Display + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        render::format_debug(
            f,
            "Error",
            self.try_get_state(),
            (!rtti::is_same_ty::<L, context::Unit>()).then_some(&self.context),
            (!rtti::is_same_ty::<P, payload::Empty>()).then_some(&self.payload),
            self.source(),
            WithBacktrace::search_debug(self),
        )
    }
}

impl<S, E, P, L> fmt::Display for DynBody<S, E, P, L>
where
    S: Debug + Send + Sync + 'static,
    E: error::Error + Send + Sync + 'static,
    P: Display + Send + Sync + 'static,
    L: Display + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        render::format_display(
            f,
            self.try_get_state(),
            (!rtti::is_same_ty::<L, context::Unit>()).then_some(&self.context),
            (!rtti::is_same_ty::<P, payload::Empty>()).then_some(&self.payload),
            self.source(),
            WithBacktrace::search_display(self),
        )
    }
}

impl<S, E, P, L> error::Error for DynBody<S, E, P, L>
where
    S: Debug + Send + Sync + 'static,
    E: error::Error + Send + Sync + 'static,
    P: Display + Send + Sync + 'static,
    L: Display + Send + Sync + 'static,
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

#[cfg(test)]
mod tests {
    use alloc::{
        format,
        string::{String, ToString},
    };
    use core::{
        convert::Infallible,
        error,
        fmt::{self, Display},
        mem,
    };

    use super::*;
    use crate::{
        context::{Blank, Literal},
        nae::Nae,
        payload,
    };

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
    struct TestContext;

    impl Literal for TestContext {
        const LITERAL: &'static str = "test context";
    }

    /// A custom payload type.
    #[derive(Debug, PartialEq)]
    struct TestPayload(u32);

    impl Display for TestPayload {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "payload({})", self.0)
        }
    }

    // --- RawError kind() discrimination ---

    #[cfg(not(feature = "backtrace"))]
    #[test]
    fn kind_discriminates_const() {
        let err = RawError::new_const::<TestContext>();
        assert_eq!(err.kind(), RawError::<()>::KIND_CONST);
    }

    #[cfg(not(feature = "backtrace"))]
    #[test]
    fn kind_discriminates_inline() {
        let err = RawError::try_new_inline(4216u16).unwrap();
        assert_eq!(err.kind(), RawError::<u16>::KIND_INLINE);
    }

    #[cfg(not(feature = "backtrace"))]
    #[test]
    fn kind_discriminates_boxed() {
        let err = RawError::new_boxed::<_, _, Blank>(
            None::<Infallible>,
            TestError("oops"),
            payload::Empty::new(),
        );
        assert_eq!(err.kind(), RawError::<()>::KIND_BOXED);
    }

    // --- Const variant ---

    #[test]
    fn const_variant_context() {
        let err = RawError::new_const::<TestContext>();
        let ctx = err.context();
        assert!(ctx.is_some());
        assert_eq!(ctx.unwrap().to_string(), "test context");
    }

    #[test]
    fn const_variant_source_is_none() {
        let err = RawError::new_const::<TestContext>();
        assert!(err.source().is_none());
    }

    #[test]
    fn const_variant_payload_is_none() {
        let err = RawError::new_const::<TestContext>();
        assert!(err.payload().is_none());
    }

    #[test]
    fn const_variant_into_state() {
        let err = RawError::new_const::<TestContext>();
        let state = err.into_state();
        assert!(matches!(state, None));
    }

    // --- Inline variant ---

    #[test]
    fn inline_variant_state() {
        let err = RawError::try_new_inline(42u16).unwrap();
        assert!(matches!(err.state(), Some(42)));
    }

    #[test]
    fn inline_variant_into_state() {
        let err = RawError::try_new_inline(42u16).unwrap();
        assert!(matches!(err.state(), Some(42)));
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

    #[test]
    fn inline_variant_payload_is_none() {
        let err = RawError::try_new_inline(42u16).unwrap();
        assert!(err.payload().is_none());
    }

    // Boxed variant ---

    #[test]
    fn boxed_variant_source() {
        let err = RawError::new_boxed::<_, _, Blank>(
            None::<Infallible>,
            TestError("oops"),
            payload::Empty::new(),
        );
        let src = err.source();
        assert!(src.is_some());
        assert_eq!(src.unwrap().to_string(), "oops");
    }

    #[test]
    fn boxed_variant_downcast_source() {
        let err = RawError::new_boxed::<_, _, Blank>(
            None::<Infallible>,
            TestError("oops"),
            payload::Empty::new(),
        );
        let downcasted = err.downcast_source_ref::<TestError>();
        assert!(downcasted.is_some());
        assert_eq!(downcasted.unwrap().0, "oops");
    }

    #[test]
    fn boxed_variant_downcast_source_wrong_type() {
        let err = RawError::new_boxed::<_, _, Blank>(
            None::<Infallible>,
            TestError("oops"),
            payload::Empty::new(),
        );
        let downcasted = err.downcast_source_ref::<Nae>();
        assert!(downcasted.is_none());
    }

    #[test]
    fn boxed_variant_payload() {
        let err = RawError::new_boxed::<_, _, Blank>(
            None::<Infallible>,
            TestError("oops"),
            TestPayload(42),
        );
        let pl = err.payload();
        assert!(pl.is_some());
        assert_eq!(pl.unwrap().to_string(), "payload(42)");
    }

    #[test]
    fn boxed_variant_downcast_payload() {
        let err = RawError::new_boxed::<_, _, Blank>(
            None::<Infallible>,
            TestError("oops"),
            TestPayload(42),
        );
        let downcasted = err.downcast_payload_ref::<TestPayload>();
        assert!(downcasted.is_some());
        assert_eq!(downcasted.unwrap().0, 42);
    }

    #[test]
    fn boxed_variant_context() {
        let err = RawError::new_boxed::<_, _, TestContext>(
            None::<Infallible>,
            TestError("oops"),
            payload::Empty::new(),
        );
        let ctx = err.context();
        assert!(ctx.is_some());
        assert_eq!(ctx.unwrap().to_string(), "test context");
    }

    #[test]
    fn boxed_variant_nae_source_is_none() {
        // When source is `Nae`, `.source()` should return `None`.
        let err =
            RawError::new_boxed::<_, _, Blank>(Some(42u32), Nae::new(), payload::Empty::new());
        assert!(err.source().is_none());
        assert!(matches!(err.state(), Some(42)));
    }

    #[test]
    fn boxed_variant_empty_payload_is_none() {
        let err = RawError::new_boxed::<_, _, Blank>(
            None::<Infallible>,
            TestError("oops"),
            payload::Empty::new(),
        );
        assert!(err.payload().is_none());
    }

    // --- into_source ---

    #[test]
    fn boxed_variant_into_source_returns_boxed_error() {
        let err = RawError::new_boxed::<_, _, Blank>(
            None::<Infallible>,
            TestError("oops"),
            payload::Empty::new(),
        );
        let src = err.into_source();
        assert!(src.is_some());
        assert_eq!(src.unwrap().to_string(), "oops");
    }

    #[test]
    fn boxed_variant_into_source_nae_returns_none() {
        let err = RawError::new_boxed::<_, _, Blank>(
            None::<Infallible>,
            Nae::new(),
            payload::Empty::new(),
        );
        assert!(err.into_source().is_none());
    }

    // --- into_parts ---

    #[test]
    fn boxed_variant_into_parts_matches_types() {
        let err = RawError::new_boxed::<_, _, TestContext>(
            Some("state"),
            TestError("oops"),
            TestPayload(99),
        );
        let (state, context, payload, source) = err.into_parts::<TestPayload, TestError>();
        assert!(matches!(state, Some("state")));
        assert!(source.is_some());
        assert_eq!(source.unwrap().0, "oops");
        assert!(payload.is_some());
        assert_eq!(payload.unwrap().0, 99);
        assert!(matches!(context, Some(TestContext::LITERAL)))
    }

    #[test]
    fn boxed_variant_into_parts_wrong_source_type() {
        let err = RawError::new_boxed::<_, _, Blank>(
            None::<Infallible>,
            TestError("oops"),
            payload::Empty::new(),
        );
        let (_, _, payload, source) = err.into_parts::<payload::Empty, String>();
        assert!(source.is_none());
        assert!(payload.is_none());
    }

    #[test]
    fn const_variant_into_parts() {
        let err = RawError::new_const::<TestContext>();
        let (state, _, payload, source) = err.into_parts::<TestPayload, TestError>();
        assert!(source.is_none());
        assert!(payload.is_none());
        assert_eq!(state, None);
    }

    #[test]
    fn inline_variant_into_parts() {
        let err = RawError::try_new_inline(42u16).unwrap();
        let (state, _, payload, source) = err.into_parts::<TestPayload, TestError>();
        assert!(source.is_none());
        assert!(payload.is_none());
        assert!(matches!(state, Some(42)));
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
            let _err = RawError::new_boxed::<_, _, Blank>(
                None::<Infallible>,
                DropWatch,
                payload::Empty::new(),
            );
        } // drop here
        assert!(DROPPED.load(Ordering::SeqCst));
    }

    // --- State round-trip for const variant (S = ()) ---

    #[test]
    fn const_variant_state_is_none() {
        let err = RawError::new_const::<TestContext>();
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
            let err = RawError::new_inline_or_boxed(42u16);
            assert!(matches!(err.extract_state(), Ok((42, None))));
        }
        {
            let err = RawError::new_inline_or_boxed(42u128);
            assert!(matches!(err.extract_state(), Ok((42, None))));
        }
        {
            let err =
                RawError::new_boxed::<_, _, Blank>(None::<Infallible>, Nae::new(), format!("oops"));
            assert!(matches!(err.extract_state(), Err(err) if format!("{err:-}") == "oops"));
        }
        {
            let err = RawError::new_boxed::<_, _, Blank>(Some(42i32), Nae::new(), format!("oops"));
            assert!(
                matches!(err.extract_state(), Ok((42i32, Some(err))) if format!("{err:-}") == "oops")
            );
        }
    }
}
