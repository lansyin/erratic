mod align4;
mod backtrace;
mod erased;
mod exclude;
mod ptr;
mod source;
mod state;

use alloc::{boxed::Box, format};
use core::{
    any::{self, Any, TypeId},
    convert::Infallible,
    error,
    fmt::{self, Debug, Display},
    mem::{self, ManuallyDrop, MaybeUninit},
    ops::{Deref, DerefMut},
};

use crate::{
    context::{Context, Printable, Value},
    match_else,
    raw::{
        align4::{Align4, Align4Own, Align4PtrCompat, Align4Ref, Metadata},
        ptr::{Mut, Ref},
        source::{BoxedSource, ErasedSource, NoSource, Source, WithBacktraceSource},
        state::{Discriminator, Metastate, Ministate, StatefulState, StatefulStateMut, Store},
    },
    render, rtti,
};
use backtrace::WithBacktrace;

pub(crate) use erased::ErasedRawError;

/// Three-variant error storage.
///
/// # Safety Invariants
///
/// The least significant 2 bits of its first byte must contain a [`Metadata`].
/// The metadata indicates the discriminant of the union and cannot be changed.
#[repr(C)]
pub(crate) union RawError<S = Infallible>
where
    S: 'static,
{
    /// # Safety Invariants
    ///
    /// The least significant 2 bits of the first byte must be `00`.
    const_body: ManuallyDrop<Align4Ref<'static, ConstBody>>,
    /// # Safety Invariants
    ///
    /// The least significant 2 bits of the first byte must be `01`.
    boxed_body: ManuallyDrop<ErasedDynBody>,
    /// # Safety Invariants
    ///
    /// The least significant 2 bits of the first byte must be `10`.
    inline_body: ManuallyDrop<Align4PtrCompat<S>>,
}

