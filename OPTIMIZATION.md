# Binary size optimization

> How to build each profile on each platform: **[BUILD.md](BUILD.md)**. This file is the rationale
> and the measured sizes; it refers to profiles by name and does not repeat the commands.

Reference (before): **396,288** bytes — the first crossterm+ratatui TUI, unoptimized.

The original goal was to go below ~300 KB, roughly the size of the repo's **initial commit**
(`3b320c2`, *"wordle game"*) — a plain stdin/stdout wordle solver (`rand` its only dependency, no
TUI, word lists as raw text), written as throwaway beginner Rust with no size effort at all. It
stands for what an unoptimised, naive CLI happens to compile to; the point of the exercise was to
bring the full interactive TUI below even that.

Initial `.text` breakdown (cargo-bloat): std 72.5 KB, ratatui_core 30 KB, wordle_tui 13.9 KB,
crossterm 10.3 KB, ratatui_widgets 8.5 KB, hashbrown 8.2 KB, kasuari 6.8 KB, parking_lot 3 KB,
unicode_width 2.8 KB, unicode_segmentation 2.5 KB, parking_lot_core 2.2 KB.
Embedded data: valid.bin 62,570 + answers.bin 11,695 = ~74 KB (raw 5-letter ASCII).

## History (stable, `cargo build --release`)

| # | Change | Size | Delta |
|---|--------|------|-------|
| 0 | Baseline | 396,288 | — |
| 1 | Words packed as 3-byte base-26 (instead of 5) | 366,592 | −29,696 |
| 2 | **Dropped ratatui → pure crossterm rendering** (layout reimplemented, fidelity snapshot-verified) | 242,176 | −124,416 |
| 3 | crossterm `default-features=false`, features `[windows, events]` | 241,664 | −512 |
| 4 | Data as LEB128 delta (gaps), streamed at runtime (instead of 3 fixed bytes) | 219,136 | −22,528 |
| 5 | render: raw bytes instead of String/from_utf8_lossy | 217,600 | −1,536 |
| 6 | main no longer returns io::Result (no Termination formatting) | 215,552 | −2,048 |
| 7 | Streaming varint lookup (drops OnceLock/Vec/binary_search) | 214,528 | −1,024 |
| 8 | Data as a single arithmetic-coded union (see `codec.rs`); source split into `build/` modules; idiomatic pass | 214,016 | −512 |

**~300 KB goal beaten at step 2** — mostly by removing ratatui (which dragged in kasuari,
hashbrown, lru, compact_str, unicode-*, parking_lot, …) and rendering directly through
crossterm. The ratatui cassowary layout algorithm was reimplemented by hand; its exact
integer rounding is reproduced by the unified spacer model
`spacer[i] = round((i+1)·E/g) − round(i·E/g)`.

### Visual fidelity (historical)

The crossterm renderer was validated byte-for-byte against reference snapshots of the original
ratatui output (`TestBackend`) across 15 scenarios — glyphs and colors. That ratatui oracle and
its snapshots have since been removed from the tree (commit *"Remove ratatui & snapshots"*); the
note is kept here as a record of how the rewrite's fidelity was established.

### Data layer

The data went through several encodings (3-byte base-26 → LEB128 gaps → streaming varint) and is
now a single **arithmetic-coded union** of both word lists (`src/codec.rs`, built by
`build/`): 14,853 words in `union.bin` at **~15.2 KB** (~1.02 B/word), searched by walking the
stream. Compression is essentially at the information-theoretic limit; nothing more to shave on
the data side. The encoder (build) and decoder (game) share `codec.rs` verbatim so they agree
bit-for-bit — a `union_round_trips` test guards this.

## Measuring (`tools/stats.rs`, `cargo run --example stats`)

The shipped `.exe` is a PE file whose sections are padded to 512 B, so a real saving of a few
dozen bytes often does **not** change the file size — it hides in the padding. Compare the sum of
each section's `VirtualSize` (un-padded), not the file size.

