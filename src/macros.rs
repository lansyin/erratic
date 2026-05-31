#[doc(hidden)]
pub mod __priv_reexport {
    pub use alloc::{format, string};
    pub use core;
}

/// Like `let-else`, with access to variant bindings in other branches, for [`Result`][core::result::Result] only.
///
/// # Examples
///
/// ```
/// # use erratic::match_else;
/// # fn try_send(_: ()) -> Result<i32, ()> { unimplemented!() }
/// # fn foo(packet: ()) -> i32 {
/// let Err(packet) = match_else!(try_send(packet), Ok(n) => {
///     return n;
/// });
/// # 0i32
/// # }
/// ```
#[macro_export]
macro_rules! match_else {
    ($exp:expr, Ok($pat:pat) => $body:expr $(,)?) => {
        match $exp {
            Ok($pat) => {
                #[allow(clippy::diverging_sub_expression)]
                let _: $crate::macros::__priv_reexport::core::convert::Infallible = $body;
            }
            Err(err) => $crate::macros::__priv_reexport::core::result::Result::<
                $crate::macros::__priv_reexport::core::convert::Infallible,
                _,
            >::Err(err),
        }
    };
    ($exp:expr, Err($pat:pat) => $body:expr $(,)?) => {
        match $exp {
            Err($pat) => {
                #[allow(clippy::diverging_sub_expression)]
                let _: $crate::macros::__priv_reexport::core::convert::Infallible = $body;
            }
            Ok(value) => $crate::macros::__priv_reexport::core::result::Result::<
                _,
                $crate::macros::__priv_reexport::core::convert::Infallible,
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
/// // Creates an ad-hoc literal value.
/// foo().with_context(literal!("file not found"))?;
///
/// // Defines a list of typed literals.
/// literal!{
///     pub NotFound: "file not found";
///     pub InternalError: "internal error";
/// }
/// foo().with_context(NotFound)?;
/// foo().with_context_ty::<InternalError>()?;
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
/// # #[derive(Debug)]
/// # enum State { NotFound }
/// # fn foo() {
/// # let filename = "";
/// # let something_impl_error_or_display = "";
/// # let err = mkerr!("oops").stateless().erase();
/// let _err = mkerr!("404 not found").stateless();
/// let _err = mkerr!("{filename} not found").stateless();
/// let _err = mkerr!("{} not found", filename).stateless();
/// let _err = mkerr!(state = State::NotFound);
/// let _err = mkerr!(context = "file not found").stateless();
/// let _err = mkerr!(context = "failed to open", payload = filename).stateless();
/// let _err = mkerr!(
///     state = State::NotFound,
///     context = "while opening",
///     payload = filename,
///     error = err,
/// );
/// # let err = mkerr!("oops").stateless().erase();
/// # let get_user_manual_url = || "";
/// let _err = mkerr!(
///     state = State::NotFound,
///     error = err,
///     "{filename} not found, check guides at {}",
///     get_user_manual_url(),
/// );
/// # }
/// ```
///
/// # Format String
///
/// The format string is mutually exclusive with the payload.
///
/// # Argument Order
///
/// Key-value pairs can be provided in any order, but must appear **before** the format string.
#[macro_export]
macro_rules! mkerr {
    ($($key:ident=$value:expr),+ $(, $($fmt:literal $($args:tt)*)?)?) => {
        $crate::__priv_mkerr_kvs!(@sort[,,,] $($key=$value,)+ $($(payload=$crate::macros::__priv_reexport::format!($fmt $($args)*),)?)?)
    };
    ($fmt:literal $($args:tt)*) => {{
        fn make_error<'a, S>(args: $crate::macros::__priv_reexport::core::fmt::Arguments<'a>) -> $crate::Error<S>
        where
            S: $crate::state::State + ?Sized,
        {
            if args.as_str().is_some() {
                struct Literal;

                impl $crate::context::Literal for Literal {
                    const LITERAL: &'static str = $fmt;
                }

                $crate::Error::from_context(Literal)
            } else {
                $crate::Error::from_payload($crate::macros::__priv_reexport::string::ToString::to_string(&args))
            }
        }
        make_error($crate::macros::__priv_reexport::core::format_args!($fmt $($args)*))
    }};
}

#[macro_export]
#[doc(hidden)]
macro_rules! __priv_mkerr_kvs {
    (@sort[$($_:expr)?, $($c:expr)?, $($p:expr)?, $($e:expr)?] state=$s:expr, $($k:ident=$v:expr,)*) => {{
        $( let _ = $_; $crate::macros::__priv_reexport::core::compile_error!("state can only be set once");)?
        $crate::__priv_mkerr_kvs!(@sort[$s, $($c)?, $($p)?, $($e)?] $($k=$v,)*)
    }};
    (@sort[$($s:expr)?, $($_:expr)?, $($p:expr)?, $($e:expr)?] context=$c:expr, $($k:ident=$v:expr,)*) => {{
        $( let _ = $_; $crate::macros::__priv_reexport::core::compile_error!("context can only be set once");)?
        $crate::__priv_mkerr_kvs!(@sort[$($s)?, $c, $($p)?, $($e)?] $($k=$v,)*)
    }};
    (@sort[$($s:expr)?, $($c:expr)?, $($_:expr)?, $($e:expr)?] payload=$p:expr, $($k:ident=$v:expr,)*) => {{
        $( let _ = $_; $crate::macros::__priv_reexport::core::compile_error!("payload can only be set once. note: the format string counts as a payload.");)?
        $crate::__priv_mkerr_kvs!(@sort[$($s)?, $($c)?, $p, $($e)?] $($k=$v,)*)
    }};
    (@sort[$($s:expr)?, $($c:expr)?, $($p:expr)?, $($_:expr)?] error=$e:expr, $($k:ident=$v:expr,)*) => {{
        $( let _ = $_; $crate::macros::__priv_reexport::core::compile_error!("error can only be set once");)?
        $crate::__priv_mkerr_kvs!(@sort[$($s)?, $($c)?, $($p)?, $e] $($k=$v,)*)
    }};
    (@sort[$($s:expr)?, $($c:expr)?, $($p:expr)?, $($e:expr)?]) => {{
        let builder = ($crate::macros::__priv_reexport::core::option::Option::None::<()>);
        $(let builder = builder.ok_or($e);)?
        $(let builder = $crate::BuilderExt::with_state(builder, $s);)?
        $(let builder = $crate::BuilderExt::with_context(builder, $crate::literal!($c));)?
        $(let builder = $crate::BuilderExt::with_payload(builder, $p);)?
        $crate::__priv_mkerr_kvs!(@infer[$($s)?] builder.unwrap_err())
    }};
    (@infer[] $builder:expr) => {
        $crate::macros::__priv_reexport::core::convert::Into::<$crate::Error::<_>>::into($builder)
    };
    (@infer[$state:expr] $builder:expr) => {
        $crate::ErrorExt::build_error($builder)
    };
}

/// Shorthand for [`Err(mkerr!(..))`][`mkerr!`].
#[macro_export]
macro_rules! mkres {
    ($($key:ident=$value:expr),+ $(, $($fmt:literal $($args:tt)*)?)?) => {
        $crate::macros::__priv_reexport::core::result::Result::Err($crate::mkerr!($($key=$value),+ $($(, $fmt $($args)*)?)?))
    };
    ($fmt:literal $($args:tt)*) => {
        $crate::macros::__priv_reexport::core::result::Result::Err($crate::mkerr!($fmt $($args)*))
    };
}

#[cfg(test)]
mod tests {
    use alloc::{
        format,
        string::{String, ToString},
    };

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
            error = mkerr!("source").stateless().erase(),
        );
        let err_from_builder = Error::with_error(mkerr!("source").stateless().erase())
            .with_state(42)
            .with_context(literal!("test"))
            .with_payload("error message")
            .build();

        assert_eq!(
            format!("{err_from_mkerr:-#}"),
            format!("{err_from_builder:-#}")
        );
    }

    #[test]
    fn error_from_kvs_unordered() {
        let err_from_mkerr = mkerr!(
            context = "test",
            error = mkerr!("source").stateless().erase(),
            payload = "error message",
            state = 42,
        );
        let err_from_builder = Error::with_error(mkerr!("source").stateless().erase())
            .with_state(42)
            .with_context(literal!("test"))
            .with_payload("error message")
            .build();

        assert_eq!(
            format!("{err_from_mkerr:-#}"),
            format!("{err_from_builder:-#}")
        );
    }

    #[test]
    fn error_from_hybrid() {
        let world = "world!";
        let err_from_mkerr = mkerr!(
            context = "test",
            error = mkerr!("source").stateless().erase(),
            state = 42,
            "hello {world}"
        );
        let err_from_builder = Error::with_error(mkerr!("source").stateless().erase())
            .with_state(42)
            .with_context(literal!("test"))
            .with_payload(format!("hello {world}"))
            .build();

        assert_eq!(
            format!("{err_from_mkerr:-#}"),
            format!("{err_from_builder:-#}")
        );
    }

    #[test]
    fn infer_default_state_if_state_is_not_specified() {
        let _: Error<i32> = mkerr!(context = "test");
        let _ = || -> result::Result<(), Error<i32>> {
            return mkres!(context = "test");
        };
    }

    #[test]
    fn no_need_for_type_hint_if_state_is_specified() {
        let _ = mkerr!(state = 42, context = "test");
        let _ = mkerr!(context = "test").stateless();
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

    #[test]
    fn mkerr_and_mkres_share_same_capabilities() {
        let world = "world";
        let exclamation = "!";
        let err_from_mkerr = mkerr!(
            context = "test",
            error = mkerr!("source").stateless().erase(),
            state = 42,
            "hello {world}{}",
            exclamation,
        );
        let err_from_mkres: result::Result<(), _> = mkres!(
            context = "test",
            error = mkerr!("source").stateless().erase(),
            state = 42,
            "hello {world}{}",
            exclamation,
        );
        assert_eq!(
            err_from_mkerr.to_string(),
            err_from_mkres.unwrap_err().to_string()
        );
    }
}
