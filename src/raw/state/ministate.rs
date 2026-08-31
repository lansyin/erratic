use core::{
    any::Any,
    fmt::Debug,
    mem::{self, ManuallyDrop, MaybeUninit},
};

use crate::raw::ptr::{Mut, Ref};
use crate::rtti;

use super::Store;

// Note: The repr/align attribute is required because it is used to compute the offset
// that satisfies T's alignment.
#[cfg_attr(target_pointer_width = "16", repr(C, align(2)))]
#[cfg_attr(target_pointer_width = "32", repr(C, align(4)))]
#[cfg_attr(target_pointer_width = "64", repr(C, align(8)))]
pub struct Ministate {
    /// # Safety Invariants
    ///
    /// `store` contains an erased state value before [`Ministate::drop`] is called.
    store: MaybeUninit<[u8; (usize::BITS / u8::BITS) as usize]>,
    /// # Safety Invariants
    ///
    /// All functions in `vtable` receive the state value inside `self.store`.
    vtable: &'static MinistateVtable,
}

const _: () = const {
    assert!(mem::size_of::<Ministate>() == 2 * mem::size_of::<usize>());
};

pub struct MinistateVtable {
    /// # Safety
    ///
    /// `this` must point to a valid value inside [`Ministate::store`].
    assume_init_drop: unsafe fn(this: Mut<'_, MaybeUninit<()>>),
    /// # Safety
    ///
    /// - `this` must point to a valid value inside [`Ministate::store`].
    assume_init_debug: unsafe fn(this: Ref<'_, ()>) -> &'_ dyn Debug,
    /// # Safety
    ///
    /// `this` must point to a valid value inside [`Ministate::store`].
    assume_init_get: unsafe fn(this: Ref<'_, ()>) -> &'_ dyn Any,
    /// # Safety
    ///
    /// `this` must point to a valid value inside [`Ministate::store`]. After this method has been
    /// invoked, the place `this` points to is invalidated, even if the caller does not take the
    /// value out in the callback (that value will be dropped). Since `Ministate` will drop the
    /// stored value when it is dropped, any instance that invoked this function must be forgotten
    /// eventually (e.g. put into a `ManuallyDrop`).
    assume_init_take:
        unsafe fn(this: Mut<'_, MaybeUninit<()>>, callback: &mut dyn FnMut(&mut dyn Any)),
}

impl Ministate {
    const STORE_SIZE: usize = rtti::size_of_field(|v: Self| v.store);
    const STORE_ALIGN: usize = mem::align_of::<Self>();

    /// Returns a vtable for type `S` if the type `S` fits [`Ministate::store`].
    const fn try_get_vtable_for<S>() -> Option<&'static MinistateVtable>
    where
        S: Debug + 'static,
    {
        let state_size: usize = mem::size_of::<S>();
        let state_align: usize = mem::align_of::<S>();

        if state_size > Self::STORE_SIZE || state_align > Self::STORE_ALIGN {
            return None;
        }

        Some(&MinistateVtable {
            assume_init_drop: |store| unsafe {
                // Safety: By the invariants of `assume_init_drop`, `store` is guaranteed to contain a valid value.
                let this = store.cast::<MaybeUninit<S>>().deref_mut();
                this.assume_init_drop();
            },
            assume_init_debug: |store| unsafe {
                // Safety: By the invariants of `assume_init_debug`, `store` is guaranteed to contain a valid value.
                store.cast::<S>().deref()
            },
            assume_init_get: |store| unsafe {
                // Safety: By the invariants of `assume_init_get`, `store` is guaranteed to contain a valid value.
                store.cast::<S>().deref()
            },
            assume_init_take: |store, callback| unsafe {
                // Safety: By the invariants of `assume_init_take`, `store` is guaranteed to contain a valid value.
                let this = store.cast::<MaybeUninit<S>>().deref_mut();
                let mut value = Some(this.assume_init_read());

                callback(&mut value);
            },
        })
    }

    pub fn try_new<S>(state: S) -> Result<Self, S>
    where
        S: Debug + 'static,
    {
        let Some(vtable) = Self::try_get_vtable_for::<S>() else {
            return Err(state);
        };

        let mut this = Ministate {
            store: MaybeUninit::uninit(),
            vtable,
        };

        // Safety: `S` fits the store since we obtained a valid vtable from `try_get_vtable_for::<S>`.
        unsafe {
            (this.store.as_mut_ptr() as *mut S).write(state);
        }

        Ok(this)
    }

    pub fn into_inner<S>(self) -> Option<S>
    where
        S: 'static,
    {
        let mut this = ManuallyDrop::new(self);
        let mut output = None;
        // Safety: The value is behind `ManuallyDrop` so it won't be double-dropped.
        unsafe {
            (this.vtable.assume_init_take)(
                Mut::from_ptr(this.store.as_mut_ptr() as *mut MaybeUninit<()>),
                &mut (|value: &mut dyn Any| {
                    let Some(value) = value.downcast_mut::<Option<S>>() else {
                        return;
                    };
                    mem::swap(&mut output, value);
                }),
            );
        }
        output
    }

    pub const fn is_state_compact<S>() -> bool
    where
        S: Debug + 'static,
    {
        Self::try_get_vtable_for::<S>().is_some()
    }
}

impl Drop for Ministate {
    fn drop(&mut self) {
        // Safety: The stored value is still valid when dropped.
        unsafe {
            (self.vtable.assume_init_drop)(Mut::from_ptr(
                self.store.as_mut_ptr() as *mut MaybeUninit<()>
            ))
        }
    }
}

impl Store for ManuallyDrop<Ministate> {
    unsafe fn assume_init_debug(&self) -> &dyn Debug {
        // Safety: By the invariants of `assume_init_debug`, `store` is guaranteed to contain a valid value.
        unsafe { (self.vtable.assume_init_debug)(Ref::from_ptr(self.store.as_ptr() as *mut ())) }
    }

    unsafe fn assume_init_get(&self) -> &dyn Any {
        // Safety: By the invariants of `assume_init_get`, `store` is guaranteed to contain a valid value.
        unsafe { (self.vtable.assume_init_get)(Ref::from_ptr(self.store.as_ptr() as *mut ())) }
    }

    unsafe fn assume_init_drop(&mut self) {
        // Safety: By the invariants of `assume_init_drop`, `store` is guaranteed to contain a valid value.
        unsafe {
            (self.vtable.assume_init_drop)(Mut::from_ptr(
                self.store.as_mut_ptr() as *mut MaybeUninit<()>
            ))
        }
    }

    unsafe fn assume_init_take(&mut self, callback: &mut dyn FnMut(&mut dyn Any)) {
        // Safety: By the invariants of `assume_init_take`, `store` is guaranteed to contain a valid value.
        unsafe {
            (self.vtable.assume_init_take)(
                Mut::from_ptr(self.store.as_mut_ptr() as *mut MaybeUninit<()>),
                callback,
            )
        }
    }

    unsafe fn try_set(&mut self, value: &mut dyn Any) -> bool {
        let Some(value) = value.downcast_mut::<Option<Ministate>>() else {
            return false;
        };
        let Some(value) = value.take() else {
            return false;
        };
        *self = ManuallyDrop::new(value);
        true
    }
}

#[cfg(test)]
mod tests {
    use core::mem;

    use super::Ministate;

    #[test]
    fn try_new_then_into_inner_round_trips() {
        let value: usize = 0xDEAD_BEEF;
        let state = Ministate::try_new(value).unwrap();
        assert_eq!(state.into_inner::<usize>(), Some(value));
    }

    #[test]
    fn try_new_then_into_inner_round_trips_drop_type() {
        #[derive(Debug, PartialEq)]
        struct Dropping(usize);
        impl Drop for Dropping {
            fn drop(&mut self) {}
        }

        let value = Dropping(42);
        let state = Ministate::try_new(value).unwrap();
        assert_eq!(state.into_inner::<Dropping>(), Some(Dropping(42)));
    }

    #[test]
    fn int_sizes_respect_word_size() {
        macro_rules! check {
            ($($ty:ty),* $(,)?) => {
                $(
                    let word_size = mem::size_of::<usize>();
                    let should_fit = mem::size_of::<$ty>() <= word_size
                        && mem::align_of::<$ty>() <= mem::align_of::<Ministate>();
                    assert_eq!(
                        Ministate::try_new::<$ty>(1 as $ty).is_ok(),
                        should_fit,
                        "`{}` (size={}, align={}) on a {}-bit target: expected fit={should_fit}",
                        stringify!($ty),
                        mem::size_of::<$ty>(),
                        mem::align_of::<$ty>(),
                        usize::BITS,
                    );
                )*
            };
        }
        check!(u8, u16, u32, u64, u128);
    }
}
