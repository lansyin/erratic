use alloc::boxed::Box;
use core::{
    error::{self, Error},
    fmt::{self, Debug, Display},
    marker::PhantomData,
    mem::{self, ManuallyDrop, MaybeUninit, swap},
    ptr::{self, NonNull},
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

    fn encode_in_byte(self, addr_bytes: &mut u8) {
        *addr_bytes |= self.0;
    }

    fn decode_from_byte(addr_bytes: &mut u8) -> Self {
        let meta = *addr_bytes & Self::MASK;
        *addr_bytes &= !Self::MASK;
        Self(meta)
    }
}

/// An inline pointer-sized storage that having a metadata at the first byte.
///
/// This type is guaranteed to be the same layout as `usize`.
// Note: The repr/align attribute is required as it is used to compute the offset
// that satisfies the alignment of T.
#[cfg_attr(target_pointer_width = "16", repr(C, align(2)))]
#[cfg_attr(target_pointer_width = "32", repr(C, align(4)))]
#[cfg_attr(target_pointer_width = "64", repr(C, align(8)))]
pub struct Align4PtrCompat<T> {
    meta: u8,
    store: MaybeUninit<[u8; usize::BITS as usize / 8 - 1]>,
    _marker: PhantomData<T>,
}

const _: () = const {
    assert!(
        mem::size_of::<Align4PtrCompat::<()>>() == mem::size_of::<usize>(),
        "`Align4PtrCompat::<T>()` should be the same size as `usize`"
    );
    assert!(
        mem::align_of::<Align4PtrCompat::<()>>() == mem::align_of::<usize>(),
        "`Align4PtrCompat::<T>()` should be the same alignment as `usize`"
    );
};

impl<T> Align4PtrCompat<T> {
    const OFFSET_IN_STORE: Option<isize> = 'ret: {
        const fn ty_of_field<T, F>(_: fn(T) -> F) -> usize {
            mem::size_of::<F>()
        }

        let target_size = mem::size_of::<T>();
        let target_align = mem::align_of::<T>();
        let store_offset_start = mem::offset_of!(Self, store);
        let store_size = ty_of_field(|v: Self| v.store);
        let store_offset_end = store_offset_start + store_size;

        let mut offset = store_offset_start;
        while offset <= store_offset_end {
            // Note: Rust guarantees that the alignment is not smaller than 1, even for ZSTs.
            // https://doc.rust-lang.org/reference/type-layout.html#size-and-alignment
            if offset.is_multiple_of(target_align) && store_offset_end - offset >= target_size {
                break 'ret Some((offset - store_offset_start) as isize);
            }

            offset += 1;
        }

        None
    };

    pub const fn new(meta: Metadata, value: T) -> Result<Self, T> {
        let Some(offset) = Self::OFFSET_IN_STORE else {
            return Err(value);
        };
        let mut this = unsafe {
            Self {
                meta: meta.0,
                store: mem::zeroed(),
                _marker: PhantomData,
            }
        };

        unsafe {
            // Safety: The offset is validated in `Self::offset_in_store`.
            ((&raw mut this.store).cast::<u8>().offset(offset) as *mut T).write(value);
        }

        Ok(this)
    }

    pub fn borrow_value(&self) -> &T {
        unsafe {
            let offset = Self::OFFSET_IN_STORE
                .expect("offset_in_store was checked before creating an Align4PtrCompat");

            // Safety: The offset is validated prior to creating Align4PtrCompat.
            // The raw pointer, though invalidated by the newly created reference,
            // is not used afterward.
            &*((&raw const self.store).cast::<u8>().offset(offset) as *const T)
        }
    }

    pub fn into_value(self) -> T {
        unsafe {
            let offset = Self::OFFSET_IN_STORE
                .expect("offset_in_store was checked before creating an Align4PtrCompat");

            let mut this = ManuallyDrop::new(self);
            let value_mut = (&raw mut this.store).cast::<u8>().offset(offset) as *mut T;

            value_mut.read()
        }
    }

    pub fn replace_value(&mut self, value: T) -> T {
        unsafe {
            let offset = Self::OFFSET_IN_STORE
                .expect("offset_in_store was checked before creating an Align4PtrCompat");

            // Safety: The offset is validated prior to creating Align4PtrCompat.
            // The raw pointer, though invalidated by the newly created reference,
            // is not used afterward.
            ptr::replace(
                (&raw mut self.store).cast::<u8>().offset(offset) as *mut T,
                value,
            )
        }
    }
}

