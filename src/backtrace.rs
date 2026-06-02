use alloc::boxed::Box;
use core::{
    any::TypeId,
    error::{self},
    fmt::{self, Debug, Display},
    ptr::NonNull,
    result,
};

#[cfg(feature = "backtrace")]
extern crate std;

#[cfg(feature = "backtrace")]
use core::{
    cell::Cell,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::nae::Nae;

pub struct WithBacktrace {
    err: Box<dyn error::Error + Send + Sync + 'static>,
    take_err: unsafe fn(Self, TypeId, NonNull<()>) -> Option<Self>,
    #[cfg(feature = "backtrace")]
    backtrace: std::backtrace::Backtrace,
}

#[cfg(feature = "backtrace")]
std::thread_local! {
    static SEARCHING: Cell<bool> = Cell::new(false);
}

#[cfg(feature = "backtrace")]
static DISABLED: AtomicBool = AtomicBool::new(false);

impl WithBacktrace {
    pub fn try_attach<E>(
        err: E,
    ) -> result::Result<
        impl error::Error + Send + Sync + 'static,
        impl error::Error + Send + Sync + 'static,
    >
    where
        E: error::Error + Send + Sync + 'static,
    {
        cfg_select! {
            feature = "backtrace" => {
                if DISABLED.load(Ordering::Relaxed) {
                    return Err(err);
                }
                match Self::search(|| err.source()) {
                    Some(_) => Err(err),
                    None => {
                        use std::backtrace::{Backtrace, BacktraceStatus};

                        let backtrace = Backtrace::capture();
                        match backtrace.status() {
                            BacktraceStatus::Captured => Ok(WithBacktrace {
                                err: Box::new(err),
                                take_err: Self::take_source_::<E>,
                                backtrace,
                            }),
                            _ => {
                                DISABLED.store(true, Ordering::Relaxed);
                                Err(err)
                            },
                        }
                    }
                }
            }
            _ => Err::<Self, _>(err),
        }
    }

    #[cfg(feature = "backtrace")]
    pub fn search<'a>(
        f: impl FnOnce() -> Option<&'a (dyn error::Error + 'static)>,
    ) -> Option<&'a std::backtrace::Backtrace> {
        struct DeferSetSearchingBacktrace(bool);

        impl Drop for DeferSetSearchingBacktrace {
            fn drop(&mut self) {
                SEARCHING.set(self.0);
            }
        }

        SEARCHING.set(true);
        let _reset_guard = DeferSetSearchingBacktrace(false);

        let mut source = f();
        while let Some(err) = source {
            if let Some(this) = err.downcast_ref::<WithBacktrace>() {
                return Some(&this.backtrace);
            }
            source = err.source();
        }
        None
    }

    pub fn searching() -> bool {
        cfg_select! {
            feature = "backtrace" => SEARCHING.get(),
            _ => false,
        }
    }

    pub fn search_debug<'a>(
        #[allow(unused_variables)] f: impl FnOnce() -> Option<&'a (dyn error::Error + 'static)>,
    ) -> Option<&'a dyn Backtrace> {
        cfg_select! {
            feature = "backtrace" => {
                Self::search(f).map(|b| b as _)
            }
            _ => None,
        }
    }

    pub fn search_display<'a>(
        #[allow(unused_variables)] f: impl FnOnce() -> Option<&'a (dyn error::Error + 'static)>,
    ) -> Option<&'a dyn Backtrace> {
        cfg_select! {
            feature = "backtrace" => {
                Self::search(f).map(|b| b as _)
            }
            _ => None,
        }
    }

    /// Take the error from this backtrace and put it in the dst pointer.
    ///
    /// # Safety
    ///
    /// The dst pointer must be valid and point to a valid Option<Ty>.
    #[cfg(feature = "backtrace")]
    unsafe fn take_source_<E>(self, ty: TypeId, dst: NonNull<()>) -> Option<Self>
    where
        E: error::Error + 'static,
    {
        if crate::rtti::is_same_ty::<E, Nae>() {
            return None;
        }
        if TypeId::of::<E>() != ty {
            return Some(self);
        }
        let this = self
            .err
            .downcast::<E>()
            .expect("WithBacktrace provides correct type");
        let dst = unsafe { dst.cast::<Option<E>>().as_mut() };
        dst.replace(*this);
        None
    }

    /// Take the error from this backtrace and put it in the dst pointer.
    ///
    /// # Safety
    ///
    /// The dst pointer must be valid and point to a valid Option<Ty>.
    pub unsafe fn take_source(self, ty: TypeId, dst: NonNull<()>) -> Option<Self> {
        unsafe { (self.take_err)(self, ty, dst) }
    }

    pub fn into_source(self) -> Option<Box<dyn error::Error + Send + Sync + 'static>> {
        if self.err.is::<Nae>() {
            return None;
        }
        Some(self.err)
    }
}

impl Debug for WithBacktrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<Backtrace>")
    }
}

impl Display for WithBacktrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<Backtrace>")
    }
}

impl error::Error for WithBacktrace {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        if self.err.is::<Nae>() {
            return None;
        }
        Some(&*self.err)
    }
}

pub trait Backtrace: Debug + Display {}

#[cfg(feature = "backtrace")]
impl Backtrace for std::backtrace::Backtrace {}