`cargo run --example stats` prints the compression report **and**, once the game is built, the
binary's on-disk size plus every PE section's `VirtualSize` and their total. It only *measures*
(it does not build), so build first, then run it:

```
cargo build --release                          # or any BUILD.md profile
cargo run --example stats                       # measures the freshest release binary it finds
cargo run --example stats -- <path-to-binary>   # measure a specific binary
```

Without an argument it looks for the release outputs BUILD.md documents (the MSVC triple first,
then plain `target/release`); the argument overrides that.

- **file on disk** = what you distribute (512 B-aligned).
- **total VirtualSize** = real code + data, no padding. Compare *this* between changes.

(The build script runs before linking and can't see the final binary, so the report is a separate
post-build tool, not a `cargo:warning`.)

The tool prints the section breakdown on both platforms: a PE parser (`#[cfg(windows)]`) and an ELF
parser (`#[cfg(target_os = "linux")]`) whose per-section sizes and total match binutils `size -A`
exactly (verified). Elsewhere it prints the on-disk size only. For an independent cross-check or a
symbol-level breakdown on Linux:

```
size -A target/x86_64-unknown-linux-gnu/release/wordle_tui   # per-section, plus a Total
bloaty  target/x86_64-unknown-linux-gnu/release/wordle_tui   # symbol-level attribution
```

The on-disk file size (`wc -c` / `ls -l`) is the distributable; unlike PE's 512 B section padding,
ELF pads only to page alignment, so small savings still often hide in padding — compare the section
`Total`, not the file size.

## build-std levers (nightly, `.cargo/config.toml`)

Recompiling std from source with our profile (opt-level=z, LTO). NOT source changes; the game is
identical; the nightly toolchain needs `rust-src`. The `[unstable]` table is ignored by stable
cargo, so the stable build is untouched.

Three profiles, by increasing aggressiveness. **Build commands for every platform live in
[BUILD.md](BUILD.md)** — this file only names the profile and its effect:

- **Stable (default)** — 100% preserved, no prerequisites, cross-platform.
- **build-std, no `backtrace`** — std recompiled without its backtrace machinery; the panic hook
  still restores the terminal. The recommended no-compromise build.
- **immediate-abort** — every panic lowers to a bare abort; the terminal is **not** restored on a
  bug-panic (edge case — see the panic audit above).

Windows `.exe`:

| Profile | `.exe` | Behavior |
|---------|-------:|----------|
| Stable (default) | **214,016** | 100% preserved, no prerequisites |
| build-std, no `backtrace` | **117,248** | terminal still restored on panic |
| immediate-abort | **87,040** | panic → bare abort: terminal **not** restored (edge case) |

`build-std` requires an explicit `--target`; the config intentionally does **not** pin one in
`[build]`, so plain `cargo build` stays host-native and the project builds on any platform. The
MSVC linker flags are keyed under `[target.x86_64-pc-windows-msvc]` and are simply inert elsewhere.

### On Linux

The same three profiles build on Linux (link optimizations under
`[target.x86_64-unknown-linux-gnu]` in `.cargo/config.toml`). ICF (identical-code folding) is
folded into each profile's build command rather than being a profile of its own.