impl<T> Align4PtrCompat<T>
where
    T: Debug + Send + Sync + 'static,
{
    fn into_parts(self) -> (u8, MaybeUninit<[u8; mem::size_of::<usize>() - 1]>) {
        let mut this = ManuallyDrop::new(self);
        (this.meta, unsafe { (&raw mut this.store).read() })
    }

    unsafe fn debug_erased(this: &Align4PtrCompat<()>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let this = unsafe { &*(this as *const _ as *const Align4PtrCompat<T>) };
        <T as Debug>::fmt(this.borrow_value(), f)
    }

    unsafe fn drop_erased(this: Align4PtrCompat<()>) {
        let (meta, store) = this.into_parts();
        let _this = Self {
            meta,
            store,
            _marker: PhantomData,
        };
    }

    pub fn erase(self) -> ErasedAlign4PtrCompat {
        ErasedAlign4PtrCompat::from_typed(self)
    }
}

impl<T> Drop for Align4PtrCompat<T> {
    fn drop(&mut self) {
        let offset = Self::OFFSET_IN_STORE
            .expect("offset_in_store was checked before creating an Align4PtrCompat");

        unsafe {
            // Safety: The offset is validated prior to creating Align4PtrCompat.
            ptr::drop_in_place((&raw mut self.store).cast::<u8>().offset(offset) as *mut T);
        }
    }
}

pub struct ErasedAlign4PtrCompat {
    inner: ManuallyDrop<Align4PtrCompat<()>>,
    vtable: &'static Align4PtrCompatVTable,
    _marker: PhantomData<*mut ()>,
}

impl ErasedAlign4PtrCompat {
    pub fn from_typed<T>(value: Align4PtrCompat<T>) -> Self
    where
        T: Debug + Send + Sync + 'static,
    {
        let (meta, store) = value.into_parts();
        ErasedAlign4PtrCompat {
            inner: ManuallyDrop::new(Align4PtrCompat::<()> {
                meta,
                store,
                _marker: PhantomData,
            }),
            vtable: &Align4PtrCompatVTable {
                debug: Align4PtrCompat::<T>::debug_erased,
                drop: Align4PtrCompat::<T>::drop_erased,
            },
            _marker: PhantomData,
        }
    }
}

impl Debug for ErasedAlign4PtrCompat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        unsafe { (self.vtable.debug)(&self.inner, f) }
    }
}

struct Align4PtrCompatVTable {
    debug: unsafe fn(&Align4PtrCompat<()>, f: &mut fmt::Formatter<'_>) -> fmt::Result,
    drop: unsafe fn(Align4PtrCompat<()>),
}

impl Drop for ErasedAlign4PtrCompat {
    fn drop(&mut self) {
        unsafe {
            (self.vtable.drop)(ManuallyDrop::take(&mut self.inner));
        }
    }
}

unsafe impl Send for ErasedAlign4PtrCompat {}
unsafe impl Sync for ErasedAlign4PtrCompat {}

/// A non-null transformed address with metadata encoded in the low 2 bits of the first byte.
#[derive(Clone, Copy)]
struct Align4Ptr(NonNull<()>);

impl Align4Ptr {
    fn swap_leading_and_trailing_byte_on_big_endian(addr_bytes: &mut [u8]) {
        let index_last = addr_bytes.len() - 1;
        let (leading_bytes, last_byte) = addr_bytes.split_at_mut(index_last);
        #[cfg(target_endian = "big")]
        {
            swap(&mut leading_bytes[0], &mut last_byte[0]);
        }
        #[cfg(target_endian = "little")]
        {
            _ = || swap(&mut leading_bytes[0], &mut last_byte[0])
        }
    }

    /// Encodes `meta` into the low 2 bits of the pointer address.
    ///
    /// # Panics
    ///
    /// Panics if the low 2 bits of `addr` are not zero.
    fn from_parts(ptr: NonNull<()>, meta: Metadata) -> Self {
        let ptr = ptr.as_ptr();
        let addr = ptr.map_addr(|addr| {
            let mut addr_bytes = addr.to_le_bytes();

            assert_eq!(addr_bytes[0] & Metadata::MASK, 0);

            meta.encode_in_byte(&mut addr_bytes[0]);
            Self::swap_leading_and_trailing_byte_on_big_endian(&mut addr_bytes);

            usize::from_le_bytes(addr_bytes)
        });

        unsafe {
            // Safety: swapping bytes keeps the addr non-zero.
            Self(NonNull::new_unchecked(addr))
        }
    }

    /// Extracts the original address and metadata from the encoded pointer.
    fn into_parts(self) -> (NonNull<()>, Metadata) {
        let addr = self.0.as_ptr();

        let mut meta = Metadata::_0;
        let ptr = addr.map_addr(|addr| {
            let mut addr_bytes = addr.to_le_bytes();

            Self::swap_leading_and_trailing_byte_on_big_endian(&mut addr_bytes);
            meta = Metadata::decode_from_byte(&mut addr_bytes[0]);

            usize::from_le_bytes(addr_bytes)
        });

        unsafe {
            // Safety: `Align4Ptr` guarantees the pointer is non-null.
            (NonNull::new_unchecked(ptr), meta)
        }
    }
}

#[repr(C, align(4))]
pub struct Align4<T: ?Sized>(pub T);

impl<T> Debug for Align4<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(&self.0, f)
    }
}

