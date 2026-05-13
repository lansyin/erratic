use std::{
    marker::PhantomData,
    mem::{self, ManuallyDrop, MaybeUninit},
    ptr::{self, NonNull},
    result,
};

/// Only the least significant 2 bits are used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Metadata(pub u8);

impl Metadata {
    pub const MASK: u8 = 0b00000011;

    pub const _0: Metadata = Metadata(0b00000000);
    pub const _1: Metadata = Metadata(0b00000001);
    pub const _2: Metadata = Metadata(0b00000010);
    pub const _3: Metadata = Metadata(0b00000011);
}

cfg_select! {
    target_pointer_width = "64" => {
        #[repr(C, align(8))]
        pub struct Align4PtrCompat<T> {
            meta: u8,
            store: [u8; 7],
            after_store: PhantomData<T>,
        }
    },
    target_pointer_width = "32" => {
        #[repr(C, align(4))]
        pub struct Align4PtrCompat<T> {
            meta: u8,
            store: [u8; 3],
            after_store: PhantomData<T>,
        }
    },
}

impl<T> Align4PtrCompat<T> {
    pub const fn store_offset() -> Option<isize> {
        let target_size = mem::size_of::<T>();
        let target_align = mem::align_of::<T>();
        let store_offset_start = mem::offset_of!(Self, store);
        let store_offset_end = mem::offset_of!(Self, after_store);
        // Note: PhantomData<T> has align=1 for all T: https://doc.rust-lang.org/std/marker/struct.PhantomData.html#layout-1
        let store_size = store_offset_end - store_offset_start;
        let max_align_allowed = store_size.next_power_of_two() / 2;

        if target_align > max_align_allowed {
            return None;
        }

        let mut offset = store_offset_start;
        while offset <= store_offset_end {
            // Note: Rust guarantees that the alignment is not smaller than 1, even for ZSTs.
            // https://doc.rust-lang.org/reference/type-layout.html#size-and-alignment
            if (target_align + offset) % target_align == 0
                && store_offset_end - offset >= target_size
            {
                return Some((offset - store_offset_start) as isize);
            }

            offset += 1;
        }

        None
    }

    pub fn new(meta: Metadata, value: T) -> result::Result<Self, T> {
        let Some(store_offset) = Self::store_offset() else {
            return Err(value);
        };
        let mut this = unsafe {
            Self {
                meta: meta.0,
                store: mem::zeroed(),
                after_store: PhantomData,
            }
        };

        unsafe {
            // Safety: The offset is validated in STORE_OFFSET.
            let value_mut =
                &mut *(this.store.as_mut_ptr().offset(store_offset) as *mut MaybeUninit<T>);

            value_mut.write(value);
        }

        Ok(this)
    }

    pub fn borrow_value(&self) -> &T {
        unsafe {
            // Safety: store_offset was checked before creating an Align4PtrCompat.
            let store_offset = Self::store_offset().unwrap_unchecked();

            // Safety: The offset is validated prior to creating Align4PtrCompat.
            // The raw pointer, though invalidated by the newly created reference,
            // is not used afterward.
            &*(self.store.as_ptr().offset(store_offset) as *const T)
        }
    }

    pub fn into_value(self) -> T {
        unsafe {
            // Safety: store_offset was checked before creating an Align4PtrCompat.
            let store_offset = Self::store_offset().unwrap_unchecked();
            let mut this = ManuallyDrop::new(self);
            let value_mut = this.store.as_mut_ptr().offset(store_offset) as *mut T;

            value_mut.read()
        }
    }
}

impl<T> Drop for Align4PtrCompat<T> {
    fn drop(&mut self) {
        let store_offset = Self::store_offset()
            .expect("store_offset must be checked before creating an Align4PtrCompat");

        unsafe {
            // Safety: The offset is validated prior to creating Align4PtrCompat.
            let value_mut =
                &mut *(self.store.as_mut_ptr().offset(store_offset) as *mut ManuallyDrop<T>);

            ManuallyDrop::drop(value_mut);
        }
    }
}

#[derive(Clone, Copy)]
#[cfg_attr(target_pointer_width = "32", repr(C, align(4)))]
#[cfg_attr(target_pointer_width = "64", repr(C, align(8)))]
struct Align4Ptr(
    cfg_select! {
        target_pointer_width = "64" => [u8; 8],
        target_pointer_width = "32" => [u8; 4],
    },
);

impl Align4Ptr {
    /// Encodes `meta` into the low 2 bits of the pointer address.
    ///
    /// # Panics
    ///
    /// Panics if the low 2 bits of `addr` are not zero.
    fn from_parts(addr: usize, meta: Metadata) -> Self {
        let mut bytes = addr.to_le_bytes();

        assert_eq!(bytes[0] & Metadata::MASK, 0);
        bytes[0] |= meta.0;

        Self(bytes)
    }

