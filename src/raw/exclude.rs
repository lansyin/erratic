use core::marker::PhantomData;

use crate::rtti;

pub(super) struct Exclude<T, X> {
    value: T,
    _marker: PhantomData<X>,
}

impl<T, X> Exclude<T, X>
where
    T: 'static,
    X: 'static,
{
    pub fn new(value: T) -> Self {
        Self {
            value,
            _marker: PhantomData,
        }
    }

    pub fn get(&self) -> Option<&T> {
        if rtti::is_same_ty::<T, X>() {
            None
        } else {
            Some(&self.value)
        }
    }

    pub fn get_mut(&mut self) -> Option<&mut T> {
        if rtti::is_same_ty::<T, X>() {
            None
        } else {
            Some(&mut self.value)
        }
    }

    pub fn into_inner(self) -> Option<T> {
        if rtti::is_same_ty::<T, X>() {
            None
        } else {
            Some(self.value)
        }
    }
}