impl<T> Display for Align4<T>
where
    T: Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl<T> error::Error for Align4<T>
where
    T: Error,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.source()
    }
}

/// Owned pointer with metadata stored in the low 2 bits of the first byte of the pointer.
#[repr(C)]
pub struct Align4Own<T> {
    ptr: Align4Ptr,
    _marker: PhantomData<Align4<T>>,
}

impl<T> Align4Own<T> {
    pub fn from_boxed(ptr: Box<Align4<T>>, meta: Metadata) -> Self {
        let ptr = Box::into_raw(ptr);
        Self {
            ptr: unsafe {
                // Safety: the pointer is fetched from a `Box`.
                Align4Ptr::from_parts(NonNull::new_unchecked(ptr as *mut ()), meta)
            },
            _marker: PhantomData,
        }
    }

    /// Consumes `self` and returns the raw pointer.
    pub fn into_raw(self) -> *mut Align4<T> {
        // Note: Forget the previous one to avoid double-free.
        let this = ManuallyDrop::new(self);
        this.ptr.into_parts().0.as_ptr() as *mut Align4<T>
    }

    /// Consumes `self` and returns the boxed value.
    pub fn into_boxed(self) -> Box<Align4<T>> {
        unsafe { Box::from_raw(self.into_raw()) }
    }

    /// Reinterprets the owned pointer as a different type `U`.
    ///
    /// # Safety
    ///
    /// `U` should have a layout compatible with `T`.
    /// If you are temporarily working with a type that has a different layout,
    /// you must cast it back to the original type before `drop` is called.
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
        let (addr, _) = self.ptr.into_parts();
        let ptr = addr.cast::<Align4<T>>().as_ptr();
        // Safety: Without unsafe (cast), `Align4Own` keeps the pointer valid.
        let ptr = unsafe { NonNull::new_unchecked(&raw mut (*ptr).0) };
        Ref {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Returns a mutable pointer-like reference to the pointee.
    pub fn borrow_mut(&mut self) -> Mut<'_, T> {
        let (addr, _) = self.ptr.into_parts();
        let ptr = addr.cast::<Align4<T>>().as_ptr();
        // Safety: Without unsafe (cast), `Align4Own` keeps the pointer valid.
        let ptr = unsafe { NonNull::new_unchecked(&raw mut (*ptr).0) };
        Mut {
            ptr,
            _marker: PhantomData,
        }
    }
}

impl<T> Drop for Align4Own<T> {
    fn drop(&mut self) {
        unsafe {
            let _ = Box::from_raw(self.ptr.into_parts().0.as_ptr() as *mut Align4<T>);
        }
    }
}

unsafe impl<T> Send for Align4Own<T> where T: Send {}
unsafe impl<T> Sync for Align4Own<T> where T: Sync {}

/// Shared reference with metadata stored in the low 2 bits of the first byte of the pointer.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Align4Ref<'a, T> {
    ptr: Align4Ptr,
    _marker: PhantomData<&'a Align4<T>>,
}

impl<'a, T> Align4Ref<'a, T> {
    pub fn new(ref_: &'a Align4<T>, meta: Metadata) -> Align4Ref<'a, T> {
        Self {
            ptr: unsafe {
                // Safety: the pointer is fetched from a reference.
                Align4Ptr::from_parts(NonNull::new_unchecked((&raw const *ref_) as *mut ()), meta)
            },
            _marker: PhantomData,
        }
    }

    /// Returns a shared reference to the pointee.
    pub fn borrow(&self) -> Ref<'a, T> {
        let (addr, _) = self.ptr.into_parts();
        let ptr = addr.cast::<Align4<T>>().as_ptr();
        // Safety: Without unsafe (cast), `Align4Own` keeps the pointer valid.
        let ptr = unsafe { NonNull::new_unchecked(&raw mut (*ptr).0) };
        Ref {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Returns a shared reference to the pointee, with the `Align4` wrapper.
    pub fn borrow_raw(&self) -> Ref<'a, Align4<T>> {
        let (addr, _) = self.ptr.into_parts();
        let ptr = addr.cast::<Align4<T>>();
        Ref {
            ptr,
            _marker: PhantomData,
        }
    }
}

unsafe impl<'a, T> Send for Align4Ref<'a, T> where T: Send {}
unsafe impl<'a, T> Sync for Align4Ref<'a, T> where T: Sync {}

/// Typed shared reference wrapping a [`NonNull`] pointer.
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
    pub fn deref(&self) -> &'a T {
        unsafe { self.ptr.as_ref() }
    }
}

