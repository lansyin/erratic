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

pub fn format_debug_struct<S>(
    f: &mut fmt::Formatter<'_>,
    container_name: &'static str,
    state: Option<&S>,
    context: Option<&dyn Display>,
    payload: Option<&dyn Display>,
    source: Option<&(dyn error::Error + 'static)>,
    backtrace: Option<&dyn Backtrace>,
) -> fmt::Result
where
    S: Debug + 'static,
{
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

    if let Some(backtrace) = backtrace {
        ds.field("backtrace", &backtrace);
    }

    ds.finish()
}

pub fn format_chain<S>(
    f: &mut fmt::Formatter<'_>,
    state: Option<&S>,
    context: Option<&dyn Display>,
    payload: Option<&dyn Display>,
    source: Option<&(dyn error::Error + 'static)>,
) -> fmt::Result
where
    S: Debug + 'static,
{
    const SOURCE_PREFIX: &str = "\n  -> ";

    let mut source = source;
    let has_additional_info =
        format_state_context_payload(f, state, context.as_ref(), payload.as_ref())?;
    let mut dedup = DedupLast::new();

    if !has_additional_info {
        let Some(err) = source else {
            unreachable!();
        };
        dedup.format_one(&mut Blackhole, |w| write!(w, "{SOURCE_PREFIX}{err}"))?;
        write!(f, "{err}")?;
        source = err.source();
    }

    while let Some(err) = source {
        dedup.format_one(f, |w| write!(w, "{SOURCE_PREFIX}{err}"))?;
        source = err.source();
    }

    Ok(())
}

pub fn format_debug<S>(
    f: &mut fmt::Formatter<'_>,
    state: Option<&S>,
    context: Option<&dyn Display>,
    payload: Option<&dyn Display>,
    source: Option<&(dyn error::Error + 'static)>,
    backtrace: Option<&dyn Backtrace>,
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
            payload,
            source,
            show_less.then_some(backtrace).flatten(),
        )
    } else {
        format_chain(f, state, context, payload, source)?;

        if !show_less && let Some(backtrace) = backtrace {
            write!(f, "\nBacktrace:\n{backtrace}")?;
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
        format_chain(f, state, context, payload, source)?;
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

    fn format_one<'a, W>(
        &'a mut self,
        w: &'a mut W,
        f: impl FnOnce(&mut FormatOne<'a, W>) -> fmt::Result,
    ) -> fmt::Result
    where
        W: Write,
    {
        let mut w = FormatOne {
            writer: w,
            last: &mut self.last,
            index: Some(0),
        };

        f(&mut w)?;

        w.finish()
    }
}

struct FormatOne<'a, W>
where
    W: Write,
{
    writer: &'a mut W,
    last: &'a mut String,
    index: Option<usize>,
}

impl<'a, W> Write for FormatOne<'a, W>
where
    W: Write,
{
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let Some(index) = self.index.as_mut() else {
            self.last.push_str(s);
            return Ok(());
        };

        if self.last[(*index)..].starts_with(s) {
            *index += s.len();
            return Ok(());
        }

        self.last.truncate(*index);
        self.index = None;
        self.last.push_str(s);
        Ok(())
    }
}

impl<'a, W> FormatOne<'a, W>
where
    W: Write,
{
    fn finish(self) -> fmt::Result {
        match self.index {
            Some(index) => {
                if index < self.last.len() {
                    self.writer.write_str(&self.last[..index])?;
                }
            }
            None => {
                self.writer.write_str(self.last)?;
            }
        }
        Ok(())
    }
}

struct Blackhole;

impl Write for Blackhole {
    fn write_str(&mut self, _: &str) -> fmt::Result {
        Ok(())
    }
}
