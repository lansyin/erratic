use core::{
    any::TypeId,
    fmt::{self, Debug},
    marker::PhantomData,
    mem::{self, ManuallyDrop, MaybeUninit},
    ptr,
};

use crate::rtti;

use super::Metadata;

/// An inline pointer-sized storage with metadata attached.
///
/// # ABI
///
/// This type guarantees that the least significant 2 bits of its first byte encode a [`Metadata`]
/// and that it has exactly the layout of `usize`.
//
// Note: The repr/align attribute is required because it is used to compute the offset
// that satisfies T's alignment.
#[cfg_attr(target_pointer_width = "16", repr(C, align(2)))]
#[cfg_attr(target_pointer_width = "32", repr(C, align(4)))]
#[cfg_attr(target_pointer_width = "64", repr(C, align(8)))]
pub struct Align4PtrCompat<T> {
    meta: u8,
    /// # Safety Invariants
    ///
    /// [`Self::store`] always stores a valid `T` at the offset given by [`Self::OFFSET`]
    /// before `Self::drop` is invoked.
    store: MaybeUninit<[u8; usize::BITS as usize / 8 - 1]>,
    _marker: PhantomData<T>,
}

/// To uphold [`Align4PtrCompat`]'s ABI guarantee, the following invariants must hold:
///
/// - [`Align4PtrCompat::meta`] is the first field.
/// - [`Align4PtrCompat`] has the same size as `usize`.
/// - [`Align4PtrCompat`] has the same alignment as `usize`.
const _: () = const {
    assert!(mem::offset_of!(Align4PtrCompat<()>, meta) == 0);
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
    const CORRECT_OFFSET: &'static str = "offset was checked at compile-time";

    /// [`Self::OFFSET`] is `Some` if an instance of `Align4PtrCompat<T>` can be created.
    const OFFSET: Option<isize> = 'ret: {
        let target_size = mem::size_of::<T>();
        let target_align = mem::align_of::<T>();
        let store_offset_start = mem::offset_of!(Self, store);
        let store_size = rtti::size_of_field(|v: Self| v.store);
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
        let Some(offset) = Self::OFFSET else {
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
            // Safety: The offset was validated by `Self::OFFSET`.
            ((&raw mut this.store).cast::<u8>().offset(offset) as *mut T).write(value);
        }

        Ok(this)
    }

    pub fn borrow_value(&self) -> &T {
        unsafe {
            let offset = Self::OFFSET.expect(Self::CORRECT_OFFSET);

            // Safety: The offset is validated prior to creating Align4PtrCompat.
            // The raw pointer, though invalidated by the newly created reference,
            // is not used afterward.
            &*((&raw const self.store).cast::<u8>().offset(offset) as *const T)
        }
    }

    pub fn into_value(self) -> T {
        unsafe {
            let offset = Self::OFFSET.expect(Self::CORRECT_OFFSET);

            let mut this = ManuallyDrop::new(self);
            let value_mut = (&raw mut this.store).cast::<u8>().offset(offset) as *mut T;

            value_mut.read()
        }
    }

    pub const fn is_inlinable() -> bool {
        Self::OFFSET.is_some()
    }
}

impl<T> Align4PtrCompat<T>
where
    T: Debug + Send + Sync + 'static,
{
    fn into_parts(self) -> (u8, MaybeUninit<[u8; mem::size_of::<usize>() - 1]>) {
        let mut this = ManuallyDrop::new(self);
        // Safety: `this` is forgotten and is not accessed afterwards. It's safe
        // to move it out.
        (this.meta, unsafe { (&raw mut this.store).read() })
    }

    pub fn erase(self) -> ErasedAlign4PtrCompat {
        ErasedAlign4PtrCompat::from_typed(self)
    }
}

impl<T> Drop for Align4PtrCompat<T> {
    fn drop(&mut self) {
        let offset = Self::OFFSET.expect(Self::CORRECT_OFFSET);

        unsafe {
            // Safety: The offset is validated prior to creating Align4PtrCompat.
            ptr::drop_in_place((&raw mut self.store).cast::<u8>().offset(offset) as *mut T);
        }
    }
}

