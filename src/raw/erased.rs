use core::{
    convert::Infallible,
    error::Error,
    fmt::{Debug, Display},
    mem::ManuallyDrop,
};

use crate::{
    fmt,
    raw::{
        ConstBody, DynBody, ErasedDynBody, RawError, SelectOwn,
        ptr::{Align4Ref, ErasedAlign4PtrCompat},
    },
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

    pub fn try_into_stateless(self) -> Result<RawError, Self> {
        match self.0 {
            ErasedRawErrorInner::Const(body) => Ok(RawError {
                const_body: ManuallyDrop::new(body),
            }),
            ErasedRawErrorInner::Boxed(body) => {
                let body_ref = body.borrow();
                let vt = DynBody::vtable(body_ref);
                let has_state = unsafe { (vt.has_state)(body_ref) };
                if has_state {
                    Err(ErasedRawError(ErasedRawErrorInner::Boxed(body)))
                } else {
                    Ok(RawError {
                        boxed_body: ManuallyDrop::new(body),
                    })
                }
            }
            this @ ErasedRawErrorInner::Inline(_) => Err(ErasedRawError(this)),
        }
    }
}

impl Debug for ErasedRawError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match &self.0 {
            ErasedRawErrorInner::Const(body) => fmt::format_debug::<()>(
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
                fmt::format_debug(f, Some(body), None::<Infallible>, None, None::<&Infallible>)
            }
        }
    }
}

impl Display for ErasedRawError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match &self.0 {
            ErasedRawErrorInner::Const(body) => fmt::format_display::<()>(
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
            ErasedRawErrorInner::Inline(body) => fmt::format_display(
                f,
                Some(body),
                None::<&Infallible>,
                None,
                None::<&Infallible>,
            ),
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
