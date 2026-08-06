# Vendored crossterm 0.29.0 — local patch

The crates.io source of **crossterm 0.29.0** with three targeted changes, kept in-tree and used **opt-in** by the size-optimized builds.
This file is the single place that explains the vendoring; `Cargo.toml`, `.cargo/*`, [BUILD.md](../../BUILD.md) and [OPTIMIZATION.md](../../OPTIMIZATION.md) only point here.
Every patched site carries a `// LOCAL PATCH — see …LOCAL_PATCH.md` marker.

All three changes rest on the same fact: **this app is single-threaded** — one event-loop thread in `main::run`, no thread is spawned anywhere — so crossterm's global synchronization, and its fallback paths for hostile environments, are pure overhead.

| # | Change | Patched file | Δ Windows | Δ Linux |
|---|--------|--------------|----------:|--------:|
| 1 | ioctl-only `terminal::size()` | `src/terminal/sys/unix.rs` | 0 | −28,705 |
| 2 | Event reader without `parking_lot::Mutex` | `src/event.rs` | −5,792 | ≈ 0 |
| 3 | Raw-mode state without `parking_lot::Mutex` | `src/terminal/sys/unix.rs` | 0 | −4,210 |

Bytes are un-padded section totals on the immediate-abort profile, the metric [OPTIMIZATION.md](../../OPTIMIZATION.md) compares.
A `0` marks a platform the change structurally cannot affect (both patched files are Unix-only; Windows has its own `sys` module).
Changes 2 and 3 remove the same dependency from opposite ends, so their per-platform figures are not independent — see change 3.

## How it is wired

A `[patch.crates-io]` table cannot be gated behind a cargo feature, and pinning every build to a frozen vendored copy is not wanted.
So the patch lives in **`.cargo/crossterm-patch.toml`**, which cargo does *not* auto-load — only `.cargo/config.toml` is — and is injected explicitly on the size-optimized command lines:

```bash
cargo build --release --config .cargo/crossterm-patch.toml
```

- plain `cargo build` → **upstream** crossterm 0.29.x from crates.io, kept updatable;
- `--config .cargo/crossterm-patch.toml` → this **vendored** copy.

BUILD.md's commands already append the flag where it belongs.
Relative paths inside a `--config` file resolve from the parent of that file's directory, which is why the file sits in `.cargo/`: `path = "vendor/crossterm"` then resolves to `<repo>/vendor/crossterm`.

> **Caveat — `Cargo.lock` churn.**
> A `--config` build rewrites the lockfile's crossterm entry (registry → path).
> The committed lockfile tracks upstream, so that churn must not be committed: `git checkout -- Cargo.lock` afterwards, or leave it unstaged.
> `tools/validate.sh` restores it automatically.

The patch does **not** touch crossterm's `Cargo.toml`: `parking_lot` stays in the dependency graph and is still compiled, so `cargo tree` keeps listing it.
What changes is that nothing references it any more, so none of its code reaches the linked binary.

## 1. ioctl-only `terminal::size()`

`src/terminal/sys/unix.rs`, `pub(crate) fn size()` — the `tput` fallback is dropped:

```rust
// upstream:
//     tput_size().ok_or_else(|| std::io::Error::last_os_error().into())
// patched:
    Err(std::io::Error::last_os_error())
```

Upstream falls back to spawning `tput cols` / `tput lines` when the `TIOCGWINSZ` ioctl fails, and crossterm's own Unix event source calls `size()` on every resize, so that fallback stays reachable whatever our code does.
That single `std::process::Command` anchored a large subtree: `Command` itself, a `BTreeMap<OsString, OsString>` holding the child's environment copy, and the `OsString`/`Path`/`ByteStr` `Debug`/`fmt` machinery they pull in.

Measured on Linux (immediate-abort): **118,608 → 89,903 B, −28,705 B (−24%)**; `nm` confirms `tput_value`, `Command` and `BTreeMap` are gone from the binary.
Windows is unaffected — its `size()` uses `GetConsoleScreenBufferInfo` and never involved `tput`.

Behaviour changes only on the fallback path: the ioctl always succeeds on a real terminal, so output is identical; if it ever failed, `size()` now returns `Err` (the app exits cleanly) instead of shelling out.
`tput_size`/`tput_value` are left in the file rather than deleted, to keep the diff to that one return; being unreferenced, they contribute nothing to the binary.

