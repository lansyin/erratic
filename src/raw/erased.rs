use alloc::boxed::Box;
use core::{
    convert::Infallible,
    error::Error,
    fmt::{self, Debug, Display},
    mem::ManuallyDrop,
};

use crate::{
    raw::{
        ConstBody, ErasedDynBody, RawError, SelectOwn,
        align4::{Align4Ref, ErasedAlign4PtrCompat},
    },
    render,
};

pub(crate) struct ErasedRawError(Inner);

enum Inner {
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
            SelectOwn::Const(body) => ErasedRawError(Inner::Const(body)),
            SelectOwn::Boxed(body) => ErasedRawError(Inner::Boxed(body)),
            SelectOwn::Inline(body) => ErasedRawError(Inner::Inline(body.erase())),
        }
    }

    pub fn try_into_stateless(self) -> Result<RawError, Self> {
        match self.0 {
            Inner::Const(body) => Ok(RawError {
                const_body: ManuallyDrop::new(body),
            }),
            Inner::Boxed(body) => {
                let vt = body.vtable();
                let body_ref = body.borrow();
                let has_state = (vt.has_state)(body_ref);
                if has_state {
                    Err(ErasedRawError(Inner::Boxed(body)))
                } else {
                    Ok(RawError {
                        boxed_body: ManuallyDrop::new(body),
                    })
                }
            }
            this @ Inner::Inline(_) => Err(ErasedRawError(this)),
        }
    }

    pub fn error_ref(&self) -> &(dyn Error + Send + Sync + 'static) {
        match &self.0 {
            Inner::Const(_) | Inner::Inline(_) => &self.0,
            Inner::Boxed(body) => {
                let vt = body.vtable();
                let body_ref = body.borrow();
                if (vt.is_source_only)(body_ref) {
                    // Safety: `is_source_only` guarantees a source is present.
                    (vt.source)(body_ref).unwrap()
                } else {
                    &self.0
                }
            }
        }
    }

    pub fn error_mut(&mut self) -> &mut (dyn Error + Send + Sync + 'static) {
        match &mut self.0 {
            Inner::Const(_) | Inner::Inline(_) => {}
            Inner::Boxed(body) => {
                let vt = body.vtable();
                let body_ref = body.borrow();
                if (vt.is_source_only)(body_ref) {
                    // Safety: `is_source_only` guarantees a source is present.
                    let source = (vt.source_mut)(body.borrow_mut()).unwrap();
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

impl Debug for Inner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Inner::Const(body) => render::format_debug(
                f,
                None,
                Some(&body.borrow().deref().context),
                None,
                None::<(Infallible, _)>,
            ),
            Inner::Boxed(body) => {
                let vt = body.vtable();
                let body = body.borrow();
                (vt.debug)(body, f)
            }
            Inner::Inline(body) => render::format_debug(
                f,
                Some(body),
                None::<Infallible>,
                None,
                None::<(Infallible, _)>,
            ),
        }
    }
}

impl Display for Inner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Inner::Const(body) => {
                render::format_display(f, None, Some(&body.borrow().deref().context), None)
            }
            Inner::Boxed(body) => {
                let vt = body.vtable();
                let body = body.borrow();
                (vt.display)(body, f)
            }
            Inner::Inline(body) => render::format_display(f, Some(body), None::<&Infallible>, None),
        }
    }
}

impl Error for Inner {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Inner::Const(_body) => None,
            Inner::Boxed(body) => {
                let vt = body.vtable();
                let body = body.borrow();
                (vt.source)(body).map(|v| v as _)
            }
            Inner::Inline(_body) => None,
        }
    }
}
