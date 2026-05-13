#[doc(hidden)]
pub mod __private {
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
                let _: $crate::macros::__private::std::convert::Infallible = $body;
            }
            Err(err) => $crate::macros::__private::std::result::Result::<
                $crate::macros::__private::std::convert::Infallible,
                _,
            >::Err(err),
        }
    };
    ($exp:expr, Err($pat:pat) => $body:expr $(,)?) => {
        match $exp {
            Err($pat) => {
                #[allow(clippy::diverging_sub_expression)]
                let _: $crate::macros::__private::std::convert::Infallible = $body;
            }
            Ok(value) => $crate::macros::__private::std::result::Result::<
                _,
                $crate::macros::__private::std::convert::Infallible,
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
/// # fn foo() {
/// Error::from_context(literal!("file not found"));
///
/// literal!{
///     pub NotFound: "404 not found";
///     pub InternalServerError: "500 internal server error";
/// }
/// Error::from_context(NotFound);
/// Error::from_context_ty::<InternalServerError>();
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

    use crate::{Error, ErrorExt};

    pub struct FromDisplay;

    impl FromDisplay {
        pub fn from(self, value: impl Display) -> Error {
            Error::with_payload(|| format!("{}", value)).build_error()
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
        pub fn from(self, err: impl std::error::Error + Send + Sync + 'static) -> Error {
            Error::with_error(err).build_error()
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
/// return Err(erratic!("404 not found"));
/// return Err(erratic!("{} not found", filename));
/// return Err(erratic!(something_impl_error_or_display));
/// # }
/// ```
#[macro_export]
macro_rules! erratic {
    ($lit:literal $(,)?) => {
        $crate::ErrorExt::build_error($crate::Error::with_context(
            $crate::literal!($lit),
        ))
    };
    ($exp:expr $(,)?) => {{
        #[allow(unused_imports)]
        use $crate::macros::__specialization::{SelectDisplay, SelectError};

        match $exp {
            err => (&err).select().from(err),
        }
    }};
    ($fmt:expr, $($arg:tt)*) => {
        $crate::ErrorExt::build_error($crate::Error::with_payload(
            || $crate::macros::__private::std::format!($fmt, $($arg)*)
        ))
    };
}
