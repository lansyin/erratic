use core::{
    error::{self, Error},
    fmt::{self, Debug, Display},
};

mod compat;
mod own;
mod r#ref;

pub use compat::{Align4PtrCompat, ErasedAlign4PtrCompat};
pub use own::Align4Own;
pub use r#ref::Align4Ref;

/// A 2-bits metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Metadata(pub u8);

impl Metadata {
    pub const MASK: u8 = 0b00000011;

    pub const _0: Metadata = Metadata(0b00000000);
    pub const _1: Metadata = Metadata(0b00000001);
    pub const _2: Metadata = Metadata(0b00000010);
    pub const _3: Metadata = Metadata(0b00000011);

    pub(super) fn encode_to_byte(self, addr_bytes: &mut u8) {
        *addr_bytes |= self.0;
    }

    pub(super) fn decode_from_byte(addr_bytes: &mut u8) -> Self {
        let meta = *addr_bytes & Self::MASK;
        *addr_bytes &= !Self::MASK;
        Self(meta)
    }
}

/// A non-null raw pointer with metadata attached.
///
/// # ABI
///
/// This type guarantees that the least significant 2 bits of its first byte encode a [`Metadata`].
#[derive(Clone, Copy)]
#[repr(C)]
pub(super) struct Align4Ptr(*mut ());

impl Align4Ptr {
    fn swap_leading_and_trailing_byte_on_big_endian(addr_bytes: &mut [u8]) {
        let index_last = addr_bytes.len() - 1;
        let (leading_bytes, last_byte) = addr_bytes.split_at_mut(index_last);
        #[cfg(target_endian = "big")]
        {
            core::mem::swap(&mut leading_bytes[0], &mut last_byte[0]);
        }
        #[cfg(target_endian = "little")]
        {
            _ = leading_bytes;
            _ = last_byte;
        }
    }

    /// Encodes `meta` into the low 2 bits of the pointer address.
    ///
    /// # Panics
    ///
    /// Panics if the low 2 bits of `addr` are not zero.
    pub(super) fn from_parts(ptr: *mut (), meta: Metadata) -> Self {
        let addr = ptr.map_addr(|addr| {
            let mut addr_bytes = addr.to_le_bytes();

            assert_eq!(addr_bytes[0] & Metadata::MASK, 0);

            meta.encode_to_byte(&mut addr_bytes[0]);
            Self::swap_leading_and_trailing_byte_on_big_endian(&mut addr_bytes);

            usize::from_le_bytes(addr_bytes)
        });

        Self(addr)
    }

    /// Extracts the original address and metadata from the encoded pointer.
    pub(super) fn to_parts(self) -> (*mut (), Metadata) {
        let mut meta = Metadata::_0;
        let ptr = self.0.map_addr(|addr| {
            let mut addr_bytes = addr.to_le_bytes();

            Self::swap_leading_and_trailing_byte_on_big_endian(&mut addr_bytes);
            meta = Metadata::decode_from_byte(&mut addr_bytes[0]);

            usize::from_le_bytes(addr_bytes)
        });

        (ptr, meta)
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

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem;

    #[test]
    fn align4_ptr_round_trip() {
        let value = Align4(0u32); // 4-byte-aligned, low 2 bits are 00
        let addr = &raw const value as *mut ();
        for meta in [Metadata::_0, Metadata::_1, Metadata::_2, Metadata::_3] {
            let ptr = Align4Ptr::from_parts(addr, meta);
            let (restored_addr, restored_meta) = ptr.to_parts();
            assert_eq!(restored_addr, addr);
            assert_eq!(restored_meta, meta);
        }
    }

    #[test]
    #[should_panic]
    fn align4_ptr_panics_on_unaligned() {
        let bytes: [u8; 2] = [0, 0];
        for byte in &bytes {
            let unaligned = byte as *const u8 as *mut ();
            Align4Ptr::from_parts(unaligned, Metadata::_0);
        }
    }

    #[test]
    fn align4_guarantees_alignment() {
        assert!(mem::align_of::<Align4<u8>>() >= 4);
        assert!(mem::align_of::<Align4<u64>>() >= 4);
    }
}
