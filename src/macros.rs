#[doc(hidden)]
pub mod __priv_reexport {
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
                let _: $crate::macros::__priv_reexport::std::convert::Infallible = $body;
            }
            Err(err) => $crate::macros::__priv_reexport::std::result::Result::<
                $crate::macros::__priv_reexport::std::convert::Infallible,
                _,
            >::Err(err),
        }
    };
    ($exp:expr, Err($pat:pat) => $body:expr $(,)?) => {
        match $exp {
            Err($pat) => {
                #[allow(clippy::diverging_sub_expression)]
                let _: $crate::macros::__priv_reexport::std::convert::Infallible = $body;
            }
            Ok(value) => $crate::macros::__priv_reexport::std::result::Result::<
                _,
                $crate::macros::__priv_reexport::std::convert::Infallible,
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
/// # let foo = || -> std::result::Result<(), std::io::Error> { unimplemented!() };
/// foo()
///     .with_context(literal!("file not found"))?;
/// literal!{
///     pub NotFound: "file not found";
///     pub InternalError: "internal error";
/// }
/// foo()
///     .with_context(NotFound)?;
/// foo()
///     .with_context_ty::<InternalError>()?;
/// # Ok(())
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

/// Constructs an [`Error`][crate::Error] from a variety of input types.
///
/// # Examples
///
/// ```
/// # use erratic::*;
/// # #[derive(Debug, Default)]
/// # enum State { #[default] NotFound }
/// # fn foo() {
/// # let filename = "";
/// # let something_impl_error_or_display = "";
/// # let err = mkerr!("oops").stateless().erase();
/// let _err = mkerr!("404 not found").stateless();
/// let _err = mkerr!("{filename} not found").stateless();
/// let _err = mkerr!("{} not found", filename).stateless();
/// let _err = mkerr!(state=State::NotFound);
/// let _err = mkerr!(context="while opening").stateless();
/// let _err = mkerr!(context="while opening", payload=filename).stateless();
/// let _err = mkerr!(state=State::NotFound, context="while opening", payload=filename, error=err);
/// # }
/// ```
#[macro_export]
macro_rules! mkerr {
    ($($key:ident=$value:expr),+ $(,)?) => {
        $crate::mkerr!(@priv kvs $($key=$value,)?)
    };
    (@priv kvs $(state=$state:expr,)? $(context=$context:expr,)? $(payload=$payload:expr,)? $(error=$error:expr,)?) => {
        {
            use $crate::{BuilderExt, ErrorExt};

            ($crate::macros::__priv_reexport::std::option::Option::None::<()>)
                $(.ok_or($error))?
                $(.with_state($state))?
                $(.with_context($crate::literal!($context)))?
                $(.with_payload($payload))?
                .build_error()
                .unwrap_err()
        }
    };
    ($fmt:literal $($rest:tt)*) => {{
        fn make_error<'a, S>(args: $crate::macros::__priv_reexport::std::fmt::Arguments<'a>) -> $crate::Error<S>
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
                $crate::Error::from_payload($crate::macros::__priv_reexport::std::string::ToString::to_string(&args))
            }
        }
        make_error($crate::macros::__priv_reexport::std::format_args!($fmt $($rest)*))
    }};
}

/// Shorthand for `Err(mkerr!(..))`; see [`mkerr!`].
#[macro_export]
macro_rules! mkres {
    ($($tt:tt)*) => {
        $crate::macros::__priv_reexport::std::result::Result::Err($crate::mkerr!($($tt)*))
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
    fn error_from_format_string() {
        let filename = "file.txt";
        let _ = mkerr!("{} not found", filename).stateless();
    }

    #[test]
    fn error_from_kvs() {
        let err_from_mkerr = mkerr!(
            state = 42,
            context = "test",
            payload = "error message",
            error = mkerr!("source").stateless().erase()
        );
        let err_from_builder = Error::with_error(mkerr!("source").stateless().erase())
            .with_state(42)
            .with_context(literal!("test"))
            .with_payload("error message")
            .build();

        assert_eq!(err_from_mkerr.to_string(), err_from_builder.to_string());
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
