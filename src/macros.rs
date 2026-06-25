#[doc(hidden)]
pub mod __priv {
    pub use alloc::{
        format,
        string::{String, ToString},
    };
    pub use core::{
        compile_error,
        convert::{Infallible, Into, identity},
        fmt::Debug,
        format_args,
        option::Option::{self, None, Some},
        result::Result::{self, Err, Ok},
        stringify,
    };
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
                let _: $crate::macros::__priv::Infallible = $body;
            }
            Err(err) => {
                $crate::macros::__priv::Result::<$crate::macros::__priv::Infallible, _>::Err(err)
            }
        }
    };
    ($exp:expr, Err($pat:pat) => $body:expr $(,)?) => {
        match $exp {
            Err($pat) => {
                #[allow(clippy::diverging_sub_expression)]
                let _: $crate::macros::__priv::Infallible = $body;
            }
            Ok(value) => {
                $crate::macros::__priv::Result::<_, $crate::macros::__priv::Infallible>::Ok(value)
            }
        }
    };
}

/// Creates a lazily-evaluated context from a format string.
///
/// If the format string contains only a literal, it will be converted to a [typed literal][literal].
/// This eliminates all allocations when it's the only component of the error, e.g. building a
/// stateless error from an `Option`.
///
/// [literal]: crate::context::Literal
///
/// # Examples
///
/// ```
/// # use erratic::*;
/// # fn foo() -> Result<()> {
/// # let foo = || -> std::result::Result<(), std::io::Error> { unimplemented!() };
/// # let stream_id = 1;
/// // A plain literal, no allocation.
/// foo().with_context(mkctx!("file not found"))?;
/// // A runtime value, one allocation for the error.
/// foo().with_context(stream_id)?;
/// // With format args, the format string adds a second allocation when materializing the error.
/// foo().with_context(mkctx!("failed to read from stream {stream_id}"))?;
/// # Ok(())
/// # }
/// ```
#[macro_export]
macro_rules! mkctx {
    ($fmt:literal $($args:tt)*) => {{
        struct Literal;

        impl $crate::context::Literal for Literal {
            const LITERAL: &'static str = $fmt;
        }

        $crate::context::Mkctx::__priv_new(|| -> $crate::macros::__priv::Option<$crate::macros::__priv::String> {
            let args = $crate::macros::__priv::format_args!($fmt $($args)*);

            if args.as_str().is_some() {
                return $crate::macros::__priv::None;
            }

            $crate::macros::__priv::Some($crate::macros::__priv::ToString::to_string(&args))
        }, Literal)
    }};
}

/// Constructs an error from a variety of input types.
///
/// If the only component is a string literal or a small state, no allocation occurs. A state is
/// considered "small" when its size is under a pointer and its alignment is relaxed enough to fit
/// within the inline storage.
///
/// # Examples
///
/// ```
/// # use erratic::*;
/// # #[derive(Debug)]
/// # enum State { NotFound }
/// # fn foo() {
/// # let filename = "";
/// # let err = mkerr!("oops").erase();
/// let _: Error = mkerr!("404 not found");
/// let _: Error = mkerr!("{filename} not found");
/// let _: Error = mkerr!("{} not found", filename);
/// let _: Error<State> = mkerr!(state = State::NotFound);
/// let _: Error<State> = mkerr!(
///     state = State::NotFound,
///     error = err,
///     "failed to open {filename}",
/// );
/// # }
/// ```
///
/// # Format String
///
/// The format string is mutually exclusive with the context.
///
/// # Argument Order
///
/// Key-value pairs can be provided in any order, but must appear **before** the format string.
#[macro_export]
macro_rules! mkerr {
    ($($key:ident=$value:expr),+ $(, $($fmt:literal $($args:tt)*)?)?) => {
        $crate::__priv_mkerr!(@sort [$crate::Error] [,,] $($key=$value,)+ $($(context=$crate::mkctx!($fmt $($args)*),)?)?)
    };
    ($fmt:literal $($args:tt)*) => {{
        <$crate::Error>::from_context($crate::mkctx!($fmt $($args)*))
    }};
}

