use core::{any::Any, fmt::Debug, mem::MaybeUninit};

use crate::{
    raw::align4::{Align4Ref, Metadata},
    state::State,
};

mod ministate;

pub use ministate::Ministate;

pub enum Metastate {
    Empty,
    Present,
    Frozen,
}

pub trait Discriminator {
    fn get(&self) -> Metastate;

    /// # Safety
    ///
    /// This method must track a specific [`Store`] exclusively.
    unsafe fn set(&mut self, value: Metastate);
}

impl<T> Discriminator for Align4Ref<'static, T> {
    fn get(&self) -> Metastate {
        match self.metadata() {
            Metadata::_0 => Metastate::Empty,
            Metadata::_1 => Metastate::Present,
            Metadata::_2 => Metastate::Frozen,
            _ => unreachable!(),
        }
    }

    unsafe fn set(&mut self, value: Metastate) {
        let vtable = self.borrow_raw().deref();
        *self = Self::new(
            vtable,
            match value {
                Metastate::Empty => Metadata::_0,
                Metastate::Present => Metadata::_1,
                Metastate::Frozen => Metadata::_2,
            },
        )
    }
}

pub trait Store: Send + Sync + 'static {
    /// # Safety
    ///
    /// `self` must be initialized.
    unsafe fn assume_init_debug(&self) -> &dyn Debug;
    /// # Safety
    ///
    /// `self` must be initialized.
    unsafe fn assume_init_get(&self) -> &dyn Any;
    /// # Safety
    ///
    /// `self` must be initialized.
    unsafe fn assume_init_drop(&mut self);
    /// Takes the state out and provides it as `Option<S>` in a callback. Drops the state if
    /// the caller does not take the option.
    ///
    /// # Safety
    ///
    /// `self` must be initialized.
    unsafe fn assume_init_take(&mut self, callback: &mut dyn FnMut(&mut dyn Any));
    /// Attempts to store the value taken from `value` (expects an `Option<S>` behind `Any`).
    ///
    /// Returns `true` if and only if the value was stored. It fails in the following cases:
    ///
    /// - `self` already contains a value.
    /// - The provided `Option<S>` behind `Any` is `None`.
    /// - The provided `Option<S>` behind `Any` is `Some` but the type `S` is not compatible.
    unsafe fn try_set(&mut self, value: &mut dyn Any) -> bool;

    /// Returns a shared view of the state storage, tracked by `discriminator`.
    ///
    /// # Safety
    ///
    /// `discriminator` must be the exclusive one that tracks all operations on the store.
    unsafe fn with_discriminator<'a>(
        &'a self,
        discriminator: &'a dyn Discriminator,
    ) -> StatefulState<'a, Self>
    where
        Self: Sized,
    {
        StatefulState {
            store: self,
            discriminator,
        }
    }

    /// Returns a mutable view of the state storage, tracked by `discriminator`.
    ///
    /// # Safety
    ///
    /// `discriminator` must be the exclusive one that tracks all operations on the store.
    unsafe fn with_discriminator_mut<'a>(
        &'a mut self,
        discriminator: &'a mut dyn Discriminator,
    ) -> StatefulStateMut<'a, Self>
    where
        Self: Sized,
    {
        StatefulStateMut {
            store: self,
            discriminator,
        }
    }
}

impl<S> Store for MaybeUninit<S>
where
    S: State,
{
    unsafe fn assume_init_debug(&self) -> &dyn Debug {
        // Safety: By `assume_init_debug`'s invariants, `self` is guaranteed to contain a valid value.
        unsafe { self.assume_init_ref() }
    }

    unsafe fn assume_init_get(&self) -> &dyn Any {
        // Safety: By `assume_init_get`'s invariants, `self` is guaranteed to contain a valid value.
        unsafe { self.assume_init_ref() }
    }

    unsafe fn assume_init_drop(&mut self) {
        // Safety: By `assume_init_drop`'s invariants, `self` is guaranteed to contain a valid value.
        unsafe { self.assume_init_drop() }
    }

    unsafe fn assume_init_take(&mut self, callback: &mut dyn FnMut(&mut dyn Any)) {
        // Safety: By `assume_init_take`'s invariants, `self` is guaranteed to contain a valid value.
        unsafe {
            let mut state = Some(self.assume_init_read());
            callback(&mut state);
        }
    }

    unsafe fn try_set(&mut self, value: &mut dyn Any) -> bool {
        let Some(value) = value.downcast_mut::<Option<S>>() else {
            return false;
        };
        let Some(value) = value.take() else {
            return false;
        };
        self.write(value);
        true
    }
}

