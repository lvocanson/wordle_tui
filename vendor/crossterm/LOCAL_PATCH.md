# Vendored crossterm 0.29.0 — local patch

The crates.io source of **crossterm 0.29.0** with eight targeted changes.
Every patched site carries a `// LOCAL PATCH — see …LOCAL_PATCH.md` marker.

Changes 1–3 rest on the same fact: **this app is single-threaded** — one event-loop thread in `main::run`, no thread is spawned anywhere — so crossterm's global synchronization, and its fallback paths for hostile environments, are pure overhead.
Changes 4–7 remove what this app never reads: error-message payloads, full-unicode key case mapping, sub-millisecond poll timing, and every event kind the game does not consume.
Change 8 removes the event queue outright, on the same single-threaded, single-consumer reading of how this app uses the crate.

| # | Change | Patched file | Δ Windows | Δ Linux |
|---|--------|--------------|----------:|--------:|
| 1 | ioctl-only `terminal::size()` | `src/terminal/sys/unix.rs` | 0 | −28,705 |
| 2 | Event reader without `parking_lot::Mutex` | `src/event.rs` | −5,792 | ≈ 0 |
| 3 | Raw-mode state without `parking_lot::Mutex` | `src/terminal/sys/unix.rs` | 0 | −4,210 |
| 4 | No `io::Error` message payloads | `event/read.rs`, `event/sys/windows{,/poll}.rs`, `event/sys/unix/parse.rs` | −6,580 | ≈ 0 * |
| 5 | ASCII-only key case correction | `event/sys/windows/parse.rs` | −6,108 | 0 |
| 6 | Millisecond poll clock | `event/timeout.rs` | −534 | 0 |
| 7 | Event set reduced to the game's inputs | `event/source/windows.rs`, `event/sys/windows/parse.rs`, `event/sys/unix/parse.rs` | −1,716 | −8,184 |
| 8 | Blocking event source, no queue | `event.rs` | −2,529 | −70 ** |

Bytes are un-padded section totals on the immediate-abort profile, the metric [OPTIMIZATION.md](../../OPTIMIZATION.md) compares.
A `0` marks a platform the change structurally cannot affect (the patched file or branch is for the other platform).
Changes 2 and 3 remove the same dependency from opposite ends, so their per-platform figures are not independent — see change 3.
`**` Change 8 removes 3,256 shipped bytes on Linux (76,112 → 72,856 B on disk); `.relro_padding` grows by almost as much to keep the page alignment, so the section total only moves −70 B.
`*` Changes 4–6 were measured on Linux only in aggregate: sections shrink ~430 B (`.text`, `.rodata`, `.eh_frame`, `.rela.dyn`) but RELRO page alignment grows `.relro_padding` by almost exactly that, for a net −1 B total.

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
That is precisely why it stays in-tree and out of upstream.

## 4. No `io::Error` message payloads

Every **live** `io::Error::new(kind, "message")` becomes `io::Error::from(kind)`; dead sites (bracketed-paste, keyboard-enhancement, `set_size`, cursor) are left untouched since LTO already drops them.
Patched sites: `event/read.rs` (reader-init failure), `event/sys/windows/poll.rs` (unexpected `WaitForMultipleObjects` result), `event/sys/windows.rs` (`original_console_mode`), `event/sys/unix/parse.rs` (`could_not_parse_event_error`).

Why it is so large: a `&str` payload monomorphizes `From<&str> for Box<dyn Error>`, and the boxed `StringError`'s `Debug` vtable reaches `str`'s `escape_debug`, anchoring `core::unicode`'s printable-class and grapheme tables (~3.4 KB of `.rdata`) plus `core::fmt` glue (~3.2 KB of `.text`) — found by linker-map attribution, see OPTIMIZATION.md.
Nothing in this app ever reads an error message: `main` handles every `Result` with `if let Ok` and exits.

Measured on Windows (immediate-abort): **64,296 → 57,716 B, −6,580 B**.

## 5. ASCII-only key case correction