#[macro_export]
#[doc(hidden)]
macro_rules! __priv_mkerr {
    (@sort [$fallback:ty] [$($_:expr)?, $($c:expr)?,  $($e:expr)?] state=$s:expr, $($k:ident=$v:expr,)*) => {{
        $( let _ = $_; $crate::macros::__priv::compile_error!("state can only be set once");)?
        $crate::__priv_mkerr!(@sort [$fallback] [$s, $($c)?, $($e)?] $($k=$v,)*)
    }};
    (@sort [$fallback:ty] [$($s:expr)?, $($_:expr)?,  $($e:expr)?] context=$c:expr, $($k:ident=$v:expr,)*) => {{
        $( let _ = $_; $crate::macros::__priv::compile_error!("context can only be set once. note: the format string counts as a context.");)?
        $crate::__priv_mkerr!(@sort [$fallback] [$($s)?, $c, $($e)?] $($k=$v,)*)
    }};
    (@sort [$fallback:ty] [$($s:expr)?, $($c:expr)?,  $($_:expr)?] error=$e:expr, $($k:ident=$v:expr,)*) => {{
        $( let _ = $_; $crate::macros::__priv::compile_error!("error can only be set once");)?
        $crate::__priv_mkerr!(@sort [$fallback] [$($s)?, $($c)?, $e] $($k=$v,)*)
    }};
    (@sort [$fallback:ty] [$($s:expr)?, $($c:expr)?,  $($e:expr)?]) => {{
        let builder = ($crate::macros::__priv::None::<()>);
        $(let builder = builder.ok_or($e);)?
        $(let builder = $crate::BuilderExt::with_state(builder, $s);)?
        $(let builder = $crate::BuilderExt::with_context(builder, $c);)?
        $crate::__priv_mkerr!(@infer [$fallback] [$($s)?] builder.unwrap_err())
    }};
    (@infer [$fallback:ty] [] $builder:expr) => {
        $crate::macros::__priv::Into::<$fallback>::into($builder)
    };
    (@infer [$fallback:ty] [$state:expr] $builder:expr) => {
        $crate::ErrorExt::build_error($builder)
    };
}

/// Shorthand for constructing an error wrapped in `Result`, with its state type inferred.
#[macro_export]
macro_rules! mkres {
    ($($key:ident=$value:expr),+ $(, $($fmt:literal $($args:tt)*)?)?) => {
        $crate::macros::__priv::Err(
            $crate::__priv_mkerr!(@sort [$crate::Error<_>] [,,] $($key=$value,)+ $($(context=$crate::mkctx!($fmt $($args)*),)?)?)
        )
    };
    ($fmt:literal $($args:tt)*) => {
        $crate::macros::__priv::Err($crate::mkerr!($fmt $($args)*).with_phantom_state())
    };
}

// Autoref specialization for mksure to printf operands.
// https://github.com/dtolnay/case-studies/tree/056fa5ca3d6cbfa4d8ee12bd37abd8a375029bcd/autoref-specialization
#[doc(hidden)]
pub mod __priv_mksure {
    use core::fmt::Debug;

    pub struct FromAll;

    impl FromAll {
        pub fn from(self, _value: impl Sized) -> Option<&'static dyn Debug> {
            None
        }
    }

    pub trait SelectAll {
        fn select(&self) -> FromAll {
            FromAll
        }
    }

    impl<D> SelectAll for &D {}

    pub struct FromDebug;

    impl FromDebug {
        pub fn from(self, value: &impl Debug) -> Option<&dyn Debug> {
            Some(value)
        }
    }

    pub trait SelectDebug {
        fn select(&self) -> FromDebug {
            FromDebug
        }
    }

    impl<E: Debug> SelectDebug for E {}
}

