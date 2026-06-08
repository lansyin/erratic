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

## Attaching Context

When constructing an error, you can optionally attach a context. If a context is attached, it's memory
will be merged into a single allocation when the error is materialized. If the context is the only component
of the error, no heap allocation occurs.

```rust
use erratic::*;

fn read_weak(r: &mut Weak<Reader>, buf: &mut [u8]) -> Result<u64> {
    if buf.is_empty() {
        return mkres!("buf must not be empty"); // No alloc so long as no format args.
    }
    let r = r.upgrade()
        .with_context(mkctx!("stream expired"))?; // Works the same as `mkres`, no alloc.
    let n = r.read(buf)
        .with_context(mkctx!("failed to read from stream {}", r.id()))?; // Evaluated lazily.
    Ok(n)
}
```

## Binding State

When propagating an error that requires special handling, you can attach a generic state to it.
If the state is small enough and it's the only component of the error, the state is inlined
without any heap allocation.

```rust
use erratic::*;

#[derive(Debug)]
enum State { RetryLater }

fn try_write(w: &mut Writer, data: &[u8; 64]) -> Result<(), Error<State>> {
    w.reserve_chunk(64)
        .ok()
        .with_state(State::RetryLater)?; // No alloc.
    w.write(data)
        .with_context(mkctx!("failed to write to stream: {}", w.id()))?;
    Ok(())
}
```

When no runtime state is actually stored, errors can be cheaply converted between different state types,
which allows infrastructure errors to cross any number of layers with no extra allocation, domain errors
avoid the heap entirely, and both share the same `Error<S>` type. All compose orthogonally.

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

The `?` operator covers the most common cases, notably including conversion from `Error` to `Error<S>`:

- `impl Error`  -> `Error`
- `impl Error`  -> `Error<S>`
- `Error`       -> `Error<S>`

Stateful errors are meant to be handled explicitly. Several utility methods are provided:

  - `erase_error()?`:    Propagate the error.
  - `extract_state()?`:  Take the state out, or propagate the error.
  - `map_state()?`:      Transform the state with a closure.
  - `lift_state()?`:     Transform via `From<S>`.

## Backtrace

When the `backtrace` feature is enabled and backtrace capture is configured via
[environment variables][backtrace-conf], `Error<S>` automatically captures a backtrace if there isn't
already one in the source chain. The backtrace will be appended after the error chain during debug
formatting, unless the minus sign, e.g. `{:-?}`, is specified to suppress it.

[backtrace-conf]: https://doc.rust-lang.org/std/backtrace/index.html#environment-variables

## Representation

If the error contains only a source, the error message is inherited from the source. Otherwise, the
error message is constructed from other attached components.

```
<error> ::= <source>
          | <state>": "<context>
          | <context>
          | <state>
```

By default, only the top-level error is shown during formatting. To display the full error chain,
format with alternate or debug specifiers.

- `{}`:       Displays only the top-level error.
- `{:#}`:     Displays the full error chain.
- `{:?}`:     Displays the full error chain with backtrace (if captured).
- `{:#?}`:    Displays all information in a struct-like format.

The error chain is defined as follows:

```
<chain> ::= <error>
          | <error>"\n  -> "<chain>
```

## Layout

Type-wise, `Error<S>` is an internally tagged union, and it requires pointers to be aligned to 4 bytes,
freeing up the lower 2 bits to encode its discriminant. Pointer tagging in this crate fully follows
[strict provenance][strict_provenance], and is verified by Miri.

[strict_provenance]: https://doc.rust-lang.org/std/ptr/index.html#strict-provenance

```plaintext
(32-bit platform, little-endian)
(Context Only)
[......00|........|........|........]
                                    \
                                     `rodata-> [Context]
(Allocation Required)
[......01|........|........|........]
                                    \
                                     `heap-> [VTable|State|Error|Context]
(Small State Only)
[00000010|     ~    State     ~     ]
```


## Contributing
Contributions are warmly welcomed! Whether you have a bug report, feature request, or 
an improvement in mind, feel free to open an issue or submit a pull request. 
All ideas—big or small—help make this library better for everyone.
