# Erratic /ɪˈrætɪk/

[![license](https://img.shields.io/badge/license-MIT-hotpink)](https://github.com/lansyin/erratic)
[![MSRV](https://img.shields.io/badge/MSRV-Rust_1.89-lightcoral)](https://github.com/lansyin/erratic/blob/main/Cargo.toml#L5)
[![crates.io](https://img.shields.io/crates/v/erratic)](https://crates.io/crates/erratic)
[![docs.rs](https://img.shields.io/docsrs/erratic)](https://docs.rs/erratic/latest/erratic/)

This crate provides `Error<S = Stateless>`, an error type with typed state,
enabling applications to handle errors uniformly across different scenarios.

## Quick Start

In most cases, `Error` can serve as a drop-in replacement for `Box<dyn Error>`.
Compared to the latter, it occupies only 1 usize and eliminates allocations
altogether when constructed from a literal string or a small state.

```rust
fn say_hi(filename: &str) -> erratic::Result<()> {
    File::open(filename)?.write_all(b"Hello, World!")?;
    Ok(())
}
```

## Attaching Context

When constructing an error, you can optionally attach a context to it. All helper macros support
constructing context from a format string.

```rust
use erratic::*;

fn connect_timeout() -> Result<u64> {
    let raw = env::var("APP_TIMEOUT").with_context("timeout is not set")?;
    mksure!(raw.len() > 0)?; // Prints the operand values on failure.

    let secs: u64 = raw.trim().parse::<u64>()
        .with_context(mkctx!("invalid timeout: {raw}"))?; // Evaluates lazily.
    mksure!(secs > 0, "timeout must be positive")?; // No alloc.

    Ok(secs)
}
```

## Binding State

When propagating domain errors, you can optionally attach a state to it. A small state
with no other components incurs no heap allocation.

```rust
use erratic::*;

#[derive(Debug)]
enum State { RetryLater } // Smaller than 1 usize.

fn try_write(w: &mut Writer, data: &[u8]) -> Result<(), Error<State>> {
    w.reserve_chunk(data.len())
        .ok()
        .with_state(State::RetryLater)?; // No alloc.
    w.write(data)
        .with_context(mkctx!("failed to write to {}", w.id))?;
    Ok(())
}
```

When no runtime state is actually stored, errors can be cheaply converted between different state types.
This allows infrastructure errors to cross any number of layers with no extra allocation, domain errors
avoid the heap entirely, and both share the same `Error<S>` type.

```rust
fn write(w: &mut Writer, data: &[u8]) -> Result<()> {
    while let Err((state, _)) = try_write(w, data).extract_state()? {
        // Handle domain errors.                                  ^ Bubble up infra errors.
        match state {
            State::RetryLater => thread::yield_now(),
            // ..
        }
    }
    Ok(())
}
```

The `?` operator covers the most common cases, regardless of whether the return type carries state:

| Source Type        | Return Type   | Explanation                                           |
| :----------------- | :------------ | :---------------------------------------------------- |
| `impl Error`       | `Error<_>`    | Wraps any standard error type.                        |
| `Builder<..>`      | `Error<_>`    | Builds an error from state, context, and/or source.   |
| `Error<Stateless>` | `Error<S>`    | Cheaply converts a stateless error to a stateful one. |

States are meant to be handled explicitly. Several utility methods are provided:

| Method          | Conversion                                    | Explanation                                  |
| :-------------- | :-------------------------------------------- | :------------------------------------------- |
| `extract_state` | `Error<S>` -> `Result<(S, Vacant<S>), Error>` | Takes the state out, or propagate the error. |
| `map_state`     | `Error<S>` -> `Error<S2>`                     | Transforms the state with a closure.         |
| `lift_state`    | `Error<S>` -> `Error<S2>` where `S2: From<S>` | Transforms the state via `From`.             |
| `erase_state`   | `Error<S>` -> `Error<Stateless>`              | Erases the state while preserving the error. |

## Formatting

If the error has a state and/or a context, it builds error message from them. Otherwise, it acts as an error container,
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

By default, only the top-level error is shown during formatting. To display the full error chain,
format with alternate or debug specifiers.

| Specifier | Explanation                                               |
| :-------- | :-------------------------------------------------------- |
| `{}`      | Displays only the top-level error.                         |
| `{:#}`    | Displays the full error chain.                             |
| `{:?}`    | Displays the full error chain with backtrace, if captured. |
| `{:#?}`   | Displays all information in a struct-like format.          |

## Backtrace

When the `backtrace` feature is enabled and backtrace capture is configured via
[environment variables][backtrace-conf], `Error<S>` automatically captures a backtrace if there isn't
one already in the source chain. The backtrace will be appended after the error chain during debug
formatting, unless the minus sign, e.g. `{:-?}`, is specified to suppress it.

[backtrace-conf]: https://doc.rust-lang.org/std/backtrace/index.html#environment-variables

## Representation

Type-wise, `Error<S>` is an internally tagged union, and it requires pointers to be aligned to 4 bytes,
freeing up the lower 2 bits to encode its discriminant. Pointer tagging in this crate fully follows
[strict provenance][strict-provenance], and is verified by Miri.

[strict-provenance]: https://doc.rust-lang.org/1.89.0/std/ptr/index.html#strict-provenance

The error has three possible layouts. When constructed from a literal, it stores a pointer to the literal.
When constructed from a small state, it stores the state inline. Otherwise, it points to a heap-allocated object
containing a vtable and potentially a state, source, and/or context.

```plaintext
┌Error<State>─────────╎───┐   ┌ConstBody─────┐
│ Align4Ref<ConstBody>╎00 ├───┤ &'static str │
└─────────────────────╎───┘   └──────────────┘
┌Error<State>─────────╎───┐   ┌BoxedBody─────────────┬────────────────────┬────────┬─────────┐
│ Align4Own<BoxedBody>╎01 ├───┤ Align4Ref<VTable>╎ST │ MaybeUninit<State> │ Source │ Context │
└─────────────────────╎───┘   └─────────┼──────────┼─┴──────────────┼─────┴────────┴─────────┘
┌Error<State>─┬───────╎───┐   ┌VTable───┴──┬─╌     │╭ ST=00:Uninit ╮│
│    State    │ 000000╎10 │   │ Drop::drop │       ╰┤ ST=01:Active ├╯
└─────────────┴───────╎───┘   └────────────┴─╌      ╰ ST=10:Erased ╯
```


## Contributing
Contributions are warmly welcomed! Whether you have a bug report, feature request, or 
an improvement in mind, feel free to open an issue or submit a pull request. Appreciate any thoughts! 