/// Shared view of the state storage for read-only queries.
pub struct StatefulState<'a, S> {
    store: &'a S,
    discriminator: &'a dyn Discriminator,
}

impl<'a, S> StatefulState<'a, S>
where
    S: Store,
{
    /// Returns an object that can be used to format the state.
    ///
    /// This method works with the `Present` and `Frozen` states.
    pub fn format_debug(&self) -> Option<&'a dyn Debug> {
        match self.discriminator.get() {
            Metastate::Empty => None,
            Metastate::Present | Metastate::Frozen => Some(unsafe {
                // Safety: We are in the branch with state `present` or `frozen`, the store contains a valid state.
                self.store.assume_init_debug()
            }),
        }
    }

    /// Returns an opaque reference to the state.
    ///
    /// This method works with the `Present` state.
    pub fn get(&self) -> Option<&'a dyn Any> {
        match self.discriminator.get() {
            Metastate::Frozen | Metastate::Empty => None,
            Metastate::Present => Some(unsafe {
                // Safety: We are in the branch with state `present`, the store contains a valid state.
                self.store.assume_init_get()
            }),
        }
    }
}

/// Mutable view of the state storage for read/write operations.
pub struct StatefulStateMut<'a, S> {
    /// # Safety Invariants
    ///
    /// `self.store` must be tracked by `self.discriminator` exclusively.
    store: &'a mut S,
    /// # Safety Invariants
    ///
    /// `self.discriminator` must track `self.store` exclusively.
    discriminator: &'a mut dyn Discriminator,
}

impl<'a, S> StatefulStateMut<'a, S>
where
    S: Store,
{
    /// Takes the state out via a callback with `Option<S>` behind the opaque `&mut dyn Any`.
    ///
    /// Note that even if you don't move the state out in the callback, it is still dropped.
    pub fn take(&mut self, callback: &mut dyn FnMut(&mut dyn Any)) {
        match self.discriminator.get() {
            Metastate::Empty | Metastate::Frozen => {}
            Metastate::Present => {
                unsafe {
                    // Safety: By `Self::discriminator`'s invariants, `self.discriminator` is exclusively for `self.store`.
                    self.discriminator.set(Metastate::Empty);
                    // Safety: We are in the branch with state `present`, the store contains a valid state.
                    self.store.assume_init_take(callback);
                }
            }
        }
    }

    /// Stores the value taken from `state` into the underlying store.
    ///
    /// Fails in these cases:
    ///
    /// - There is a state inside the store.
    /// - The underlying store is not compatible.
    pub fn try_set(&mut self, state: &mut dyn Any) -> bool {
        match self.discriminator.get() {
            Metastate::Frozen | Metastate::Present => false,
            Metastate::Empty => {
                // Safety: The discriminator confirmed the store is empty.
                if unsafe { self.store.try_set(state) } {
                    unsafe {
                        // Safety: By `Self::discriminator`'s invariants, `self.discriminator` is exclusively for `self.store`.
                        self.discriminator.set(Metastate::Present);
                    }
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Marks the state as frozen (erased, but still dropped).
    pub fn freeze(&mut self) -> bool {
        match self.discriminator.get() {
            Metastate::Empty => false,
            Metastate::Frozen => true,
            Metastate::Present => {
                unsafe {
                    // Safety: By `Self::discriminator`'s invariants, `self.discriminator` is exclusively for `self.store`.
                    self.discriminator.set(Metastate::Frozen);
                }
                true
            }
        }
    }

    /// Drops the stored state, if any, and marks the store as empty.
    pub fn drop_in_place(&mut self) {
        match self.discriminator.get() {
            Metastate::Empty => {}
            Metastate::Present | Metastate::Frozen => {
                unsafe {
                    // Safety: By `Self::discriminator`'s invariants, `self.discriminator` is exclusively for `self.store`.
                    self.discriminator.set(Metastate::Empty);
                    // Safety: We are in the branch with state `Present` or `Frozen`, the store contains a valid state.
                    self.store.assume_init_drop();
                }
            }
        }
    }
}
