use core::{
    error::{self},
    fmt::{self, Debug, Display, Write},
    result,
};

use alloc::string::String;

use crate::backtrace::Backtrace;

struct DisplayAsDebug<'a>(pub &'a dyn Display);

impl<'a> Debug for DisplayAsDebug<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, r#""{}""#, self.0)
    }
}

fn format_state_context_payload(
    f: &mut fmt::Formatter<'_>,
    state: Option<&impl Debug>,
    context: Option<&impl Display>,
    payload: Option<&impl Display>,
) -> result::Result<bool, fmt::Error> {
    let context_payload = match (context, payload) {
        (None, None) => None,
        (None, Some(payload)) => Some(format_args!("{}", *payload)),
        (Some(context), None) => Some(format_args!("{}", *context)),
        (Some(context), Some(payload)) => Some(format_args!("{}{}", *context, *payload)),
    };
    match (state, context_payload) {
        (None, None) => return Ok(false),
        (None, Some(context_payload)) => {
            write!(f, "{}", context_payload)?;
        }
        (Some(state), None) => {
            write!(f, "{state:?}")?;
        }
        (Some(state), Some(context_payload)) => {
            write!(f, "{:?}: {}", state, context_payload)?;
        }
    }
    Ok(true)
}

pub fn format_debug<S>(
    f: &mut fmt::Formatter<'_>,
    container_name: &'static str,
    state: Option<&S>,
    context: Option<&(dyn Display + Send + Sync + 'static)>,
    payload: Option<&(dyn Display + Send + Sync + 'static)>,
    source: Option<&(dyn error::Error + 'static)>,
    backtrace: Option<&dyn Backtrace>,
) -> fmt::Result
where
    S: Debug + 'static,
{
    let show_less = f.sign_minus();

    if f.alternate() {
        let ds = &mut f.debug_struct(container_name);

        if let Some(state) = state {
            ds.field("state", state);
        }

        if let Some(context) = context {
            ds.field("context", &DisplayAsDebug(context));
        }

        if let Some(payload) = payload {
            ds.field("payload", &DisplayAsDebug(payload));
        }

        if let Some(source) = source {
            ds.field("source", source);
        }

        if !show_less && let Some(backtrace) = backtrace {
            ds.field("backtrace", &backtrace);
        }

        ds.finish()
    } else {
        let mut source = source;
        let has_additional_info =
            format_state_context_payload(f, state, context.as_ref(), payload.as_ref())?;
        let mut dedup = DedupLast::new();

        if !has_additional_info {
            let Some(err) = source else {
                unreachable!();
            };
            write!(dedup.format_one(f), "{err}")?;
            source = err.source();
        }

        while let Some(err) = source {
            write!(f, "\n  -> ")?;
            write!(dedup.format_one(f), "{err}")?;
            source = err.source();
        }

        if !show_less && let Some(backtrace) = backtrace {
            write!(f, "\nBacktrace:\n")?;
            write!(f, "{backtrace}")?;
        }

        Ok(())
    }
}

pub fn format_display<S>(
    f: &mut fmt::Formatter<'_>,
    state: Option<&S>,
    context: Option<&dyn Display>,
    payload: Option<&dyn Display>,
    source: Option<&(dyn error::Error + 'static)>,
    _backtrace: Option<&dyn Backtrace>,
) -> fmt::Result
where
    S: Debug + 'static,
{
    if f.alternate() {
        let mut source = source;
        let has_additional_info =
            format_state_context_payload(f, state, context.as_ref(), payload.as_ref())?;
        let mut dedup = DedupLast::new();

        if !has_additional_info {
            let Some(err) = source else {
                unreachable!();
            };
            write!(dedup.format_one(f), "{err}")?;
            source = err.source();
        }

        while let Some(err) = source {
            write!(f, "\n  -> ")?;
            write!(dedup.format_one(f), "{err}")?;
            source = err.source();
        }
    } else {
        let has_additional_info =
            format_state_context_payload(f, state, context.as_ref(), payload.as_ref())?;

        if !has_additional_info {
            let Some(err) = source else {
                unreachable!();
            };
            Display::fmt(err, f)?;
        }
    }

    Ok(())
}

struct DedupLast {
    last: String,
}

impl DedupLast {
    fn new() -> Self {
        Self {
            last: String::new(),
        }
    }

    fn format_one<'a, 'b>(&'a mut self, f: &'a mut fmt::Formatter<'b>) -> FormatOne<'a, 'b> {
        FormatOne {
            formatter: f,
            last: &mut self.last,
            index: Some(0),
        }
    }
}

struct FormatOne<'a, 'b> {
    formatter: &'a mut fmt::Formatter<'b>,
    last: &'a mut String,
    index: Option<usize>,
}

impl<'a, 'b> Write for FormatOne<'a, 'b> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let Some(index) = self.index.as_mut() else {
            self.last.push_str(s);
            return self.formatter.write_str(s);
        };

        if let Some((_, last)) = self.last.split_at_checked(*index)
            && last.starts_with(s)
        {
            *index += s.len();
            return Ok(());
        }

        self.last.truncate(*index);
        self.index = None;

        self.formatter.write_str(&self.last)?;
        Write::write_str(self, s)
    }
}