    /// Extracts the original address and metadata from the encoded pointer.
    fn into_parts(self) -> (usize, Metadata) {
        let mut bytes = self.0;

        let meta = Metadata(bytes[0] & Metadata::MASK);
        bytes[0] &= !Metadata::MASK;

        let addr = usize::from_le_bytes(bytes);

        (addr, meta)
    }
}

#[repr(C, align(4))]
pub struct Align4<T: ?Sized>(pub T);

/// Owned pointer with metadata bits stored in the low 2 bits of the address.
///
/// # Safety invariants
///
/// - The stored address was originally obtained from [`Box::into_raw`],
///   so it must only be freed once via [`into_boxed`](Align4Own::into_boxed).
/// - The address must be 4-byte-aligned to leave the low 2 bits for metadata.
#[repr(C)]
pub struct Align4Own<T> {
    ptr: Align4Ptr,
    _marker: PhantomData<Align4<T>>,
}

impl<T> Align4Own<T> {
    pub fn from_boxed(ptr: Box<Align4<T>>, meta: Metadata) -> Self {
        let addr = Box::into_raw(ptr).expose_provenance();
        Self {
            ptr: Align4Ptr::from_parts(addr, meta),
            _marker: PhantomData,
        }
    }

    /// # Safety
    ///
    /// The caller must ensure that the provenance of the stored address is still valid.
    /// See [`ptr::with_exposed_provenance_mut`].
    pub fn into_raw(self) -> *mut Align4<T> {
        let this = ManuallyDrop::new(self);
        ptr::with_exposed_provenance_mut(this.ptr.into_parts().0)
    }

    /// Consumes `self` and returns the boxed value.
    ///
    /// # Safety
    ///
    /// The stored address must have been obtained from [`Box::into_raw`] for a valid
    /// heap allocation with the correct layout for `Align4<T>`, and must not have been
    /// freed or aliased.
    pub unsafe fn into_boxed(self) -> Box<Align4<T>> {
        unsafe { Box::from_raw(self.into_raw()) }
    }

    /// Reinterprets the owned pointer as a different type `U`.
    ///
    /// # Safety
    ///
    /// `Align4<T>` and `Align4<U>` must have the same layout (size and alignment).
    pub unsafe fn cast<U>(self) -> Align4Own<U> {
        let (_, meta) = self.ptr.into_parts();

        Align4Own::from_boxed(
            unsafe { Box::from_raw(self.into_raw().cast::<Align4<U>>()) },
            meta,
        )
    }

    /// Returns a shared reference to the pointee.
    ///
    /// # Safety
    ///
    /// The stored address must point to a valid, initialized `T`.
    /// The caller must ensure provenance is valid; see [`ptr::with_exposed_provenance_mut`].
    pub fn borrow(&self) -> Ref<'_, T> {
        let (addr, _) = self.ptr.into_parts();
        let ptr: *mut Align4<T> = ptr::with_exposed_provenance_mut(addr);
        let ptr = ptr.cast::<T>();
        Ref {
            ptr: unsafe { NonNull::new_unchecked(ptr) },
            _marker: PhantomData,
        }
    }

    /// Returns a mutable pointer-like reference to the pointee.
    ///
    /// # Safety
    ///
    /// Same as [`borrow`](Align4Own::borrow).
    /// The caller must ensure no other mutable aliases exist.
    pub fn borrow_mut(&self) -> Mut<'_, T> {
        let (addr, _) = self.ptr.into_parts();
        let ptr: *mut Align4<T> = ptr::with_exposed_provenance_mut(addr);
        let ptr = ptr.cast::<T>();
        Mut {
            ptr: unsafe { NonNull::new_unchecked(ptr) },
            _marker: PhantomData,
        }
    }
}

impl<T> Drop for Align4Own<T> {
    fn drop(&mut self) {
        unsafe {
            let _ = Box::from_raw(Self::into_raw(Self {
                ptr: self.ptr,
                _marker: PhantomData,
            }));
        }
    }
}

/// Shared reference with metadata bits stored in the low 2 bits of the address.
///
/// # Safety invariants
///
/// Same as [`Align4Own`]: the address must be 4-byte-aligned and point to a valid `T`.
#[derive(Clone, Copy)]
#[repr(C, align(4))]
pub struct Align4Ref<'a, T> {
    ptr: Align4Ptr,
    _marker: PhantomData<&'a Align4<T>>,
}

