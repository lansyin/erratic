use std::{
    any::{Any, TypeId},
    mem::ManuallyDrop,
    result,
};

/// Attempts to concretize `value` into type `U` if `T == U` at the type-id level.
///
/// Returns `Ok(value as U)` on match, or `Err(value)` otherwise.
pub fn concretize<T, U>(value: T) -> result::Result<U, T>
where
    T: 'static,
    U: 'static,
{
    if TypeId::of::<T>() == TypeId::of::<U>() {
        // # Safety
        //
        // It is sound only when `TypeId::of::<T>() == TypeId::of::<U>()`, which guarantees
        // that `T` and `U` have the same layout. The [`unwrap_unchecked`] is justified by
        // the preceding `TypeId` check.
        unsafe {
            Ok(ManuallyDrop::take(
                (&mut ManuallyDrop::new(value) as &mut dyn Any)
                    .downcast_mut::<ManuallyDrop<U>>()
                    .unwrap_unchecked(),
            ))
        }
    } else {
        Err(value)
    }
}

/// Returns `true` if `T` and `U` have the same [`TypeId`].
pub fn is_same_ty<T, U>() -> bool
where
    T: ?Sized + 'static,
    U: ?Sized + 'static,
{
    TypeId::of::<T>() == TypeId::of::<U>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concretize_same_type_returns_ok() {
        let value: i32 = 42;
        let result = concretize::<i32, i32>(value);
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn concretize_different_type_returns_err() {
        let value: i32 = 42;
        let result = concretize::<i32, String>(value);
        assert_eq!(result.unwrap_err(), 42);
    }

    #[test]
    fn concretize_zst_match() {
        struct Marker;
        let value = Marker;
        let result = concretize::<Marker, Marker>(value);
        assert!(result.is_ok());
    }

    #[test]
    fn concretize_transparent_newtype() {
        // TypeId distinguishes between wrapper and inner.
        struct Wrapper(#[allow(unused)] i32);
        let value = Wrapper(42);
        let result = concretize::<Wrapper, i32>(value);
        assert!(result.is_err()); // Different TypeId
    }

    #[test]
    fn is_same_ty_identical() {
        assert!(is_same_ty::<i32, i32>());
    }

    #[test]
    fn is_same_ty_different() {
        assert!(!is_same_ty::<i32, u32>());
    }

    #[test]
    fn is_same_ty_zst() {
        struct A;
        struct B;
        assert!(is_same_ty::<A, A>());
        assert!(!is_same_ty::<A, B>());
    }
}
