use alloc::boxed::Box;
use core::{any::Any, error::Error, result};

use crate::raw::RawError;

use super::backtrace::WithBacktrace;

/// An error container that can be used as the source of [`RawError`].
pub trait Source: Any + Send + Sync + 'static {
    fn error_ref(&self) -> Option<&(dyn Error + Send + Sync + 'static)>;
    fn error_mut(&mut self) -> Option<&mut (dyn Error + Send + Sync + 'static)>;
    fn into_boxed(self) -> Option<Box<dyn Error + Send + Sync + 'static>>;

    /// Downcasts the source to its container type, e.g. `std::io::Error`, `ErasedRawError`,
    /// or `Box<dyn Error + Send + Sync + 'static>`.
    fn downcast_container(self, dst: &mut dyn Any) -> Result<(), Self>
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

    fn downcast_container(self, dst: &mut dyn Any) -> Result<(), Self>
    where
        Self: Sized,
    {
        if let Some(dst) = dst.downcast_mut::<Option<Self>>() {
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

    fn downcast_container(self, _dst: &mut dyn Any) -> Result<(), Self>
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

    fn downcast_container(self, dst: &mut dyn Any) -> Result<(), Self>
    where
        Self: Sized,
    {
        if let Some(dst) = dst.downcast_mut::<Option<Box<dyn Error + Send + Sync + 'static>>>() {
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

    fn downcast_container(self, dst: &mut dyn Any) -> Result<(), Self>
    where
        Self: Sized,
    {
        if let Some(dst) = dst.downcast_mut::<Option<RawError>>() {
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

    fn downcast_container(self, dst: &mut dyn Any) -> Result<(), Self>
    where
        Self: Sized,
    {
        self.0.take_source(dst).map_err(Self)
    }

    fn into_backtrace(self) -> Option<WithBacktrace>
    where
        Self: Sized,
    {
        Some(self.0)
    }
}