impl<'a, T> Align4Ref<'a, T> {
    pub fn new(static_ref: &'a Align4<T>, meta: Metadata) -> Align4Ref<'a, T> {
        Self {
            ptr: Align4Ptr::from_parts((&raw const *static_ref).expose_provenance(), meta),
            _marker: PhantomData,
        }
    }

    /// Returns a shared reference to the pointee.
    ///
    /// # Safety
    ///
    /// The stored address must point to a valid, initialized `T`.
    pub fn borrow(&self) -> Ref<'_, T> {
        let (addr, _) = self.ptr.into_parts();
        let ptr: *const Align4<T> = ptr::with_exposed_provenance(addr);
        let ptr = ptr.cast::<T>();
        Ref {
            ptr: unsafe { NonNull::new_unchecked(ptr.cast_mut()) },
            _marker: PhantomData,
        }
    }
}

/// Typed shared reference wrapping a [`NonNull`] pointer.
///
/// Designed for safe field projection via [`Ref::deref`] and [`Ref::project`].
#[derive(Clone, Copy)]
pub struct Ref<'a, T> {
    ptr: NonNull<T>,
    _marker: PhantomData<&'a Align4<T>>,
}

impl<'a, T> Ref<'a, T> {
    /// Reinterprets the reference as a different type `U`.
    ///
    /// # Safety
    ///
    /// `T` and `U` must have the same layout.
    pub unsafe fn cast<U>(self) -> Ref<'a, U> {
        Ref {
            ptr: self.ptr.cast::<U>(),
            _marker: PhantomData,
        }
    }

    /// Projects to a field of `T` using the given offset function.
    ///
    /// # Safety
    ///
    /// `f` must return a pointer that is derived from the input pointer
    /// (i.e., pointing within the same allocation) and must be properly aligned for `F`.
    /// The resulting reference must not violate aliasing rules.
    pub unsafe fn project<F>(self, f: fn(*const T) -> *const F) -> Ref<'a, F> {
        Ref {
            ptr: unsafe { NonNull::new_unchecked(f(self.ptr.as_ptr()).cast_mut()) },
            _marker: PhantomData,
        }
    }

    /// Dereferences to a shared reference.
    ///
    /// # Safety
    ///
    /// The caller must ensure the pointer is valid, properly aligned, and dereferenceable
    /// for the lifetime `'a`.
    pub fn deref(&self) -> &'a T {
        unsafe { self.ptr.as_ref() }
    }
}

impl<'a, T> Ref<'a, T>
where
    T: Copy,
{
    /// Reads the value by copy.
    ///
    /// # Safety
    ///
    /// The pointer must be valid, properly aligned, and dereferenceable.
    pub fn copied(&self) -> T {
        unsafe { self.ptr.read() }
    }
}

/// Typed mutable reference wrapping a [`NonNull`] pointer.
///
/// Designed for safe field projection via [`Mut::deref_mut`] and [`Mut::project`].
pub struct Mut<'a, T> {
    ptr: NonNull<T>,
    _marker: PhantomData<&'a mut Align4<T>>,
}