**ELF vs PE flag mapping.** `/DEBUG:NONE` → `-Wl,--build-id=none` (drops `.note.gnu.build-id`, in
the `[target]` block; measured **−112 B** vs. the linker's default build-id). `/OPT:ICF` has **no
default-linker equivalent** — GNU `bfd` does no identical-code folding, so ICF is done by `lld`:
the nightly profiles use the bundled `rust-lld` (`-Clinker-features=+lld`, no install), the stable
profile the system `lld` (`-fuse-ld=lld`). An env `RUSTFLAGS` overrides the `[target]` block rather
than merging, so `--build-id=none` is repeated in every command (same gotcha as Windows). ICF's
yield is small (~1–3 KB), matching `/OPT:ICF` on Windows.

### Measured Linux sizes (WSL Ubuntu, rustc 1.97, on-disk bytes; smallest per profile)

**glibc — dynamically linked** (the on-disk size *excludes* the system libc, loaded at runtime):

| Profile | Size | Δ baseline |
|---------|-----:|-----------:|
| Stable | 418,184 | — |
| build-std, no `backtrace` | 184,760 | −55.8% |
| immediate-abort | 125,896 | −69.9% |

**musl — statically linked** (self-contained, *embeds* libc + unwinder; zero runtime deps):

| Profile | Size | Δ baseline |
|---------|-----:|-----------:|
| Stable | 505,872 | — |
| build-std, no `backtrace` | 279,568 | −44.7% |
| immediate-abort | 148,784 | −70.6% |

Note musl static is **larger on disk than the glibc builds**, not smaller: it bakes the whole libc
into the file. The glibc numbers look smaller only because they offload libc to the system at load
time — the musl binary is the honest "everything included" size and runs on any Linux with no deps.

**build-std + musl gotcha.** build-std rebuilds the Rust sysroot from source but does **not** build
the musl C runtime; the CRT objects (`rcrt1.o`, `libunwind.a`, …) come from the *prebuilt* target's
`lib/rustlib/x86_64-unknown-linux-musl/lib/self-contained/`. So that target must be installed on the
**nightly** toolchain (not just stable), else linking fails with `cannot find rcrt1.o` / `-lunwind`;
`musl-tools` is *not* what fixes it. BUILD.md's musl section spells out the `rustup target add` step.

**Cross-platform caveat.** These are **not** directly comparable to the Windows `.exe` numbers in
this file — different linker, CRT, and section layout (and glibc offloads libc while the Windows
`.exe` and musl do not). To compare platforms, re-measure a Windows and a Linux build under the
same lever rather than reading across the two tables.

The immediate-abort command also passes `--cfg immediate_abort`, which gates the panic hook out of
`main.rs`: that build lowers every panic to a bare abort that bypasses the hook, so registering one
is dead weight (declared via `cargo::rustc-check-cfg` in `build/main.rs` to keep the lint quiet).

### The big lever: build std without the `backtrace` feature

**Saved ~30 KB of `.text` / ~39 KB of `.exe` (156.7 KB → 117.8 KB).** `cargo bloat` showed the
largest non-game mass was the panic **backtrace/demangle** machinery: `rustc_demangle` (~8 KB),
`std::sys::backtrace` (~6 KB), `backtrace_rs::symbolize` + the `dbghelp` walk, plus satellites
(`getenv` for `RUST_BACKTRACE`, `path::components`, `slice_error_fail`) — **57 symbols**. It is
all dead weight: nothing in the game prints a backtrace.

It cannot be dropped from source: std's panic runtime keeps `match HOOK { Hook::Default =>
default_hook(..) }` compiled, and LTO cannot prove the hook is never `Default`, so `default_hook`
(and everything it calls) stays reachable no matter what custom hook we install. Confirmed:
replacing the hook left all 57 symbols linked (only ~432 B of hook plumbing went).

The lever is at the std level:

```toml
[unstable]
build-std = ["std", "panic_abort"]
build-std-features = []   # none of std's defaults -> no `backtrace`
```

Result: 57 → 4 backtrace symbols (~45 B of inert stubs). Crucially still a **safe** build: the
panic hook runs and restores the terminal; a bug-panic just aborts without a symbolized trace.

### The panic hook

`main` installs a minimal hook that restores the terminal and then `std::process::exit(101)`:

1. **Not delegating to the default hook.** Printing the panic message would pull `io::stderr` +
   `writeln!` machinery the default path doesn't share; every message-printing variant measured
   *larger* (delegating to `default_hook`: +416 B; `writeln!("{info}")`: more). On a bug-panic the
   restored screen is what matters, not the text.
2. **`exit()` instead of returning.** If the hook returns, `panic = "abort"` calls `abort()`,
   which on Windows hands off to Windows Error Reporting — a **~1 s stall** before the shell
   prompt returns. `exit(101)` terminates first and skips WER. Costs +80 B `.text`, 0 in the
   `.exe`. 101 is Rust's conventional panic exit code.

### Where panics come from (and why removing them doesn't help)

- **Explicit (ours):** one in the runtime path — `unreachable!(..)` in `pick_target`. No
  `unwrap`/`expect` at runtime (`main` handles every `Result` with `if let Ok`).
- **Implicit (~99 %):** bounds checks on every index, the range-coder divisions (`range /= tot`,
  `div_round`), slice ranges.

Removing our explicit panics saves almost nothing: the implicit bounds checks keep
`panic_bounds_check` → the panic runtime → the message formatting (`panic_with_hook` + the still-
reachable `default_hook` + `core::fmt`) all linked.

The lever that *does* work is `-Cpanic=immediate-abort`: it makes every panic — bounds check,
divide, slice, `unreachable!` — compile to a bare abort, so every call site loses its location
struct / argument setup / message string program-wide (in std too, not just our code). Measured:
**117,760 → 87,040 (−30 KB)**, far more than the ~3 KB of message formatting alone. The catch is
that immediate-abort bypasses the hook mechanism, so `restore_terminal` never runs and the
terminal is left dirty on a bug-panic. That trade — 30 KB vs. a clean screen after a crash — is
the user's call; the default here keeps the safe build.

### immediate-abort safety (manual panic audit)

The catch above — a dirty terminal on panic — only bites if a panic is actually *reachable*.
It is not. Every runtime panic site was traced by hand (release profile has
`overflow-checks = false`, so integer overflow/underflow **wraps** and is not itself a panic;
the only panic families left are out-of-bounds index/slice, divide-by-zero, `copy_from_slice`
length mismatch, and explicit panics). The audit's verdict: **no panic is reachable while the
embedded `union.bin` is intact**, so under immediate-abort the dirty-terminal case is purely
theoretical (it would require a corrupted binary, at which point terminal state is moot).

**Locally guaranteed** (no external assumption):

- **`Grid` rendering (`ui.rs`)** — every write/read goes through `set`/`text`/`hit_rect`/
  `hit_test`, all guarded by `px < self.w && py < self.h`. Off-screen access is a silent no-op;
  the renderer cannot panic regardless of terminal size (even a `w == 0` underflow wraps to a
  huge index that the bound rejects).
- **`app.rs` indexing** — `history[input_idx]` is safe because `input_idx ∈ 0..=5` while
  `Playing` (`submit` flips to `Lost` as soon as `input_idx >= 6`) and every draft/typing path
  runs only in `Playing`; `keyboard_letter_states` slices `history[..input_idx]` with
  `input_idx <= 6 = history.len()`. `type_letter`/`backspace` are guarded on `input_len`.
  `copy_from_slice` copies two `[u8; 5]`. The lost-message builds its string by pushing the
  target's uppercased bytes as `char`s (all ASCII, infallible — no `str::from_utf8`).
- **`ui.rs` layout math** — the sole runtime division (`div_round`) is only called from `gaps`
  with divisor `d = n - 1 >= 1` (guarded `n <= 1`); `gaps`/`stack_sizes` run with
  `n <= SECTION_COUNT = 4 < MAX_GUESSES`, so their fixed-size arrays never overflow;
  `col_budget` steps `cell` down from an odd `full` and stops at 1; `saturating_sub`/`div_ceil`
  guard every spot that could underflow.
- **`game::check`** — bounds `0..WORD_LEN` over `[u8; 5]` arrays passed by `submit`.
- **Range-coder divisions** (`decode_freq`: `range /= tot`, then `code / range`) — `range >= TOP
  = 2^24` is held by the renorm loops, and every `tot <= WORD_COUNT = 14853 < 2^24`, so
  `range / tot >= 1`: `code / range` can never divide by zero.

**Load-bearing invariant** — two sites are *not* locally guarded; both reduce to the same
assumption, that `union.bin` is exactly the encoder's output (fixed at build time, checked by
the `union_round_trips` test). They carry a source comment pointing here:

1. **`unreachable!` in `pick_target` (`game.rs`)** — hit only if the stream holds fewer than
   `ANSWER_COUNT` colour-A words. The encoder emits exactly `ANSWER_COUNT`.
2. **Divide-by-zero in `decode_freq` via `tot == 0`** — the only zero total is
   `char_tot(ctx, lo)` with `lo == 26` (empty sum), which needs a word whose first differing
   character exceeds `'z'`. The sort proves `w[p] > prev[p]` with `w[p] <= 25`, so
   `prev[p] <= 24` ⟹ `lo <= 25` ⟹ at least the `s = 25` term ⟹ `char_tot >= 1`. The colour
   decode's `remaining` likewise stays in `1..=WORD_COUNT`, and zero `freq` values are excluded
   by the sampling-without-replacement logic.

No runtime `assert!` was added to "lock" these: that would create panic sites and grow the
binary — the opposite of the goal. The invariant is enforced where it belongs, at build time.

## Tried and rejected (don't redo these)

- **`ORDER` as a const generic on `Model`.** −48 B `.text`, **0 B in the `.exe`**. The compiler
  already const-propagates `ORDER` (single call site, value baked into `constants.rs`), so the
  const generic recovers only the residue, which hides in padding. Not worth threading a generic
  through `Model`/`decode_word`/`encode_*` + the build's search dispatch.
- **Generating the decoder as source in the build.** Rejected: nothing in the decoder's *shape*
  varies with the word data — only 3 numbers (`ORDER`/`INC`/`WORD_LEN`), already compile-time
  constants. Codegen would buy at most what const generics buy (≈0) while sacrificing the single
  shared `codec.rs` that makes encode/decode provably agree.
- **`App::submit`: array-copy → `guess == &self.target`.** Cleaner, but **+48 B**. Kept the copy.
- **`ui::gaps`: indexed loop → `iter_mut().take(d).enumerate()`** (clippy's suggestion). **+16 B**
  (the `.take` iterator machinery). Kept the indexed loop under `#[allow(needless_range_loop)]`.