const _: () = {
    assert!(mem::size_of::<RawError<()>>() == mem::size_of::<usize>());
    assert!(mem::size_of::<RawError<u128>>() == mem::size_of::<usize>());
};

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

    /// Reads the discriminant of the union.
    ///
    /// # Safety Invariants
    ///
    /// The returned [`Metadata`] indicates the correct variant of `self`
    fn kind(&self) -> Metadata {
        // Safety: By the invariants of [`Self`], the least significant 2 bits of the first byte
        // contain a [`Metadata`].
        unsafe { Metadata((&raw const (*self) as *const u8).read() & Metadata::MASK) }
    }

    /// Selects a shared reference to the active union variant.
    fn select_ref(&self) -> SelectRef<'_, S> {
        // Safety: `RawError::kind` returns the correct discriminant of `self`.
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
        // Safety: `RawError::kind` returns the correct discriminant of `self`.
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

        // Safety: `RawError::kind` returns the correct discriminant of `self`.
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
        // Note: Explicitly check the fallback context first as we CANNOT return early in the const block.
        if !matches!(C::VALUE, Value::Literal(_)) {
            return None;
        }
        // Note: Relies on const promotion to produce a new constant.
        let body: &'static Align4<ConstBody> = &const {
            let literal = match C::VALUE {
                Value::Literal(lit) => lit,
                Value::None | Value::Lazy(_) => "", // Note: This branch is never taken; it only exists to keep rustc happy.
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
    fn try_new_inline(state: S) -> Result<Self, S>
    where
        S: Debug + Send + Sync + 'static,
    {
        Ok(Self {
            inline_body: ManuallyDrop::new(Align4PtrCompat::new(Self::KIND_INLINE, state)?),
        })
    }

    /// Checks if the [`RawError`] contains only a source.
    fn is_source_only(&self) -> bool {
        match self.select_ref() {
            SelectRef::Const(_) | SelectRef::Inline(_) => false,
            SelectRef::Boxed(body) => {
                let vt = body.vtable();
                (vt.is_source_only)(body.borrow())
            }
        }
    }

    fn new_boxed<E, C>(state: Option<S>, source: E, context: C) -> Self
    where
        S: Debug + Send + Sync + 'static,
        E: Source + Send + Sync + 'static,
        C: Debug + Display + Send + Sync + 'static,
    {
        /// # Safety
        ///
        /// `metastate` must be the correct state of `state`.
        unsafe fn staged_0_build_state<S, St, E, C>(
            metastate: Metastate,
            state: St,
            source: E,
            context: C,
        ) -> RawError<S>
        where
            St: Store,
            E: Source + Send + Sync + 'static,
            C: Debug + Display + Send + Sync + 'static,
        {
            let mut vtable = Align4Ref::new(
                &const { Align4(DynBodyVTable::new::<St, E, C>()) },
                Metadata::_0,
            );
            unsafe {
                // Safety: By the invariants of `staged_0_build_state`, `metastate` is the correct state of `state`.
                Discriminator::set(&mut vtable, metastate);
            }

            RawError::<S> {
                boxed_body: ManuallyDrop::new(ErasedDynBody::from_typed(Align4Own::from_boxed(
                    Box::new(Align4(DynBody::<St, E, C> {
                        vtable,
                        state,
                        source,
                        context: exclude::Exclude::new(context),
                    })),
                    RawError::<S>::KIND_BOXED,
                ))),
            }
        }

        let Some(state) = state else {
            return unsafe {
                // Safety: `MaybeUninit::<Infallible>` always has a metastate of `Empty`.
                staged_0_build_state(
                    Metastate::Empty,
                    MaybeUninit::<Infallible>::uninit(),
                    source,
                    context,
                )
            };
        };
        let Err(state) = match_else!(Ministate::try_new(state), Ok(ministate) => {
            return unsafe {
                // Safety: A fresh `ministate` has a metastate of `Present`.
                staged_0_build_state(Metastate::Present, ManuallyDrop::new(ministate), source, context)
            };
        });

        unsafe {
            // Safety: An initialized `MaybeUninit` has a metastate of `Present`.
            staged_0_build_state(Metastate::Present, MaybeUninit::new(state), source, context)
        }
    }

    fn new<E, C>(state: Option<S>, source: Option<E>, context: C) -> Self
    where
        S: Debug + Send + Sync + 'static,
        E: Source + Send + Sync + 'static,
        C: Context,
    {
        fn stage_0_select_context<S, E, C>(state: Option<S>, source: E, context: C) -> RawError<S>
        where
            S: Debug + Send + Sync + 'static,
            E: Source + Send + Sync + 'static,
            C: Context,
        {
            match context.select() {
                Ok(context) => stage_1_dedup_self(state, source, context),
                Err(context) => stage_1_dedup_self(state, source, context),
            }
        }

        fn stage_1_dedup_self<S, E, C>(state: Option<S>, source: E, context: C) -> RawError<S>
        where
            S: Debug + Send + Sync + 'static,
            E: Source + Send + Sync + 'static,
            C: Context<Alt = Infallible>,
        {
            let has_state = state.is_some();
            let has_context = !matches!(C::VALUE, Value::None);

            if has_state || has_context {
                return stage_2_attach_backtrace_or_eliminate_alloc(state, source, context);
            }

            let Err(source) = match_else!(rtti::concretize::<_, ErasedSource>(source), Ok(ErasedSource(erased)) => {
                let Err(erased) = match_else!(erased.try_into_stateless(), Ok(err) => {
                    // Note: Backtrace capture in stage 2 can be skipped here. When capture is active, any in-process
                    // `ErasedRawError` already carries a `WithBacktrace` in its chain, so returning
                    // the erased error directly preserves it.
                    return err.with_phantom_state();
                });
                return stage_2_attach_backtrace_or_eliminate_alloc(state, ErasedSource(erased), context)
            });

            stage_2_attach_backtrace_or_eliminate_alloc(state, source, context)
        }

        fn stage_2_attach_backtrace_or_eliminate_alloc<S, E, C>(
            state: Option<S>,
            source: E,
            context: C,
        ) -> RawError<S>
        where
            S: Debug + Send + Sync + 'static,
            E: Source + Send + Sync + 'static,
            C: Context<Alt = Infallible>,
        {
            match WithBacktrace::try_attach(source) {
                Ok(source) => stage_3_evaluate_context(state, source, context),
                Err(source) => {
                    let has_source = source.error_ref().is_some();
                    let has_context = !matches!(C::VALUE, Value::None);

                    match (state, has_context, has_source) {
                        (Some(state), false, false) => {
                            let Err(state) = match_else!(RawError::try_new_inline(state), Ok(this) => {
                                return this;
                            });
                            stage_3_evaluate_context(Some(state), source, context)
                        }
                        (None, true, false) => match RawError::try_new_const::<C>() {
                            Some(raw) => raw.with_phantom_state(),
                            None => stage_3_evaluate_context(None, source, context),
                        },
                        (state, _, _) => stage_3_evaluate_context(state, source, context),
                    }
                }
            }
        }

        fn stage_3_evaluate_context<S, E, C>(state: Option<S>, source: E, context: C) -> RawError<S>
        where
            S: Debug + Send + Sync + 'static,
            E: Source + Send + Sync + 'static,
            C: Context<Alt = Infallible>,
        {
            match C::VALUE {
                Value::None => RawError::new_boxed(state, source, NoContext),
                Value::Literal(context) => RawError::new_boxed(state, source, context),
                Value::Lazy(f) => RawError::new_boxed(state, source, f(context)),
            }
        }

        let Some(source) = source else {
            return stage_0_select_context(state, NoSource, context);
        };

        stage_0_select_context(state, source, context)
    }

    #[cfg(test)]
    fn get_alloc_fingerprint(&self) -> Option<usize> {
        match self.select_ref() {
            SelectRef::Const(_) | SelectRef::Inline(_) => None,
            SelectRef::Boxed(erased_dyn_body) => {
                Some(erased_dyn_body.borrow().deref() as *const _ as usize)
            }
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
    /// Constructs from a standard error.
    pub fn from_error<E, C>(state: Option<S>, source: Option<E>, context: C) -> Self
    where
        S: Debug + Send + Sync + 'static,
        E: error::Error + Send + Sync + 'static,
        C: Context,
    {
        Self::new(state, source, context)
    }

    /// Constructs from an erased error.
    pub fn from_erased<C>(state: Option<S>, source: Option<ErasedRawError>, context: C) -> Self
    where
        S: Debug + Send + Sync + 'static,
        C: Context,
    {
        Self::new(state, source.map(ErasedSource), context)
    }

    /// Constructs from a boxed error.
    pub fn from_boxed<C>(
        state: Option<S>,
        source: Box<dyn error::Error + Send + Sync + 'static>,
        context: C,
    ) -> Self
    where
        S: Debug + Send + Sync + 'static,
        C: Context,
    {
        Self::new(state, Some(BoxedSource(source)), context)
    }

    /// Returns a reference to the displayable context.
    pub fn context(&self) -> Option<&(dyn Printable + Send + Sync + 'static)> {
        match self.select_ref() {
            SelectRef::Const(body) => Some(&body.borrow().deref().context),
            SelectRef::Boxed(body) => (body.vtable().context)(body.borrow()).map(|v| v as _),
            SelectRef::Inline(_body) => None,
        }
    }

    /// Returns a reference to the wrapped source error, if present.
    pub fn source(&self) -> Option<&(dyn error::Error + Send + Sync + 'static)> {
        match self.select_ref() {
            SelectRef::Const(_body) => None,
            SelectRef::Inline(_body) => None,
            SelectRef::Boxed(body) => (body.vtable().source)(body.borrow()),
        }
    }

    /// Returns a mutable reference to the wrapped source error, if present.
    pub fn source_mut(&mut self) -> Option<&mut (dyn error::Error + Send + Sync + 'static)> {
        match self.select_mut() {
            SelectMut::Const(_body) => None,
            SelectMut::Inline(_body) => None,
            SelectMut::Boxed(body) => (body.vtable().source_mut)(body.borrow_mut()),
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
            SelectRef::Boxed(body) => {
                (body.vtable().downcast_context_ref)(body.borrow(), TypeId::of::<C>())
                    .and_then(|c| c.downcast_ref::<C>())
            }
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
            SelectMut::Boxed(body) => {
                (body.vtable().downcast_context_mut)(body.borrow_mut(), TypeId::of::<C>())
                    .and_then(|c| c.downcast_mut::<C>())
            }
        }
    }

    /// Returns a shared reference to the stored state.
    pub fn state(&self) -> Option<&S> {
        match self.select_ref() {
            SelectRef::Const(_body) => None,
            SelectRef::Inline(body) => Some(body.borrow_value()),
            SelectRef::Boxed(body) => {
                (body.vtable().state)(body.borrow()).and_then(|state| state.downcast_ref::<S>())
            }
        }
    }

    /// Consumes `self` and returns the boxed source error, if any.
    pub fn into_source(self) -> Option<Box<dyn error::Error + Send + Sync + 'static>> {
        match self.select_own() {
            SelectOwn::Const(_body) => None,
            SelectOwn::Inline(_body) => None,
            SelectOwn::Boxed(body) => (body.vtable().into_source)(body),
        }
    }

    /// Consumes `self` and extracts its components by type.
    ///
    /// The corresponding element in the returned tuple is `None` if the
    /// requested component does not exist or has a different type.
    ///
    /// # Existence Guarantee
    /// It's guaranteed that at least one component exists.
    pub fn into_parts<C, E>(self) -> (Option<S>, Option<C>, Option<E>)
    where
        E: 'static,
        C: 'static,
    {
        match self.select_own() {
            SelectOwn::Const(body) => {
                // Safety: The context projection is valid.
                let context = rtti::concretize::<_, C>(body.borrow().deref().context).ok();
                (None, context, None)
            }
            SelectOwn::Inline(body) => (Some(body.into_value()), None, None),
            SelectOwn::Boxed(body) => {
                let mut state: Option<S> = None;
                let mut context: Option<C> = None;
                let mut err: Option<E> = None;
                let mut state_extractor = |value: &mut dyn Any| {
                    if let Some(dst) = value.downcast_mut::<Option<S>>() {
                        mem::swap(&mut state, dst);
                    }
                };

                (body.vtable().into_parts)(
                    body,
                    &mut err as &mut dyn Any,
                    &mut context as &mut dyn Any,
                    &mut state_extractor,
                );

                (state, context, err)
            }
        }
    }

    pub fn extract_state(self) -> Result<(S, Option<RawVacant>), RawError> {
        match self.select_own() {
            SelectOwn::Const(body) => Err(RawError {
                const_body: ManuallyDrop::new(body),
            }),
            SelectOwn::Inline(body) => Ok((body.into_value(), None)),
            SelectOwn::Boxed(body) => {
                let vt = body.vtable();
                let mut state_dst = None::<S>;
                let mut state_extractor = |value: &mut dyn Any| {
                    if let Some(dst) = value.downcast_mut::<Option<S>>() {
                        mem::swap(&mut state_dst, dst);
                    }
                };

                let body = (vt.extract_state)(body, &mut state_extractor);

                match state_dst {
                    Some(state) => Ok((state, Some(RawVacant(body)))),
                    None => Err(RawError {
                        boxed_body: ManuallyDrop::new(body),
                    }),
                }
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

    /// Converts into a boxed error, avoiding reallocation when possible.
    pub fn into_boxed_error(self) -> Box<dyn error::Error + Send + Sync + 'static>
    where
        S: Debug,
    {
        match self.select_own() {
            SelectOwn::Const(body) => (body.borrow().deref().context).into(),
            SelectOwn::Inline(body) => format!("{:?}", body.borrow_value()).into(),
            SelectOwn::Boxed(body) => (body.vtable().into_boxed_error)(body),
        }
    }

    pub fn into_erased(self) -> ErasedRawError
    where
        S: Debug + Send + Sync + 'static,
    {
        ErasedRawError::from_typed(self)
    }

    pub fn erase_state(self) -> RawError
    where
        S: Debug + Send + Sync + 'static,
    {
        match self.select_own() {
            SelectOwn::Const(body) => RawError {
                const_body: ManuallyDrop::new(body),
            },
            SelectOwn::Inline(body) => {
                let erased = ErasedRawError::from_typed(RawError {
                    inline_body: ManuallyDrop::new(body),
                });
                RawError::new(None::<Infallible>, Some(ErasedSource(erased)), NoContext)
            }
            SelectOwn::Boxed(mut body) => {
                let vt = body.vtable();
                (vt.erase_state)(body.borrow_mut());
                RawError {
                    boxed_body: ManuallyDrop::new(body),
                }
            }
        }
    }

    pub const fn is_state_inlinable() -> bool {
        Align4PtrCompat::<S>::is_inlinable()
    }

    pub const fn is_state_compact() -> bool
    where
        S: Debug + 'static,
    {
        Ministate::is_state_compact::<S>()
    }

    #[cfg(feature = "backtrace")]
    pub fn backtrace(&self) -> Option<&std::backtrace::Backtrace> {
        WithBacktrace::search(|| self.source().map(|v| v as _)).map(|(backtrace, _)| backtrace)
    }
}

impl<S> Drop for RawError<S> {
    fn drop(&mut self) {
        match self.kind() {
            Self::KIND_CONST => {}
            Self::KIND_INLINE => unsafe {
                // Safety: The inline variant is active.
                ManuallyDrop::drop(&mut self.inline_body);
            },
            Self::KIND_BOXED => unsafe {
                // Safety: The boxed variant is active; taking it out runs the vtable's drop glue.
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.select_ref() {
            SelectRef::Const(body) => render::format_debug(
                f,
                None,
                Some(body.borrow().deref().context),
                None,
                None::<(Infallible, _)>,
            ),
            SelectRef::Inline(_) => render::format_debug(
                f,
                self.state().map(|s| s as &dyn Debug),
                None::<&str>,
                None,
                None::<(Infallible, _)>,
            ),
            SelectRef::Boxed(body) => (body.vtable().debug)(body.borrow(), f),
        }
    }
}

impl<S> Display for RawError<S>
where
    S: Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.select_ref() {
            SelectRef::Const(_) | SelectRef::Inline(_) => render::format_display(
                f,
                self.state().map(|s| s as &dyn Debug),
                self.context(),
                None,
            ),
            SelectRef::Boxed(body) => (body.vtable().display)(body.borrow(), f),
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
struct ConstBody {
    context: &'static str,
}

/// Heap-allocated error body with type-erased state, source and context.
///
/// # ABI
///
/// It's safe to temporarily work on a [`DynBody`] with its state `S`, source `E`, and context `C`
/// erased (replaced by ZSTs), but it must be restored to the original `DynBody<S, E, C>` before
/// being dropped.
#[repr(C)]
struct DynBody<S = MaybeUninit<Infallible>, E = (), C = ()>
where
    S: Store,
{
    /// # Safety Invariants
    ///
    /// - [`DynBody::vtable`] must be the first field, since the remaining fields are replaced
    ///   by ZSTs during type erasure.
    /// - [`DynBody::vtable`] must be the exclusive [`Discriminator`]
    ///   for [`DynBody::state`].
    vtable: Align4Ref<'static, DynBodyVTable>,
    /// # Safety Invariants
    ///
    /// [`DynBody::state`] should be tracked by [`DynBody::vtable`] exclusively.
    state: S,
    source: E,
    context: exclude::Exclude<C, NoContext>,
}

/// To uphold [`DynBody`]'s ABI guarantee, [`DynBody::vtable`] must be the first field.
const _: () = const {
    assert!(mem::offset_of!(DynBody, vtable) == 0);
};

/// A zero-sized type used as a context placeholder.
#[derive(Debug)]
pub struct NoContext;

impl Display for NoContext {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

/// Virtual function table for type-erased operations on [`DynBody`].
struct DynBodyVTable {
    /// The `TypeId` of the concrete type corresponding to this vtable.
    body_id: TypeId,
    /// See [DynBody::drop].
    drop: fn(ManuallyDrop<Align4Own<DynBody>>),
    /// See [DynBody::into_source].
    into_source: fn(ErasedDynBody) -> Option<Box<dyn error::Error + Send + Sync + 'static>>,
    /// See [DynBody::into_backtrace].
    into_backtrace: fn(ErasedDynBody) -> Option<WithBacktrace>,
    /// See [DynBody::into_parts].
    into_parts: fn(ErasedDynBody, &mut dyn Any, &mut dyn Any, &mut dyn FnMut(&mut dyn Any)),
    /// See [DynBody::extract_state].
    extract_state: fn(ErasedDynBody, &mut dyn FnMut(&mut dyn Any)) -> ErasedDynBody,
    /// See [DynBody::into_boxed_error].
    into_boxed_error: fn(ErasedDynBody) -> Box<dyn error::Error + Send + Sync + 'static>,
    /// See [DynBody::debug].
    debug: fn(Ref<'_, DynBody>, &mut fmt::Formatter<'_>) -> fmt::Result,
    /// See [DynBody::display].
    display: fn(Ref<'_, DynBody>, &mut fmt::Formatter<'_>) -> fmt::Result,
    /// See [DynBody::try_set_state].
    try_set_state: fn(Mut<DynBody>, &mut dyn Any) -> bool,
    /// See [DynBody::has_state].
    has_state: fn(Ref<'_, DynBody>) -> bool,
    /// See [DynBody::erase_state].
    erase_state: fn(Mut<DynBody>),
    /// See [DynBody::is_source_only].
    is_source_only: fn(Ref<'_, DynBody>) -> bool,
    /// See [DynBody::source].
    source: fn(Ref<'_, DynBody>) -> Option<&(dyn error::Error + Send + Sync + 'static)>,
    /// See [DynBody::source_mut].
    source_mut: fn(Mut<'_, DynBody>) -> Option<&mut (dyn error::Error + Send + Sync + 'static)>,
    /// See [DynBody::state].
    state: fn(Ref<'_, DynBody>) -> Option<&dyn Any>,
    /// See [DynBody::context].
    context: fn(Ref<'_, DynBody>) -> Option<&(dyn Printable + Send + Sync + 'static)>,
    /// See [DynBody::downcast_context_ref].
    downcast_context_ref: fn(Ref<'_, DynBody>, TypeId) -> Option<&(dyn Any + 'static)>,
    /// See [DynBody::downcast_context_mut].
    downcast_context_mut: fn(Mut<'_, DynBody>, TypeId) -> Option<&mut (dyn Any + 'static)>,
}

impl DynBodyVTable {
    const fn new<S, E, C>() -> Self
    where
        S: Store,
        E: Source + Send + Sync + 'static,
        C: Debug + Display + Send + Sync + 'static,
    {
        DynBodyVTable {
            body_id: any::TypeId::of::<DynBody<S, E, C>>(),
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
            erase_state: DynBody::<S, E, C>::erase_state,
            is_source_only: DynBody::<S, E, C>::is_source_only,
            source: DynBody::<S, E, C>::source,
            source_mut: DynBody::<S, E, C>::source_mut,
            state: DynBody::<S, E, C>::state,
            context: DynBody::<S, E, C>::context,
            downcast_context_ref: DynBody::<S, E, C>::downcast_context_ref,
            downcast_context_mut: DynBody::<S, E, C>::downcast_context_mut,
        }
    }
}

impl<S, E, C> DynBody<S, E, C>
where
    S: Store,
{
    fn downcast_ref<T>(this: Ref<'_, Self>) -> Option<&T>
    where
        T: 'static,
    {
        if TypeId::of::<T>() != this.deref().vtable.borrow().deref().body_id {
            return None;
        }
        // Safety: The types are verified to match here, so the cast is sound.
        Some(unsafe { this.cast::<T>().deref() })
    }

    fn downcast_mut<T>(mut this: Mut<'_, Self>) -> Option<&mut T>
    where
        T: 'static,
    {
        if TypeId::of::<T>() != this.reborrow().deref().vtable.borrow().deref().body_id {
            return None;
        }
        // Safety: The types are verified to match here, so the cast is sound.
        Some(unsafe { this.cast::<T>().deref_mut() })
    }

    /// Shared view of the state, tracked by the vtable's discriminant.
    fn state_ref(&self) -> StatefulState<'_, S> {
        // Safety: `self.state` is guaranteed to be fully tracked by `self.vtable`.
        unsafe { self.state.with_discriminator(&self.vtable) }
    }

    /// Mutable view of the state, tracked by the vtable's discriminant.
    fn state_mut(&mut self) -> StatefulStateMut<'_, S> {
        // Safety: `self.state` is guaranteed to be fully tracked by `self.vtable`.
        unsafe { self.state.with_discriminator_mut(&mut self.vtable) }
    }
}

impl<S, E, C> DynBody<S, E, C>
where
    S: Store,
    E: Source + Send + Sync + 'static,
    C: Debug + Display + Send + Sync + 'static,
{
    /// Consumes `self`, extracts the state into `state_extractor`, and returns `(source, context)`.
    ///
    /// `state_extractor` is invoked once with the state as `&mut dyn Any` (an `Option` of the
    /// store's representation); the state is dropped even if the callback does not take it out.
    fn destruct(mut self, state_extractor: &mut dyn FnMut(&mut dyn Any)) -> (E, Option<C>) {
        self.state_mut().take(state_extractor);

        let mut this = MaybeUninit::new(self);
        let this = this.as_mut_ptr();

        // Safety: `this` has been moved into `MaybeUninit` and is not accessed afterwards.
        let (context, source) = unsafe {
            let context = (&raw mut (*this).context).read().into_inner();
            let source = (&raw mut (*this).source).read();
            (context, source)
        };

        (source, context)
    }
}

impl<S, E, C> DynBody<S, E, C>
where
    S: Store,
    E: Source + Send + Sync + 'static,
    C: Debug + Display + Send + Sync + 'static,
{
    const CORRECT_VTABLE_CALL: &'static str =
        "vtable functions must be invoked with the correct `DynBody` pointer";

    /// Drops the boxed body.
    fn drop(this: ManuallyDrop<Align4Own<DynBody>>) {
        let this = ErasedDynBody::into_inner::<S, E, C>(ErasedDynBody(this))
            .expect(Self::CORRECT_VTABLE_CALL);

        let _ = this.into_boxed();
    }

    /// Extracts the source error as a boxed error.
    fn into_source(this: ErasedDynBody) -> Option<Box<dyn error::Error + Send + Sync + 'static>> {
        let this = ErasedDynBody::into_inner::<S, E, C>(this).expect(Self::CORRECT_VTABLE_CALL);
        let Align4(this) = *this.into_boxed();

        let (source, ..) = this.destruct(&mut |_| {});

        source.into_boxed()
    }

    /// Extracts the backtrace.
    fn into_backtrace(this: ErasedDynBody) -> Option<WithBacktrace> {
        let this = ErasedDynBody::into_inner::<S, E, C>(this).expect(Self::CORRECT_VTABLE_CALL);
        let Align4(this) = *this.into_boxed();

        let (source, ..) = this.destruct(&mut |_| {});

        source.into_backtrace()
    }

    /// Decomposes the boxed body: moves source/context into the caller's `Option`s (on `TypeId`
    /// match) and the state into `state_extractor`.
    #[allow(clippy::too_many_arguments)]
    fn into_parts(
        this: ErasedDynBody,
        source_dst: &mut dyn Any,
        context_dst: &mut dyn Any,
        state_extractor: &mut dyn FnMut(&mut dyn Any),
    ) {
        let this = ErasedDynBody::into_inner::<S, E, C>(this).expect(Self::CORRECT_VTABLE_CALL);
        let Align4(this) = *this.into_boxed();
        let (source, context) = this.destruct(state_extractor);

        if let Some(context) = context {
            if let Some(dst) = context_dst.downcast_mut::<Option<C>>() {
                dst.replace(context);
            }
        }

        source.downcast_container(source_dst).ok();
    }

    /// Extracts the state from the boxed body into `state_extractor`, returning the vacant body.
    fn extract_state(
        this: ErasedDynBody,
        state_extractor: &mut dyn FnMut(&mut dyn Any),
    ) -> ErasedDynBody {
        let mut this = ErasedDynBody::into_inner::<S, E, C>(this).expect(Self::CORRECT_VTABLE_CALL);

        this.borrow_mut()
            .deref_mut()
            .state_mut()
            .take(state_extractor);

        ErasedDynBody::from_typed(this)
    }

    /// Converts the boxed body into `Box<Error>`, avoiding reallocation when possible.
    fn into_boxed_error(this: ErasedDynBody) -> Box<dyn error::Error + Send + Sync + 'static> {
        let this = ErasedDynBody::into_inner::<S, E, C>(this).expect(Self::CORRECT_VTABLE_CALL);
        let has_state = {
            let this = this.borrow().deref();
            this.state_ref().format_debug().is_some()
        };
        let has_context = this.borrow().deref().context.get().is_some();

        match (has_state, has_context) {
            (false, false) => {
                let Align4(this) = *this.into_boxed();
                let (source, _) = this.destruct(&mut |_| {});
                source
                    .into_boxed()
                    .unwrap_or(Box::from("empty erratic error")) // Note: Should never happen; kept as a fallback.
            }
            (_, _) => this.into_boxed(),
        }
    }

    /// Formats the underlying boxed body using the `Debug` trait.
    fn debug(this: Ref<'_, DynBody>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let this = DynBody::downcast_ref::<Self>(this).expect(Self::CORRECT_VTABLE_CALL);
        <Self as Debug>::fmt(this, f)
    }

    /// Formats the underlying boxed body using the `Display` trait.
    fn display(this: Ref<'_, DynBody>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let this = DynBody::downcast_ref::<Self>(this).expect(Self::CORRECT_VTABLE_CALL);
        <Self as Display>::fmt(this, f)
    }

    /// Stores the state if the type matches and the body is empty. On success `state_src`'s
    /// `Option` is taken and `true` is returned; otherwise `false` is returned and `state_src`
    /// is left unchanged.
    fn try_set_state(this: Mut<'_, DynBody>, state_src: &mut dyn Any) -> bool {
        let this = DynBody::downcast_mut::<Self>(this).expect(Self::CORRECT_VTABLE_CALL);

        this.state_mut().try_set(state_src)
    }

    /// Checks if there is a state in the body.
    fn has_state(this: Ref<'_, DynBody>) -> bool {
        let this = DynBody::downcast_ref::<Self>(this).expect(Self::CORRECT_VTABLE_CALL);

        this.state_ref().get().is_some()
    }

    /// Erases the state in place.
    fn erase_state(this: Mut<'_, DynBody>) {
        let this = DynBody::downcast_mut::<Self>(this).expect(Self::CORRECT_VTABLE_CALL);

        this.state_mut().freeze();
    }

    /// Checks if the error is source-only (no state, no context, and a source).
    ///
    /// A state that has been frozen still counts, so an error with an erased state
    /// is never considered source-only.
    fn is_source_only(this: Ref<'_, DynBody>) -> bool {
        let this = DynBody::downcast_ref::<Self>(this).expect(Self::CORRECT_VTABLE_CALL);

        let has_state = this.state_ref().format_debug().is_some();
        let has_context = this.context.get().is_some();
        let has_source = this.source.error_ref().is_some();

        !has_state && !has_context && has_source
    }

    /// Returns a reference to the source error.
    fn source(this: Ref<'_, DynBody>) -> Option<&(dyn error::Error + Send + Sync + 'static)> {
        let this = DynBody::downcast_ref::<Self>(this).expect(Self::CORRECT_VTABLE_CALL);

        this.source.error_ref()
    }

    /// Returns a mutable reference to the source error.
    fn source_mut(
        this: Mut<'_, DynBody>,
    ) -> Option<&mut (dyn error::Error + Send + Sync + 'static)> {
        let this = DynBody::downcast_mut::<Self>(this).expect(Self::CORRECT_VTABLE_CALL);

        this.source.error_mut()
    }

    /// Returns an opaque reference to the state, if present.
    fn state(this: Ref<'_, DynBody>) -> Option<&dyn Any> {
        let this = DynBody::downcast_ref::<Self>(this).expect(Self::CORRECT_VTABLE_CALL);

        this.state_ref().get()
    }

    /// Returns a displayable reference to the context.
    fn context(this: Ref<'_, DynBody>) -> Option<&(dyn Printable + Send + Sync + 'static)> {
        let this = DynBody::downcast_ref::<Self>(this).expect(Self::CORRECT_VTABLE_CALL);

        this.context.get().map(|c| c as _)
    }

    /// Attempts to downcast the context field to the requested type `C`.
    ///
    /// Returns the context as `&dyn Any` if the requested `TypeId` matches, otherwise `None`.
    fn downcast_context_ref(this: Ref<'_, DynBody>, ty: TypeId) -> Option<&(dyn Any + 'static)> {
        let this = DynBody::downcast_ref::<Self>(this).expect(Self::CORRECT_VTABLE_CALL);

        let context = this.context.get()?;
        (TypeId::of::<C>() == ty).then_some(context as &dyn Any)
    }

    /// Attempts to downcast the context field to the requested type `C` (mutable).
    ///
    /// Returns the context as `&mut dyn Any` if the requested `TypeId` matches, otherwise `None`.
    fn downcast_context_mut(
        this: Mut<'_, DynBody>,
        ty: TypeId,
    ) -> Option<&mut (dyn Any + 'static)> {
        let this = DynBody::downcast_mut::<Self>(this).expect(Self::CORRECT_VTABLE_CALL);

        let context = this.context.get_mut()?;
        (TypeId::of::<C>() == ty).then_some(context as &mut dyn Any)
    }
}

impl<S, E, C> Drop for DynBody<S, E, C>
where
    S: Store,
{
    fn drop(&mut self) {
        self.state_mut().drop_in_place();
    }
}

impl<S, E, C> fmt::Debug for DynBody<S, E, C>
where
    S: Store,
    E: Source + Send + Sync + 'static,
    C: Debug + Display + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        render::format_debug(
            f,
            self.state_ref().format_debug(),
            self.context.get(),
            self.source.error_ref().map(|e| e as _),
            WithBacktrace::search_debug(|| self.source.error_ref().map(|e| e as _)),
        )
    }
}

impl<S, E, C> fmt::Display for DynBody<S, E, C>
where
    E: Source + Send + Sync + 'static,
    S: Store,
    C: Debug + Display + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        render::format_display(
            f,
            self.state_ref().format_debug(),
            self.context.get(),
            self.source.error_ref().map(|e| e as _),
        )
    }
}

impl<S, E, C> error::Error for DynBody<S, E, C>
where
    S: Store,
    E: Source + Send + Sync + 'static,
    C: Debug + Display + Send + Sync + 'static,
{
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.source.error_ref().map(|e| e as _)
    }
}

pub(crate) struct RawVacant(ErasedDynBody);

impl RawVacant {
    pub fn try_with_state<S>(mut self, state: S) -> Result<RawError<S>, (Self, S)>
    where
        S: Debug + 'static,
    {
        let vt = self.0.vtable();
        let Err(state) = match_else!(Ministate::try_new(state), Ok(ministate) => {
            let mut payload = Some(ministate);
            (vt.try_set_state)(self.0.borrow_mut(), &mut payload);
            match payload {
                Some(ministate) => return Err((self, ministate.into_inner::<S>().expect("succeed with accurate type"))),
                None => return Ok(RawError { boxed_body: ManuallyDrop::new(self.0) }),
            }
        });

        let mut payload = Some(state);
        (vt.try_set_state)(self.0.borrow_mut(), &mut payload);
        match payload {
            Some(state) => Err((self, state)),
            None => Ok(RawError {
                boxed_body: ManuallyDrop::new(self.0),
            }),
        }
    }

    pub fn try_into_stateless(self) -> Result<RawError, Self> {
        let vt = self.0.vtable();

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

    /// Derives a new error from this vacant while preserving the backtrace. This is the only way to
    /// turn a vacant into an error when no state, context, or source is left to wrap.
    pub fn derive<S, C>(self, state: Option<S>, context: C) -> RawError<S>
    where
        S: Debug + Send + Sync + 'static,
        C: Context,
    {
        let vt = self.0.vtable();

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
                Some(ErasedSource(
                    RawError::<Infallible> {
                        boxed_body: ManuallyDrop::new(self.0),
                    }
                    .into_erased(),
                )),
                context,
            ),
        }
    }
}

impl Debug for RawVacant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let body_ref = self.0.borrow();
        let vt = self.0.vtable();
        let show_less = f.sign_minus();

        let context = (vt.context)(body_ref);
        let source = (vt.source)(body_ref);
        let backtrace = WithBacktrace::search_debug(|| {
            (vt.source)(body_ref).map(|v| v as &(dyn error::Error + 'static))
        });

        render::format_debug_struct(
            f,
            "Vacant",
            None,
            context,
            source.map(|v| v as _),
            backtrace.filter(|_| !show_less),
        )
    }
}

#[repr(transparent)]
struct ErasedDynBody(ManuallyDrop<Align4Own<DynBody>>);

impl ErasedDynBody {
    fn from_typed<S, E, C>(body: Align4Own<DynBody<S, E, C>>) -> Self
    where
        S: Store,
    {
        // Safety: Erases the `DynBody` is ABI compatible.
        Self(unsafe { body.cast::<DynBody>() })
    }

    /// Returns a static shared reference to the vtable.
    fn vtable(&self) -> &'static DynBodyVTable {
        self.0.borrow().deref().vtable.borrow().deref()
    }

    /// Recovers the typed owned body `Align4Own<DynBody<S, E, C>>`.
    ///
    /// Returns `None` if the type does not match.
    fn into_inner<S, E, C>(this: Self) -> Option<Align4Own<DynBody<S, E, C>>>
    where
        S: Store,
        E: 'static,
        C: 'static,
    {
        let body_id = this.borrow().deref().vtable.borrow().deref().body_id;
        if TypeId::of::<DynBody<S, E, C>>() != body_id {
            return None;
        }

        let mut this = ManuallyDrop::new(this);
        // Safety: The types are verified to match here, so the cast is sound.
        unsafe {
            let mut this: ManuallyDrop<Align4Own<DynBody<S, E, C>>> =
                ManuallyDrop::take(&mut this.0).cast();

            Some(ManuallyDrop::take(&mut this))
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
        // Safety: The body is taken out of the `ManuallyDrop` exactly once and handed to the
        // vtable's drop glue, which consumes it.
        unsafe {
            (self.vtable().drop)(ManuallyDrop::new(ManuallyDrop::take(&mut self.0)));
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};
    use core::{assert_matches, convert::Infallible, fmt::Display, mem};

    use super::*;
    use crate::{
        context::{Contextless, Literal, Mkctx},
        test_fixtures::{TestError, TestMessage},
    };

    // --- Test helpers ---

    /// A typed literal for testing.
    #[derive(Debug)]
    struct TestContextLiteral;

    impl Literal for TestContextLiteral {
        const LITERAL: &'static str = "test context";
    }

    type TestContext = Mkctx<TestContextLiteral>;

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
        let err = RawError::new(None::<Infallible>, Some(TestError::FOO), Contextless::new());
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
        let err = RawError::new(None::<Infallible>, Some(TestError::FOO), Contextless::new());
        let src = err.source();
        assert_eq!(src.unwrap().to_string(), "foo");
    }

    #[test]
    fn boxed_variant_downcast_source() {
        let err = RawError::new(None::<Infallible>, Some(TestError::FOO), Contextless::new());
        let downcasted = err.downcast_source_ref::<TestError>();
        assert_matches!(downcasted, Some(TestError("foo")));
    }

    #[test]
    fn boxed_variant_downcast_source_wrong_type() {
        let err = RawError::new(None::<Infallible>, Some(TestError::FOO), Contextless::new());
        let downcasted = err.downcast_source_ref::<fmt::Error>();
        assert!(downcasted.is_none());
    }

    #[test]
    fn boxed_variant_context() {
        let err = RawError::new(
            None::<Infallible>,
            Some(TestError::FOO),
            TestContextLiteral::LITERAL,
        );
        let ctx = err.context();
        assert_eq!(ctx.unwrap().to_string(), TestContextLiteral::LITERAL);
    }

    #[test]
    fn boxed_variant_without_source_is_none() {
        let err = RawError::new(Some(42u32), None::<Infallible>, Contextless::new());
        assert!(err.source().is_none());
        assert_matches!(err.state(), Some(42));
    }

    // --- into_source ---

    #[test]
    fn boxed_variant_into_source_returns_boxed_error() {
        let err = RawError::new(None::<Infallible>, Some(TestError::FOO), Contextless::new());
        let src = err.into_source();
        assert_eq!(src.unwrap().to_string(), "foo");
    }

    #[test]
    fn boxed_variant_into_source_returns_none() {
        let err = RawError::new(None::<Infallible>, None::<Infallible>, Contextless::new());
        assert!(err.into_source().is_none());
    }

    // --- into_parts ---

    #[test]
    fn boxed_variant_into_parts_matches_types() {
        let err = RawError::new(
            Some("state"),
            Some(TestError::FOO),
            TestContextLiteral::LITERAL,
        );
        let (state, context, source) = err.into_parts::<&str, TestError>();
        assert_matches!(state, Some("state"));
        assert_matches!(source, Some(TestError::FOO));
        assert_eq!(context, Some(TestContextLiteral::LITERAL));
    }

    #[test]
    fn boxed_variant_into_parts_context_downcasts() {
        let err = RawError::new(None::<Infallible>, Some(TestError::FOO), Contextless::new());
        let (_, context, _) = err.into_parts::<NoContext, String>();
        assert!(context.is_none());
    }

    #[test]
    fn const_variant_into_parts() {
        let err = RawError::try_new_const::<TestContext>().unwrap();
        let (state, context, source) = err.into_parts::<&str, TestError>();
        assert!(source.is_none());
        assert_eq!(context, Some(TestContextLiteral::LITERAL));
        assert_eq!(state, None);
    }

    #[test]
    fn inline_variant_into_parts() {
        let err = RawError::try_new_inline(42u16).unwrap();
        let (state, context, source) = err.into_parts::<NoContext, TestError>();
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
            let err = RawError::new(None::<Infallible>, None::<Infallible>, TestMessage::HOGE);
            assert_matches!(err.extract_state(), Err(err) if err.to_string() == "hoge");
        }
        {
            let err = RawError::new(Some(42i32), None::<Infallible>, TestMessage::HOGE);
            match err.extract_state() {
                Ok((state, Some(vacant))) if state == 42 => {
                    let err = vacant.try_with_state(state).unwrap();
                    assert_eq!(err.state(), Some(&42));
                    assert_eq!(err.context().unwrap().to_string(), "hoge");
                }
                _ => panic!("extract should not fail"),
            }
        }
    }

    #[test]
    fn vacant_reuses_allocation_for_ministate_capable_types() {
        // A `RawError<u8>` with only a state would be stored inline. Adding a source forces
        // the boxed variant, where the state lives in a `Ministate`.
        let err = RawError::new(Some(42u8), Some(TestError::FOO), Contextless::new());
        let fingerprint = err
            .get_alloc_fingerprint()
            .expect("a state + source error should be boxed");

        // Extract the state, leaving a `RawVacant` that still holds the source.
        let (state, vacant) = err.extract_state().expect("state should be extractable");
        assert_eq!(state, 42u8);
        let vacant = vacant.expect("u8 fits a Ministate, so extraction should yield a vacant");

        // Reuse the same allocation to store a `u16` state: converting `RawError<u8>` to
        // `RawError<u16>` must not trigger a reallocation.
        let err = vacant
            .try_with_state(16u16)
            .expect("u16 fits a Ministate, so the vacant should be reused");
        assert_eq!(
            err.get_alloc_fingerprint(),
            Some(fingerprint),
            "RawError<u8> -> RawError<u16> via RawVacant must reuse the allocation"
        );
        assert_eq!(err.state(), Some(&16u16));
        assert_eq!(err.source().unwrap().to_string(), "foo");
    }

    #[test]
    fn vacant_reuses_allocation_for_same_large_type() {
        // `i128` exceeds a `Ministate`'s capacity.
        let err = RawError::new(Some(42i128), Some(TestError::FOO), Contextless::new());
        let fingerprint = err
            .get_alloc_fingerprint()
            .expect("a state-only RawError<i128> should be boxed");

        let (state, vacant) = err.extract_state().expect("state should be extractable");
        assert_eq!(state, 42i128);
        let vacant = vacant.expect("a boxed body should yield a vacant");

        // `try_with_state` cannot use a `Ministate` here (i128 is too large), but since the
        // new state type matches the body's store type, `try_set` succeeds and the same allocation
        // is reused.
        let err = vacant
            .try_with_state(42i128)
            .expect("same type should be stored back without allocating");
        assert_eq!(
            err.get_alloc_fingerprint(),
            Some(fingerprint),
            "RawError<i128> -> RawError<i128> via RawVacant must reuse the allocation"
        );
        assert_eq!(err.state(), Some(&42i128));
    }

    // --- is_source_only ---

    #[test]
    fn is_source_only_accuracy() {
        // 1. Only a source: source-only.
        let err = RawError::new(None::<Infallible>, Some(TestError::FOO), Contextless::new());
        assert!(err.is_source_only());

        // 2. Only a context: not source-only.
        // Note: `String` is a lazy context, forcing the boxed variant.
        let err = RawError::new(None::<Infallible>, None::<Infallible>, String::from("ctx"));
        assert!(
            err.get_alloc_fingerprint().is_some(),
            "expected a boxed variant"
        );
        assert!(!err.is_source_only());

        // 3. Only a state: not source-only.
        // Note: `u128` is too large for the inline variant, forcing the boxed variant.
        let err = RawError::new(Some(42u128), None::<Infallible>, Contextless::new());
        assert!(
            err.get_alloc_fingerprint().is_some(),
            "expected a boxed variant"
        );
        assert!(!err.is_source_only());

        // 3.5. Only a state, erased: a frozen state still counts, so not source-only.
        let err = RawError::new(Some(42u128), None::<Infallible>, Contextless::new());
        let err = err.erase_state();
        assert!(!err.is_source_only());

        // 4. A source and a state: not source-only.
        let err = RawError::new(Some(42u128), Some(TestError::FOO), Contextless::new());
        assert!(!err.is_source_only());

        // 5. A source and a state, erased: not source-only.
        let err = RawError::new(Some(42u128), Some(TestError::FOO), Contextless::new());
        let err = err.erase_state();
        assert!(!err.is_source_only());

        // 6. A context and a state: not source-only.
        let err = RawError::new(Some(42u128), None::<Infallible>, String::from("ctx"));
        assert!(
            err.get_alloc_fingerprint().is_some(),
            "expected a boxed variant"
        );
        assert!(!err.is_source_only());

        // 7. A context and a state, erased: not source-only.
        let err = RawError::new(Some(42u128), None::<Infallible>, String::from("ctx"));
        let err = err.erase_state();
        assert!(!err.is_source_only());
    }

    // --- Layer elimination ---

    #[test]
    fn new_eliminates_erased_layer() {
        // Build a source-only RawError: (RawError -> TestError)
        let inner = RawError::new(None::<Infallible>, Some(TestError::BAR), Contextless::new());
        // Erase the type → ErasedRawError -> TestError
        let erased = ErasedRawError::from_typed(inner);
        // Re-wrap: this should eliminate the ErasedRawError layer since it carries no extra info
        let err = RawError::new(
            None::<Infallible>,
            Some(ErasedSource(erased)),
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
    fn wrapping_source_only_erased_repeatedly_does_not_allocate() {
        // Build a source-only RawError: (RawError -> TestError)
        let mut err = RawError::new(None::<Infallible>, Some(TestError::BAR), Contextless::new());
        let fingerprint = err
            .get_alloc_fingerprint()
            .expect("source-only error should be boxed");

        for _ in 0..5 {
            // Erase the type → ErasedRawError -> TestError
            let erased = ErasedRawError::from_typed(err);
            // Re-wrap without state/context: `new_0` should eliminate the erased layer in place
            // via `try_into_stateless` + `with_phantom_state`, reusing the same allocation.
            err = RawError::new(
                None::<Infallible>,
                Some(ErasedSource(erased)),
                Contextless::new(),
            );

            assert_eq!(
                err.get_alloc_fingerprint(),
                Some(fingerprint),
                "wrapping a source-only erased error must not allocate"
            );
            assert_eq!(err.chain().count(), 1, "chain length should always be 1");
            assert!(
                err.downcast_source_ref::<TestError>().is_some(),
                "TestError should always be reachable"
            );
        }
    }

    #[test]
    fn round_trip_repeatedly_keeps_single_layer() {
        // Start with a source-only RawError: (RawError -> TestError)
        let mut err = RawError::new(None::<Infallible>, Some(TestError::BAR), Contextless::new());
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
            assert_eq!(err.chain().count(), 1, "chain length should always be 1");
            assert!(
                err.chain().last().unwrap().is::<TestError>(),
                "TestError should always be reachable"
            );
        }
    }

    #[test]
    fn erase_preserves_display() {
        // Const variant: displays the static context.
        {
            let err = RawError::try_new_const::<TestContext>().unwrap();
            let before = err.to_string();
            let erased = err.into_erased();
            assert_eq!(
                erased.error_ref().to_string(),
                before,
                "erasing a const error must preserve its display"
            );
        }
        // Inline variant: displays the inline state.
        {
            let err = RawError::try_new_inline(42u16).unwrap();
            let before = err.to_string();
            let erased = err.into_erased();
            assert_eq!(
                erased.error_ref().to_string(),
                before,
                "erasing an inline error must preserve its display"
            );
        }
        // Boxed variant with context: displays source + context.
        {
            let err = RawError::new(
                None::<Infallible>,
                Some(TestError::FOO),
                TestContextLiteral::LITERAL,
            );
            let before = err.to_string();
            let erased = err.into_erased();
            assert_eq!(
                erased.error_ref().to_string(),
                before,
                "erasing a boxed error must preserve its display"
            );
        }
        // Boxed variant with large state.
        {
            let err = RawError::new(Some(42u128), Some(TestError::FOO), Contextless::new());
            let before = err.to_string();
            let erased = err.into_erased();
            assert_eq!(
                erased.error_ref().to_string(),
                before,
                "erasing a boxed error with state must preserve its display"
            );
        }
        // Boxed variant with small state.
        {
            let err = RawError::new(Some(42u8), Some(TestError::FOO), Contextless::new());
            let before = err.to_string();
            let erased = err.into_erased();
            assert_eq!(
                erased.error_ref().to_string(),
                before,
                "erasing a boxed error with state must preserve its display"
            );
        }
        // Source-only boxed variant: `error_ref` returns the source directly.
        {
            let err = RawError::new(None::<Infallible>, Some(TestError::BAR), Contextless::new());
            let before = err.to_string();
            let erased = err.into_erased();
            assert_eq!(
                erased.error_ref().to_string(),
                before,
                "erasing a source-only error must preserve its display"
            );
        }
    }

    #[test]
    fn state_inlinability_by_size() {
        const _: () = {
            assert!(RawError::<u8>::is_state_inlinable());
            assert!(!RawError::<u128>::is_state_inlinable());
        };
    }
}
