# Local patch — crossterm 0.29.0 (vendored)

The crates.io source of **crossterm 0.29.0** with one line changed. This file is the single place
that explains the vendoring; everything else (Cargo.toml, .cargo/*, BUILD.md, OPTIMIZATION.md) just
points here.

## The change

`src/terminal/sys/unix.rs`, `pub(crate) fn size()` — the `tput` fallback is removed, leaving it
**ioctl-only**:

```rust
// upstream:
//     tput_size().ok_or_else(|| std::io::Error::last_os_error().into())
// patched:
    Err(std::io::Error::last_os_error())
```

## Why

`size()` falls back to spawning `tput cols` / `tput lines` when the `TIOCGWINSZ` ioctl fails, and
crossterm's own Unix event source calls `size()` on every resize, so that fallback stays reachable
no matter what our code does. The `tput` `std::process::Command` was the sole anchor for a large
subtree: `Command`, a `BTreeMap<OsString, OsString>` (the child's environment copy), and the
`OsString` / `Path` / `ByteStr` `Debug`/`fmt` machinery they pull. The ioctl is the only path that
ever succeeds on the terminals we target, so the fallback was dead weight.

Measured (Linux, immediate-abort): **118,608 → 89,903, −28,705 B (−24 %)**; `nm` confirms
`tput_value`/`Command`/`BTreeMap` are gone. Windows is unaffected (its `size()` uses
`GetConsoleScreenBufferInfo`, no `tput`). Behaviour changes only in the fallback: on a real terminal
the ioctl always succeeds, so output is identical; if it ever failed, `size()` now returns `Err`
(the app exits cleanly) instead of shelling out to `tput`.

## How it's wired (opt-in, not a global lock)

A `[patch.crates-io]` cannot be gated behind a Cargo feature, and we do NOT want every build pinned
to this frozen copy. So the patch lives in **`.cargo/crossterm-patch.toml`** (which cargo does *not*
auto-load — only `.cargo/config.toml` is) and is injected only on the size-optimized build commands:

```
cargo ... build ... --config .cargo/crossterm-patch.toml
```

- Plain `cargo build` → **upstream** crossterm 0.29.x from crates.io (kept updatable).
- `--config .cargo/crossterm-patch.toml` → this **vendored** copy.

Relative paths in a `--config` file resolve from the parent of the file's directory, so putting it
in `.cargo/` makes `path = "vendor/crossterm"` resolve to `<repo>/vendor/crossterm`.

**Caveat:** a `--config` build rewrites `Cargo.lock`'s crossterm entry (registry → path). The
committed `Cargo.lock` tracks upstream, so don't commit that churn — `git checkout Cargo.lock`
afterward, or leave it unstaged.

## Upgrading crossterm

Bump the upstream version in `Cargo.toml` normally for the default build. Re-vendor from the registry
and re-apply the one-line change above only when you want the optimization on the new version.

## Trimmed

Only `src/`, `Cargo.toml`, `LICENSE`, and `README.md` are kept from the upstream tarball; examples,
docs, benches, tests, and build cruft were removed (crossterm is built lib-only as a dependency).
The one-line change carries a short `// LOCAL PATCH — see …LOCAL_PATCH.md` marker at its site; no
other source file is modified.