- **Raw-handle stdout output (Windows): write to the stdout `HANDLE` via `File`/`WriteFile` to skip
  std's console UTF-16 path (`write_valid_utf8_to_console` + `EncodeUtf16`).** Measured only
  **−314 B** — the `File`/`WriteFile` path pulls back most of what the UTF-16 path freed. Not worth
  the `unsafe` handle wrapping, a platform-split writer type, and output-path risk that can't be
  verified without a real Windows console. Rejected.

## Idiomatic changes that were free or a win

Most idiomatic cleanups compile identically (confirmed byte-neutral): `Ordering` import +
`w.cmp(&target)`, `array::from_fn`, the named-constant colour palette in `ui.rs`, small renames.
One was a genuine win — **`ui::center`: `(outer-inner+1)/2` → `saturating_sub(inner).div_ceil(2)`**,
cleaner and **−32 B**. Lesson: idiomatic ≠ smaller; measure each change.

### `/DEBUG:NONE`: drop the residual Debug Directory

`strip = true` removes symbols but the linker still emits an IMAGE_DEBUG_DIRECTORY (a CodeView
entry pointing at `wordle_tui.pdb`, plus a REPRO entry) — ~84 B, and it leaks the `.pdb` path as
a string in `.rdata`. `-Clink-arg=/DEBUG:NONE` (in `[target.*] rustflags`) drops all but the 28 B
REPRO stub. Measured: build-std **117,760 → 117,248** on disk (−512 B, crossed a section-alignment
boundary); immediate-abort −56 B VirtualSize (0 on disk, hides in padding). Applies to stable too.

