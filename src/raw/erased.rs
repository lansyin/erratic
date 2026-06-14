use core::{
    convert::Infallible,
    error::Error,
    fmt::{self, Debug, Display},
    mem::ManuallyDrop,
    result,
};

use crate::{
    raw::{
        ConstBody, DynBody, ErasedDynBody, RawError, SelectOwn,
        ptr::{Align4Ref, ErasedAlign4PtrCompat},
    },
    render,
};

pub struct ErasedRawError(ErasedRawErrorInner);

enum ErasedRawErrorInner {
    Const(Align4Ref<'static, ConstBody>),
    Boxed(ErasedDynBody),
    Inline(ErasedAlign4PtrCompat),
}

impl ErasedRawError {
    pub fn from_typed<S>(value: RawError<S>) -> Self
    where
        S: Debug + Send + Sync + 'static,
    {
        match value.select_own() {
            SelectOwn::Const(body) => ErasedRawError(ErasedRawErrorInner::Const(body)),
            SelectOwn::Boxed(body) => ErasedRawError(ErasedRawErrorInner::Boxed(body)),
            SelectOwn::Inline(body) => ErasedRawError(ErasedRawErrorInner::Inline(body.erase())),
        }
    }

    pub fn try_into_stateless(self) -> result::Result<RawError, Self> {
        match self.0 {
            ErasedRawErrorInner::Const(body) => Ok(RawError {
                const_body: ManuallyDrop::new(body),
            }),
            ErasedRawErrorInner::Boxed(body) => Ok(RawError {
                // Note: This erases the generic type `S` to `Infallible`, even though the body may still
                // contain a state. It doesn't affect `Debug` / `Display` since they use dynamic dispatch, but users
                // might be surprised to see a stateless error here and later retrieve a concrete state after using
                // `with_phantom_state` and `state`.
                boxed_body: ManuallyDrop::new(body),
            }),
            this @ ErasedRawErrorInner::Inline(_) => Err(ErasedRawError(this)),
        }
    }
}

impl Debug for ErasedRawError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErasedRawErrorInner::Const(body) => render::format_debug::<()>(
                f,
                None,
                Some(&body.borrow().deref().context),
                None,
                None::<&Infallible>,
            ),
            ErasedRawErrorInner::Boxed(body) => {
                let body = body.borrow();
                let vt = DynBody::vtable(body);
                unsafe { (vt.debug)(body, f) }
            }
            ErasedRawErrorInner::Inline(body) => {
                render::format_debug(f, Some(body), None::<Infallible>, None, None::<&Infallible>)
            }
        }
    }
}

impl Display for ErasedRawError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErasedRawErrorInner::Const(body) => render::format_display::<()>(
                f,
                None,
                Some(&body.borrow().deref().context),
                None,
                None::<&Infallible>,
            ),
            ErasedRawErrorInner::Boxed(body) => {
                let body = body.borrow();
                let vt = DynBody::vtable(body);
                unsafe { (vt.display)(body, f) }
            }
            ErasedRawErrorInner::Inline(body) => {
                render::format_display(f, Some(body), None, None, None::<&Infallible>)
            }
        }
    }
}

impl Error for ErasedRawError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.0 {
            ErasedRawErrorInner::Const(_body) => None,
            ErasedRawErrorInner::Boxed(body) => {
                let body = body.borrow();
                let vt = DynBody::vtable(body);
                unsafe { (vt.source)(body).map(|v| v as _) }
            }
            ErasedRawErrorInner::Inline(_body) => None,
        }
    }
}