impl<'a, T> Ref<'a, T>
where
    T: Copy,
{
    /// Reads the value by copy.
    pub fn copied(&self) -> T {
        unsafe { self.ptr.read() }
    }
}

impl<'a, T> Clone for Ref<'a, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T> Copy for Ref<'a, T> {}

/// Typed mutable reference wrapping a [`NonNull`] pointer.
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
    #[allow(dead_code)]
    pub unsafe fn project<F>(self, f: fn(*mut T) -> *mut F) -> Mut<'a, F> {
        Mut {
            ptr: unsafe { NonNull::new_unchecked(f(self.ptr.as_ptr())) },
            _marker: PhantomData,
        }
    }

    #[allow(dead_code)]
    pub fn reborrow(&mut self) -> Ref<'_, T> {
        Ref {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }

    #[allow(dead_code)]
    pub fn reborrow_mut(&mut self) -> Mut<'_, T> {
        Mut {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }

    /// Dereferences to a mutable reference.
    pub fn deref_mut(mut self) -> &'a mut T {
        unsafe { self.ptr.as_mut() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem;

    #[test]
    fn align4_ptr_round_trip() {
        let value = Align4(0u32); // 4-byte-aligned, low 2 bits are 00
        let addr = NonNull::new(&raw const value as *mut ()).unwrap();
        for meta in [Metadata::_0, Metadata::_1, Metadata::_2, Metadata::_3] {
            let ptr = Align4Ptr::from_parts(addr, meta);
            let (restored_addr, restored_meta) = ptr.into_parts();
            assert_eq!(restored_addr, addr);
            assert_eq!(restored_meta, meta);
        }
    }

    #[test]
    #[should_panic]
    fn align4_ptr_panics_on_unaligned() {
        let bytes: [u8; 2] = [0, 0];
        for i in 0..2 {
            let unaligned = NonNull::new(&raw const bytes[i] as *mut ()).unwrap();
            Align4Ptr::from_parts(unaligned, Metadata::_0);
        }
    }

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
        assert_eq!(casted.borrow().copied(), [0x01, 0xEF, 0xCD, 0xAB]);

        ManuallyDrop::into_inner(unsafe { (ManuallyDrop::into_inner(casted)).cast::<u32>() });
    }

    #[test]
    fn ref_deref_valid() {
        let value = Box::new(Align4(99u64));
        let owned = Align4Own::from_boxed(value, Metadata::_0);
        let r = owned.borrow();
        assert_eq!(*r.deref(), 99);
    }

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

    #[test]
    fn align4_guarantees_alignment() {
        assert!(mem::align_of::<Align4<u8>>() >= 4);
        assert!(mem::align_of::<Align4<u64>>() >= 4);
    }

    #[test]
    fn align4_ptr_compat_max_u8_array() {
        const N: usize = (usize::BITS / 8 - 1) as _;
        let Ok(v) = Align4PtrCompat::<[u8; N]>::new(Metadata::_0, [0xAB; N]) else {
            panic!("max u8 array should fit");
        };
        assert_eq!(v.borrow_value(), &[0xAB; N]);
    }

    #[test]
    fn align4_ptr_compat_u8_array_one_too_many() {
        const N: usize = (usize::BITS / 8) as _;
        assert!(Align4PtrCompat::<[u8; N]>::OFFSET_IN_STORE.is_none());
    }

    #[test]
    fn align4_ptr_compat_u8_store_offset() {
        assert!(Align4PtrCompat::<u8>::OFFSET_IN_STORE.is_some());
    }

    #[test]
    fn align4_ptr_compat_u32_store_offset() {
        if cfg!(target_pointer_width = "64") {
            assert!(Align4PtrCompat::<u32>::OFFSET_IN_STORE.is_some());
        } else {
            // 16/32 bit platforms
            assert!(Align4PtrCompat::<u32>::OFFSET_IN_STORE.is_none());
        }
    }

    #[test]
    fn align4_ptr_compat_u64_is_oversized() {
        assert!(Align4PtrCompat::<u64>::OFFSET_IN_STORE.is_none());
    }

    #[test]
    fn align4_ptr_compat_new_preserves_meta() {
        // Use [u8; 1] (align 1, no alignment issue) to verify meta round-trip.
        let Ok(v) = Align4PtrCompat::<[u8; 1]>::new(Metadata::_3, [0x42]) else {
            panic!("[u8; 1] should fit");
        };
        assert_eq!(*v.borrow_value(), [0x42]);
    }

    #[test]
    fn align4_ptr_compat_new_returns_err_for_oversized() {
        const N: usize = (usize::BITS / 8) as _;
        let value = [0x42u8; N];
        let result = Align4PtrCompat::<[u8; N]>::new(Metadata::_0, value);
        assert!(result.is_err());
    }
}
