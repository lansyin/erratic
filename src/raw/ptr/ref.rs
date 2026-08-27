use core::{marker::PhantomData, ptr::NonNull};

use crate::raw::align4::Align4;

/// Typed shared reference wrapping a [`NonNull`] pointer.
pub struct Ref<'a, T> {
    /// # Safety Invariants
    ///
    /// - `ptr` points to a valid value of type `T`.
    ptr: NonNull<T>,
    _marker: PhantomData<&'a Align4<T>>,
}

impl<'a, T> Ref<'a, T> {
    /// Creates from a non-null pointer.
    ///
    /// # Panics
    ///
    /// Panics if the pointer is null.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a valid value of type `T`.
    pub unsafe fn from_ptr(ptr: *const T) -> Self {
        Self {
            ptr: NonNull::new(ptr as *mut T).expect("a non-null pointer"),
            _marker: PhantomData,
        }
    }
    /// Reinterprets the reference as a different type `U`.
    ///
    /// # Safety
    ///
    /// `T` and `U` must have compatible layout.
    pub unsafe fn cast<U>(self) -> Ref<'a, U> {
        Ref {
            ptr: self.ptr.cast::<U>(),
            _marker: PhantomData,
        }
    }

    /// Dereferences to a shared reference.
    pub fn deref(&self) -> &'a T {
        // Safety: `self.ptr` is guaranteed to be valid.
        unsafe { self.ptr.as_ref() }
    }
}

impl<'a, T> Clone for Ref<'a, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T> Copy for Ref<'a, T> {}

/// # Safety
///
/// `&T` is `Send` if and only if `T` is `Sync`.
unsafe impl<'a, T> Send for Ref<'a, T> where T: Sync {}

/// # Safety
///
/// `&T` is `Sync` if and only if `T` is `Sync`.
unsafe impl<'a, T> Sync for Ref<'a, T> where T: Sync {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;

    use crate::raw::align4::Align4Own;
    use crate::raw::align4::Metadata;

    #[test]
    fn ref_deref_valid() {
        let value = Box::new(Align4(99u64));
        let owned = Align4Own::from_boxed(value, Metadata::_0);
        let r = owned.borrow();
        assert_eq!(*r.deref(), 99);
    }
}
