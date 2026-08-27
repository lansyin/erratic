use core::marker::PhantomData;

use crate::raw::ptr::Ref;

use super::{Align4, Align4Ptr, Metadata};

/// A shared pointer with metadata attached.
///
/// # ABI
///
/// This type guarantees that the least significant 2 bits of its first byte encode a [`Metadata`].
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Align4Ref<'a, T> {
    ptr: Align4Ptr,
    _marker: PhantomData<&'a Align4<T>>,
}

impl<'a, T> Align4Ref<'a, T> {
    pub fn new(ref_: &'a Align4<T>, meta: Metadata) -> Align4Ref<'a, T> {
        Self {
            ptr: Align4Ptr::from_parts((&raw const *ref_) as *mut (), meta),
            _marker: PhantomData,
        }
    }

    pub fn metadata(&self) -> Metadata {
        self.ptr.to_parts().1
    }

    /// Returns a shared reference to the pointee.
    pub fn borrow(&self) -> Ref<'a, T> {
        let (addr, _) = self.ptr.to_parts();
        let ptr = addr.cast::<Align4<T>>();
        // Safety: `Align4Ref` keeps the pointer valid while alive.
        unsafe { Ref::from_ptr(&raw const (*ptr).0) }
    }

    /// Returns a shared reference to the pointee, with the `Align4` wrapper.
    pub fn borrow_raw(&self) -> Ref<'a, Align4<T>> {
        let (addr, _) = self.ptr.to_parts();
        let ptr = addr.cast::<Align4<T>>();
        // Safety: `Align4Ref` keeps the pointer valid while alive.
        unsafe { Ref::from_ptr(ptr) }
    }
}

/// # Safety
///
/// `&T` is `Send` if and only if `T` is `Sync`.
unsafe impl<'a, T> Send for Align4Ref<'a, T> where T: Sync {}

/// # Safety
///
/// `&T` is `Sync` if and only if `T` is `Sync`.
unsafe impl<'a, T> Sync for Align4Ref<'a, T> where T: Sync {}
