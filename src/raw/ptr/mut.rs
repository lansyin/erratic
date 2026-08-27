use core::{marker::PhantomData, ptr::NonNull};

use super::r#ref::Ref;
use crate::raw::align4::Align4;

/// Typed mutable reference wrapping a [`NonNull`] pointer.
pub struct Mut<'a, T> {
    /// # Safety Invariants
    ///
    /// `ptr` points to a valid value of type `T`.
    ptr: NonNull<T>,
    _marker: PhantomData<&'a mut Align4<T>>,
}

impl<'a, T> Mut<'a, T> {
    /// Creates from a pointer.
    ///
    /// # Panics
    ///
    /// Panics if the pointer is null.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a valid value of type `T`.
    pub unsafe fn from_ptr(ptr: *mut T) -> Self {
        Self {
            ptr: NonNull::new(ptr).expect("a non-null pointer"),
            _marker: PhantomData,
        }
    }

    /// Reinterprets the mutable reference as a different type `U`.
    ///
    /// # Safety
    ///
    /// `T` and `U` must have compatible layout.
    pub unsafe fn cast<U>(self) -> Mut<'a, U> {
        Mut {
            ptr: self.ptr.cast::<U>(),
            _marker: PhantomData,
        }
    }

    #[allow(dead_code)]
    pub fn reborrow(&mut self) -> Ref<'_, T> {
        // Safety: `self.ptr` is guaranteed to be valid.
        unsafe { Ref::from_ptr(self.ptr.as_ptr()) }
    }

    #[allow(dead_code)]
    pub fn reborrow_mut(&mut self) -> Mut<'_, T> {
        // Safety: `self.ptr` is guaranteed to be valid.
        unsafe { Mut::from_ptr(self.ptr.as_ptr()) }
    }

    /// Dereferences to a mutable reference.
    pub fn deref_mut(mut self) -> &'a mut T {
        // Safety: `self.ptr` is guaranteed to be valid.
        unsafe { self.ptr.as_mut() }
    }
}

/// # Safety
///
/// `&mut T` is `Send` if and only if `T` is `Send`.
unsafe impl<'a, T> Send for Mut<'a, T> where T: Send {}

/// # Safety
///
/// `&mut T` is `Sync` if and only if `T` is `Sync`.
unsafe impl<'a, T> Sync for Mut<'a, T> where T: Sync {}
