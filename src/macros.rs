#[doc(hidden)]
pub mod __priv {
    pub use alloc::{
        format,
        string::{String, ToString},
    };
    pub use core::{
        any::Any,
        compile_error,
        convert::{Infallible, Into, identity},
        fmt::Debug,
        format_args,
        option::Option::{self, None, Some},
        result::Result::{self, Err, Ok},
        stringify, unreachable,
    };
}

/// Like `let-else`, with access to variant bindings in other branches, for `Result` only.
///
/// It's useful for flattening nested `match`es, and pairs well with
/// [`extract_state`][crate::StateExt::extract_state].
///
/// # Examples
///
/// ```
/// # use erratic::*;
/// # #[derive(Debug)]
/// # struct Opaque;
/// # struct Foo;
/// # struct Bar;
/// # enum Typed { Foo(Foo), Bar(Bar) }
/// # fn try_into_foo(obj: Opaque) -> Result<Foo, Opaque> { Err(obj) }
/// # fn try_into_bar(obj: Opaque) -> Result<Bar, Opaque> { Err(obj) }
/// # fn parse_object(obj: Opaque) -> Typed {
/// let Err(obj) = match_else!(try_into_foo(obj), Ok(foo) => {
///     return Typed::Foo(foo);
/// });
/// let Err(obj) = match_else!(try_into_bar(obj), Ok(bar) => {
///     return Typed::Bar(bar);
/// });
/// panic!("expect a `Foo` or a `Bar`, found: {obj:?}");
/// # }
/// ```
///
/// ```
/// # use erratic::*;
/// # #[derive(Debug)]
/// # enum State { Unauthorized }
/// # struct Token;
/// # mod cli {
/// #     pub fn inquiry_credential() -> &'static str { "user" }
/// # }
/// # fn login(_: &str) -> Result<Token, Error<State>> { unimplemented!() }
/// # fn authenticate() -> Result<Token, Error<State>> {
/// loop {
///     let cred = cli::inquiry_credential();
///     let Ok(token) = match_else!(login(cred).extract_state()?, Err((state, _)) => match state {
///         State::Unauthorized => continue,
///     });
///     todo!()
/// }
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
            Err(err) => $crate::Result::<$crate::macros::__priv::Infallible, _>::Err(err),
        }
    };
    ($exp:expr, Err($pat:pat) => $body:expr $(,)?) => {
        match $exp {
            Err($pat) => {
                #[allow(clippy::diverging_sub_expression)]
                let _: $crate::macros::__priv::Infallible = $body;
            }
            Ok(value) => $crate::Result::<_, $crate::macros::__priv::Infallible>::Ok(value),
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
/// # fn bar() -> Result<()> {
/// # use std::env;
/// # let username = 1;
/// // A plain literal; no allocation.
/// let home = env::home_dir()
///     .with_context(mkctx!("failed to get the home directory"))?;
///
/// // A runtime value; materializing the error costs one allocation.
/// // It's a pity this case can't be optimized without a macro,
/// // but since we usually work with `Result`, such cases are rare.
/// let home = env::home_dir() // -> Option<PathBuf>
///     .with_context("failed to get the home directory")?;
///
/// // With format arguments, the format string adds a second allocation
/// // when the error is materialized.
/// let home = env::home_dir()
///     .with_context(mkctx!("failed to get the home directory for {username}"))?;
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

        $crate::context::Mkctx::<Literal, _>::__priv_new(|| {
            let args = $crate::macros::__priv::format_args!($fmt $($args)*);

            if args.as_str().is_some() {
                $crate::macros::__priv::None
            } else {
                $crate::macros::__priv::Some($crate::macros::__priv::ToString::to_string(&args))
            }
        })
    }};
}

