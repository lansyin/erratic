//! Trait for defining custom formatters.
use core::{
    convert::Infallible,
    error,
    fmt::{self, Debug, Display},
    result,
};

/// A formatter for [`Error`][crate::Error], works with [`FormatWith`][crate::state::FormatWith].
pub trait Formatter: 'static {
    fn format_debug(
        f: &mut fmt::Formatter<'_>,
        context: Option<impl Debug + Display>,
        source: Option<&(dyn error::Error + 'static)>,
        backtrace: Option<impl Debug + Display>,
    ) -> fmt::Result {
        format_debug(f, None::<&Infallible>, context, source, backtrace)
    }

    fn format_display(
        f: &mut fmt::Formatter<'_>,
        context: Option<impl Debug + Display>,
        source: Option<&(dyn error::Error + 'static)>,
        backtrace: Option<impl Debug + Display>,
    ) -> fmt::Result {
        format_display(f, None::<&Infallible>, context, source, backtrace)
    }
}

pub(crate) trait DebugDisplay: Debug + Display {}

impl<T> DebugDisplay for T where T: Debug + Display {}

fn format_state_context(
    f: &mut fmt::Formatter<'_>,
    state: Option<impl Debug>,
    context: Option<impl Display>,
) -> result::Result<bool, fmt::Error> {
    match (state, context) {
        (None, None) => return Ok(false),
        (None, Some(context)) => {
            write!(f, "{}", context)?;
        }
        (Some(state), None) => {
            write!(f, "{state:?}")?;
        }
        (Some(state), Some(context)) => {
            write!(f, "{:?}: {}", state, context)?;
        }
    }
    Ok(true)
}

pub(crate) fn format_debug_struct<S>(
    f: &mut fmt::Formatter<'_>,
    container_name: &'static str,
    state: Option<&S>,
    context: Option<impl Debug>,
    source: Option<&(dyn error::Error + 'static)>,
    backtrace: Option<impl Debug + Display>,
) -> fmt::Result
where
    S: Debug + 'static,
{
    let ds = &mut f.debug_struct(container_name);

    if let Some(state) = state {
        ds.field("state", state);
    }

    if let Some(context) = context {
        ds.field("context", &context);
    }

    if let Some(source) = source {
        ds.field("source", &source);
    }

    if let Some(backtrace) = backtrace {
        ds.field("backtrace", &backtrace);
    }

    ds.finish()
}

fn format_chain<S>(
    f: &mut fmt::Formatter<'_>,
    state: Option<&S>,
    context: Option<impl Display>,
    source: Option<&(dyn error::Error + 'static)>,
) -> fmt::Result
where
    S: Debug + 'static,
{
    const SOURCE_PREFIX: &str = "\n  -> ";

    let mut source = source;
    let has_additional_info = format_state_context(f, state, context.as_ref())?;

    if !has_additional_info {
        let Some(err) = source else {
            unreachable!();
        };
        // dedup.format_one(&mut Blackhole, |w| write!(w, "{SOURCE_PREFIX}{err}"))?;
        write!(f, "{err}")?;
        source = err.source();
    }

    while let Some(err) = source {
        // dedup.format_one(f, |w| write!(w, "{SOURCE_PREFIX}{err}"))?;
        write!(f, "{SOURCE_PREFIX}{err}")?;
        source = err.source();
    }

    Ok(())
}

pub(crate) fn format_debug<S>(
    f: &mut fmt::Formatter<'_>,
    state: Option<&S>,
    context: Option<impl Debug + Display>,
    source: Option<&(dyn error::Error + 'static)>,
    backtrace: Option<impl Debug + Display>,
) -> fmt::Result
where
    S: Debug + 'static,
{
    let show_less = f.sign_minus();

    if f.alternate() {
        format_debug_struct(
            f,
            "Error",
            state,
            context,
            source,
            (!show_less).then_some(backtrace).flatten(),
        )
    } else {
        format_chain(f, state, context, source)?;

        if !show_less && let Some(backtrace) = backtrace {
            write!(f, "\nBacktrace:\n{backtrace}")?;
        }

        Ok(())
    }
}

pub(crate) fn format_display<S>(
    f: &mut fmt::Formatter<'_>,
    state: Option<&S>,
    context: Option<impl Display>,
    source: Option<&(dyn error::Error + 'static)>,
    _backtrace: Option<impl Display>,
) -> fmt::Result
where
    S: Debug + 'static,
{
    if f.alternate() {
        format_chain(f, state, context, source)?;
    } else {
        let has_additional_info = format_state_context(f, state, context.as_ref())?;

        if !has_additional_info {
            let Some(err) = source else {
                unreachable!();
            };
            Display::fmt(err, f)?;
        }
    }

    Ok(())
}