`event/sys/windows/parse.rs`, `try_ensure_char_case` — the shift/capslock case fix-up uses `to_ascii_lowercase`/`to_ascii_uppercase` instead of the full-unicode `to_lowercase()`/`to_uppercase()` iterators, whose case-mapping tables cost ~5 KB of `.rdata` + ~1 KB of `.text`.
Behaviour changes only for non-ASCII keyboard input: such characters now pass through in whatever case the keyboard layout produced instead of being re-cased.
This game only accepts `is_ascii_alphabetic` input (and lowercases it itself), so the difference is unobservable here.

Measured on Windows (immediate-abort): **57,716 → 51,608 B, −6,108 B**.

## 6. Millisecond poll clock

`event/timeout.rs` — on Windows, `PollTimeout` stamps `GetTickCount64()` (u64 milliseconds) instead of `Instant`, which there is `QueryPerformanceCounter` behind a `Once`-cached frequency plus 128-bit `Duration` arithmetic.
Millisecond resolution is exactly what `WaitForMultipleObjects` consumes anyway; Unix keeps `Instant` (a thin `clock_gettime`, nothing to save).
The only `unsafe` is the `GetTickCount64` FFI call (kernel32, always linked, cannot fail).

Measured on Windows (immediate-abort): **51,608 → 51,074 B, −534 B**.

## 7. Event set reduced to the game's inputs

Both platforms' event sources and parsers deliver only what this app consumes: key **presses** (Esc, Enter, Backspace, Ctrl+letter, and printable characters), **left-button-down** mouse events with their position, and resizes.
Everything else stops at the parser.

**This is by far the least upstream-shaped patch — re-audit it first when re-vendoring.**
It is also the one crossterm's own unit tests no longer describe: the `#[cfg(test)]` module in `event/sys/unix/parse.rs` still asserts upstream behaviour and would fail if anyone ran crossterm's test suite (nothing here does — it is built lib-only as a dependency).

### Windows (`event/source/windows.rs`, `event/sys/windows/parse.rs`)