## 2. Event reader without `parking_lot::Mutex`

`src/event.rs` — the global `INTERNAL_EVENT_READER: parking_lot::Mutex<Option<InternalEventReader>>` becomes an `UnsafeCell` behind a small `EventReaderCell` wrapper, and `lock_internal_event_reader()` returns `&'static mut InternalEventReader` directly.
`poll_internal` loses its `try_lock_for` timeout juggling, along with `try_lock_internal_event_reader_for` and the `PollTimeout` import: with no lock to contend for, the full timeout always reaches the poll itself.

On **Windows** this `Mutex` was the *only* live `parking_lot` user — the `Once` in `ansi_support`/`colored` is dead code, its `env::var` call sites eliminated (verified: the `NO_COLOR`/`COLORTERM` strings are absent from the linked binary).
Removing it drops **`parking_lot` and `parking_lot_core` entirely**: `Once::call_once_slow`, `RawMutex::lock_slow`, `WordLock`, `ThreadData`, and the `core`/`std` satellites they anchor.

Measured on Windows (immediate-abort): **71,845 → 66,053 B, −5,792 B**.
Linux barely moves here, because raw-mode still anchored `parking_lot` there — that is change 3.

## 3. Raw-mode state without `parking_lot::Mutex`

`src/terminal/sys/unix.rs` — the same treatment for `TERMINAL_MODE_PRIOR_RAW_MODE: parking_lot::Mutex<Option<Termios>>`, now a `RawModeCell` whose `lock()` hands back `&mut Option<Termios>`, so `is_raw_mode_enabled`, `enable_raw_mode` and `disable_raw_mode` (both their `libc` and `rustix` variants) keep their call sites unchanged.

On Linux this was the *second* `parking_lot` anchor; with changes 2 and 3 both in, `parking_lot`/`parking_lot_core` are gone on Linux too.
Windows is unaffected: this file is Unix-only, and Windows raw-mode goes through the console API.

Measured on Linux (immediate-abort): **89,893 → 85,683 B, −4,210 B**.
(Unrelated work landed between this measurement and change 1's endpoint, so the two Linux figures are 10 B apart and do not chain exactly.)

## Soundness of changes 2 and 3

Both replace a `Mutex<T>` with an `UnsafeCell<T>` fronted by a `.lock()`-shaped accessor returning `&mut T`, plus an `unsafe impl Sync` so the cell can live in a `static`.
This is sound here because access is **single-threaded and non-reentrant**:

- the app drives crossterm from one thread only, so no two threads can hold the `&mut`;
- no call re-enters the accessor while a borrow is live — upstream's own non-reentrant `Mutex` would deadlock if it did, which makes the property upstream already relies on;
- each `&mut` is used and dropped inside one call.

Validated end-to-end on a real Linux PTY — typed a word, submitted it, quit with the terminal correctly restored — which exercises exactly the `event.rs` and raw-mode paths.

The invariant is the app's, not crossterm's: this copy is **not** safe for a multi-threaded consumer.
That is precisely why it stays opt-in and out of upstream.

## Upgrading crossterm

For the default build, bump the upstream version in the workspace `Cargo.toml` as usual — nothing here is involved.
Re-vendor and re-apply the three changes only when the optimized build should move to the new version: fetch the new source, copy `src/`, `Cargo.toml`, `LICENSE` and `README.md` over this directory (this file excepted), then grep the new source for the three sites above.

To review the current patch, or to check that a re-vendor left exactly the intended changes, diff against the registry copy:

```bash
diff -ru ~/.cargo/registry/src/index.crates.io-*/crossterm-0.29.0/src vendor/crossterm/src
```

For 0.29.0 that reports exactly two differing files — `src/event.rs` and `src/terminal/sys/unix.rs`, the latter carrying changes 1 and 3 — and `Cargo.toml` byte-identical to upstream apart from line endings.

## What is kept

Only `src/`, `Cargo.toml`, `LICENSE` and `README.md` are kept from the upstream tarball; examples, docs, benches, tests and build cruft were removed, since crossterm is built lib-only as a dependency.