/// Constructs an error from a variety of input types, with its state type inferred.
///
/// If the only component is a string literal or a [small][small] state, no allocation occurs.
///
/// [small]: crate::Error::from_state
///
/// # Allowed Keys
///
/// - [`context`][crate::BuilderExt::with_context]
/// - [`state`][crate::BuilderExt::with_state]
/// - [`error`][crate::builder::Builder::with_error]
///
/// Key-value pairs can be provided in any order, but must appear **before** the format string.
///
/// # Format String
///
/// The format string is mutually exclusive with the `context` key.
///
/// # Examples
///
/// ```
/// # use erratic::*;
/// # #[derive(Debug)]
/// # enum State { NotFound }
/// # fn foo() {
/// # let filename = "";
/// # let err = std::fmt::Error;
/// let _: Error = mkerr!("404 not found");
/// let _: Error = mkerr!("{filename} not found");
/// let _: Error = mkerr!("{} not found", filename);
/// let _: Error<State> = mkerr!(state = State::NotFound);
/// let _: Error<State> = mkerr!(state = State::NotFound, context = filename);
/// let _: Error<State> = mkerr!(
///     state = State::NotFound,
///     error = err,
///     "failed to open {filename}",
/// );
/// # }
/// ```
#[macro_export]
macro_rules! mkerr {
    ($($key:ident=$value:expr),+ $(, $($fmt:literal $($args:tt)*)?)?) => {
        $crate::__priv_mkerr!(@sort [_] [,,] $($key=$value,)+ $($(context=$crate::mkctx!($fmt $($args)*),)?)?)
    };
    ($fmt:literal $($args:tt)*) => {{
        $crate::Error::from_context($crate::mkctx!($fmt $($args)*))
    }};
}

#[macro_export]
#[doc(hidden)]
macro_rules! __priv_mkerr {
    // Note: dst:"default state type", s="state", c="context", e="error"
    (@sort [$dst:tt] [$($_:expr)?, $($c:expr)?,  $($e:expr)?] state=$s:expr, $($k:ident=$v:expr,)*) => {{
        $( let _ = $_; $crate::macros::__priv::compile_error!("state can only be set once");)?
        $crate::__priv_mkerr!(@sort [$dst] [$s, $($c)?, $($e)?] $($k=$v,)*)
    }};
    (@sort [$dst:tt] [$($s:expr)?, $($_:expr)?,  $($e:expr)?] context=$c:expr, $($k:ident=$v:expr,)*) => {{
        $( let _ = $_; $crate::macros::__priv::compile_error!("context can only be set once. note: the format string counts as a context.");)?
        $crate::__priv_mkerr!(@sort [$dst] [$($s)?, $c, $($e)?] $($k=$v,)*)
    }};
    (@sort [$dst:tt] [$($s:expr)?, $($c:expr)?,  $($_:expr)?] error=$e:expr, $($k:ident=$v:expr,)*) => {{
        $( let _ = $_; $crate::macros::__priv::compile_error!("error can only be set once");)?
        $crate::__priv_mkerr!(@sort [$dst] [$($s)?, $($c)?, $e] $($k=$v,)*)
    }};
    (@sort [$dst:tt] [$($s:expr)?, $($c:expr)?,  $($e:expr)?]) => {{
        let builder = ($crate::macros::__priv::None::<()>);
        $(let builder = builder.ok_or($e);)?
        $(let builder = $crate::BuilderExt::with_state(builder, $s);)?
        $(let builder = $crate::BuilderExt::with_context(builder, $c);)?
        $crate::__priv_mkerr!(@infer [$dst] [$($s)?] builder.unwrap_err())
    }};
    (@infer [$dst:tt] [] $builder:expr) => {
        #[allow(unused_parens)]
        $crate::macros::__priv::Into::<$crate::Error<$dst>>::into($builder)
    };
    (@infer [$dst:tt] [$state:expr] $builder:expr) => {
        $crate::ErrorExt::build_error($builder)
    };
}

/// Shorthand for constructing an error wrapped in `Result`, with its state type inferred.
///
/// It accepts the same argument patterns as [`mkerr!`].
#[macro_export]
macro_rules! mkres {
    ($($key:ident=$value:expr),+ $(, $($fmt:literal $($args:tt)*)?)?) => {
        $crate::macros::__priv::Err(
            $crate::__priv_mkerr!(@sort [_] [,,] $($key=$value,)+ $($(context=$crate::mkctx!($fmt $($args)*),)?)?)
        )
    };
    ($fmt:literal $($args:tt)*) => {
        $crate::macros::__priv::Err($crate::mkerr!($fmt $($args)*))
    };
}