Kept: presses only, with `get_char_for_key`'s `ToUnicodeEx` path intact so non-QWERTY layouts still type.
Dropped: key releases (upstream kept them for the Alt-code exception, itself dropped), Alt-codes, surrogate pairing (a lone surrogate `u_char` now falls into `char::from_u32`'s `None`), the function/navigation `KeyCode` arms (F1–F24, arrows, Home/End/PageUp/PageDown, Insert/Delete, Tab — their `u_char` resolves to no character or to a control char the game ignores), `FocusGained`/`FocusLost`, and every other mouse kind (up, drag, move, both scroll axes, right/middle buttons).
This also deleted the parser's 552 B `.rdata` jump table and the `decode_utf16` machinery.

Measured (immediate-abort): **50,898 → 49,182 B, −1,716 B**.

### Unix (`event/sys/unix/parse.rs`)

The same reduction against a very different parser — an incremental ANSI decoder rather than a WinAPI record switch — so the mechanics differ:

- `parse_event` keeps Esc, `\r`, `\x7F`, the Ctrl+letter range (which carries Ctrl+C) and the UTF-8 character path. Dropped: SS3 sequences (`ESC O x`), the Alt+key branch — which was also `parse_event`'s **recursive** call — Tab, and the remaining control-code arms. `\n` and `\t` fall into Ctrl+J/Ctrl+I; upstream only differs outside raw mode, and this app is always in raw mode.
- `parse_csi` keeps the two mouse encodings and collapses everything else — arrows, F-keys, Home/End/PageUp/PageDown, Insert/Delete, BackTab, focus in/out, the kitty protocol, cursor-position and device-attribute replies, bracketed paste — into a single drop rule.
- Both mouse decoders keep only a left-button press, and read their parameters with a small byte-level decimal scan instead of `str::parse`, whose `FromStr` machinery dwarfed the three numbers it produced. rxvt mouse encoding (mode 1015) is dropped: it is only ever selected by a terminal that supports 1015 but not SGR 1006, which in practice means rxvt itself.
- `char_code_to_event` tests `is_ascii_uppercase` rather than `char::is_uppercase`, the Unix counterpart of change 5: a non-ASCII capital now arrives without the SHIFT modifier, the character itself unchanged.

**Framing is the subtle part.** The caller feeds bytes one at a time and treats `Ok(None)` as "incomplete, keep buffering" and `Err` as "clear the buffer and carry on".
A dropped sequence must therefore still be *consumed whole* before being refused, or its tail would be re-parsed as ordinary keystrokes — an arrow key would type letters into the board.
So the drop rule waits for a CSI final byte (`0x40..=0x7E`) before erroring, `ESC O` waits for its third byte, and `ESC [ [` keeps its own arm because `[` is itself a final byte.
`tools/pty_test.sh` covers exactly this (arrows, F-keys and `ESC [ 5~` must leave the draft empty), along with the rest of the input path on both mouse encodings.

Measured (immediate-abort): **81,599 → 73,415 B, −8,184 B** — far more than on Windows, because `parse_event` was the binary's single largest symbol (4,851 B) and its parameter parsing anchored `FromStr`, `str::split` and `core::fmt` padding on top.

The unused parsers are left in the file, unreferenced, behind a file-level `#![allow(dead_code)]`, so a re-vendor stays a small diff — same reasoning as `tput_value`/`tput_size` in change 1.



## 8. Blocking event source, no queue

`event.rs` — a single `event::next() -> io::Result<Event>` that blocks on the platform's event source, replacing the `poll` + `read` pair for this app.

Upstream's two calls are a pump: `poll` asks the source for an event, **queues** it, and answers "yes there is one"; `read` then takes it back out. Everything between them exists to carry that event across the two calls — a `VecDeque` plus a `Vec` of filtered-out events, the `Filter` trait, `InternalEventReader`, and a `Box<dyn EventSource>` behind a static. But `EventSource::try_read` already returns `Result<Option<InternalEvent>>`: one call, one event. `next` holds the source in a static named by its **concrete** type (no vtable), calls `try_read(None)` and hands the `InternalEvent::Event` payload straight back.

`None` means "no timeout", which is the second half of the change: this app renders on change and only an event changes anything, so the old 200 ms poll interval woke the process up for nothing. Blocking removes `PollTimeout`'s timeout arithmetic from the caller's side and drops the process to zero wakeups while idle.

Windows keeps its source inline in the static (three words). Unix's carries the parser's buffers, so it stays boxed — inlined it would trade a pointer-sized win for ~3 KB of `.bss`. The Unix source can also yield replies to terminal queries this app never sends (cursor position, device attributes); `next` skips them and blocks again.

Nothing else changes: `poll`, `read`, `InternalEventReader` and the filters stay in the tree, unreferenced, and the linker drops them — same treatment as the unused parsers in change 7, and it keeps a re-vendor a small diff. Verified: reverting `event/read.rs` to the upstream file leaves the binary byte-identical.

Measured on Windows (immediate-abort): **48,984 → 46,455 B, −2,529 B**, which takes crossterm's own out-of-line symbols from 3,235 B to 59 B.
`tools/pty_test.sh` passes identically against a control binary built without this change.

## Upgrading crossterm

Moving to a new crossterm is a re-vendor: fetch the new source, copy `src/`, `Cargo.toml`, `LICENSE` and `README.md` over this directory (this file excepted), grep it for the sites listed above and re-apply each change, then bump the version requirement in the workspace `Cargo.toml` to match.

To review the current patch, or to check that a re-vendor left exactly the intended changes, diff against the registry copy:

```bash
diff -ru ~/.cargo/registry/src/index.crates.io-*/crossterm-0.29.0/src vendor/crossterm/src
```

For 0.29.0 that reports exactly the files listed in the table above (plus `Cargo.toml` byte-identical to upstream apart from line endings); every hunk carries its `LOCAL PATCH` marker.

## What is kept

Only `src/`, `Cargo.toml`, `LICENSE` and `README.md` are kept from the upstream tarball; examples, docs, benches, tests and build cruft were removed, since crossterm is built lib-only as a dependency.
