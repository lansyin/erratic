use alloc::boxed::Box;
use core::{marker::PhantomData, mem::ManuallyDrop};

use crate::raw::ptr::{Mut, Ref};

use super::{Align4, Align4Ptr, Metadata};

/// An owned pointer with metadata attached.
///
/// # ABI
///
/// This type guarantees that the least significant 2 bits of its first byte encode a [`Metadata`].
#[repr(C)]
pub struct Align4Own<T> {
    ptr: Align4Ptr,
    _marker: PhantomData<Align4<T>>,
}

impl<T> Align4Own<T> {
    pub fn from_boxed(ptr: Box<Align4<T>>, meta: Metadata) -> Self {
        let ptr = Box::into_raw(ptr);
        Self {
            ptr: Align4Ptr::from_parts(ptr as *mut (), meta),
            _marker: PhantomData,
        }
    }

    /// Consumes `self` and returns the raw pointer.
    pub fn into_raw(self) -> *mut Align4<T> {
        // Note: The value is forgotten here to avoid double-free.
        let this = ManuallyDrop::new(self);
        this.ptr.to_parts().0 as *mut Align4<T>
    }

    /// Consumes `self` and returns the boxed value.
    pub fn into_boxed(self) -> Box<Align4<T>> {
        unsafe { Box::from_raw(self.into_raw()) }
    }

    /// Reinterprets the owned pointer as a different type `U`.
    ///
    /// # Safety
    ///
    /// `U` should have a layout compatible with `T`. If you are temporarily working with a type
    /// that has a different layout, you must cast it back to the original type before `drop` is called.
    pub unsafe fn cast<U>(self) -> ManuallyDrop<Align4Own<U>> {
        // Note: Forget the previous one to avoid double-free.
        let this = ManuallyDrop::new(self);

        ManuallyDrop::new(Align4Own {
            ptr: this.ptr,
            _marker: PhantomData,
        })
    }

    /// Returns a shared reference to the pointee.
    pub fn borrow(&self) -> Ref<'_, T> {
        let (addr, _) = self.ptr.to_parts();
        let ptr = addr.cast::<Align4<T>>();
        // Safety: `Align4Own` keeps the pointer valid while alive.
        unsafe { Ref::from_ptr(&raw const (*ptr).0) }
    }

    /// Returns a mutable reference to the pointee.
    pub fn borrow_mut(&mut self) -> Mut<'_, T> {
        let (addr, _) = self.ptr.to_parts();
        let ptr = addr.cast::<Align4<T>>();
        // Safety: `Align4Own` keeps the pointer valid while alive.
        unsafe { Mut::from_ptr(&raw mut (*ptr).0) }
    }
}

impl<T> Drop for Align4Own<T> {
    fn drop(&mut self) {
        unsafe {
            // Safety: The pointer was created from a `Box`.
            let _ = Box::from_raw(self.ptr.to_parts().0 as *mut Align4<T>);
        }
    }
}

/// # Safety
///
/// `Align4Own<T>` is an owned pointer, so it is `Send` iff `T` is `Send`.
unsafe impl<T> Send for Align4Own<T> where T: Send {}

/// # Safety
///
/// `Align4Own<T>` is an owned pointer, so it is `Sync` iff `T` is `Sync`.
unsafe impl<T> Sync for Align4Own<T> where T: Sync {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align4_own_boxed_round_trip() {
        let value = Box::new(Align4(42u32));
        let owned = Align4Own::from_boxed(value, Metadata::_1);
        let restored = owned.into_boxed();
        assert_eq!(restored.0, 42);
    }

    #[test]
    fn align4_own_cast_preserves_data() {
        let value = Box::new(Align4(0xABCD_EF01u32));
        let owned = Align4Own::from_boxed(value, Metadata::_2);
        // Cast to the same-layout type `[u8; 4]`
        let casted = unsafe { owned.cast::<[u8; 4]>() };
        assert_eq!(casted.borrow().deref(), &[0x01, 0xEF, 0xCD, 0xAB]);

        ManuallyDrop::into_inner(unsafe { (ManuallyDrop::into_inner(casted)).cast::<u32>() });
    }
}