// Autoref specialization for mksure to print operands.
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
        fn __erratic_select(&self) -> FromAll {
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
        fn __erratic_select(&self) -> FromDebug {
            FromDebug
        }
    }

    impl<E: Debug> SelectDebug for E {}
}

/// Returns an error if the given expression evaluates to false.
///
/// Except the first argument (the condition), it accepts the same argument patterns as [`mkerr!`].
///
/// For comparison expressions, the default error message shows the values of both operands.
/// This default is omitted when a state, source error, context, or format string is provided.
///
/// # Examples
///
/// ```
/// # struct Value;
/// # use erratic::*;
/// # const PNG_HEADER_SIZE: usize = 33;
/// #[derive(Debug)]
/// enum State { UnsupportedFormat }
///
/// # fn read_png_header(filename: &str, buffer: &mut [u8]) -> Result<(), Error<State>> {
/// mksure!(buffer.len() == PNG_HEADER_SIZE)?;
/// // assertion failed (0 == 33): buffer.len() == PNG_HEADER_SIZE
///
/// mksure!(buffer.len() == PNG_HEADER_SIZE, context = 400)?;
/// // 400
///
/// mksure!(filename.ends_with(".png"))?;
/// // assertion failed: filename.ends_with(".png")
///
/// mksure!(filename.ends_with(".png"), "expected a PNG file, found `{filename}`")?;
/// // expected a PNG file, found `foo.jpg`
///
/// mksure!(filename.ends_with(".png"), state = State::UnsupportedFormat)?;
/// // UnsupportedFormat
///
/// mksure!(filename.ends_with(".png"),
///     state = State::UnsupportedFormat,
///     "expected a PNG file, found `{filename}`"
/// )?;
/// // UnsupportedFormat: expected a PNG file, found `foo.jpg`
///
/// # todo!()
/// # }
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
    ([$($lhs:tt)*] || $($rhs:tt)*) => {
        $crate::__priv_mksure!([$($lhs)* || $($rhs)*])
    };
    ([$($lhs:tt)*] && $($rhs:tt)*) => {
        $crate::__priv_mksure!([$($lhs)* && $($rhs)*])
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
    (@cmp_impl [$lhs:expr] [$op:tt] [$rhs:expr] [] []) => {'ret: {
        #[allow(unused_imports)]
        use $crate::macros::__priv_mksure::{SelectAll, SelectDebug};

        let lhs = $lhs;
        let rhs = $rhs;

        if lhs $op rhs {
            break 'ret $crate::macros::__priv::Ok(());
        }

        let lhs_value = (&lhs).__erratic_select().from(&lhs);
        let rhs_value = (&rhs).__erratic_select().from(&rhs);

        match (lhs_value, rhs_value) {
            ($crate::macros::__priv::Some(lhs_value), $crate::macros::__priv::Some(rhs_value)) => {
                let dc = $crate::mkctx!(
                    "assertion failed ({lhs_value:?} {} {rhs_value:?}): {}",
                    $crate::macros::__priv::stringify!($lhs $op $rhs),
                    $crate::macros::__priv::stringify!($op)
                );
                $crate::Result::<(), _>::Err(
                    $crate::__priv_mkerr!(@sort [($crate::state::Stateless)] [,,] context = dc,)
                )
            },
            _ => {
                struct Literal;
                impl $crate::context::Literal for Literal {
                    const LITERAL: &'static str = $crate::macros::__priv::stringify!(assertion failed: $lhs $op $rhs);
                }
                let dc = $crate::context::Mkctx::<Literal>::__priv_new_const();
                $crate::Result::<(), _>::Err(
                    $crate::__priv_mkerr!(@sort [($crate::state::Stateless)] [,,] context = dc,)
                )
            }
        }
    }};
    (@cmp_impl [$lhs:expr] [$op:tt] [$rhs:expr] [$($key:ident=$value:expr),*] [$($fmt:literal $($args:tt)*)?]) => {'ret: {
        if $lhs $op $rhs {
            break 'ret $crate::macros::__priv::Ok(());
        }
        $crate::Result::<(), _>::Err(
            $crate::__priv_mkerr!(@sort [($crate::state::Stateless)] [,,] $($key=$value,)* $(context=$crate::mkctx!($fmt $($args)*),)?)
        )
    }};
    (@fallback [$exp:expr] [] []) => {'ret: {
        if $exp {
            break 'ret $crate::macros::__priv::Ok(());
        }

        struct Literal;
        impl $crate::context::Literal for Literal {
            const LITERAL: &'static str = $crate::macros::__priv::stringify!(assertion failed: $exp);
        }
        let dc = $crate::context::Mkctx::<Literal>::__priv_new_const();

        $crate::Result::<(), _>::Err(
            $crate::__priv_mkerr!(@sort [($crate::state::Stateless)] [,,] context = dc,)
        )
    }};
    (@fallback [$exp:expr] [$($key:ident=$value:expr),*] [$($fmt:literal $($args:tt)*)?]) => {'ret: {
        if $exp {
            break 'ret $crate::macros::__priv::Ok(());
        }
        $crate::Result::<(), _>::Err(
            $crate::__priv_mkerr!(@sort [($crate::state::Stateless)] [,,] $($key=$value,)* $(context=$crate::mkctx!($fmt $($args)*),)?)
        )
    }};
}

