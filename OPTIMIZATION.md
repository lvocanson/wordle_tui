# Binary size optimization

Reference (before): **396,288** bytes. Goal: 308,224 bytes.

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

**Goal (308,224) beaten at step 2** — mostly by removing ratatui (which dragged in kasuari,
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

## Measuring (`size.ps1`)

The shipped `.exe` is a PE file whose sections are padded to 512 B, so a real saving of a few
dozen bytes often does **not** change the file size — it hides in the padding. Compare the sum of
each section's `VirtualSize` (un-padded), not the file size.

`size.ps1` (repo root) wraps `cargo build`, then prints every section's `VirtualSize` plus a
total. Use it as the measurement of record:

```
.\size.ps1                 # = cargo +nightly build --release, then the section report
.\size.ps1 build --release # stable build, then the report
```

- **file on disk** = what you distribute (512 B-aligned).
- **total VirtualSize** = real code + data, no padding. Compare *this* between changes.

(The build script runs before linking and can't see the final binary, so the report must be a
post-build wrapper, not a `cargo:warning`.)

## build-std levers (nightly, `.cargo/config.toml`)

Recompiling std from source with our profile (opt-level=z, LTO). NOT source changes; the game is
identical. Requires `rustup toolchain install nightly --component rust-src`. The `[unstable]`
table is ignored by stable cargo, so the stable build is untouched.

| Variant | Command | `.exe` | Behavior |
|---------|---------|--------|----------|
| Stable (default) | `cargo build --release` | **214,016** | 100% preserved, no prerequisites |
| build-std, no `backtrace` | `cargo +nightly build --release` | **117,760** | terminal still restored on panic |
| + immediate-abort | `RUSTFLAGS="-Zunstable-options -Cpanic=immediate-abort -Clink-arg=/OPT:ICF" cargo +nightly build --release` | **87,040** | panic → bare abort: terminal **not** restored (edge case) |

The recommended no-compromise build is now **build-std without `backtrace` (117,760)** — see below.

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
  `copy_from_slice` copies two `[u8; 5]`. `str::from_utf8(&target)` feeds `unwrap_or_else`.
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

## Idiomatic changes that were free or a win

Most idiomatic cleanups compile identically (confirmed byte-neutral): `Ordering` import +
`w.cmp(&target)`, `array::from_fn`, the named-constant colour palette in `ui.rs`, small renames.
One was a genuine win — **`ui::center`: `(outer-inner+1)/2` → `saturating_sub(inner).div_ceil(2)`**,
cleaner and **−32 B**. Lesson: idiomatic ≠ smaller; measure each change.

## Stuck costs

- **`parking_lot` (~3.3 KB):** a hard dependency of crossterm's `events` feature. Not removable
  without losing input handling. Already on `default-features = false, features = ["windows",
  "events"]`.
- On **stable**, the panic/backtrace machinery (~12 KB), io::error fmt (~3 KB) and env (~2 KB) are
  unavoidably linked by the panic runtime — the `backtrace` lever above only exists on nightly.
- Everything else in `.text` is either ours (`main`, `ui::build_grid`, `app::submit`,
  `codec::decode_word`) or genuinely-used std/crossterm.

## Summary

- Baseline: 396,288
- Stable optimized (default, no prerequisites): **214,016** (−46.0%)
- build-std nightly, no `backtrace`, terminal still restored: **117,760** (−70.3%)
- + immediate-abort (terminal not restored on panic): **87,040** (−78.0%)
