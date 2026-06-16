use alloc::boxed::Box;
use core::{
    any::Any,
    error::{self},
    fmt::{self, Debug, Display},
    result,
};

#[cfg(feature = "backtrace")]
extern crate std;

#[cfg(feature = "backtrace")]
use core::{
    cell::Cell,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::raw::source::{Source, WithBacktraceSource};

// TODO: Remove this workaround once Error::provide gets stabilized.
pub(crate) struct WithBacktrace {
    err: Box<dyn Source>,
    take_err: fn(Self, &mut dyn Any) -> Result<(), Self>,
    into_source: fn(Self) -> Option<Box<dyn error::Error + Send + Sync + 'static>>,
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
    pub fn try_attach<E>(err: E) -> result::Result<impl Source, E>
    where
        E: Source,
    {
        #[cfg(feature = "backtrace")]
        {
            if DISABLED.load(Ordering::Relaxed) {
                return Err(err);
            }
            match Self::search(|| err.error_ref().map(|e| e as _)) {
                Some(_) => Err(err),
                None => {
                    use std::backtrace::{Backtrace, BacktraceStatus};

                    let backtrace = Backtrace::capture();
                    match backtrace.status() {
                        BacktraceStatus::Captured => Ok(WithBacktraceSource(WithBacktrace {
                            err: Box::new(err),
                            take_err: Self::take_source_::<E>,
                            into_source: Self::into_source_::<E>,
                            backtrace,
                        })),
                        _ => {
                            DISABLED.store(true, Ordering::Relaxed);
                            Err(err)
                        }
                    }
                }
            }
        }
        #[cfg(not(feature = "backtrace"))]
        Err::<WithBacktraceSource, _>(err)
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
        #[cfg(feature = "backtrace")]
        {
            SEARCHING.get()
        }
        #[cfg(not(feature = "backtrace"))]
        {
            false
        }
    }

    pub fn search_debug<'a>(
        #[allow(unused_variables)] f: impl FnOnce() -> Option<&'a (dyn error::Error + 'static)>,
    ) -> Option<impl Debug + Display> {
        #[cfg(feature = "backtrace")]
        {
            Self::search(f)
        }
        #[cfg(not(feature = "backtrace"))]
        {
            None::<&i32>
        }
    }

    pub fn search_display<'a>(
        #[allow(unused_variables)] f: impl FnOnce() -> Option<&'a (dyn error::Error + 'static)>,
    ) -> Option<impl Debug + Display> {
        #[cfg(feature = "backtrace")]
        {
            Self::search(f)
        }
        #[cfg(not(feature = "backtrace"))]
        {
            None::<i32>
        }
    }

    /// Take the error from this backtrace and put it in the dst.
    #[cfg(feature = "backtrace")]
    fn take_source_<E>(self, dst: &mut dyn Any) -> Result<(), Self>
    where
        E: Source,
    {
        let this = (self.err as Box<dyn Any>)
            .downcast::<E>()
            .expect("WithBacktrace provides correct type");

        if let Err(err) = this.downcast_container(dst) {
            return Err(Self {
                err: Box::new(err),
                ..self
            });
        }

        Ok(())
    }

    #[cfg(feature = "backtrace")]
    fn into_source_<E>(self) -> Option<Box<dyn error::Error + Send + Sync + 'static>>
    where
        E: Source,
    {
        let this = (self.err as Box<dyn Any>)
            .downcast::<E>()
            .expect("WithBacktrace provides correct type");

        this.into_boxed()
    }

    /// Take the error from this backtrace and put it in the dst.
    pub fn take_source(self, dst: &mut dyn Any) -> Result<(), Self> {
        (self.take_err)(self, dst)
    }

    pub fn into_source(self) -> Option<Box<dyn error::Error + Send + Sync + 'static>> {
        (self.into_source)(self)
    }

    pub fn source(&self) -> Option<&(dyn error::Error + Send + Sync + 'static)> {
        self.err.error_ref()
    }

    pub fn source_mut(&mut self) -> Option<&mut (dyn error::Error + Send + Sync + 'static)> {
        self.err.error_mut()
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
        self.err.error_ref().map(|e| e as _)
    }
}