#[cfg(test)]
mod tests {
    use alloc::{
        format,
        string::{String, ToString},
    };

    use crate::{test_fixtures::*, *};

    // Ensure the macros do not require type annotations in the most common cases
    #[test]
    fn type_reference_check() {
        let _ = || -> Result<()> {
            return mkres!("test");
        };
        let _ = || -> Result<(), Error<i32>> {
            return mkres!("test");
        };
    }

    // Test that the macros can be used with various types of input.

    #[test]
    fn error_from_literal() {
        let _: Error = mkerr!("test");
    }

    #[test]
    fn error_from_format_string() {
        let filename = "file.txt";
        let _: Error = mkerr!("{} not found", filename);
    }

    #[test]
    fn error_from_kvs() {
        let err_from_mkerr = mkerr!(
            state = 42,
            context = TestMessage::HOGE,
            error = TestError::FOO,
        );
        let err_from_builder = Builder::with_error(TestError::FOO)
            .with_state(42)
            .with_context(TestMessage::HOGE)
            .build_error();

        assert_eq!(
            format!("{err_from_mkerr:#}"),
            format!("{err_from_builder:#}")
        );
    }

    #[test]
    fn error_from_kvs_unordered() {
        let err_from_mkerr = mkerr!(
            context = TestMessage::HOGE,
            error = TestError::FOO,
            state = 42,
        );
        let err_from_builder = Builder::with_error(TestError::FOO)
            .with_state(42)
            .with_context(TestMessage::HOGE)
            .build_error();

        assert_eq!(
            format!("{err_from_mkerr:#}"),
            format!("{err_from_builder:#}")
        );
    }

    #[test]
    fn error_from_hybrid() {
        let world = "world!";
        let err_from_mkerr = mkerr!(error = TestError::FOO, state = 42, "hello {world}");
        let err_from_builder = Builder::with_error(TestError::FOO)
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
        let _: Error<i32> = mkerr!(context = TestMessage::HOGE);
        let _ = || -> Result<(), Error<i32>> {
            return mkres!(context = TestMessage::HOGE);
        };
    }

    #[test]
    fn no_need_for_type_hint_if_state_is_specified() {
        let _ = mkerr!(state = 42, context = TestMessage::HOGE);
        let _: Error = mkerr!(context = TestMessage::HOGE);
    }

    // Test that the macros can select format string or literal based on the input.

    #[test]
    fn error_from_literal_like_format_string() {
        let filename = "file.txt";
        let err = mkerr!("{filename} not found").stateless();
        assert!(err.has_context_of::<String>());
    }

    #[test]
    fn error_from_literal_without_allocation() {
        let err = mkerr!("file not found").stateless();
        assert!(!err.has_context_of::<String>());
    }

