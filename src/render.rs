use core::{
    error,
    fmt::{self, Debug, Display},
};

use alloc::format;

use crate::rtti;

pub struct DisplayAsDebug<'a>(pub &'a dyn Display);

impl<'a> Debug for DisplayAsDebug<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, r#""{}""#, self.0)
    }
}

pub struct DebugAsDisplay<'a>(pub &'a dyn Debug);

impl<'a> Display for DebugAsDisplay<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

pub struct DebugSourceChain<'a>(pub &'a dyn error::Error);

impl<'a> fmt::Debug for DebugSourceChain<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut list = f.debug_list();

        let mut next_source = Some(self.0);
        while let Some(source) = next_source {
            next_source = source.source();

            list.entry(&format!("{source:-}"));
        }

        list.finish()
    }
}

pub fn format_debug<S>(
    f: &mut fmt::Formatter<'_>,
    state: Option<&S>,
    context: Option<&(dyn Display + Send + Sync + 'static)>,
    payload: Option<&(dyn Display + Send + Sync + 'static)>,
    source: Option<&(dyn error::Error + 'static)>,
    backtrace: Option<&dyn Debug>,
) -> fmt::Result
where
    S: Debug + 'static,
{
    let show_less = f.sign_minus();
    let ds = &mut f.debug_struct("Error");

    if !rtti::is_same_ty::<S, ()>()
        && let Some(state) = state
    {
        ds.field("state", state);
    }

    if let Some(context) = context {
        ds.field("context", &DisplayAsDebug(context));
    }

    if let Some(payload) = payload {
        ds.field("payload", &DisplayAsDebug(payload));
    }

    if let Some(source) = source {
        ds.field("source", &DebugSourceChain(source));
    }

    if !show_less && let Some(backtrace) = backtrace {
        ds.field("backtrace", &backtrace);
    }

    ds.finish()
}

pub fn format_display<S>(
    f: &mut fmt::Formatter<'_>,
    state: Option<&S>,
    context: Option<&(dyn Display + Send + Sync + 'static)>,
    payload: Option<&(dyn Display + Send + Sync + 'static)>,
    source: Option<&(dyn error::Error + 'static)>,
    backtrace: Option<&dyn Display>,
) -> fmt::Result
where
    S: Debug + 'static,
{
    let show_less = f.sign_minus();
    let state = state.map(|s| DebugAsDisplay(s));

    if f.alternate() {
        let mut segments = [
            state.as_ref().map(|s| s as &dyn Display),
            context.map(|s| s as _),
            payload.map(|s| s as _),
        ]
        .into_iter()
        .flatten()
        .peekable();
        let has_additional_info = segments.peek().is_some();

        while let Some(segment) = segments.next() {
            write!(f, "{segment}")?;

            if segments.peek().is_some() {
                write!(f, ": ")?;
            }
        }

        if let Some(source) = source {
            if has_additional_info {
                write!(f, "\n  -> ")?;
            }
            write!(f, "{source:-#}")?;
        }
    } else {
        match (&state, context, payload, source) {
            (None, None, None, None) => unreachable!(),
            (None, None, None, Some(err)) => {
                Display::fmt(err, f)?;
            }
            _ => {
                let mut segments = [
                    state.as_ref().map(|s| s as &dyn Display),
                    context.map(|s| s as _),
                    payload.map(|s| s as _),
                ]
                .into_iter()
                .flatten()
                .peekable();

                while let Some(segment) = segments.next() {
                    write!(f, "{}", segment)?;

                    if segments.peek().is_some() {
                        write!(f, ": ")?;
                    }
                }
            }
        }
    }

    if !show_less && let Some(backtrace) = backtrace {
        write!(f, "\n{backtrace}")?;
    }

    Ok(())
}
