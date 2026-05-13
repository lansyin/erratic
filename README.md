# Erratic

[![license](https://img.shields.io/github/license/lansyin/erratic)](https://github.com/lansyin/erratic)
[![crates.io](https://img.shields.io/crates/v/erratic)](https://crates.io/crates/erratic)
[![docs.rs](https://img.shields.io/docsrs/erratic)](https://docs.rs/erratic/latest/erratic/)

This library provides `Error<S = Stateless>`, an **optionally** dynamic dispatched error type,
enabling applications to handle errors uniformly across different contexts.

## Basic Usage
In most cases, `Error` can serve as a drop-in replacement for `Box<dyn Error>`.
Compared to the latter, it occupies only 1 usize, making the happy path faster.
```rust
use erratic::*;

fn write_log(filename: String) -> Result<()> {
    File::open(&filename)?.write_all(b"Hello, World!")?;
    Ok(())
}
```

## Attaching Context & Payload
When constructing an error, you can optionally attach a static context and/or a dynamic payload.
If attached, their memory is merged into a single allocation when the upstream error is erased.
If omitted, no extra memory is allocated for them. If only context is provided, no heap allocation
occurs at all.

```rust
use erratic::*;

fn write_log(filename: String) -> Result<()> {
    File::open(&filename)
        .or_context(literal!("failed to open the log file"))? // No alloc.
        .write_all(b"Hello, World!")
        .with_context(literal!("while writing file"))
        .with_payload(|| filename)?; // Alloc once for `io::Error`, `filename`, and `Context`.
    Ok(())
}
```

## Binding State
When propagating an error that requires special handling, you can supply a generic state
alongside it. If the state implements `Default`, other errors can be wrapped and
returned directly via `?` without explicitly setting the state.

When the state is small enough and none of the source error, context, or payload is attached,
the state is inlined without any heap allocation.

```rust
use erratic::*;

#[derive(Debug, Default)]
enum WriteLog {
    FileNotFound,
    #[default]
    Other,
}

fn write_log(filename: String) -> std::result::Result<(), Error<WriteLog>> {
    File::open(&filename)
        .or_state(WriteLog::FileNotFound)? // No alloc.
        .write_all(b"Hello, World!")
        .with_context(literal!("while writing to"))
        .with_payload(|| filename)?; // Falls back to the default state value.
    Ok(())
}
```

## Representation
Type-wise, `Error<S>` is an internally tagged union, and it requires pointers to constant or
heap-allocated data to be aligned to 4 bytes, freeing up the lower 2 bits to encode
the discriminant. This design allows heap allocation to be avoided when unnecessary.

```plaintext
(32-bit platform, little-endian)
(Context)
[XXXXXX00|XXXXXXXX|XXXXXXXX|XXXXXXXX]
                                     \
                                      `rodata-> [&'static str]
(Small State)
[00000010|     ~    State     ~     ]

(Otherwise)
[XXXXXX01|XXXXXXXX|XXXXXXXX|XXXXXXXX]
       \
        `heap-> [ ~ State ~ |&'static VTable| ~ Error ~ | ~ Payload ~ |&'static str/()]
```


## Contribution
Contributions are warmly welcomed! Whether you have a bug report, feature request, or 
an improvement in mind, feel free to open an issue or submit a pull request. 
All ideas—big or small—help make this library better for everyone.