    #[test]
    fn mkerr_and_mkres_share_same_capabilities() {
        let world = "world";
        let exclamation = "!";
        let err_from_mkerr = mkerr!(
            error = TestError::FOO,
            state = 42,
            "hello {world}{}",
            exclamation,
        );
        let err_from_mkres: Result<(), _> = mkres!(
            error = TestError::FOO,
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

        impl fmt::Display for CallTracker {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                CALLED.store(true, Ordering::SeqCst);
                write!(f, "tracked")
            }
        }

        // mkctx creates a closure; the closure is not called yet
        let builder = Builder::with_error(TestError::FOO).with_context(mkctx!("{}", CallTracker));

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
            ctx.select().is_ok(),
            "mkctx with a plain literal should not allocate"
        );

        let name = "world";
        let ctx = mkctx!("hello {}", name);
        assert!(
            ctx.select().is_err(),
            "mkctx with format args should allocate"
        );

        let ctx = mkctx!("hello {name}");
        assert!(
            ctx.select().is_err(),
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

        assert_eq!(err.chain().count(), 1);
        assert_eq!(err.to_string(), "magic number must be greater than zero");
    }

    #[test]
    fn mksure_compare_with_message() {
        let magic_number = -123454321;
        let err = mksure!(magic_number > 0, "magic number must be greater than zero").unwrap_err();

        assert_eq!(err.chain().count(), 1);
        assert_eq!(err.to_string(), "magic number must be greater than zero");
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

        assert_eq!(err.chain().count(), 1);
        assert_eq!(
            err.to_string(),
            format!("magic number must be greater than {lower_bound}")
        );
    }

    #[test]
    fn mksure_compare_with_state() {
        let magic_number = -123454321;
        let err = mksure!(magic_number > 0, state = -1i32).unwrap_err();

        assert_eq!(err.chain().count(), 1);
        assert_eq!(err.to_string(), format!("-1"));
    }

    #[test]
    fn mksure_compare_with_context() {
        let magic_number = -123454321;
        let err = mksure!(magic_number > 0, context = 670).unwrap_err();

        assert_eq!(err.chain().count(), 1);
        assert_eq!(err.to_string(), format!("670"));
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

        assert_eq!(err.chain().count(), 1);
        assert_eq!(
            err.to_string(),
            format!("-1: magic number must be greater than {lower_bound}")
        );
    }

    #[test]
    fn mksure_returns_error() {
        fn mksure_returns_error_() -> crate::Result<()> {
            mksure!(false)?;
            Ok(())
        }
        assert!(mksure_returns_error_().is_err());
    }

    #[test]
    fn mksure_evaluates_expression_once() -> crate::Result<()> {
        // No comparison expression, no explicit context.
        {
            let value = Some(1i32);
            mksure!(value.unwrap().is_positive())?;
        }
        // Comparison expression, no explicit context.
        {
            let value = Some(1i32);
            mksure!(value.unwrap() > 0)?;
        }
        // No comparison expression, explicit context.
        {
            let value = Some(1i32);
            mksure!(value.unwrap().is_positive(), context = TestMessage::HOGE)?;
        }
        // Comparison expression, explicit context.
        {
            let value = Some(1i32);
            mksure!(value.unwrap() > 0, context = TestMessage::HOGE)?;
        }
        Ok(())
    }

    #[test]
    fn mksure_compound_boolean_not_treated_as_single_comparison() -> crate::Result<()> {
        // `a && b > c` == `a && (b > c)`.
        {
            let a = true;
            let b = 3i32;
            let c = 2i32;
            // true && (3 > 2) == true
            mksure!(a && b > c)?;
        }
        // `a || b > c` == `a || (b > c)`.
        {
            let a = false;
            let b = 3i32;
            let c = 2i32;
            // false || (3 > 2) == true
            mksure!(a || b > c)?;
        }
        // `||` short-circuits when `a` is already true.
        {
            let a = true;
            let b = 1i32;
            let c = 2i32;
            // true || (1 > 2) == true
            mksure!(a || b > c)?;
        }
        Ok(())
    }
}
