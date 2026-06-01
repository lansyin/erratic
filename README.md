# Erratic /ɪˈrætɪk/

[![license](https://img.shields.io/badge/license-MIT-hotpink)](https://github.com/lansyin/erratic)
[![crates.io](https://img.shields.io/crates/v/erratic)](https://crates.io/crates/erratic)
[![docs.rs](https://img.shields.io/docsrs/erratic)](https://docs.rs/erratic/latest/erratic/)

This library provides `Error<S = Stateless>`, an error type with **optional** dynamic dispatch,
enabling applications to handle errors uniformly across different contexts.

## Quick Start

In most cases, `Error` can serve as a drop-in replacement for `Box<dyn Error>`.
Compared to the latter, it occupies only 1 usize, making the happy path faster.
```rust
use erratic::*;

fn write(filename: &str) -> Result<()> {
    File::open(filename)?.write_all(b"Hello, World!")?;
    Ok(())
}
```

## Attaching Context & Payload

When constructing an error, you can optionally attach a static context and/or a dynamic payload.
If attached, the memory is merged into a single allocation when the upstream error is erased.
If omitted, no extra memory is allocated for them. If only a context is provided, no heap allocation
occurs at all.

```rust
use erratic::*;

fn read_weak(r: &mut Weak<Reader>, buf: &mut [u8]) -> Result<u64> {
    if buf.is_empty() {
        return mkres!("buf must not be empty"); // No alloc so long as no format args.
    }
    let r = r.upgrade()
        .with_context(literal!("stream expired"))?; // No alloc.
    //= .with_payload("stream expired")?;
    let n = r.read(buf)
        .with_context(literal!("failed to read from stream"))
        .with_payload(r.id())?; // Alloc once for error, id, and context.
    //= .with_payload(format!("failed to write to stream: {}", w.id()))?;
    Ok(n)
}
```

## Binding State

When propagating an error that requires special handling, you can attach a generic state to it.
If the state is small enough and neither the source error, context, nor payload is attached,
the state is inlined without any heap allocation.

```rust
use erratic::*;

#[derive(Debug)]
enum State { RetryLater }

fn try_write(w: &mut Writer, data: &[u8; 64]) -> Result<(), Error<State>> {
    w.ready_for_write(64)
        .ok()
        .with_state(State::RetryLater)?; // No alloc.
    w.write(data)
        .with_context(literal!("failed to write to stream"))
        .with_payload(w.id())?;
    Ok(())
}
```

The state is optional and can be extracted at runtime, which enables errors to share a single type with different
layouts. A stateful error can be cheaply converted into a stateless one (via `extract_state`) and vice versa
(via `with_phantom_state`). Using the `?` operator between stateful and stateless errors is supported, achieved by
making `Stateless` unsized.

```rust
fn write(w: &mut Writer, data: &[u8; 64]) -> Result<()> {
    while let Err((state, _)) = try_write(w, data).extract_state()? {
        match state {
            State::RetryLater => {
                thread::yield_now();
            }
        }
    }
    Ok(())
}
```

## Backtrace

When the `backtrace` feature is enabled and either the `RUST_BACKTRACE` or `RUST_LIB_BACKTRACE`
environment variable is set, `Error<S>` automatically captures a backtrace if none is present in
the error chain.

The captured backtrace will be included in the error's output during debug formatting, unless
the minus sign (i.e. `{:-?}`) is specified to suppress it. This functionality aids debugging
for complex nested error workflows.

## Representation

When additional information (state, context, or payload) is present alongside the source
error, the output is [`<state>: <context><payload>`][mimic_erratic_display]. Otherwise, when
the error carries only a source, the output is `<source>`.

[mimic_erratic_display]: https://play.rust-lang.org/?gist=e17db5237911367388fc401e445aa2cf

Format specifiers:

- `{}`:       Displays only the top-level error.
- `{:#}`:     Displays the full error chain.
- `{:?}`:     Displays the full error chain with backtrace (if captured).
- `{:#?}`:    Displays all information in a struct-like format.

Error chain:

```
<error>
  -> <source_0>
  -> <source_1>
  -> ..
```

## Layout

Type-wise, `Error<S>` is an internally tagged union, and it requires pointers to constant or
heap-allocated data to be aligned to 4 bytes, freeing up the lower 2 bits to encode
the discriminant. This design allows heap allocation to be avoided when unnecessary.

```plaintext
(32-bit platform, little-endian)
(Context Only)
[______00|________|________|________]
                                    \
                                     `rodata-> [Context]
(Allocation Required)
[______01|________|________|________]
                                    \
                                     `heap-> [VTable|State|Error|Payload|Context]
(Small State Only)
[00000010|     ~    State     ~     ]
```


## Contributing
Contributions are warmly welcomed! Whether you have a bug report, feature request, or 
an improvement in mind, feel free to open an issue or submit a pull request. 
All ideas—big or small—help make this library better for everyone.
