use crate::{
    context::{self, Literal},
    match_else,
    nae::Nae,
    payload,
    ptr::{Align4, Align4Own, Align4PtrCompat, Align4Ref, Metadata, Mut, Ref},
};
use std::{
    self,
    any::TypeId,
    error,
    fmt::{self, Debug, Display},
    mem::{self, ManuallyDrop, MaybeUninit},
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
    boxed_body: ManuallyDrop<Align4Own<DynBody<S>>>,
    inline_body: ManuallyDrop<Align4PtrCompat<S>>,
}

enum SelectRef<'a, S>
where
    S: 'static,
{
    Const(&'a Align4Ref<'static, ConstBody>),
    Boxed(&'a Align4Own<DynBody<S>>),
    Inline(&'a Align4PtrCompat<S>),
}

enum SelectMut<'a, S>
where
    S: 'static,
{
    Const(&'a mut Align4Ref<'static, ConstBody>),
    Boxed(&'a mut Align4Own<DynBody<S>>),
    Inline(&'a mut Align4PtrCompat<S>),
}

enum SelectOwn<S>
where
    S: 'static,
{
    Const(Align4Ref<'static, ConstBody>),
    Boxed(Align4Own<DynBody<S>>),
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

impl RawError<()> {
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

    pub fn new_inline_or_boxed(state: S) -> Self {
        let Err(state) = match_else!(Self::new_inline(state),
            Ok(this) => return this,
        );
        Self::new_boxed::<Nae, payload::Empty, context::Blank>(state, Nae, payload::Empty)
    }

    /// Constructs a boxed-variant [`RawError`] containing source, payload, and context.
    ///
    /// The source, payload, and context are packed into a single heap allocation
    /// alongside a vtable for type-erased access.
    pub fn new_boxed<E, P, L>(state: S, source: E, payload: P) -> Self
    where
        E: error::Error + Send + Sync + 'static,
        P: Display + Send + Sync + 'static,
        L: Literal + context::Context + ?Sized,
    {
        // # Safety
        //
        // The `Align4Own` pointer is cast to `DynBody<S, (), ()>` for uniform storage.
        // This is valid because all monomorphizations of `DynBody<S, E, P, L::Repr>` share
        // the same `S` prefix and vtable pointer, and the concrete `E`, `P`, `C` are erased.
        // The cast only changes the type parameter defaults — it does not violate the layout
        // because `()` is a ZST.
        let ptr = unsafe {
            Align4Own::from_boxed(
                Box::new(Align4(DynBody::<S, E, P, L::Repr> {
                    state,
                    vtable: &const { DynBodyVTable::new::<E, P, L::Repr>() },
                    source,
                    store: Store {
                        payload,
                        context: L::new_context(),
                    },
                })),
                Self::KIND_BOXED,
            )
            .cast::<DynBody<S>>()
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
    pub fn state(&self) -> &S {
        match self.select_ref() {
            SelectRef::Const(_body) => unsafe {
                // Safety: The type is confirmed to be `()`.
                // The dangling pointer is never read; only its address is materialized.
                assert_eq!(TypeId::of::<S>(), TypeId::of::<()>());
                &*(&() as *const _ as *const S)
            },
            SelectRef::Inline(_body) => unsafe {
                // Safety: Access `InlineBody::value` is safe.
                &self.inline_body.borrow_value()
            },
            SelectRef::Boxed(body) => unsafe {
                // Safety: Projection from `DynBody` to `DynBody::state` is safe.
                body.borrow()
                    .project(|body| &raw const (*body).state)
                    .deref()
            },
        }
    }

    /// Consumes `self` and returns the stored state.
    pub fn into_state(self) -> S {
        match self.select_own() {
            SelectOwn::Const(_body) => unsafe {
                // Safety: The type is confirmed to be `()`.
                // `MaybeUninit::uninit().assume_init()` is valid for ZSTs.
                assert_eq!(TypeId::of::<S>(), TypeId::of::<()>());
                #[allow(clippy::uninit_assumed_init)]
                MaybeUninit::<S>::uninit().assume_init()
            },
            SelectOwn::Inline(body) => body.into_value(),
            SelectOwn::Boxed(body) => unsafe {
                // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                let vtable = body
                    .borrow()
                    .project(|body| &raw const (*body).vtable)
                    .copied();
                (vtable.into_state)(body)
            },
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
    /// Returns `(None, None, state)` if the types do not match or no source/payload exists.
    pub fn into_parts<E, P>(self) -> (Option<E>, Option<P>, S)
    where
        E: 'static,
        P: 'static,
    {
        match self.select_own() {
            SelectOwn::Const(_body) => (None, None, unsafe {
                // Safety: The type of state must be `()` when the kind is `Const`. `
                assert_eq!(TypeId::of::<S>(), TypeId::of::<()>());
                #[allow(clippy::uninit_assumed_init)]
                MaybeUninit::<S>::uninit().assume_init()
            }),
            SelectOwn::Inline(body) => (None, None, body.into_value()),
            SelectOwn::Boxed(body) => unsafe {
                // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                let vtable = body
                    .borrow()
                    .project(|body| &raw const (*body).vtable)
                    .copied();
                let mut err: Option<E> = None;
                let mut payload: Option<P> = None;
                let state = (vtable.into_parts)(
                    body,
                    TypeId::of::<E>(),
                    // Safety: `&raw mut err` is a valid mutable pointer to a local
                    // on the stack. It remains valid for the duration of the function call.
                    NonNull::new_unchecked(&raw mut err as *mut ()),
                    TypeId::of::<P>(),
                    // Safety: Same as above for `payload`.
                    NonNull::new_unchecked(&raw mut payload as *mut ()),
                );

                (err, payload, state)
            },
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
                // Safety: Projection from `DynBody` to `DynBody::vtable` is safe.
                let vtable = self
                    .boxed_body
                    .borrow()
                    .project(|body| &raw const (*body).vtable)
                    .copied();

                // Safety: The variant hasn't been moved out, as `self` remains owned and not forgotten.
                (vtable.into_state)(ManuallyDrop::take(&mut self.boxed_body));
            },
            _ => unreachable!(),
        }
    }
}

impl<S> Debug for RawError<S>
where
    S: Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = format_args!("{:?}", self.state());
        let mut segments = [
            self.context().map(|s| s as &dyn Display),
            (TypeId::of::<S>() != TypeId::of::<()>()).then_some(&state as _),
            self.payload().map(|s| s as _),
            self.source().map(|s| s as _),
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

        writeln!(f)?;

        writeln!(f, "Caused by: ")?;
        for err in self.chain() {
            write!(f, "  ")?;
            writeln!(f, "{}", err)?;
        }

        Ok(())
    }
}

impl<S> Display for RawError<S>
where
    S: Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = format_args!("{:?}", self.state());
        let mut segments = [
            self.context().map(|s| s as &dyn Display),
            (TypeId::of::<S>() != TypeId::of::<()>()).then_some(&state as _),
            self.payload().map(|s| s as _),
            self.source().map(|s| s as _),
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
/// `DynBody<S, (), ()>`.
///
/// # Safety
///
/// - The `vtable` pointer must point to a `DynBodyVTable` that was monomorphized
///   for the same `S`, `E`, `P`, `C` as the stored data.
#[repr(C)]
struct DynBody<S, E = (), P = (), C = ()>
where
    S: 'static,
    E: 'static,
    P: 'static,
    C: 'static,
{
    state: S,
    vtable: &'static DynBodyVTable<S>,
    source: E,
    store: Store<P, C>,
}

/// Container for the payload and context inside a [`DynBody`].
#[repr(C)]
struct Store<P, L>
where
    P: 'static,
    L: 'static,
{
    payload: P,
    context: L,
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
struct DynBodyVTable<S>
where
    S: 'static,
{
    into_state: unsafe fn(Align4Own<DynBody<S>>) -> S,
    into_source:
        unsafe fn(Align4Own<DynBody<S>>) -> Option<Box<dyn error::Error + Send + Sync + 'static>>,
    into_parts: unsafe fn(Align4Own<DynBody<S>>, TypeId, NonNull<()>, TypeId, NonNull<()>) -> S,
    source: unsafe fn(Ref<'_, DynBody<S>>) -> Option<&(dyn error::Error + Send + Sync + 'static)>,
    payload: unsafe fn(Ref<'_, DynBody<S>>) -> Option<&(dyn Display + Send + Sync + 'static)>,
    context: unsafe fn(Ref<'_, DynBody<S>>) -> Option<&(dyn Display + Send + Sync + 'static)>,
    downcast_source_ref: unsafe fn(Ref<'_, DynBody<S>>, TypeId) -> Option<Ref<'_, ()>>,
    downcast_payload_ref: unsafe fn(Ref<'_, DynBody<S>>, TypeId) -> Option<Ref<'_, ()>>,
    downcast_source_mut: unsafe fn(Mut<'_, DynBody<S>>, TypeId) -> Option<Mut<'_, ()>>,
    downcast_payload_mut: unsafe fn(Mut<'_, DynBody<S>>, TypeId) -> Option<Mut<'_, ()>>,
}

impl<S> DynBodyVTable<S> {
    const fn new<E, P, L>() -> Self
    where
        E: error::Error + Send + Sync + 'static,
        L: Display + Send + Sync + 'static,
        P: Display + Send + Sync + 'static,
    {
        DynBodyVTable {
            into_state: DynBody::<S, E, P, L>::into_state,
            into_source: DynBody::<S, E, P, L>::into_source,
            into_parts: DynBody::<S, E, P, L>::into_parts,
            source: DynBody::<S, E, P, L>::source,
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
    S: 'static,
    E: error::Error + Send + Sync + 'static,
    P: Display + Send + Sync + 'static,
    C: Display + Send + Sync + 'static,
{
    /// Extracts `state` from the boxed body and drops the allocation.
    ///
    /// # Safety
    ///
    /// - `this` must be a valid `Align4Own` pointing to a heap-allocated `DynBody<S, E, P, C>`.
    /// - The cast to `Self` is valid because the vtable was set to point to this
    ///   monomorphization of `into_state`.
    unsafe fn into_state(this: Align4Own<DynBody<S>>) -> S {
        let this = *unsafe { this.cast::<Self>().into_boxed() };
        this.0.state
    }

    /// Extracts the source error as a trait object from the boxed body.
    ///
    /// # Safety
    ///
    /// Same as [`into_state`](DynBody::into_state). Returns `None` if `E` is [`Nae`].
    unsafe fn into_source(
        this: Align4Own<DynBody<S>>,
    ) -> Option<Box<dyn error::Error + Send + Sync + 'static>> {
        let this = unsafe { this.cast::<Self>() };
        let Align4(this) = *unsafe { this.into_boxed() };
        if TypeId::of::<E>() == TypeId::of::<Nae>() {
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
    /// - `source_dst` and `payload_dst` must be valid, aligned, mutable pointers
    ///   to `Option<E>` and `Option<P>` respectively, or to types with compatible layout.
    unsafe fn into_parts(
        this: Align4Own<DynBody<S>>,
        source_ty: TypeId,
        source_dst: NonNull<()>,
        payload_ty: TypeId,
        payload_dst: NonNull<()>,
    ) -> S {
        let Align4(this) = *unsafe { this.cast::<Self>().into_boxed() };
        if TypeId::of::<E>() == source_ty {
            // Safety: The caller guarantees `source_dst` points to a valid `Option<E>`.
            let dst = unsafe { source_dst.cast::<Option<E>>().as_mut() };
            dst.replace(this.source);
        }
        if TypeId::of::<P>() == payload_ty {
            // Safety: The caller guarantees `payload_dst` points to a valid `Option<P>`.
            let dst = unsafe { payload_dst.cast::<Option<P>>().as_mut() };
            dst.replace(this.store.payload);
        }
        this.state
    }

    /// Returns a reference to the source error.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, P, C>`.
    /// - The `source` field must be initialized.
    unsafe fn source(
        this: Ref<'_, DynBody<S>>,
    ) -> Option<&(dyn error::Error + Send + Sync + 'static)> {
        let this = unsafe { this.cast::<Self>() };
        let source = unsafe { this.project(|body| &raw const (*body).source) };
        let err = source.deref();

        if TypeId::of::<E>() == TypeId::of::<Nae>() {
            None
        } else {
            Some(err as &(dyn error::Error + Send + Sync + 'static))
        }
    }

    /// Returns a reference to the displayable payload.
    ///
    /// # Safety
    ///
    /// - `this` must point to a valid `DynBody<S, E, P, C>`.
    /// - The `store.payload` field must be initialized.
    unsafe fn payload(this: Ref<'_, DynBody<S>>) -> Option<&(dyn Display + Send + Sync + 'static)> {
        let this = unsafe { this.cast::<Self>() };
        let payload = unsafe { this.project(|body| &raw const (*body).store.payload) };

        if TypeId::of::<P>() == TypeId::of::<payload::Empty>() {
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
    unsafe fn context(this: Ref<'_, DynBody<S>>) -> Option<&(dyn Display + Send + Sync + 'static)> {
        let this = unsafe { this.cast::<Self>() };
        let context = unsafe { this.project(|body| &raw const (*body).store.context) };

        if TypeId::of::<C>() == TypeId::of::<context::Unit>() {
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
    unsafe fn downcast_source_ref(this: Ref<'_, DynBody<S>>, ty: TypeId) -> Option<Ref<'_, ()>> {
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
    unsafe fn downcast_payload_ref(this: Ref<'_, DynBody<S>>, _ty: TypeId) -> Option<Ref<'_, ()>> {
        let this = unsafe { this.cast::<Self>() };
        if _ty == TypeId::of::<P>() {
            Some(unsafe {
                this.project(|body| &raw const (*body).store.payload)
                    .cast::<()>()
            })
        } else {
            None
        }
    }

    /// Attempts to downcast the source field to the requested type `E` (mutable).
    ///
    /// # Safety
    ///
    /// Same as [`downcast_source_ref`](DynBody::downcast_source_ref) with mutable access.
    unsafe fn downcast_source_mut(this: Mut<'_, DynBody<S>>, _ty: TypeId) -> Option<Mut<'_, ()>> {
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
    unsafe fn downcast_payload_mut(this: Mut<'_, DynBody<S>>, _ty: TypeId) -> Option<Mut<'_, ()>> {
        let this = unsafe { this.cast::<Self>() };

        if _ty == TypeId::of::<P>() {
            Some(unsafe {
                this.project(|body| &raw mut (*body).store.payload)
                    .cast::<()>()
            })
        } else {
            None
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
        let err = RawError::<()>::new_const::<TestContext>();
        assert_eq!(err.kind(), RawError::<()>::KIND_CONST);
    }

    #[test]
    fn kind_discriminates_inline() {
        let err = RawError::new_inline(4216).unwrap();
        assert_eq!(err.kind(), RawError::<u32>::KIND_INLINE);
    }

    #[test]
    fn kind_discriminates_boxed() {
        let err = RawError::new_boxed::<_, _, Blank>((), TestError("oops"), payload::Empty);
        assert_eq!(err.kind(), RawError::<()>::KIND_BOXED);
    }

    // --- Const variant ---

    #[test]
    fn const_variant_context() {
        let err = RawError::<()>::new_const::<TestContext>();
        let ctx = err.context();
        assert!(ctx.is_some());
        assert_eq!(ctx.unwrap().to_string(), "test context");
    }

    #[test]
    fn const_variant_source_is_none() {
        let err = RawError::<()>::new_const::<TestContext>();
        assert!(err.source().is_none());
    }

    #[test]
    fn const_variant_payload_is_none() {
        let err = RawError::<()>::new_const::<TestContext>();
        assert!(err.payload().is_none());
    }

    #[test]
    fn const_variant_into_state() {
        let err = RawError::<()>::new_const::<TestContext>();
        let state = err.into_state();
        // `()` is the only valid state for const variant
        assert_eq!(state, ());
    }

    // --- Inline variant ---

    #[test]
    fn inline_variant_state() {
        let err = RawError::new_inline(42u16).unwrap();
        assert_eq!(*err.state(), 42);
    }

    #[test]
    fn inline_variant_into_state() {
        let err = RawError::new_inline(42u16).unwrap();
        assert_eq!(err.into_state(), 42);
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
        let err = RawError::new_boxed::<_, _, Blank>((), TestError("oops"), payload::Empty);
        let src = err.source();
        assert!(src.is_some());
        assert_eq!(src.unwrap().to_string(), "oops");
    }

    #[test]
    fn boxed_variant_downcast_source() {
        let err = RawError::new_boxed::<_, _, Blank>((), TestError("oops"), payload::Empty);
        let downcasted = err.downcast_source_ref::<TestError>();
        assert!(downcasted.is_some());
        assert_eq!(downcasted.unwrap().0, "oops");
    }

    #[test]
    fn boxed_variant_downcast_source_wrong_type() {
        let err = RawError::new_boxed::<_, _, Blank>((), TestError("oops"), payload::Empty);
        let downcasted = err.downcast_source_ref::<String>();
        assert!(downcasted.is_none());
    }

    #[test]
    fn boxed_variant_downcast_source_mut() {
        let mut err = RawError::new_boxed::<_, _, Blank>((), TestError("oops"), payload::Empty);
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
        let err = RawError::new_boxed::<_, _, Blank>((), TestError("oops"), TestPayload(42));
        let pl = err.payload();
        assert!(pl.is_some());
        assert_eq!(pl.unwrap().to_string(), "payload(42)");
    }

    #[test]
    fn boxed_variant_downcast_payload() {
        let err = RawError::new_boxed::<_, _, Blank>((), TestError("oops"), TestPayload(42));
        let downcasted = err.downcast_payload_ref::<TestPayload>();
        assert!(downcasted.is_some());
        assert_eq!(downcasted.unwrap().0, 42);
    }

    #[test]
    fn boxed_variant_context() {
        let err = RawError::new_boxed::<_, _, TestContext>((), TestError("oops"), payload::Empty);
        let ctx = err.context();
        assert!(ctx.is_some());
        assert_eq!(ctx.unwrap().to_string(), "test context");
    }

    #[test]
    fn boxed_variant_nae_source_is_none() {
        // When source is `Nae`, `.source()` should return `None`.
        let err = RawError::new_boxed::<_, _, Blank>(42u32, Nae, payload::Empty);
        assert!(err.source().is_none());
        assert_eq!(*err.state(), 42);
    }

    #[test]
    fn boxed_variant_empty_payload_is_none() {
        let err = RawError::new_boxed::<_, _, Blank>((), TestError("oops"), payload::Empty);
        assert!(err.payload().is_none());
    }

    // --- into_source ---

    #[test]
    fn boxed_variant_into_source_returns_boxed_error() {
        let err = RawError::new_boxed::<_, _, Blank>((), TestError("oops"), payload::Empty);
        let src = err.into_source();
        assert!(src.is_some());
        assert_eq!(src.unwrap().to_string(), "oops");
    }

    #[test]
    fn boxed_variant_into_source_nae_returns_none() {
        let err = RawError::new_boxed::<_, _, Blank>((), Nae, payload::Empty);
        assert!(err.into_source().is_none());
    }

    // --- into_parts ---

    #[test]
    fn boxed_variant_into_parts_matches_types() {
        let err =
            RawError::new_boxed::<_, _, TestContext>("state", TestError("oops"), TestPayload(99));
        let (source, payload, state) = err.into_parts::<TestError, TestPayload>();
        assert_eq!(state, "state");
        assert!(source.is_some());
        assert_eq!(source.unwrap().0, "oops");
        assert!(payload.is_some());
        assert_eq!(payload.unwrap().0, 99);
    }

    #[test]
    fn boxed_variant_into_parts_wrong_source_type() {
        let err = RawError::new_boxed::<_, _, Blank>((), TestError("oops"), payload::Empty);
        let (source, payload, _) = err.into_parts::<String, payload::Empty>();
        assert!(source.is_none());
        assert!(payload.is_some());
    }

    #[test]
    fn const_variant_into_parts() {
        let err = RawError::<()>::new_const::<TestContext>();
        let (source, payload, state) = err.into_parts::<TestError, TestPayload>();
        assert!(source.is_none());
        assert!(payload.is_none());
        assert_eq!(state, ());
    }

    #[test]
    fn inline_variant_into_parts() {
        let err = RawError::new_inline(42u16).unwrap();
        let (source, payload, state) = err.into_parts::<TestError, TestPayload>();
        assert!(source.is_none());
        assert!(payload.is_none());
        assert_eq!(state, 42);
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
            let _err = RawError::new_boxed::<_, _, Blank>((), DropWatch, payload::Empty);
        } // drop here
        assert!(DROPPED.load(Ordering::SeqCst));
    }

    // --- State round-trip for const variant (S = ()) ---

    #[test]
    fn const_variant_state_is_unit() {
        let err = RawError::<()>::new_const::<TestContext>();
        // state() should return a valid &() for const variant
        let s: &() = err.state();
        assert_eq!(*s, ());
    }

    // --- Size checks ---

    #[test]
    fn raw_error_size() {
        // RawError<()> should be 1 usize on common platforms
        assert_eq!(mem::size_of::<RawError<()>>(), mem::size_of::<usize>());
    }
}