pub struct ErasedAlign4PtrCompat {
    inner: MaybeUninit<Align4PtrCompat<()>>,
    vtable: &'static Align4PtrCompatVTable,
    _marker: PhantomData<*mut ()>,
}

impl ErasedAlign4PtrCompat {
    const CORRECT_VTABLE_CALL: &'static str =
        "vtable functions must be invoked with the correct `Align4PtrCompat` pointer";

    pub fn from_typed<T>(value: Align4PtrCompat<T>) -> Self
    where
        T: Debug + Send + Sync + 'static,
    {
        let (meta, store) = value.into_parts();
        ErasedAlign4PtrCompat {
            inner: MaybeUninit::new(Align4PtrCompat::<()> {
                meta,
                store,
                _marker: PhantomData,
            }),
            vtable: &const {
                Align4PtrCompatVTable {
                    type_id: TypeId::of::<T>(),
                    debug: Self::debug_erased::<T>,
                    drop: Self::drop_erased::<T>,
                }
            },
            _marker: PhantomData,
        }
    }

    fn downcast_ref<T>(&self) -> Option<&Align4PtrCompat<T>>
    where
        T: 'static,
    {
        if self.vtable.type_id != TypeId::of::<T>() {
            return None;
        }
        Some(unsafe { &*(self.inner.as_ptr() as *const Align4PtrCompat<T>) })
    }

    fn debug_erased<T>(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result
    where
        T: Debug + 'static,
    {
        let value = self.downcast_ref::<T>().expect(Self::CORRECT_VTABLE_CALL);
        <T as Debug>::fmt(value.borrow_value(), f)
    }

    /// Drops the inner value.
    ///
    /// # Safety
    ///
    /// The type `T` must be the original type before erasure. After this method is called, `self`
    /// must never be used.
    unsafe fn drop_erased<T>(&mut self)
    where
        T: 'static,
    {
        // Safety: The only code that moves `inner` out is in `drop` (here).
        let this = unsafe { self.inner.assume_init_read() };
        let (meta, store) = this.into_parts();
        let _this = Align4PtrCompat::<T> {
            meta,
            store,
            _marker: PhantomData,
        };
    }
}

impl Debug for ErasedAlign4PtrCompat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (self.vtable.debug)(self, f)
    }
}

struct Align4PtrCompatVTable {
    type_id: TypeId,
    debug: fn(&ErasedAlign4PtrCompat, &mut fmt::Formatter<'_>) -> fmt::Result,
    drop: unsafe fn(&mut ErasedAlign4PtrCompat),
}

impl Drop for ErasedAlign4PtrCompat {
    fn drop(&mut self) {
        // Safety: The value inside an `ErasedAlign4PtrCompat` cannot be moved out.
        // So the value is always valid before we drop it.
        unsafe {
            (self.vtable.drop)(self);
        }
    }
}

/// # Safety
///
/// Though an unconditional `Send` impl is generally considered wrong, the constructor of `ErasedAlign4PtrCompat`
/// requires its erased inner value to be `Send`. So it's sound.
unsafe impl Send for ErasedAlign4PtrCompat {}

/// # Safety
///
/// The constructor of `ErasedAlign4PtrCompat` requires its erased inner value to be `Sync`.
unsafe impl Sync for ErasedAlign4PtrCompat {}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!Align4PtrCompat::<[u8; N]>::is_inlinable());
    }

    #[test]
    fn align4_ptr_compat_u8_store_offset() {
        assert!(Align4PtrCompat::<u8>::is_inlinable());
    }

    #[test]
    fn align4_ptr_compat_u32_store_offset() {
        if cfg!(target_pointer_width = "64") {
            assert!(Align4PtrCompat::<u32>::is_inlinable());
        } else {
            // 16/32 bit platforms
            assert!(!Align4PtrCompat::<u32>::is_inlinable());
        }
    }

    #[test]
    fn align4_ptr_compat_u64_is_oversized() {
        assert!(!Align4PtrCompat::<u64>::is_inlinable());
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
