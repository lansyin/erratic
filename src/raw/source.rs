use alloc::boxed::Box;
use core::{
    any::{Any, TypeId},
    convert::Infallible,
    error::Error,
    ptr::NonNull,
    result,
};

use crate::raw::RawError;

use super::backtrace::WithBacktrace;

/// An error container that can be used as the source of [`RawError`].
pub trait Source: Any + Send + Sync + 'static {
    fn error_ref(&self) -> Option<&(dyn Error + Send + Sync + 'static)>;
    fn error_mut(&mut self) -> Option<&mut (dyn Error + Send + Sync + 'static)>;
    fn into_boxed(self) -> Option<Box<dyn Error + Send + Sync + 'static>>;

    /// Downcasts the source to its container type, e.g. `std::io::Error`, `ErasedRawError`,
    /// or `Box<dyn Error + Send + Sync + 'static>`.
    ///
    /// # Safety
    ///
    /// `dst` must be a valid pointer to a `Option<Ty>`.
    unsafe fn downcast_container(self, ty: TypeId, dst: NonNull<()>) -> Result<(), Self>
    where
        Self: Sized;

    fn into_backtrace(self) -> Option<WithBacktrace>
    where
        Self: Sized,
    {
        None
    }
}

impl<E> Source for E
where
    E: Error + Send + Sync + 'static,
{
    fn error_ref(&self) -> Option<&(dyn Error + Send + Sync + 'static)> {
        Some(self)
    }

    fn error_mut(&mut self) -> Option<&mut (dyn Error + Send + Sync + 'static)> {
        Some(self)
    }

    fn into_boxed(self) -> Option<Box<dyn Error + Send + Sync + 'static>> {
        Some(Box::new(self))
    }

    unsafe fn downcast_container(self, ty: TypeId, dst: NonNull<()>) -> Result<(), Self>
    where
        Self: Sized,
    {
        if ty == TypeId::of::<Self>() {
            let dst = unsafe { dst.cast::<Option<Self>>().as_mut() };
            dst.replace(self);
            Ok(())
        } else {
            Err(self)
        }
    }
}

pub struct NoSource;

impl Source for NoSource {
    fn error_ref(&self) -> Option<&(dyn Error + Send + Sync + 'static)> {
        None
    }

    fn error_mut(&mut self) -> Option<&mut (dyn Error + Send + Sync + 'static)> {
        None
    }

    fn into_boxed(self) -> Option<Box<dyn Error + Send + Sync + 'static>> {
        None
    }

    unsafe fn downcast_container(self, _ty: TypeId, _dst: NonNull<()>) -> Result<(), Self>
    where
        Self: Sized,
    {
        Err(self)
    }
}

pub struct BoxedSource(pub Box<dyn Error + Send + Sync + 'static>);

impl Source for BoxedSource {
    fn error_ref(&self) -> Option<&(dyn Error + Send + Sync + 'static)> {
        Some(&*self.0)
    }

    fn error_mut(&mut self) -> Option<&mut (dyn Error + Send + Sync + 'static)> {
        Some(&mut *self.0)
    }

    fn into_boxed(self) -> Option<Box<dyn Error + Send + Sync + 'static>> {
        Some(self.0)
    }

    unsafe fn downcast_container(self, ty: TypeId, dst: NonNull<()>) -> Result<(), Self>
    where
        Self: Sized,
    {
        if ty == TypeId::of::<Box<dyn Error + Send + Sync + 'static>>() {
            let dst = unsafe {
                dst.cast::<Option<Box<dyn Error + Send + Sync + 'static>>>()
                    .as_mut()
            };
            dst.replace(self.0);
            Ok(())
        } else {
            Err(self)
        }
    }
}

pub struct IndirectSource(RawError);

impl IndirectSource {
    pub(crate) fn try_new(raw: super::RawError) -> result::Result<Self, RawError> {
        if raw.is_source_only() {
            Ok(Self(raw))
        } else {
            Err(raw)
        }
    }
}

impl Source for IndirectSource {
    fn error_ref(&self) -> Option<&(dyn Error + Send + Sync + 'static)> {
        self.0.source()
    }

    fn error_mut(&mut self) -> Option<&mut (dyn Error + Send + Sync + 'static)> {
        self.0.source_mut()
    }

    fn into_boxed(self) -> Option<Box<dyn Error + Send + Sync + 'static>> {
        self.0.into_source()
    }

    unsafe fn downcast_container(self, ty: TypeId, dst: NonNull<()>) -> Result<(), Self>
    where
        Self: Sized,
    {
        if ty == TypeId::of::<RawError<Infallible>>() {
            let dst = unsafe { dst.cast::<Option<RawError<Infallible>>>().as_mut() };
            dst.replace(self.0);
            Ok(())
        } else {
            Err(self)
        }
    }
}

pub struct WithBacktraceSource(pub WithBacktrace);

impl Source for WithBacktraceSource {
    fn error_ref(&self) -> Option<&(dyn Error + Send + Sync + 'static)> {
        if WithBacktrace::searching() {
            Some(&self.0)
        } else {
            self.0.source()
        }
    }

    fn error_mut(&mut self) -> Option<&mut (dyn Error + Send + Sync + 'static)> {
        self.0.source_mut()
    }

    fn into_boxed(self) -> Option<Box<dyn Error + Send + Sync + 'static>> {
        self.0.into_source()
    }

    unsafe fn downcast_container(self, ty: TypeId, dst: NonNull<()>) -> Result<(), Self>
    where
        Self: Sized,
    {
        unsafe { self.0.take_source(ty, dst).map_err(Self) }
    }

    fn into_backtrace(self) -> Option<WithBacktrace>
    where
        Self: Sized,
    {
        Some(self.0)
    }
}