**Gotcha:** setting `RUSTFLAGS` in the environment (the immediate-abort command) *overrides* the
config's `[target.*] rustflags` — it does not merge. So `/OPT:ICF` and `/DEBUG:NONE` must be
repeated in that command line or they are silently lost.

### Dependency feature audit (crossterm & its deps) — nothing to gain

Checked whether trimming crossterm or its transitive deps' cargo features could shed more:

- **crossterm** is already at the floor: `default-features = false, features = ["windows",
  "events"]`. That drops `bracketed-paste` and `derive-more` (→ no `derive_more`). `events` is
  required for input; `windows` for raw-mode/console. crossterm has **no `no_std` support** (it is
  std-only, via parking_lot/mio/etc.) — not an option.
- **`events`** pulls `mio`/`signal-hook`/`signal-hook-mio` only under `cfg(unix)`; on Windows they
  are not compiled (no such symbols in the binary — crossterm uses its WinAPI event source).
- **`parking_lot` has `default = []`** — no default features to strip; the ~2.8 KB is its base
  mutex/`Once` (used by crossterm's `INTERNAL_EVENT_READER: Mutex<…>`), irreducible by feature.
  Every parking_lot feature (`deadlock_detection`, `hardware-lock-elision`, `send_guard`, …) only
  *adds* code.
- **Cargo unifies features (union) across the graph**, so from our `Cargo.toml` we can only *add*
  a transitive dep's features, never remove one that crossterm's own edge requests. Tuning
  crossterm's deps from here is therefore impossible; the only lever would be `[patch]`-ing
  crossterm, which buys nothing given `parking_lot`'s empty defaults.

## Raw ANSI output (dropped crossterm's `style`/`Command` layer)

`ui::render` and `main.rs`'s terminal setup no longer go through crossterm's
`SetForegroundColor`/`MoveTo`/`EnterAlternateScreen`/`SetTitle`/`Clear`/… commands; they write the
escape bytes directly. Colors are precomputed `&'static [u8]` SGR constants stored in each `Cell`;
alt-screen/title/cursor-hide/clear are literal CSI/OSC byte strings (byte-identical to what
crossterm emits). What this reclaims on **Windows**:

- the **integer `fmt` machinery** the color/cursor commands pulled in via `write!`
  (`Colored::fmt`, `pad_integral`, `fmt::write`) — `render` was the only integer-formatting site;
- the **WinAPI fallback** of every terminal/cursor command (`SetConsoleTitleW` + its UTF-16
  `EncodeUtf16` path, alt-screen/cursor/clear via the console API): each command compiled *both* an
  ANSI and a WinAPI path, picked at runtime by `supports_ansi()`, so both were linked. Deleting the
  commands drops the WinAPI halves.

Mouse capture **stays** on crossterm: on Windows `EnableMouseCapture` is WinAPI-only
(`is_ansi_code_supported() == false`) because the console event source reads mouse from the input
buffer, not from ANSI reports — the `?1000h…` sequences would not work. Since we no longer call
`supports_ansi()` (which enabled VT as a side effect), `init_terminal` sets
`ENABLE_VIRTUAL_TERMINAL_PROCESSING` itself via `crossterm_winapi` (`#[cfg(windows)]`, already a
transitive dep of crossterm → **0 graph cost**).

Measured Windows immediate-abort: **77,838 → 72,061 (−5,777 B)**. On **Linux** it is byte-neutral
(+15 B): crossterm already emits ANSI there and `supports_ansi` is `#[cfg(windows)]`, so there was
no WinAPI path to reclaim. What it did **not** reclaim is `supports_ansi`'s `env::var` + `Once` —
see Stuck costs.

## Vendored crossterm: ioctl-only `terminal::size()` (Linux −28,705 B)

crossterm's `terminal::size()` spawns `tput` as a fallback; because the event source calls `size()`
on every resize, that fallback is always reachable and anchors `std::process::Command` + a
`BTreeMap<OsString, OsString>` env copy + their `Debug`/`fmt` subtree. A **vendored crossterm** with
an ioctl-only `size()` drops it: **Linux 118,608 → 89,903, −28,705 B**; Windows unaffected. It is
**opt-in** (a `[patch]` can't be a Cargo feature), injected via `--config .cargo/crossterm-patch.toml`
so a plain `cargo build` stays on upstream. Full rationale, wiring, and the `Cargo.lock` caveat:
**`vendor/crossterm/LOCAL_PATCH.md`**.

Measured Linux: **118,608 → 89,903 (−28,705 B, −24 %)**; `nm` confirms `tput_value`/`Command`/
`BTreeMap` are gone. Windows is unaffected (its `size()` uses `GetConsoleScreenBufferInfo`, no
`tput`). Only crossterm's lib target is built as a dependency, so the vendored copy is trimmed to
`src/` + `Cargo.toml` + `LICENSE` + `README.md`.

## Stuck costs

- **`parking_lot` (~3.3 KB) + the `supports_ansi` probe (~2.5 KB): `env::var` + a
  `parking_lot::Once`, plus a `Colored::fmt`/`pad_integral` residue.** Both are anchored by
  `crossterm::event`/`terminal` on Windows — the global `INTERNAL_EVENT_READER` mutex, and a
  still-reachable `supports_ansi()`. **Confirmed stuck by experiment:** after moving *all* rendering
  and terminal setup off crossterm's `style`/`Command` layer (see "Raw ANSI output") and even
  stubbing out the last `execute!` (mouse) calls, `supports_ansi`/`env::var`/`Once` remain in the
  binary. So the earlier belief that this ~2.5 KB was reclaimable "by bypassing style/Color" was
  **wrong**: it is held by `event`/`terminal`, not by styling, and cannot go without dropping input.
- On **stable**, the panic/backtrace machinery (~12 KB), io::error fmt (~3 KB) and env (~2 KB) are
  unavoidably linked by the panic runtime — the `backtrace` lever above only exists on nightly.
- **Linux `.eh_frame` + `.eh_frame_hdr` (~18.6 KB).** Unwind tables from the *prebuilt* std. Dead
  under `panic=abort`/immediate-abort (nothing unwinds), but this profile links prebuilt std, so the
  build flags cannot drop them. Reclaimable via build-std (`-Cforce-unwind-tables=no`) or a
  post-link `objcopy --remove-section .eh_frame --remove-section .eh_frame_hdr` (measured Linux
  **118,608 → 99,956, −18,652 B**; runtime-validate before relying on it — it leaves a dangling
  `PT_GNU_EH_FRAME` program header).
  (The `tput` spawn cluster, once the biggest Linux stuck cost at ~28.7 KB, is now **reclaimed** —
  see "Vendored crossterm" above.)
- Everything else in `.text` is either ours (`main`, `ui::build_grid`, `app::submit`,
  `codec::decode_word`) or genuinely-used std/crossterm.

## Summary

- Baseline: 396,288
- Stable optimized (default, no prerequisites): **214,016** (−46.0%)
- build-std nightly, no `backtrace`, terminal still restored: **117,248** (−70.4%)
- + immediate-abort, **Windows** (terminal not restored on panic): **72,061** (−81.8%)
- + immediate-abort, **Linux** (x86_64-unknown-linux-gnu, vendored crossterm): **89,903** (−77.3%)

The two immediate-abort figures are the current numbers, after the arithmetic-coded data layer, the
raw-ANSI output refactor, and the vendored ioctl-only `size()`; the stable / build-std-with-restore
rows above predate that recent work and were not re-measured this round. Linux is now ~18 KB heavier
than Windows, almost all of it the `.eh_frame` unwind tables (~18.6 KB, still stuck — see above),
plus the genuinely-needed Unix input stack (`parse_event`, signal-hook, mio).
