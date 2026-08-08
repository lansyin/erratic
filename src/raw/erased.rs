use alloc::boxed::Box;
use core::{
    convert::Infallible,
    error::Error,
    fmt::{self, Debug, Display},
    mem::ManuallyDrop,
};

use crate::{
    raw::{
        ConstBody, DynBody, ErasedDynBody, RawError, SelectOwn,
        ptr::{Align4Ref, ErasedAlign4PtrCompat},
    },
    render,
};

pub(crate) struct ErasedRawError(ErasedRawErrorInner);

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

    pub fn error_ref(&self) -> &(dyn Error + Send + Sync + 'static) {
        match &self.0 {
            ErasedRawErrorInner::Const(_) | ErasedRawErrorInner::Inline(_) => &self.0,
            ErasedRawErrorInner::Boxed(body) => {
                let body_ref = body.borrow();
                let vt = DynBody::vtable(body_ref);
                if unsafe { (vt.is_source_only)(body_ref) } {
                    // Safety: `is_source_only` guarantees a source is present.
                    unsafe { (vt.source)(body_ref) }.unwrap()
                } else {
                    &self.0
                }
            }
        }
    }

    pub fn error_mut(&mut self) -> &mut (dyn Error + Send + Sync + 'static) {
        match &mut self.0 {
            ErasedRawErrorInner::Const(_) | ErasedRawErrorInner::Inline(_) => {}
            ErasedRawErrorInner::Boxed(body) => {
                let body_ref = body.borrow();
                let vt = DynBody::vtable(body_ref);
                if unsafe { (vt.is_source_only)(body_ref) } {
                    // Safety: `is_source_only` guarantees a source is present.
                    let source = unsafe { (vt.source_mut)(body.borrow_mut()) }.unwrap();
                    // Safety: There is no overlapping mutable reference to the source.
                    return unsafe { &mut *(source as *mut _) };
                }
            }
        }

        &mut self.0
    }

    pub fn into_boxed(self) -> Box<dyn Error + Send + Sync + 'static> {
        match self.try_into_stateless() {
            Ok(raw) => raw.into_boxed_error(),
            Err(this) => Box::new(this.0),
        }
    }
}

impl Debug for ErasedRawErrorInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErasedRawErrorInner::Const(body) => render::format_debug::<()>(
                f,
                None,
                Some(&body.borrow().deref().context),
                None,
                None::<(Infallible, _)>,
            ),
            ErasedRawErrorInner::Boxed(body) => {
                let body = body.borrow();
                let vt = DynBody::vtable(body);
                unsafe { (vt.debug)(body, f) }
            }
            ErasedRawErrorInner::Inline(body) => render::format_debug(
                f,
                Some(body),
                None::<Infallible>,
                None,
                None::<(Infallible, _)>,
            ),
        }
    }
}

impl Display for ErasedRawErrorInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErasedRawErrorInner::Const(body) => {
                render::format_display::<()>(f, None, Some(&body.borrow().deref().context), None)
            }
            ErasedRawErrorInner::Boxed(body) => {
                let body = body.borrow();
                let vt = DynBody::vtable(body);
                unsafe { (vt.display)(body, f) }
            }
            ErasedRawErrorInner::Inline(body) => {
                render::format_display(f, Some(body), None::<&Infallible>, None)
            }
        }
    }
}

impl Error for ErasedRawErrorInner {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
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
