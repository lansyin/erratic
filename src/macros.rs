#[doc(hidden)]
pub mod __reexport {
    pub use std;
}

/// Like `let-else`, but also handles the remaining cases — for [`Result`][core::result::Result] only.
///
/// # Examples
///
/// ```
/// # use erratic::match_else;
/// # fn try_send(_: ()) -> Result<(), ()> { unimplemented!() }
/// # fn foo(packet: ()) {
/// let Err(packet) = match_else!(try_send(packet),
///     Ok(_) => return,
/// );
/// # }
/// ```
#[macro_export]
macro_rules! match_else {
    ($exp:expr, Ok($pat:pat) => $body:expr $(,)?) => {
        match $exp {
            Ok($pat) => {
                #[allow(clippy::diverging_sub_expression)]
                let _: $crate::macros::__reexport::std::convert::Infallible = $body;
            }
            Err(err) => $crate::macros::__reexport::std::result::Result::<
                $crate::macros::__reexport::std::convert::Infallible,
                _,
            >::Err(err),
        }
    };
    ($exp:expr, Err($pat:pat) => $body:expr $(,)?) => {
        match $exp {
            Err($pat) => {
                #[allow(clippy::diverging_sub_expression)]
                let _: $crate::macros::__reexport::std::convert::Infallible = $body;
            }
            Ok(value) => $crate::macros::__reexport::std::result::Result::<
                _,
                $crate::macros::__reexport::std::convert::Infallible,
            >::Ok(value),
        }
    };
}

/// Creates a literal value, or declares one or more named literal types.
///
/// # Examples
///
/// ```
/// # use erratic::*;
/// # fn foo() -> Result<()> {
/// # let trunc_file = || -> std::result::Result<(), std::io::Error> { unimplemented!() };
/// literal!{
///     pub NotFound: "file not found";
///     pub InternalError: "internal error";
/// }
/// trunc_file()
///     .with_context(literal!("file not found"))?;
/// trunc_file()
///     .with_context(NotFound)?;
/// trunc_file()
///     .with_context_ty::<InternalError>()?;
/// Ok(())
/// # }
/// ```
#[macro_export]
macro_rules! literal {
    ($lit:literal) => {{
        struct Literal;

        impl $crate::context::Literal for Literal {
            const LITERAL: &'static str = $lit;
        }

        Literal
    }};
    {
        $( $vis:vis $name:ident: $lit:literal; )+
    } => {
        $(
            $vis struct $name;

            impl $crate::context::Literal for $name {
                const LITERAL: &'static str = $lit;
            }
        )+
    };
}

#[doc(hidden)]
pub mod __specialization {
    use std::{error, fmt::Display};

    use crate::{Error, state::State};

    pub struct FromDisplay;

    impl FromDisplay {
        pub fn from<S>(self, value: impl Display + Send + Sync + 'static) -> Error<S>
        where
            S: State + ?Sized,
            S::Repr: Default,
        {
            Error::from_payload(value)
        }
    }

    pub trait SelectDisplay {
        fn select(&self) -> FromDisplay {
            FromDisplay
        }
    }

    impl<D> SelectDisplay for &D {}

    pub struct FromError;

    impl FromError {
        pub fn from<S>(self, err: impl std::error::Error + Send + Sync + 'static) -> Error<S>
        where
            S: State + ?Sized,
            S::Repr: Default,
        {
            Error::from_error(err)
        }
    }

    pub trait SelectError {
        fn select(&self) -> FromError {
            FromError
        }
    }

    impl<E: error::Error> SelectError for E {}
}

/// Constructs an [`Error`][crate::Error] from a literal, [`Error`][std::error::Error], [`Display`][std::fmt::Display], or [format string][std::format].
///
/// # Examples
///
/// ```
/// # use erratic::*;
/// # fn foo() -> Result<()> {
/// # let filename = "";
/// # let something_impl_error_or_display = "";
/// return Err(mkerr!("404 not found"));
/// return Err(mkerr!("{} not found", filename));
/// return Err(mkerr!(something_impl_error_or_display));
/// # }
/// ```
#[macro_export]
macro_rules! mkerr {
    ($fmt:literal $($rest:tt)*) => {{
        fn make_error<'a, S>(args: $crate::macros::__reexport::std::fmt::Arguments<'a>) -> $crate::Error<S>
        where
            S: $crate::state::State + ?Sized,
            S::Repr: Default,
        {
            if args.as_str().is_some() {
                struct Literal;

                impl $crate::context::Literal for Literal {
                    const LITERAL: &'static str = $fmt;
                }

                $crate::Error::from_context(Literal)
            } else {
                $crate::Error::from_payload(args.to_string())
            }
        }
        make_error($crate::macros::__reexport::std::format_args!($fmt $($rest)*))
    }};
    ($exp:expr $(,)?) => {{
        #[allow(unused_imports)]
        use $crate::macros::__specialization::{SelectDisplay, SelectError};

        match $exp {
            err => (&err).select().from(err),
        }
    }};
}

/// Constructs a [`Result`][crate::Result] from a literal, [`Error`][std::error::Error], [`Display`][std::fmt::Display], or [format string][std::format].
#[macro_export]
macro_rules! mkres {
    ($($tt:tt)*) => {
        $crate::macros::__reexport::std::result::Result::Err($crate::mkerr!($($tt)*))
    };
}

#[cfg(test)]
mod tests {
    use crate::*;

    // Ensure the macros do not require type annotations in the most common cases
    #[test]
    fn type_reference_check() {
        let _ = || -> Result<()> {
            let err = mkerr!("test");
            return Err(err);
        };
        let _ = || -> Result<()> {
            return mkres!("test");
        };
        let _ = || -> result::Result<(), Error<i32>> {
            let err = mkerr!("test");
            return Err(err);
        };
        let _ = || -> result::Result<(), Error<i32>> {
            return mkres!("test");
        };
    }

    // Test that the macros can be used with various types of input.

    #[test]
    fn error_from_literal() {
        let _ = mkerr!("test").stateless();
    }

    #[test]
    fn error_from_error() {
        let err = mkerr!("test").stateless();
        let _ = mkerr!(err).stateless();
    }

    #[test]
    fn error_from_display() {
        let text = String::from("foo");
        let _ = mkerr!(text).stateless();
    }

    #[test]
    fn error_from_format_string() {
        let filename = "file.txt";
        let _ = mkerr!("{} not found", filename).stateless();
    }

    // Test that the macros can select format string or literal based on the input.

    #[test]
    fn error_from_literal_like_format_string() {
        let filename = "file.txt";
        let err = mkerr!("{filename} not found").stateless();
        assert!(err.has_payload_of::<String>());
    }

    #[test]
    fn error_from_literal_without_allocation() {
        let err = mkerr!("file not found").stateless();
        assert!(!err.has_payload_of::<String>());
    }
}
