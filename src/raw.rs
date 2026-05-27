use crate::{
    context::{self, Literal},
    match_else,
    nae::Nae,
    payload,
    ptr::{Align4, Align4Own, Align4PtrCompat, Align4Ref, Metadata, Mut, Ref},
    rtti,
};
use std::{
    self,
    any::TypeId,
    convert::Infallible,
    error,
    fmt::{self, Debug, Display},
    mem::{self, ManuallyDrop},
    ptr::NonNull,
    result,
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
        L: Literal + ?Sized,
    {
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
}

impl<S> RawError<S> {
    /// Constructs an inline-variant [`RawError`] with `state` stored directly.
    pub fn new_inline(state: S) -> result::Result<Self, S> {
        Ok(Self {
            inline_body: ManuallyDrop::new(Align4PtrCompat::new(Self::KIND_INLINE, state)?),
        })
    }

    pub fn new_inline_or_boxed(state: S) -> Self
    where
        S: Debug + Send + Sync + 'static,
    {
        let Err(state) = match_else!(Self::new_inline(state),
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
        L: Literal + context::Context + ?Sized,
    {
        // # Safety
        //
        // The `Align4Own` pointer is cast to `DynBody<(), (), (), ()>` for uniform storage.
        // This is valid because all monomorphizations of `DynBody<S, E, P, L::Repr>` share
        // the same vtable pointer, and the concrete `S`, `E`, `P`, `C` are erased.
        // The cast only changes the type parameter defaults — it does not violate the layout
        // because `()` is a ZST.
        // Due to `Align4Own`assumes a correct layout, and that's not true after the cast,
        // we need to put it in a `ManuallyDrop` and drop it via vtable instead.
        let ptr = unsafe {
            Align4Own::from_boxed(
                Box::new(Align4(DynBody::<S, E, P, L::Repr> {
                    vtable: &const { DynBodyVTable::new::<S, E, P, L::Repr>() },
                    state,
                    source,
                    payload,
                    context: L::new_context(),
                })),
                Self::KIND_BOXED,
            )
            .cast::<DynBody>()
        };

        Self {
            boxed_body: ManuallyDrop::new(ptr),
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
                // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                let vtable = body
                    .borrow()
                    .project(|body| &raw const (*body).vtable)
                    .copied();

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
                // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                let vtable = body
                    .borrow()
                    .project(|body| &raw const (*body).vtable)
                    .copied();

                (vtable.payload)(body.borrow())
            },
        }
    }

    /// Returns a reference to the wrapped source error, if present.
    pub fn source(&self) -> Option<&(dyn error::Error + Send + Sync + 'static)> {
        match self.select_ref() {
            SelectRef::Const(_body) => None,
            SelectRef::Inline(_body) => None,
            SelectRef::Boxed(body) => unsafe {
                // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                let vtable = body
                    .borrow()
                    .project(|body| &raw const (*body).vtable)
                    .copied();
                (vtable.source)(body.borrow())
            },
        }
    }

    /// Attempts to downcast the stored source error to `E`.
    ///
    /// Returns `None` if the source is not of type `E` or does not exist.
    pub fn downcast_source_ref<E>(&self) -> Option<&E>
    where
        E: 'static,
    {
        match self.select_ref() {
            SelectRef::Const(_body) => None,
            SelectRef::Inline(_body) => None,
            SelectRef::Boxed(body) => unsafe {
                // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                let vtable = body
                    .borrow()
                    .project(|body| &raw const (*body).vtable)
                    .copied();
                (vtable.downcast_source_ref)(body.borrow(), TypeId::of::<E>())
                    .map(|err| err.cast::<E>().deref())
            },
        }
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
                // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                let vtable = body
                    .borrow()
                    .project(|body| &raw const (*body).vtable)
                    .copied();
                (vtable.downcast_payload_ref)(body.borrow(), TypeId::of::<P>())
                    .map(|err| err.cast::<P>().deref())
            },
        }
    }

    /// Attempts to downcast the stored source error to `E` by mutable reference.
    pub fn downcast_source_mut<E>(&mut self) -> Option<&mut E>
    where
        E: 'static,
    {
        match self.select_mut() {
            SelectMut::Const(_body) => None,
            SelectMut::Inline(_body) => None,
            SelectMut::Boxed(body) => unsafe {
                // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                let vtable = body
                    .borrow()
                    .project(|body| &raw const (*body).vtable)
                    .copied();
                (vtable.downcast_source_mut)(body.borrow_mut(), TypeId::of::<E>())
                    .map(|err| err.cast::<E>().deref_mut())
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
                // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                let vtable = body
                    .borrow()
                    .project(|body| &raw const (*body).vtable)
                    .copied();
                (vtable.downcast_payload_mut)(body.borrow_mut(), TypeId::of::<P>())
                    .map(|err| err.cast::<P>().deref_mut())
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
                // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                let vtable = body
                    .borrow()
                    .project(|body| &raw const (*body).vtable)
                    .copied();

                let mut state = None::<&S>;
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
                    // Safety:
                    // Projection from `DynBody` to `DynBody::state` is safe as the only exception that
                    // the state was erased to Infallible is excluded.
                    let vtable = body
                        .borrow()
                        .project(|body| &raw const (*body).vtable)
                        .copied();
                    let mut state = None::<S>;
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
                // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                let vtable = body
                    .borrow()
                    .project(|body| &raw const (*body).vtable)
                    .copied();
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
                // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                let vtable = body
                    .borrow()
                    .project(|body| &raw const (*body).vtable)
                    .copied();

                let mut state: Option<S> = None;
                let mut context: Option<&'static str> = None;
                let mut payload: Option<P> = None;
                let mut err: Option<E> = None;
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
                    // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                    let vtable = body
                        .borrow()
                        .project(|body| &raw const (*body).vtable)
                        .copied();
                    let mut state_dst = None::<S>;
                    let mut error_dst = None::<ManuallyDrop<Align4Own<DynBody>>>;
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
                context.to_owned().into()
            },
            SelectOwn::Inline(body) => format!("{:?}", body.borrow_value()).into(),
            SelectOwn::Boxed(body) => unsafe {
                // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                let vtable = body
                    .borrow()
                    .project(|body| &raw const (*body).vtable)
                    .copied();
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
                    // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                    let vtable = self
                        .boxed_body
                        .borrow()
                        .project(|body| &raw const (*body).vtable)
                        .copied();

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
        format_debug(
            f,
            self.state(),
            self.context(),
            self.payload(),
            self.source(),
        )
    }
}

impl<S> Display for RawError<S>
where
    S: Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        format_display(
            f,
            self.state(),
            self.context(),
            self.payload(),
            self.source(),
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
/// The concrete types `E`, `P`, `C` are only known at construction time and at
/// the monomorphized vtable function sites. The `RawError` stores the body as
/// `DynBody<S, (), (), ()>`. The `S` can also be erased when the state is
/// extracted.
///
/// # Safety
///
/// The `vtable` pointer must point to a `DynBodyVTable` that was monomorphized
/// for the same `S`, `E`, `P`, `C` as the stored data.
#[repr(C)]
struct DynBody<S = Infallible, E = (), P = (), C = ()>
where
    S: 'static,
    E: 'static,
    P: 'static,
    C: 'static,
{
    vtable: &'static DynBodyVTable, // Note: The vtable must be the first field as the other fields may be erased.
    state: Option<S>,
    source: E,
    payload: P,
    context: C,
}

/// Virtual function table for type-erased operations on [`DynBody`].
///
/// Each function pointer is monomorphized for the concrete `S`, `E`, `P`, `C`.
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
    /// See [DynBody::source].
    source: unsafe fn(Ref<'_, DynBody>) -> Option<&(dyn error::Error + Send + Sync + 'static)>,
    /// See [DynBody::state].
    state: unsafe fn(Ref<'_, DynBody>, TypeId, NonNull<()>),
    /// See [DynBody::payload].
    payload: unsafe fn(Ref<'_, DynBody>) -> Option<&(dyn Display + Send + Sync + 'static)>,
    /// See [DynBody::context].
    context: unsafe fn(Ref<'_, DynBody>) -> Option<&(dyn Display + Send + Sync + 'static)>,
    /// See [DynBody::downcast_source_ref].
    downcast_source_ref: unsafe fn(Ref<'_, DynBody>, TypeId) -> Option<Ref<'_, ()>>,
    /// See [DynBody::downcast_payload_ref].
    downcast_payload_ref: unsafe fn(Ref<'_, DynBody>, TypeId) -> Option<Ref<'_, ()>>,
    /// See [DynBody::downcast_source_mut].
    downcast_source_mut: unsafe fn(Mut<'_, DynBody>, TypeId) -> Option<Mut<'_, ()>>,
    /// See [DynBody::downcast_payload_mut].
    downcast_payload_mut: unsafe fn(Mut<'_, DynBody>, TypeId) -> Option<Mut<'_, ()>>,
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
            source: DynBody::<S, E, P, L>::source,
            state: DynBody::<S, E, P, L>::state,
            payload: DynBody::<S, E, P, L>::payload,
            context: DynBody::<S, E, P, L>::context,
            downcast_source_ref: DynBody::<S, E, P, L>::downcast_source_ref,
            downcast_payload_ref: DynBody::<S, E, P, L>::downcast_payload_ref,
            downcast_source_mut: DynBody::<S, E, P, L>::downcast_source_mut,
            downcast_payload_mut: DynBody::<S, E, P, L>::downcast_payload_mut,
        }
    }
}

impl<S, E, P, C> DynBody<S, E, P, C>
where
    S: Debug + Send + Sync + 'static,
    E: error::Error + Send + Sync + 'static,
    P: Display + Send + Sync + 'static,
    C: Display + Send + Sync + 'static,
{
    /// Drops the boxed body.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to a heap-allocated `DynBody<S, E, P, C>`.
    unsafe fn drop(mut this: ManuallyDrop<Align4Own<DynBody>>) {
        let _ = unsafe { ManuallyDrop::take(&mut this).cast::<Self>().into_boxed() };
    }

    /// Extracts `state` from the boxed body and drops the allocation.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to a heap-allocated `DynBody<S, E, P, C>`.
    /// - `state_dst` must be a valid, aligned, mutable pointer to `Option<S>`.
    unsafe fn into_state(
        mut this: ManuallyDrop<Align4Own<DynBody>>,
        state_ty: TypeId,
        state_dst: NonNull<()>,
    ) {
        let Align4(mut this) =
            *unsafe { ManuallyDrop::take(&mut this).cast::<Self>().into_boxed() };
        if TypeId::of::<S>() == state_ty {
            // Safety: The caller guarantees `state_dst` points to a valid `Option<S>`.
            let dst = unsafe { state_dst.cast::<Option<S>>().as_mut() };
            *dst = this.state.take();
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
        let this = unsafe { ManuallyDrop::take(&mut this).cast::<Self>() };
        let Align4(this) = *this.into_boxed();
        if rtti::is_same_ty::<E, Nae>() {
            None
        } else {
            Some(Box::new(this.source))
        }
    }

    /// Decomposes the boxed body: extracts source and payload into caller-provided
    /// `Option`s (if the `TypeId` matches), and returns the state.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to `DynBody<S, E, P, C>`.
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
        let Align4(mut this) =
            *unsafe { ManuallyDrop::take(&mut this).cast::<Self>().into_boxed() };
        if TypeId::of::<E>() == source_ty {
            // Safety: The caller guarantees `source_dst` points to a valid `Option<E>`.
            let dst = unsafe { source_dst.cast::<Option<E>>().as_mut() };
            dst.replace(this.source);
        }
        if TypeId::of::<P>() == payload_ty {
            // Safety: The caller guarantees `payload_dst` points to a valid `Option<P>`.
            let dst = unsafe { payload_dst.cast::<Option<P>>().as_mut() };
            dst.replace(this.payload);
        }
        if !rtti::is_same_ty::<C, context::Unit>() {
            // Safety: The caller guarantees `context_dst` points to a valid `Option<&'static str>`.
            let dst = unsafe { context_dst.cast::<Option<C>>().as_mut() };
            dst.replace(this.context);
        }
        if TypeId::of::<S>() == state_ty {
            let dst = unsafe { state_dst.cast::<Option<S>>().as_mut() };
            *dst = this.state.take();
        }
    }

    /// Extracts the state from the boxed body. This function guarantees at least one of the `Option` will be filled.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to `DynBody<S, E, P, C>`.
    /// - `state_dst` must be a valid, aligned, mutable pointer to `Option<StateTy>`.
    /// - `error_dst` must be a valid, aligned, mutable pointer to `Option<ManuallyDrop<Align4Own<DynBody>>>`
    unsafe fn extract_state(
        mut this: ManuallyDrop<Align4Own<DynBody>>,
        state_ty: TypeId,
        state_dst: NonNull<()>,
        error_dst: NonNull<()>,
    ) {
        let this = unsafe { ManuallyDrop::take(&mut this).cast::<Self>() };

        if TypeId::of::<S>() == state_ty {
            let dst = unsafe { state_dst.cast::<Option<S>>().as_mut() };
            // Safety: Projecting to `state` is inbound.
            *dst = unsafe {
                this.borrow_mut()
                    .project(|e| &raw mut (*e).state)
                    .deref_mut()
                    .take()
            };
        }

        let has_context = !rtti::is_same_ty::<C, context::Unit>();
        let has_payload = !rtti::is_same_ty::<P, payload::Empty>();
        let has_source = !rtti::is_same_ty::<E, Nae>();

        match (has_context, has_payload, has_source) {
            (false, false, false) => {
                mem::drop(this.into_boxed());
            }
            _ => {
                let error_dst = unsafe {
                    error_dst
                        .cast::<Option<ManuallyDrop<Align4Own<DynBody>>>>()
                        .as_mut()
                };
                // Safety:
                // Erase the remaining fields to ZSTs is dangerous as the destructor assumes a correct layout.
                // So we wrap it in `ManuallyDrop` to prevent the destructor from being automatically called.
                error_dst.replace(ManuallyDrop::new(unsafe { this.cast::<DynBody>() }));
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
        let this = unsafe { ManuallyDrop::take(&mut this).cast::<Self>().into_boxed() };
        let this_raw = Box::into_raw(this);
        // # Safety
        //
        // The `Ailgn4` wrapper is `repr(C)` and has only one field, so the pointer cast is valid.
        unsafe { Box::from_raw(this_raw as *mut DynBody<S, E, P, C>) }
    }

    /// Returns a reference to the source error.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, P, C>`.
    /// - The `source` field must be initialized.
    unsafe fn source(
        this: Ref<'_, DynBody>,
    ) -> Option<&(dyn error::Error + Send + Sync + 'static)> {
        let this = unsafe { this.cast::<Self>() };
        let source = unsafe { this.project(|body| &raw const (*body).source) };
        let err = source.deref();

        if rtti::is_same_ty::<E, Nae>() {
            None
        } else {
            Some(err as &(dyn error::Error + Send + Sync + 'static))
        }
    }

    /// Returns a reference to the state.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, P, C>`.
    /// - The `source` field must be initialized.
    unsafe fn state(this: Ref<'_, DynBody>, state_ty: TypeId, state_dst: NonNull<()>) {
        let this = unsafe { this.cast::<Self>() };

        if TypeId::of::<S>() == state_ty {
            let dst = unsafe { state_dst.cast::<Option<&S>>().as_mut() };
            let state = unsafe { this.project(|body| &raw const (*body).state) }.deref();

            if let Some(state) = state {
                dst.replace(state);
            }
        }
    }

    /// Returns a reference to the displayable payload.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, P, C>`.
    /// - The `store.payload` field must be initialized.
    unsafe fn payload(this: Ref<'_, DynBody>) -> Option<&(dyn Display + Send + Sync + 'static)> {
        let this = unsafe { this.cast::<Self>() };
        let payload = unsafe { this.project(|body| &raw const (*body).payload) };

        if rtti::is_same_ty::<P, payload::Empty>() {
            None
        } else {
            Some(payload.deref() as &(dyn Display + Send + Sync + 'static))
        }
    }

    /// Returns a reference to the displayable context.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, P, C>`.
    unsafe fn context(this: Ref<'_, DynBody>) -> Option<&(dyn Display + Send + Sync + 'static)> {
        let this = unsafe { this.cast::<Self>() };
        let context = unsafe { this.project(|body| &raw const (*body).context) };

        if rtti::is_same_ty::<C, context::Unit>() {
            None
        } else {
            Some(context.deref() as &(dyn Display + Send + Sync + 'static))
        }
    }

    /// Attempts to downcast the source field to the requested type `E`.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, P, C>`.
    unsafe fn downcast_source_ref(this: Ref<'_, DynBody>, ty: TypeId) -> Option<Ref<'_, ()>> {
        let this = unsafe { this.cast::<Self>() };

        if ty == TypeId::of::<E>() {
            Some(unsafe { this.project(|body| &raw const (*body).source).cast::<()>() })
        } else {
            None
        }
    }

    /// Attempts to downcast the payload field to the requested type `P`.
    ///
    /// # Safety
    ///
    /// Same as [`downcast_source_ref`](DynBody::downcast_source_ref) for the payload field.
    unsafe fn downcast_payload_ref(this: Ref<'_, DynBody>, _ty: TypeId) -> Option<Ref<'_, ()>> {
        let this = unsafe { this.cast::<Self>() };
        if _ty == TypeId::of::<P>() {
            Some(unsafe { this.project(|body| &raw const (*body).payload).cast::<()>() })
        } else {
            None
        }
    }

    /// Attempts to downcast the source field to the requested type `E` (mutable).
    ///
    /// # Safety
    ///
    /// Same as [`downcast_source_ref`](DynBody::downcast_source_ref) with mutable access.
    unsafe fn downcast_source_mut(this: Mut<'_, DynBody>, _ty: TypeId) -> Option<Mut<'_, ()>> {
        let this = unsafe { this.cast::<Self>() };

        if _ty == TypeId::of::<E>() {
            Some(unsafe { this.project(|body| &raw mut (*body).source).cast::<()>() })
        } else {
            None
        }
    }

    /// Attempts to downcast the payload field to the requested type `P` (mutable).
    ///
    /// # Safety
    ///
    /// Same as [`downcast_payload_ref`](DynBody::downcast_payload_ref) with mutable access.
    unsafe fn downcast_payload_mut(this: Mut<'_, DynBody>, _ty: TypeId) -> Option<Mut<'_, ()>> {
        let this = unsafe { this.cast::<Self>() };

        if _ty == TypeId::of::<P>() {
            Some(unsafe { this.project(|body| &raw mut (*body).payload).cast::<()>() })
        } else {
            None
        }
    }
}

impl<S, E, P, C> fmt::Debug for DynBody<S, E, P, C>
where
    S: Debug + Send + Sync + 'static,
    E: error::Error + Send + Sync + 'static,
    P: Display + Send + Sync + 'static,
    C: Display + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        format_debug(
            f,
            self.state.as_ref(),
            (!rtti::is_same_ty::<C, context::Unit>()).then_some(&self.context),
            (!rtti::is_same_ty::<P, payload::Empty>()).then_some(&self.payload),
            Some(&self.source),
        )
    }
}

impl<S, E, P, C> fmt::Display for DynBody<S, E, P, C>
where
    S: Debug + 'static,
    E: error::Error + Send + Sync + 'static,
    P: Display + Send + Sync + 'static,
    C: Display + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        format_display(
            f,
            self.state.as_ref(),
            (!rtti::is_same_ty::<C, context::Unit>()).then_some(&self.context),
            (!rtti::is_same_ty::<P, payload::Empty>()).then_some(&self.payload),
            Some(&self.source),
        )
    }
}

impl<S, E, P, C> error::Error for DynBody<S, E, P, C>
where
    S: Debug + Send + Sync + 'static,
    E: error::Error + Send + Sync + 'static,
    P: Display + Send + Sync + 'static,
    C: Display + Send + Sync + 'static,
{
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        (!rtti::is_same_ty::<E, Nae>()).then_some(&self.source as _)
    }
}

fn format_debug<S>(
    f: &mut fmt::Formatter<'_>,
    state: Option<&S>,
    context: Option<&(dyn Display + Send + Sync + 'static)>,
    payload: Option<&(dyn Display + Send + Sync + 'static)>,
    source: Option<&(dyn error::Error + Send + Sync + 'static)>,
) -> fmt::Result
where
    S: Debug + 'static,
{
    struct DisplayAsDebug<'a>(pub &'a dyn Display);

    impl<'a> Debug for DisplayAsDebug<'a> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, r#""{}""#, self.0)
        }
    }

    struct DebugSourceChain<'a>(&'a dyn error::Error);

    impl<'a> fmt::Debug for DebugSourceChain<'a> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let mut list = f.debug_list();

            let mut next_source = Some(self.0);
            while let Some(source) = next_source {
                next_source = source.source();

                list.entry(&format!("{source}"));
            }

            list.finish()
        }
    }

    let ds = &mut f.debug_struct("Error");

    if !rtti::is_same_ty::<S, ()>()
        && let Some(state) = state
    {
        ds.field("state", state);
    }

    if let Some(context) = context {
        ds.field("context", &DisplayAsDebug(context));
    }

    if let Some(payload) = payload {
        ds.field("payload", &DisplayAsDebug(payload));
    }

    if let Some(source) = source {
        ds.field("source", &DebugSourceChain(source));
    }

    ds.finish()
}

fn format_display<S>(
    f: &mut fmt::Formatter<'_>,
    state: Option<&S>,
    context: Option<&(dyn Display + Send + Sync + 'static)>,
    payload: Option<&(dyn Display + Send + Sync + 'static)>,
    source: Option<&(dyn error::Error + Send + Sync + 'static)>,
) -> fmt::Result
where
    S: Debug + 'static,
{
    struct DebugAsDisplay<'a>(pub &'a dyn Debug);

    impl<'a> Display for DebugAsDisplay<'a> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{:?}", self.0)
        }
    }

    let state = state.map(|s| DebugAsDisplay(s));

    if f.alternate() {
        let mut segments = [
            state.as_ref().map(|s| s as &dyn Display),
            context.map(|s| s as _),
            payload.map(|s| s as _),
            source.map(|s| s as _),
        ]
        .into_iter()
        .flatten()
        .peekable();

        while let Some(segment) = segments.next() {
            write!(f, "{:#}", segment)?;

            if segments.peek().is_some() {
                write!(f, ": ")?;
            }
        }

        Ok(())
    } else {
        match (&state, context, payload, source) {
            (None, None, None, None) => unreachable!(),
            (None, None, None, Some(err)) => Display::fmt(err, f),
            _ => {
                let mut segments = [
                    state.as_ref().map(|s| s as &dyn Display),
                    context.map(|s| s as _),
                    payload.map(|s| s as _),
                ]
                .into_iter()
                .flatten()
                .peekable();

                while let Some(segment) = segments.next() {
                    write!(f, "{}", segment)?;

                    if segments.peek().is_some() {
                        write!(f, ": ")?;
                    }
                }

                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::{Blank, Literal},
        nae::Nae,
        payload,
    };
    use std::{
        convert::Infallible,
        error,
        fmt::{self, Display},
        mem,
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

    #[test]
    fn kind_discriminates_const() {
        let err = RawError::new_const::<TestContext>();
        assert_eq!(err.kind(), RawError::<()>::KIND_CONST);
    }

    #[test]
    fn kind_discriminates_inline() {
        let err = RawError::new_inline(4216u16).unwrap();
        assert_eq!(err.kind(), RawError::<u16>::KIND_INLINE);
    }

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
        let err = RawError::new_inline(42u16).unwrap();
        assert!(matches!(err.state(), Some(42)));
    }

    #[test]
    fn inline_variant_into_state() {
        let err = RawError::new_inline(42u16).unwrap();
        assert!(matches!(err.state(), Some(42)));
    }

    #[test]
    fn inline_variant_context_is_none() {
        let err = RawError::new_inline(42u16).unwrap();
        assert!(err.context().is_none());
    }

    #[test]
    fn inline_variant_source_is_none() {
        let err = RawError::new_inline(42u16).unwrap();
        assert!(err.source().is_none());
    }

    #[test]
    fn inline_variant_payload_is_none() {
        let err = RawError::new_inline(42u16).unwrap();
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
        let downcasted = err.downcast_source_ref::<String>();
        assert!(downcasted.is_none());
    }

    #[test]
    fn boxed_variant_downcast_source_mut() {
        let mut err = RawError::new_boxed::<_, _, Blank>(
            None::<Infallible>,
            TestError("oops"),
            payload::Empty::new(),
        );
        {
            let downcasted = err.downcast_source_mut::<TestError>();
            assert!(downcasted.is_some());
            downcasted.unwrap().0 = "fixed";
        }
        let downcasted = err.downcast_source_ref::<TestError>();
        assert_eq!(downcasted.unwrap().0, "fixed");
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
        assert!(payload.is_some());
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
        let err = RawError::new_inline(42u16).unwrap();
        let (state, _, payload, source) = err.into_parts::<TestPayload, TestError>();
        assert!(source.is_none());
        assert!(payload.is_none());
        assert!(matches!(state, Some(42)));
    }

    // --- Drop safety (checked via sanitizer / basic leak check) ---

    /// Allocate a boxed variant and ensure it can be observed to drop.
    #[test]
    fn boxed_variant_drop_does_not_leak() {
        use std::sync::atomic::{AtomicBool, Ordering};

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
            assert!(matches!(err.extract_state(), Err(err) if err.to_string() == "oops"));
        }
        {
            let err = RawError::new_boxed::<_, _, Blank>(Some(42i32), Nae::new(), format!("oops"));
            assert!(
                matches!(err.extract_state(), Ok((42i32, Some(err))) if err.to_string() == "oops")
            );
        }
    }
}
