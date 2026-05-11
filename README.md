# Erratic

This library provides the `Error<S = Stateless>` type, enabling applications to handle errors uniformly
across different contexts.

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
If attached, their memory is merged into a single allocation when the upstream error type is
erased. If omitted, no extra memory is allocated for them. If only context is provided, no heap
allocation occurs at all.

```rust
use erratic::*;

fn write_log(filename: String) -> Result<()> {
    File::open(&filename)
        .or_context(literal!("failed to open the log file"))? // No alloc.
        .write_all(b"Hello, World!")
        .with_context(literal!("while writing file"))
        .with_payload(|| filename)?; // One alloc to store both `io::Error` and `filename`.
    Ok(())
}
```

## Binding State

When returning an error that may require special handling, you can supply a generic state object
alongside it. The state must be explicitly erased (via `erase()`) before it can be propagated
upward with `?`. If the state type implements `Default`, other errors can be wrapped and returned
directly through `?`.
When no error is wrapped and no context/payload is attached, the state object is inlined
without triggering any heap allocation. On 32-bit targets, the error stays at 1 usize when the
state is no larger than 2 bytes; on 64-bit targets, it stays at 1 usize when the state is no
larger than 4 bytes.

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

Type-wise, `Error<S>` is a internal tagged union, and it requires pointers to constant or
heap-allocated data to be aligned to 4 bytes, freeing up the lower 2 bits to encode discriminant.
This makes it possible to avoid heap allocation when not needed.

```plaintext
(32-bit platform, little-endian)
(Context)
[XXXXXX00|XXXXXXXX|XXXXXXXX|XXXXXXXX]
                                 \
                                  `rodata-> [&'static str] --rodata--> [ ~ str ~ ]
(Error, Payload, or State & Context)
[XXXXXX01|XXXXXXXX|XXXXXXXX|XXXXXXXX]
          \
           `heap-> [ ~ State/() ~ | ~ VTable ~ | ~ Error ~ | ~ Payload/() ~ |&'static str/()]
(State)
[00000010|     ~    State     ~     ]
```
