# Erratic /ɪˈrætɪk/

[![license](https://img.shields.io/badge/license-MIT-hotpink)](https://github.com/lansyin/erratic)
[![MSRV](https://img.shields.io/badge/MSRV-Rust_1.89-lightcoral)](https://github.com/lansyin/erratic/blob/main/Cargo.toml#L5)
[![crates.io](https://img.shields.io/crates/v/erratic)](https://crates.io/crates/erratic)
[![docs.rs](https://img.shields.io/docsrs/erratic)](https://docs.rs/erratic/latest/erratic/)

This crate provides `Error<S = Stateless>`, an error type with typed state,
enabling applications to handle errors uniformly across different scenarios.

![splash](https://raw.githubusercontent.com/lansyin/erratic/c384432ef0b892c98d3c303d0fc6c6322a8a8389/splash.svg)

## Quick Start

In most cases, `Error` can serve as a drop-in replacement for `Box<dyn Error>`,
with the `?` operator converting any standard error into `Error` automatically.

```rust
fn say_hi(filename: &str) -> erratic::Result<()> {
    File::open(filename)?.write_all(b"Hello, World!")?;
    Ok(())
}
```

## Attaching Context

When constructing an error, you can attach a context to it. Use `mkctx!` to
construct a lazily-evaluated context from a format string.

```rust
use erratic::*;

fn ringbuf(size: usize) -> Result<(Writer, Reader)> {
    let pair = rb::ringbuf(size)
        .with_context(mkctx!("expected a power of two, found {size}"))?;
    let pair = metrics::attach(pair)
        .with_context("failed to attach metrics for ringbuf")?;
    Ok(pair)
}
```

## Binding State

When propagating domain errors, you can attach a state to them. A small state
with no other components incurs no heap allocation.

```rust
use erratic::*;

#[derive(Debug)]
enum State { RetryLater } // Smaller than 1 usize.

fn try_write(w: &mut Writer, chunk: &[u8]) -> Result<(), Error<State>> {
    w.reserve_chunk(chunk.len())
        .ok()
        .with_state(State::RetryLater)?; // No alloc.
    w.write(chunk)
        .with_context(mkctx!("failed to write to {}", w.id))?;
    Ok(())
}
```

When no runtime state is actually stored, errors can be cheaply converted between different state types.
Infrastructure errors can thus cross any number of layers with no extra allocation, while each layer
can pick the state type that suits it best.

```rust
fn write(w: &mut Writer, chunk: &[u8]) -> Result<()> {
    while let Err((state, _)) = try_write(w, chunk).extract_state()? {
        // Handle domain errors.                                  ^ Bubble up infra errors.
        match state {
            State::RetryLater => thread::yield_now(),
            // ..
        }
    }
    Ok(())
}
```

The `?` operator covers the most common cases, regardless of whether the return type carries a state:

| Source Type        | Return Type   | Explanation                                                     |
| :----------------- | :------------ | :-------------------------------------------------------------- |
| `impl Error`       | `Error<_>`    | Wraps any standard error type.                                  |
| `Builder<..>`      | `Error<_>`    | Builds an error from state, context, and/or source.             |
| `Error<Stateless>` | `Error<_>`    | Cheaply converts a stateless error to one with a phantom state. |

States are meant to be handled explicitly. Several utility methods are provided:

| Method          | Conversion                                    | Explanation                                      |
| :-------------- | :-------------------------------------------- | :----------------------------------------------- |
| `extract_state` | `Error<S>` -> `Result<(S, Vacant<S>), Error>` | Takes the state out, or propagates the error.    |
| `map_state`     | `Error<S>` -> `Error<S2>`                     | Transforms the state with a closure.             |
| `lift_state`    | `Error<S>` -> `Error<S2>` where `S2: From<S>` | Transforms the state via `From`.                 |
| `erase_state`   | `Error<S>` -> `Error<Stateless>`              | Erases the state, keeping the message unchanged. |

## Formatting

If the error has a state and/or a context, it builds error messages from them. Otherwise, it acts as an error container,
inheriting the message from its source. When wrapped, the container itself will not be added as another source layer,
preventing duplicate messages in the chain.

```
<error> ::= <source>
          | <state>": "<context>
          | <context>
          | <state>
<chain> ::= <error>
          | <error>"\n  -> "<chain>
```

By default, only the top-level error is shown during formatting. To print the full error chain,
format with alternate or debug specifiers.

| Specifier | Explanation                                               |
| :-------- | :-------------------------------------------------------- |
| `{}`      | Prints only the top-level error.                          |
| `{:#}`    | Prints the full error chain.                              |
| `{:?}`    | Prints the full error chain with backtrace, if captured.  |
| `{:#?}`   | Prints all information in a struct-like format.           |

## Backtrace

When the `backtrace` feature is enabled and backtrace capture is configured via
[environment variables][backtrace-conf], `Error<S>` automatically captures a backtrace if there isn't
one already in the source chain. The backtrace will be appended after the error chain during debug
formatting, unless the minus sign, e.g. `{:-?}`, is specified to suppress it.

[backtrace-conf]: https://doc.rust-lang.org/std/backtrace/index.html#environment-variables


## Contributing
Contributions are warmly welcomed! Whether you have a bug report, feature request, or 
an improvement in mind, feel free to open an issue or submit a pull request. Appreciate any thoughts! 
