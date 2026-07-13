# Local patch — crossterm 0.29.0 (vendored)

The crates.io source of **crossterm 0.29.0** with a few targeted changes. This file is the single
place that explains the vendoring; everything else (Cargo.toml, .cargo/*, BUILD.md, OPTIMIZATION.md)
just points here. Every patched site carries a `// LOCAL PATCH — see …LOCAL_PATCH.md` marker.

All three changes lean on the same fact: **this app is single-threaded** (one event-loop thread in
`main::run`, no threads spawned anywhere), so crossterm's global synchronization is pure overhead.

## 1. `size()` — ioctl-only (Linux −28,705 B)

`src/terminal/sys/unix.rs`, `pub(crate) fn size()` — the `tput` fallback is removed, leaving it
**ioctl-only**:

```rust
// upstream:
//     tput_size().ok_or_else(|| std::io::Error::last_os_error().into())
// patched:
    Err(std::io::Error::last_os_error())
```

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

## 2. Event reader — single-threaded cell (Windows −5,792 B)

`src/event.rs` — the global `INTERNAL_EVENT_READER: parking_lot::Mutex<Option<InternalEventReader>>`
becomes an `UnsafeCell` (`EventReaderCell`), and `poll_internal` drops its `try_lock_for` timeout
juggling (with no lock to contend for, the full timeout always reaches the poll itself).

On **Windows** this Mutex was the *only* live `parking_lot` user (the `Once` in `ansi_support`/
`colored` is dead code — its `env::var` call sites are DCE'd, verified: the `NO_COLOR`/`COLORTERM`
strings are absent from the linked binary). Removing it drops **`parking_lot` + `parking_lot_core`
entirely**: `Once::call_once_slow`, `RawMutex::lock_slow`, `WordLock`, `ThreadData`, and the `core`/
`std` satellites they anchor.

Measured (Windows, immediate-abort): **71,845 → 66,053, −5,792 B**. Linux barely moves here because
raw-mode (patch 3) still anchored `parking_lot` there.

## 3. Raw-mode state — single-threaded cell (Linux −4,210 B)

`src/terminal/sys/unix.rs` — the same treatment for `TERMINAL_MODE_PRIOR_RAW_MODE:
parking_lot::Mutex<Option<Termios>>` (a `RawModeCell` `UnsafeCell`), which `enable_raw_mode`/
`disable_raw_mode`/`is_raw_mode_enabled` lock. On Linux this was the *second* `parking_lot` anchor;
with both patches 2 and 3 in, `parking_lot`/`parking_lot_core` are gone on Linux too. Windows is
unaffected (this file is Unix-only; Windows raw-mode uses the console API).

Measured (Linux, immediate-abort): **89,893 → 85,683, −4,210 B**.

### Safety of patches 2 and 3

Both replace a `Mutex<T>` with an `UnsafeCell<T>` fronted by a `.lock()`-shaped accessor returning
`&mut T`, plus an `unsafe impl Sync` so it can live in a `static`. Sound because access is
**single-threaded and non-reentrant**: crossterm's own non-reentrant Mutex would already deadlock on
reentry, so the reader's `poll`/`read` and the raw-mode fns never re-enter the accessor, and no two
`&mut` are ever live at once. Validated end-to-end (typed a word, submitted, quit with the terminal
restored) on a real Linux PTY, which exercises the exact `event.rs` + raw-mode paths.

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
and re-apply the three changes above (grep the new source for the sites) only when you want the
optimization on the new version.

## Trimmed

Only `src/`, `Cargo.toml`, `LICENSE`, and `README.md` are kept from the upstream tarball; examples,
docs, benches, tests, and build cruft were removed (crossterm is built lib-only as a dependency).
Each change carries a short `// LOCAL PATCH — see …LOCAL_PATCH.md` marker at its site; the three
patched files are `src/terminal/sys/unix.rs`, `src/event.rs`, and (again) `src/terminal/sys/unix.rs`.