/// Returns an error if the given expression evaluates to false.
///
/// For comparison expressions, the default error shows the values of both operands.
/// If a source, context, or format string is given, the default message is attached as the source error.
///
/// # Examples
///
/// ```
/// # struct Value;
/// # use erratic::*;
/// # use std::result::Result;
/// const PNG_HEADER_SIZE: usize = 33;
///
/// #[derive(Debug)]
/// enum State { UnsupportedFormat }
///
/// fn read_png_header(filename: &str, buffer: &mut [u8]) -> Result<(), Error<State>> {
///     mksure!(buffer.len() > PNG_HEADER_SIZE)?;
///     // Error: assertion failed (0 > 33): buffer.len() > PNG_HEADER_SIZE
///
///     mksure!(buffer.len() > PNG_HEADER_SIZE, context = 501)?;
///     // Error: 501
///     // Source: assertion failed (0 > 33): buffer.len() > PNG_HEADER_SIZE
///     
///     mksure!(!filename.ends_with(".png"), "expect a PNG file, found `{filename}`")?;
///     // Error: expect a PNG file, found `foo.jpg`
///     // Source: assertion failed: !filename.ends_with(".png")
///     
///     mksure!(!filename.ends_with(".png"), state = State::UnsupportedFormat)?;
///     // Error: State::UnsupportedFormat
///     // Source: assertion failed: !filename.ends_with(".png")
///
///     mksure!(!filename.ends_with(".png"),
///         state = State::UnsupportedFormat,
///         "expect a PNG file, found `{filename}`"
///     )?;
///     // Error: State::UnsupportedFormat: expect a PNG file, found `foo.jpg`
///     // Source: assertion failed: !filename.ends_with(".png")
///     
///     // ..
///     # todo!()
/// }
/// ```
#[macro_export]
macro_rules! mksure {
    ($($exp:tt)*) => {
        $crate::__priv_mksure!(@conv [$($exp)*])
    };
    ($exp:expr, $fmt:literal $($args:tt)*) => {
        $crate::macros::__priv::compile_error!("for docs only, an equivalent impl is inside the first branch");
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __priv_mksure {
    (@conv [$($exp:tt)*]) => {
        $crate::__priv_mksure!([] $($exp)*)
    };
    ([$($lhs:tt)*] > $($rhs:tt)+) => {
        $crate::__priv_mksure!(@cmp [$($lhs)*] [>] [$($rhs)*])
    };
    ([$($lhs:tt)*] < $($rhs:tt)+) => {
        $crate::__priv_mksure!(@cmp [$($lhs)*] [<] [$($rhs)*])
    };
    ([$($lhs:tt)*] >= $($rhs:tt)+) => {
        $crate::__priv_mksure!(@cmp [$($lhs)*] [>=] [$($rhs)*])
    };
    ([$($lhs:tt)*] <= $($rhs:tt)+) => {
        $crate::__priv_mksure!(@cmp [$($lhs)*] [<=] [$($rhs)*])
    };
    ([$($lhs:tt)*] == $($rhs:tt)+) => {
        $crate::__priv_mksure!(@cmp [$($lhs)*] [==] [$($rhs)*])
    };
    ([$($lhs:tt)*] != $($rhs:tt)+) => {
        $crate::__priv_mksure!(@cmp [$($lhs)*] [!=] [$($rhs)*])
    };
    ([$($lhs:tt)*] , $($rhs:tt)*) => {
        $crate::__priv_mksure!([$($lhs)*, $($rhs)*])
    };
    ([$($lhs:tt)*] $token:tt $($rhs:tt)*) => {
        $crate::__priv_mksure!([$($lhs)* $token] $($rhs)*)
    };
    ([$exp:expr $(, $($($key:ident=$value:expr),+ $(, $($fmt:literal $($args:tt)*)?)?)?)?]) => {
        $crate::__priv_mksure!(@fallback [$exp] [$($($($key=$value),+)?)?] [$($($($($fmt $($args)*)?)?)?)?])
    };
    ([$exp:expr $(, $($fmt:literal $($args:tt)*)?)?]) => {
        $crate::__priv_mksure!(@fallback [$exp] [] [$($($fmt $($args)*)?)?])
    };
    (@cmp [$lhs:expr] [$op:tt] [$rhs:expr $(, $($($key:ident=$value:expr),+ $(, $($fmt:literal $($args:tt)*)?)?)?)?]) => {
        $crate::__priv_mksure!(@cmp_impl [$lhs] [$op] [$rhs] [$($($($key=$value),+)?)?] [$($($($($fmt $($args)*)?)?)?)?])
    };
    (@cmp [$lhs:expr] [$op:tt] [$rhs:expr $(, $($fmt:literal $($args:tt)*)?)?]) => {
        $crate::__priv_mksure!(@cmp_impl [$lhs] [$op] [$rhs] [] [$($($fmt $($args)*)?)?])
    };
    (@cmp_impl [$lhs:expr] [$op:tt] [$rhs:expr] [$($key:ident=$value:expr),*] [$($fmt:literal $($args:tt)*)?]) => {'ret: {
        #[allow(unused_imports)]
        use $crate::macros::__priv_mksure::{SelectAll, SelectDebug};

        let lhs = $lhs;
        let rhs = $rhs;

        if lhs $op rhs {
            break 'ret $crate::macros::__priv::Ok(());
        }

        let lhs_value = (&lhs).select().from(&lhs);
        let rhs_value = (&rhs).select().from(&rhs);

        let err = match (lhs_value, rhs_value) {
            ($crate::macros::__priv::Some(lhs_value), $crate::macros::__priv::Some(rhs_value)) => {
                $crate::mkerr!(
                    "assertion failed ({}): {lhs_value:?} {} {rhs_value:?}",
                    $crate::macros::__priv::stringify!(assertion failed: $lhs $op $rhs),
                    $crate::macros::__priv::stringify!($op)
                )
            },
            _ => {
                struct Literal;

                impl $crate::context::Literal for Literal {
                    const LITERAL: &'static str = $crate::macros::__priv::stringify!(assertion failed: $lhs $op $rhs);
                }

                let ctx = $crate::context::Mkctx::__priv_new(|| -> $crate::macros::__priv::Option<$crate::macros::__priv::String> {
                    None
                }, Literal);

                $crate::mkerr!(context=ctx)
            }
        };

        $crate::__priv_mksure!(@check [$($key)*] [] {
            $crate::__priv_mksure!(@mkres [err] [$($key=$value),*] [$($fmt $($args)*)?])
        })
    }};
    (@fallback [$exp:expr] [$($key:ident=$value:expr),*] [$($fmt:literal $($args:tt)*)?]) => {'ret: {
        if $exp {
            break 'ret $crate::macros::__priv::Ok(());
        }

        struct Literal;

        impl $crate::context::Literal for Literal {
            const LITERAL: &'static str = $crate::macros::__priv::stringify!(assertion failed: $exp);
        }

        let err: $crate::Error = $crate::Error::from_context($crate::context::Mkctx::__priv_new(|| {
            $crate::macros::__priv::None
        }, Literal));

        $crate::__priv_mksure!(@check [$($key)*] [] {
            $crate::__priv_mksure!(@mkres [err] [$($key=$value),*] [$($fmt $($args)*)?])
        })
    }};
    (@check [error $($key:ident)*] [$($stateful:ident)?] { $_:expr }) => {
        $crate::macros::__priv::compile_error!("builder key `error` is not allowed in assertions")
    };
    (@check [state $($key:ident)*] [$($stateful:ident)?] { $mkres:expr }) => {
        $crate::__priv_mksure!(@check [$($key)*] [STATEFUL] { $mkres })
    };
    (@check [$_:ident $($key:ident)*] [$($stateful:ident)?] { $mkres:expr }) => {
        $crate::__priv_mksure!(@check [$($key)*] [$($stateful)?] { $mkres })
    };
    (@check [] [STATEFUL] { $mkres:expr }) => {
        $mkres
    };
    (@check [] [] { $mkres:expr }) => {
        $crate::macros::__priv::identity::<$crate::Result<()>>($mkres)
    };
    (@mkres [$err:ident] [$($key:ident=$value:expr),*] [$($fmt:literal $($args:tt)*)?]) => {
        $crate::mkres!(error=$err, $($key=$value,)* $($fmt, $($args)*)?)
    };
    (@mkres [$err:ident] [] []) => {
        $crate::macros::__priv::Ok($err)
    }
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
            return mkres!("test");
        };
        let _ = || -> result::Result<(), Error<i32>> {
            return mkres!("test");
        };
    }

    // Test that the macros can be used with various types of input.

    #[test]
    fn error_from_literal() {
        let _ = mkerr!("test");
    }

    #[test]
    fn error_from_format_string() {
        let filename = "file.txt";
        let _ = mkerr!("{} not found", filename);
    }

    #[test]
    fn error_from_kvs() {
        let err_from_mkerr = mkerr!(
            state = 42,
            context = "test",
            error = mkerr!("source").erase(),
        );
        let err_from_builder = Builder::with_error(mkerr!("source").erase())
            .with_state(42)
            .with_context("test")
            .build_error();

        assert_eq!(
            format!("{err_from_mkerr:#}"),
            format!("{err_from_builder:#}")
        );
    }

    #[test]
    fn error_from_kvs_unordered() {
        let err_from_mkerr = mkerr!(
            context = "test",
            error = mkerr!("source").erase(),
            state = 42,
        );
        let err_from_builder = Builder::with_error(mkerr!("source").erase())
            .with_state(42)
            .with_context("test")
            .build_error();

        assert_eq!(
            format!("{err_from_mkerr:#}"),
            format!("{err_from_builder:#}")
        );
    }

    #[test]
    fn error_from_hybrid() {
        let world = "world!";
        let err_from_mkerr = mkerr!(
            error = mkerr!("source").erase(),
            state = 42,
            "hello {world}"
        );
        let err_from_builder = Builder::with_error(mkerr!("source").erase())
            .with_state(42)
            .with_context(format!("hello {world}"))
            .build_error();

        assert_eq!(
            format!("{err_from_mkerr:#}"),
            format!("{err_from_builder:#}")
        );
    }

    #[test]
    fn infer_default_state_if_state_is_not_specified() {
        let _: Error<i32> = mkerr!(context = "test").into();
        let _ = || -> result::Result<(), Error<i32>> {
            return mkres!(context = "test");
        };
    }

    #[test]
    fn no_need_for_type_hint_if_state_is_specified() {
        let _ = mkerr!(state = 42, context = "test");
        let _ = mkerr!(context = "test");
    }

    // Test that the macros can select format string or literal based on the input.

    #[test]
    fn error_from_literal_like_format_string() {
        let filename = "file.txt";
        let err = mkerr!("{filename} not found");
        assert!(err.has_context_of::<String>());
    }

    #[test]
    fn error_from_literal_without_allocation() {
        let err = mkerr!("file not found");
        assert!(!err.has_context_of::<String>());
    }

    #[test]
    fn mkerr_and_mkres_share_same_capabilities() {
        let world = "world";
        let exclamation = "!";
        let err_from_mkerr = mkerr!(
            error = mkerr!("source").erase(),
            state = 42,
            "hello {world}{}",
            exclamation,
        );
        let err_from_mkres: result::Result<(), _> = mkres!(
            error = mkerr!("source").erase(),
            state = 42,
            "hello {world}{}",
            exclamation,
        );
        assert_eq!(
            err_from_mkerr.to_string(),
            err_from_mkres.unwrap_err().to_string()
        );
    }

    #[test]
    fn mkctx_is_lazy() {
        use core::sync::atomic::{AtomicBool, Ordering};

        static CALLED: AtomicBool = AtomicBool::new(false);

        struct CallTracker;

        impl core::fmt::Display for CallTracker {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                CALLED.store(true, Ordering::SeqCst);
                write!(f, "tracked")
            }
        }

        // mkctx creates a closure; the closure is not called yet
        let builder =
            Builder::with_error(mkerr!("oops").erase()).with_context(mkctx!("{}", CallTracker));

        assert!(
            !CALLED.load(Ordering::SeqCst),
            "mkctx should not execute the closure before materialization"
        );

        // Materialize the error, which runs the closure
        let _err: Error = builder.build_error();

        assert!(
            CALLED.load(Ordering::SeqCst),
            "mkctx should execute the closure when materialized"
        );
    }

    #[test]
    fn mkctx_plain_literal_does_not_allocate() {
        let ctx = mkctx!("hello");
        assert!(
            ctx.try_into_repr().is_none(),
            "mkctx with a plain literal should not allocate"
        );

        let name = "world";
        let ctx = mkctx!("hello {}", name);
        assert_eq!(
            ctx.try_into_repr(),
            Some("hello world".into()),
            "mkctx with format args should allocate"
        );

        let ctx = mkctx!("hello {name}");
        assert_eq!(
            ctx.try_into_repr(),
            Some("hello world".into()),
            "mkctx with format args should allocate"
        );
    }

    #[test]
    fn mksure_compare_non_debug() {
        #[derive(PartialEq, Eq, PartialOrd, Ord, Debug)]
        struct A;

        assert!(mksure!(A > A).is_err());
    }

    #[test]
    fn mksure_compare_debug_with_eval() {
        let magic_number = 123454321;
        let err_msg: Error = mksure!(magic_number != magic_number).unwrap_err();

        assert!(err_msg.to_string().find("123454321").is_some());
    }

    #[test]
    fn mksure_assert_with_message() {
        let magic_number = -123454321i32;
        let err: Error = mksure!(
            magic_number.is_positive(),
            "magic number must be greater than zero"
        )
        .unwrap_err();

        assert_eq!(err.chain().count(), 2);
        assert_eq!(err.to_string(), "magic number must be greater than zero");
        assert!(
            err.root()
                .unwrap()
                .to_string()
                .find("magic_number.is_positive()")
                .is_some()
        );
    }

    #[test]
    fn mksure_compare_with_message() {
        let magic_number = -123454321;
        let err = mksure!(magic_number > 0, "magic number must be greater than zero").unwrap_err();

        assert_eq!(err.chain().count(), 2);
        assert_eq!(err.to_string(), "magic number must be greater than zero");
        assert!(err.root().unwrap().to_string().find("-123454321").is_some());
    }

    #[test]
    fn mksure_compare_with_message_args() {
        let magic_number = -123454321;
        let lower_bound = 32;
        let err = mksure!(
            magic_number > lower_bound,
            "magic number must be greater than {lower_bound}"
        )
        .unwrap_err();

        assert_eq!(err.chain().count(), 2);
        assert_eq!(
            err.to_string(),
            format!("magic number must be greater than {lower_bound}")
        );
        assert!(err.root().unwrap().to_string().find("-123454321").is_some());
    }

    #[test]
    fn mksure_compare_with_state() {
        let magic_number = -123454321;
        let err = mksure!(magic_number > 0, state = -1i32).unwrap_err();

        assert_eq!(err.chain().count(), 2);
        assert_eq!(err.to_string(), format!("-1"));
        assert!(err.root().unwrap().to_string().find("-123454321").is_some());
    }

    #[test]
    fn mksure_compare_with_context() {
        let magic_number = -123454321;
        let err = mksure!(magic_number > 0, context = 670).unwrap_err();

        assert_eq!(err.chain().count(), 2);
        assert_eq!(err.to_string(), format!("670"));
        assert!(err.root().unwrap().to_string().find("-123454321").is_some());
    }

    #[test]
    fn mksure_compare_with_state_and_message_args() {
        let magic_number = -123454321;
        let lower_bound = 32;
        let err = mksure!(
            magic_number > lower_bound,
            state = -1i32,
            "magic number must be greater than {lower_bound}"
        )
        .unwrap_err();

        assert_eq!(err.chain().count(), 2);
        assert_eq!(
            err.to_string(),
            format!("-1: magic number must be greater than {lower_bound}")
        );
        assert!(err.root().unwrap().to_string().find("-123454321").is_some());
    }

    #[test]
    fn mksure_returns_error() {
        fn mksure_returns_error_() -> crate::Result<()> {
            mksure!(false)?;
            Ok(())
        }
        assert!(mksure_returns_error_().is_err());
    }
}