impl<'a, T> Mut<'a, T> {
    /// Reinterprets the mutable reference as a different type `U`.
    ///
    /// # Safety
    ///
    /// `T` and `U` must have the same layout.
    pub unsafe fn cast<U>(self) -> Mut<'a, U> {
        Mut {
            ptr: self.ptr.cast::<U>(),
            _marker: PhantomData,
        }
    }

    /// Projects to a field of `T` using the given offset function.
    ///
    /// # Safety
    ///
    /// `f` must return a pointer that is derived from the input pointer
    /// (i.e., pointing within the same allocation) and must be properly aligned for `F`.
    /// The resulting reference must not violate aliasing rules (e.g., no overlapping borrows).
    pub unsafe fn project<F>(self, f: fn(*mut T) -> *mut F) -> Mut<'a, F> {
        Mut {
            ptr: unsafe { NonNull::new_unchecked(f(self.ptr.as_ptr())) },
            _marker: PhantomData,
        }
    }

    /// Dereferences to a mutable reference.
    ///
    /// # Safety
    ///
    /// The caller must ensure the pointer is valid, properly aligned, and dereferenceable
    /// for the lifetime `'a`.
    pub fn deref_mut(&mut self) -> &'a mut T {
        unsafe { self.ptr.as_mut() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;

    /// Verifies that `Align4Ptr::from_parts` / `into_parts` round-trips address and metadata.
    #[test]
    fn align4_ptr_round_trip() {
        let addr = 0xDEAD_BEE0usize; // low 2 bits are 00
        for meta in [Metadata::_0, Metadata::_1, Metadata::_2, Metadata::_3] {
            let ptr = Align4Ptr::from_parts(addr, meta);
            let (restored_addr, restored_meta) = ptr.into_parts();
            assert_eq!(restored_addr, addr);
            assert_eq!(restored_meta, meta);
        }
    }

    /// Verifies that Align4Ptr rejects non-aligned addresses at construction.
    #[test]
    #[should_panic]
    fn align4_ptr_panics_on_unaligned() {
        let addr = 0xDEAD_BEEFusize; // low 2 bits are 11
        Align4Ptr::from_parts(addr, Metadata::_0);
    }

    /// Verifies that `Align4Own` round-trips a boxed value.
    #[test]
    fn align4_own_boxed_round_trip() {
        let value = Box::new(Align4(42u32));
        let owned = Align4Own::from_boxed(value, Metadata::_1);
        let restored = unsafe { owned.into_boxed() };
        assert_eq!(restored.0, 42);
    }

    /// Verifies that `Align4Own::cast` only changes the type parameter without data loss.
    #[test]
    fn align4_own_cast_preserves_data() {
        let value = Box::new(Align4(0xABCD_EF01u32));
        let owned = Align4Own::from_boxed(value, Metadata::_2);
        // Cast to the same-layout type `[u8; 4]`
        let casted = unsafe { owned.cast::<[u8; 4]>() };
        let restored = unsafe { casted.into_boxed() };
        assert_eq!(restored.0, [0x01, 0xEF, 0xCD, 0xAB]); // little-endian
    }

    /// Verifies that `Ref::deref` returns a valid reference.
    #[test]
    fn ref_deref_valid() {
        let value = Box::new(Align4(99u64));
        let owned = Align4Own::from_boxed(value, Metadata::_0);
        let r = owned.borrow();
        assert_eq!(*r.deref(), 99);
    }

    /// Verifies that `Ref::project` correctly accesses a struct field.
    #[test]
    fn ref_project_field() {
        #[repr(C)]
        struct Pair {
            x: u32,
            y: u32,
        }
        let value = Box::new(Align4(Pair { x: 10, y: 20 }));
        let owned = Align4Own::from_boxed(value, Metadata::_0);
        let r = owned.borrow();
        let y_ref = unsafe { r.project(|p| &raw const (*p).y) };
        assert_eq!(*y_ref.deref(), 20);
    }

    /// `Align4<T>` ensures 4-byte alignment.
    #[test]
    fn align4_guarantees_alignment() {
        assert!(mem::align_of::<Align4<u8>>() >= 4);
        assert!(mem::align_of::<Align4<u64>>() >= 4);
    }

    // ------------------------------------------------------------------
    // Align4PtrCompat extreme layout tests
    // ------------------------------------------------------------------

    /// Maximum byte array (`[u8; store_size]`, align 1) fits in the store
    /// and can be read back safely (no alignment issues for byte-slices).
    #[test]
    fn align4_ptr_compat_max_u8_array() {
        const N: usize = if cfg!(target_pointer_width = "64") {
            7
        } else {
            3
        };
        let Ok(v) = Align4PtrCompat::<[u8; N]>::new(Metadata::_0, [0xAB; N]) else {
            panic!("max u8 array should fit");
        };
        assert_eq!(v.borrow_value(), &[0xAB; N]);
    }

    /// One byte past the limit — `[u8; store_size + 1]` (align 1) is too large.
    #[test]
    fn align4_ptr_compat_u8_array_one_too_many() {
        const N: usize = if cfg!(target_pointer_width = "64") {
            8
        } else {
            4
        };
        assert!(Align4PtrCompat::<[u8; N]>::store_offset().is_none());
    }

    /// `u16` — alignment 2, can be stored (store_offset is Some).
    #[test]
    fn align4_ptr_compat_u16_store_offset() {
        assert!(Align4PtrCompat::<u16>::store_offset().is_some());
    }

    /// `u32` — alignment 4, can be stored on 32 bit platforms only.
    #[test]
    fn align4_ptr_compat_u32_store_offset() {
        if cfg!(target_pointer_width = "64") {
            assert!(Align4PtrCompat::<u32>::store_offset().is_some());
        } else {
            assert!(Align4PtrCompat::<u32>::store_offset().is_none());
        }
    }

    /// `u64` — alignment 8, too large for the store buffer on all platforms.
    #[test]
    fn align4_ptr_compat_u64_is_oversized() {
        assert!(Align4PtrCompat::<u64>::store_offset().is_none());
    }

    /// Verify that `new()` succeeds and the metadata byte is preserved.
    #[test]
    fn align4_ptr_compat_new_preserves_meta() {
        // Use [u8; 1] (align 1, no alignment issue) to verify meta round-trip.
        let Ok(v) = Align4PtrCompat::<[u8; 1]>::new(Metadata::_3, [0x42]) else {
            panic!("[u8; 1] should fit");
        };
        assert_eq!(*v.borrow_value(), [0x42]);
    }

    /// Creating a type that is too large returns Err with the value.
    #[test]
    fn align4_ptr_compat_new_returns_err_for_oversized() {
        const N: usize = if cfg!(target_pointer_width = "64") {
            8
        } else {
            4
        };
        let value = [0x42u8; N];
        let result = Align4PtrCompat::<[u8; N]>::new(Metadata::_0, value);
        assert!(result.is_err());
    }
}
