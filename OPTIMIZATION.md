# Binary size optimization

The measured record of shrinking the release binary from **396,288 B** to **40,448 B** on Windows, and the reasoning behind each change.

[README.md](README.md) presents the game; **[BUILD.md](BUILD.md) holds the build command for every profile on every platform**.
This file never repeats a command — it names a profile and reports what it measured.

**Contents** — [Where it stands](#where-it-stands) · [How sizes are measured](#how-sizes-are-measured) · [Changelog](#changelog) · [Rejected experiments](#rejected-experiments) · [Stuck costs](#stuck-costs) · [Panic handling](#panic-handling)

## Where it stands

| Profile | Windows | Linux (glibc) | Linux (musl) |
|---------|--------:|--------------:|-------------:|
| Baseline — first crossterm+ratatui TUI | 396,288 | — | — |
| Stable — no prerequisites, cross-platform | 135,680 (−65.8%) | 332,752 | 426,280 |
| `ship` — pinned nightly, most aggressive; terminal **not** restored on panic | **40,448** (−89.8%) | 70,808 | 86,008 |

Caveats on that table, all of them about *when* a number was taken:

- Every profile figure comes from the current source, all six re-measured 2026-08-29 — stable on whatever toolchain the machine had, `ship` on the pinned nightly.
  The **baseline** is the one number not re-measured, kept for scale. It predates the corpus format, so read the percentages as indicative rather than like-for-like.
- Windows and Linux numbers are **not** comparable to each other: different linker, CRT and section layout, and glibc offloads libc to the system while the Windows `.exe` and musl do not.
  To compare platforms, re-measure both under the same lever.
  Linux currently runs ~30 KB heavier than Windows, about half of it the `.eh_frame` unwind tables (see [Stuck costs](#stuck-costs)) and most of the rest the Unix input stack — mio, signal-hook and the signal-driven resize path — which has no Windows counterpart.

The embedded corpus accounts for 14,283 B — 35% of the 40,448 B Windows binary, 38% of its section total (the share `tools/stats.rs` prints).

## How sizes are measured

Build with a profile from [BUILD.md](BUILD.md), then measure with `tools/stats.rs`.
Totals are only comparable at equal rustc — a toolchain bump alone can move the Windows total by tens of bytes — so the measurement toolchain is **pinned** (in `wtui-ship.sh`); bump it deliberately and re-measure the reference totals when you do.
The tool only *measures* — it never builds, so build first:

```bash
cargo run --example stats
```

With no argument it measures the freshest of the release outputs BUILD.md documents (the target triple first, then plain `target/release`).
Pass a path to pin the binary — necessary when several profiles coexist in `target/`:

```bash
cargo run --example stats -- target/x86_64-pc-windows-msvc/release/wordle_tui.exe
```

It prints two reports: the **compression report** (word counts, packed size, B/word, and the model constants the build chose) and the **binary report** — the on-disk size plus every section's un-padded size and their total.
The format is sniffed from the file itself — PE `VirtualSize` or ELF `sh_size` (where the per-section figures and the total match binutils `size -A` exactly) — so either platform's binary can be measured from either host.
Each run also records its section sizes in a `.sections` sidecar next to the measured binary and prints the per-section delta against the previous run of that same binary, and it flags any zero-initialized data (`.bss`-like bytes that sit in the total but ship no file bytes — currently zero).

> **Compare the section total, not the file on disk.**
> PE pads sections to 512 B and ELF to page alignment, so a genuine saving of a few dozen bytes usually shows as 0 on disk — and occasionally as a −512 cliff that credits one change with the padding several changes filled.
> The file on disk is what you ship — the number README and [Where it stands](#where-it-stands) quote; the section total is what you compare.
>
> This is why the two halves of the [changelog](#changelog) use different metrics: changes #1–#8 predate the tool and are on-disk `.exe` bytes (each one happens to be a multiple of 512), while #12 onward are section totals.

The build script runs before linking and cannot see the final binary, which is why this is a post-build tool rather than a `cargo:warning`.

**Full validation run.**
`tools/validate.sh` does everything in one invocation — tests, the Windows build and its size, the Linux build and size through WSL (both measured with the same `stats.rs` reporter), and the symbol-bloat report over both linker maps (`tools/bloat.rs`) — on the shipping profile (ship).
It exits non-zero with a summary if any step failed.
Use Git Bash on Windows:

```bash
bash tools/validate.sh
```

```bash
bash tools/validate.sh --quick
```

(`--quick` = tests + Windows size only; `--no-linux` and `--no-bloat` are also accepted.)
The `--config` patch build rewrites `Cargo.lock`; the script restores it on exit.

**Independent cross-checks on Linux:**

```bash
size -A target/x86_64-unknown-linux-gnu/release/wordle_tui
```

```bash
bloaty target/x86_64-unknown-linux-gnu/release/wordle_tui
```

**Symbol attribution: `tools/bloat.rs` over the linker map.**
BUILD.md's ship commands make the link also emit its symbol map (`/MAP:target/wordle_tui.map` on MSVC, `-Wl,-Map=target/wordle_tui-linux.map` on lld) — **without changing a byte of the binary** (verified: only the 6 link-timestamp bytes differ).

```bash
cargo run --example bloat
```

reads the freshest map (pass a path to pin one, `-n N` for the list length) and prints a by-crate table plus the top symbols per section, sized post-LTO/ICF on the exact shipping binary: MSVC sizes are consecutive-address deltas (ICF folds share an address and are listed together instead of misattributed), lld sizes are exact per-input-section.
Crates come from the demangled names — under fat LTO every Rust symbol lands in one object, so object-based attribution is meaningless.
This method found changes [#24–#26](#changelog).

**`cargo bloat`** is not used here: it has two failure modes, both of them visible on this binary — identical-code folding makes it attribute a folded body to an arbitrary, often-dead symbol name, which is how `supports_ansi` and its `env::var` turn up in reports of a binary that does not contain them, and it re-runs its own build rather than reading the link that produced the shipping binary.
Cargo's *"patch … was not used in the crate graph"* warning is **not** a signal either way — it also fires on correct builds whenever the patched version equals the registry one; `tools/validate.sh` instead probes the built binary for upstream-crossterm markers (`NO_COLOR`/`COLORTERM`) and fails if they are present.
Confirm any lead by string-probing the binary and by a measured rebuild.

## Changelog

Every measured change, oldest first.
Each row is explained in the section of the same number below.

**Reading the delta columns:** `−1,234` / `+24` — measured on that platform.
`0` — measured, no change.
`—` — not measured on that platform.
`≈` — the source measurement was taken at KB resolution.

| # | Change | Δ Windows | Δ Linux | Windows after |
|---|--------|----------:|--------:|--------------:|
| | **A — Data & rendering rewrite** *(stable profile, on-disk `.exe`; from 396,288)* | | | |
| 1 | Base-26 word packing | −29,696 | — | 366,592 |
| 2 | ratatui dropped, direct crossterm rendering | −124,416 | — | 242,176 |
| 3 | crossterm feature floor | −512 | — | 241,664 |
| 4 | LEB128 gap-delta corpus | −22,528 | — | 219,136 |
| 5 | Byte-level render output | −1,536 | — | 217,600 |
| 6 | `main` no longer returns `io::Result` | −2,048 | — | 215,552 |
| 7 | Streaming varint lookup | −1,024 | — | 214,528 |
| 8 | Arithmetic-coded corpus | −512 | — | 214,016 |
| | **B — Build & link levers** *(profile transitions, not source changes; not additive with A or C)* | | | |
| 9 | `/DEBUG:NONE` (PE) / `--build-id=none` (ELF) | −56 | −112 | — |
| 10 | `build-std` without std's `backtrace` | ≈ −39 KB | ≈ −233 KB | — |
| 11 | `-Cpanic=immediate-abort` | −30,720 | −58,864 | — |
| | **C — Source & data changes since** *(ship profile, section totals; Windows from 77,838)* | | | |
| 12 | Raw ANSI output | −5,777 | +15 | 72,061 |
| 13 | Vendored crossterm: ioctl-only `size()` | 0 | −28,705 | 72,061 |
| 14 | Decoder without per-word heap `Vec` | −216 | — | 71,845 |
| 15 | Vendored crossterm: single-threaded, `parking_lot` dropped | −5,792 | −4,210 | 66,053 |
| 16 | `ui::render`: home once + CNL | −96 | +8 | 65,957 |
| 17 | `codec::Model` without the parallel `total` Vec | −180 | −12 | 65,777 |
| 18 | Asymmetric decoder: fixed-array model storage | −608 | +24 | 65,169 |
| 19 | Position-conditioned context | −696 | — | 64,473 |
| 20 | Searched word ordering | 0 | — | 64,473 |
| 21 | Conditioned colour bit | −176 | — | 64,297 |
| | **D — Measured standalone** *(not points on either chain)* | | | |
| 22 | `ui::center` via `saturating_sub`/`div_ceil` | −32 | — | — |
| 23 | Panic hook: `exit(101)` instead of returning | 0 *(+80 `.text`)* | — | — |
| | **E — Linker-map hunt** *(pinned `nightly-2026-08-25`; from the re-measured 64,296)* | | | |
| 24 | Vendored crossterm: no `io::Error` message payloads | −6,580 | ≈ 0 | 57,716 |
| 25 | Vendored crossterm: ASCII-only key case correction | −6,108 | 0 | 51,608 |
| 26 | Vendored crossterm: millisecond poll clock | −534 | 0 | 51,074 |
| 27 | `App` message in a fixed buffer | −176 | ≈ 0 | 50,898 |
| 28 | Vendored crossterm: event set reduced to the game's inputs | −1,716 | 0 | 49,182 |
| 29 | `ui::draw_board`/`draw_keyboard` outlined | −56 | — | 49,126 |
| 30 | Vendored crossterm: the same event-set reduction on Unix | 0 | −8,184 | 49,126 |
| 31 | VT enabling doubles as a startup probe | −142 | — | 48,984 |
| 32 | Vendored crossterm: blocking event source, no queue | −2,529 | −70 | 46,455 |
| 33 | Windows raw mode is already set by mouse capture | −124 | 0 | 46,331 |
| 34 | Vendored crossterm: key characters without the keyboard layout | −764 | 0 | 45,567 |
| 35 | Vendored crossterm: blocking console read, no poll | −136 | 0 | 45,431 |
| 36 | Vendored crossterm: console geometry without `CONOUT$` | −440 | 0 | 44,991 |
| 37 | `#![no_main]`: the process starts without Rust's runtime | −7,528 | −4,130 | 37,463 |

### 1 — Base-26 word packing

Both lists shipped as raw 5-byte ASCII (62,570 B of valid words + 11,695 B of answers ≈ 74 KB).
A five-letter word is a base-26 number below 26⁵ ≈ 1.19 M, so it fits in 3 bytes instead of 5.

### 2 — ratatui dropped, direct crossterm rendering

The single largest change, and the one that beat the ~300 KB goal.
ratatui dragged in kasuari, hashbrown, lru, compact_str, the unicode-* crates and parking_lot; rendering now goes straight through crossterm.
Its cassowary layout was reimplemented by hand, and the exact integer rounding is reproduced by the unified spacer model `spacer[i] = round((i+1)·E/g) − round(i·E/g)`.

Fidelity was established byte-for-byte — glyphs and colours — against reference snapshots of the original ratatui output (`TestBackend`) across 15 scenarios.
That oracle and its snapshots were removed from the tree afterwards (commit *"Remove ratatui & snapshots"*); this note is the record.

### 3 — crossterm feature floor

`default-features = false, features = ["windows", "events"]`, dropping `bracketed-paste` and `derive-more`.
This is the floor, not a step towards one — see the dependency audit under [Stuck costs](#stuck-costs).

### 4 — LEB128 gap-delta corpus

The words are sorted, so consecutive base-26 codes differ by small gaps.
Storing LEB128 gaps instead of 3 fixed bytes per word, and decoding them by streaming at runtime, roughly halves the data again.

### 5 — Byte-level render output

The render path builds raw byte buffers rather than `String` + `from_utf8_lossy`, dropping the UTF-8 validation and conversion machinery.

### 6 — `main` no longer returns `io::Result`

Returning a `Result` from `main` links the `Termination` implementation, which formats the error via `Debug`.
`main` now handles every `Result` itself.

### 7 — Streaming varint lookup

Lookups walk the compressed stream directly instead of decoding it once into a `Vec` behind a `OnceLock` and running `binary_search` — that removed the `OnceLock`, the allocation and the search.

### 8 — Arithmetic-coded corpus

Both lists merged into one adaptive-model arithmetic-coded stream (`src/codec.rs`, encoder in `build/`), replacing the varint scheme; the source was split into `build/` modules in the same pass.
Small on the binary, but it is the foundation the data work in [#19](#19--position-conditioned-context) and [#21](#21--conditioned-colour-bit) builds on.

The encoder (build) and decoder (game) share `codec.rs`'s model *math*, so they agree bit-for-bit — the `corpus_round_trips` test guards this.
The halves that only one end runs live apart: the range decoder and the reverse lookups in `src/codec.rs`, the range encoder and the forward lookups in `build/codec_enc.rs`, which the binary never compiles.
They no longer share its *storage*; see [#18](#18--asymmetric-decoder-fixed-array-model-storage).

### 9 — `/DEBUG:NONE` (PE) / `--build-id=none` (ELF)

`strip = true` removes symbols, but the MSVC linker still emits an `IMAGE_DEBUG_DIRECTORY` — a CodeView entry pointing at `wordle_tui.pdb` plus a REPRO entry, ~84 B — and leaks the `.pdb` path as a string in `.rdata`.
`-Clink-arg=/DEBUG:NONE` leaves only the 28 B REPRO stub.
On ELF the equivalent is `-Wl,--build-id=none`, which drops `.note.gnu.build-id` (−112 B versus the linker default).
Both live in `[target.*] rustflags` and apply to every profile.

The Windows figure is −56 B of `VirtualSize`; on disk the same change once showed −512 B because it crossed a section-alignment boundary — exactly the padding effect described in [How sizes are measured](#how-sizes-are-measured).

**Gotcha:** setting `RUSTFLAGS` in the environment *overrides* the config's `[target.*] rustflags` rather than merging, so these flags have to be repeated on any command line that sets `RUSTFLAGS`.
BUILD.md's commands already do.

### 10 — `build-std` without std's `backtrace`

`cargo bloat` showed the largest non-game mass was the panic **backtrace/demangle** machinery: `rustc_demangle` (~8 KB), `std::sys::backtrace` (~6 KB), `backtrace_rs::symbolize` and the `dbghelp` walk, plus satellites (`getenv` for `RUST_BACKTRACE`, `path::components`, `slice_error_fail`) — **57 symbols**, all dead weight, since nothing in the game prints a backtrace.

It cannot be dropped from source.
std's panic runtime keeps `match HOOK { Hook::Default => default_hook(..) }` compiled, and LTO cannot prove the hook is never `Default`, so `default_hook` and everything it calls stay reachable whatever custom hook we install — confirmed by measurement: replacing the hook left all 57 symbols linked and freed only ~432 B of plumbing.

The lever is at the std level, in `.cargo/config.toml` (the `[unstable]` table is ignored by stable cargo, so the stable build is untouched):

```toml
[unstable]
build-std = ["std", "panic_abort"]
build-std-features = []   # none of std's defaults -> no `backtrace`
```

Result: 57 → 4 backtrace symbols (~45 B of inert stubs), `.exe` 156.7 KB → 117.8 KB.
Crucially it stays a **safe** build — the panic hook still runs and restores the terminal; a bug-panic just aborts without a symbolized trace.
This is the recommended no-compromise profile.

The Linux gain is far larger because std's backtrace there carries its own DWARF stack (gimli, addr2line, object, miniz_oxide) instead of calling into a system library.

`build-std` requires an explicit `--target`; the config deliberately pins none in `[build]`, so a plain `cargo build` stays host-native and the project still builds anywhere.
Note that build-std rebuilds the Rust sysroot but **not** the musl C runtime: the CRT objects come from the *prebuilt* target, which must therefore be installed on the **nightly** toolchain as well, or linking fails with `cannot find rcrt1.o` / `-lunwind` (`musl-tools` is not what fixes it).
BUILD.md spells out the `rustup target add` step.

### 11 — `-Cpanic=immediate-abort`

Every panic — bounds check, division, slice, `unreachable!` — compiles to a bare abort, so every call site loses its location struct, argument setup and message string program-wide, std included.
That is why it is worth ~30 KB rather than the ~3 KB of message formatting alone.

The catch: immediate-abort bypasses the hook mechanism, so `restore_terminal` never runs and a bug-panic leaves the terminal dirty.
The command also passes `--cfg immediate_abort`, which gates the then-dead hook out of `main.rs` (declared via `cargo::rustc-check-cfg` in `build/main.rs`).
See [Panic handling](#panic-handling) for why that dirty-terminal case is theoretical.

ICF (identical-code folding) is folded into each profile's build command rather than being a profile of its own; it yields ~1–3 KB on both platforms.
On ELF it needs `lld` — `/OPT:ICF` has no default-linker equivalent, as GNU `bfd` does no folding at all — which is why BUILD.md's nightly Linux commands use the bundled `rust-lld` and the stable one the system `lld`.

### 12 — Raw ANSI output

`ui::render` and the terminal setup in `main.rs` no longer go through crossterm's `SetForegroundColor` / `MoveTo` / `EnterAlternateScreen` / `SetTitle` / `Clear` commands; they write the escape bytes directly.
Colours are precomputed `&'static [u8]` SGR constants stored in each `Cell`; alt-screen, title, cursor-hide and clear are literal CSI/OSC byte strings, byte-identical to what crossterm emits.
On Windows that reclaims two things:

- the **integer `fmt` machinery** the colour and cursor commands pulled in via `write!` (`Colored::fmt`, `pad_integral`, `fmt::write`) — `render` was the only integer-formatting site;
- the **WinAPI fallback** of every terminal and cursor command (`SetConsoleTitleW` with its UTF-16 `EncodeUtf16` path, alt-screen/cursor/clear through the console API).
  Each command compiled *both* an ANSI and a WinAPI path, selected at runtime by `supports_ansi()`, so both were linked.

Mouse capture **stays** on crossterm: on Windows `EnableMouseCapture` is WinAPI-only, because the console event source reads mouse input from the input buffer rather than from ANSI reports, so the `?1000h…` sequences would not work.
Since we no longer call `supports_ansi()` (which enabled VT as a side effect), `init_terminal` sets `ENABLE_VIRTUAL_TERMINAL_PROCESSING` itself, through a three-function `extern "system"` block (`GetStdHandle` / `GetConsoleMode` / `SetConsoleMode`) rather than `crossterm_winapi`'s `Handle` + `ConsoleMode` wrappers: those carry an `Arc` and two `Drop` flavours, and `Handle::current_out_handle()` reaches the console through `CreateFileW("CONOUT$")` with its UTF-16 path. Going direct is **-142 B** and doubles as the startup probe — the standard output handle fails `GetConsoleMode` when stdout is a file or a pipe, and an old console fails `SetConsoleMode`; in both cases no escape we emit would be honoured, so `init_terminal` refuses and `main` exits silently instead of painting the user's terminal with escape bytes.

Linux is byte-neutral (+15 B): crossterm already emits ANSI there and `supports_ansi` is Windows-only, so there was no second path to reclaim.

### 13 — Vendored crossterm: ioctl-only `size()`

crossterm's `terminal::size()` falls back to spawning `tput` when the `TIOCGWINSZ` ioctl fails, and its Unix event source calls `size()` on every resize, so that fallback is always reachable.
It anchored `std::process::Command`, a `BTreeMap<OsString, OsString>` environment copy, and their `Debug`/`fmt` subtree.
A vendored crossterm with an ioctl-only `size()` drops all of it; `nm` confirms `tput_value`, `Command` and `BTreeMap` are gone.
Windows is unaffected — its `size()` uses `GetConsoleScreenBufferInfo`.

The copy is wired in by a `[patch.crates-io]` table in `Cargo.toml`, so every build links it.
Only crossterm's lib target is built as a dependency, so the vendored copy is trimmed to `src/`, `Cargo.toml`, `LICENSE` and `README.md`.
Full rationale, wiring and the `Cargo.lock` caveat: [vendor/crossterm/LOCAL_PATCH.md](vendor/crossterm/LOCAL_PATCH.md).

### 14 — Decoder without per-word heap `Vec`

`decode_word` and `words::Corpus` allocated a `Vec` per decoded word; both now use fixed `[u8; WORD_LEN]` buffers.
The corpus walk runs per lookup, so this sat on the hot path.

### 15 — Vendored crossterm: single-threaded, `parking_lot` dropped

`parking_lot` was long listed as an unreclaimable cost held by `crossterm::event`/`terminal`.
It had exactly two live anchors, both global `Mutex`es in crossterm: `INTERNAL_EVENT_READER` (`event.rs`) and `TERMINAL_MODE_PRIOR_RAW_MODE` (`terminal/sys/unix.rs`, Linux only).
The app is single-threaded — one event-loop thread, no threads spawned anywhere — so the vendored copy replaces both with unsynchronized `UnsafeCell`s, and `parking_lot` + `parking_lot_core` drop out entirely on both platforms.
The two changes are not independent per platform: the event reader is what frees Windows, the raw-mode cell what frees Linux — see [LOCAL_PATCH.md](vendor/crossterm/LOCAL_PATCH.md) changes 2 and 3.

### 16 — `ui::render`: home once + CNL

`render` positioned every row with an absolute `MoveTo(0, y)` — `CSI <row+1>;1H` — which meant formatting `row+1` in base-10 for each row (a `buf[5]` divide-by-ten loop).
Rows are painted top-to-bottom and every row writes exactly `width` printable ASCII glyphs (SGR escapes do not move the cursor), so the cursor position after each row is deterministic: emit `CSI H` (home) once, then step down with `CSI E` (CNL, column 1 of the next line — what `crossterm::MoveToNextLine` emits) *between* rows only, never after the last, so it cannot scroll the alternate screen at the bottom.
That deletes the per-row decimal-formatting loop from `.text`.

Linux gains nothing (+8 B): the loop was already ICF-folded there, and the restructure crosses a `.text` function-alignment boundary.
Kept for the Windows win; rendered output is identical, only cursor moves changed.

### 17 — `codec::Model` without the parallel `total` Vec

The model kept `counts` (`[u16; 26]` per context) *and* a parallel `total: Vec<u32>` of one running total per context.
A context's total is exactly the sum of its counts — the invariant holds through every `update` (both `+= inc`) and every halving (`total` was recomputed from the halved counts) — so it is no longer stored: `update` sums the 26 counts to test the `LIMIT` trigger, and the whole `vec![0u32; n_ctx]` disappears, taking with it a distinct `from_elem::<u32>` monomorphization and the `Vec<u32>` drop path, both of which were compiled into the decoder because `Model::new` runs on the corpus walk.
The compressed stream is unchanged; the extra summing fits the codec's standing "trade CPU for size" stance.

### 18 — Asymmetric decoder: fixed-array model storage

`codec.rs` used to hold one `Model` struct used verbatim by both ends, which forced its storage to be `Vec`s: the **encoder** searches `order`/`inc`/`word_len` and genuinely needs runtime-sized tables, but the **decoder** only ever runs the single winning `ORDER`/`INC`/`WORD_LEN`, baked into `constants.rs`.
The shared struct was the only thing forcing an allocation on it — the same wall behind the rejected const-generic `ORDER`, since sizing `[[u16; 26]; 27^ORDER]` in a *shared* type would need `generic_const_exprs`.

`codec.rs` now exposes the range coder **and the model math as storage-agnostic free functions** (taking a `&[u16; 26]` count row, a `&[u16]` prefix slice); each end owns a thin wrapper that delegates to them — the encoder's `Vec`-backed `Model` (`build/encode.rs`), the decoder's array-backed `DecodeModel` (`src/words.rs`).
Single source of truth for the probabilities, so the two ends still agree bit-for-bit; only the container differs.

That deletes both `vec![[0u16; 26]; …]` and `vec![0u16; …]` from the decoder — two `from_elem` monomorphizations, the `RawVec` allocation and the drop/dealloc glue, all on the corpus walk (Windows: `.text` −576, `.pdata` −24, `.rdata` −8).
At `ORDER = 1` the `counts` array is 27·26·2 = 1404 B, cheap to zero on the stack per lookup; a `const` assertion refuses to compile if a future corpus pushed `ORDER` past 2 (≈1 MB, stack-busting — box it there instead).

Linux gains nothing (+24 B): it keeps a full allocator anyway, since crossterm's Unix input stack allocates, so dropping two `Vec`s frees almost nothing and the restructure crosses an `lld` ICF/alignment boundary.
Kept for the Windows win.

### 19 — Position-conditioned context

The adaptive character model conditioned only on the previous `ORDER` characters; position in the word was not part of the context, so all five slots shared one set of statistics.
Fixed-length words are strongly positional, so folding the position into the context index is a large, cheap win.
It is a searched knob (`use_pos` in `build/encode.rs`, baked into `constants.rs` as `USE_POS` alongside `ORDER`/`INC`) and a zero-byte channel, since the decoder const-folds it: `codec::ctx` multiplies the position in only when `USE_POS`, and `codec::n_ctx` sizes the table accordingly.
The build searches `{order 1..3} × {pos off/on} × {inc 1..32}` and keeps the smallest.

On this corpus the winner is **position + order-1** (27·5 = 135 contexts): blob 15,206 → 14,449 B (−757 B, 1.02 → 0.97 B/word).
Order-2 and order-3 lose either way — the table outruns the data.
The Windows total moves by −696 B: the −757 B of blob, less ~61 B of position arithmetic in `ctx`.
The decoder also gains a larger stack count table (135·26·2 = 7020 B, still cheap to zero per lookup; the `const` assertion still guards against a scheme too big for the stack).

Searching further knobs on top — a separate `inc` for the prefix and character models, a tunable halving `LIMIT` — buys only ~35 B more and overfits the corpus, so they were left out.

### 20 — Searched word ordering

The stored order used to be implicitly forward-lexicographic-ascending.
It is now searched: two booleans, `REVERSE_WORD` (store words reversed, sharing suffixes instead of prefixes) and `DESCENDING`, are tried in all four combinations and baked into `constants.rs` like `USE_POS`.
The decoder honours them through const-folded branches — the first differing character is bounded `[floor+1, 26)` ascending, `[0, floor)` descending, and `is_valid`/`pick_target` reverse the key when `REVERSE_WORD`.

Forward-ascending still wins on this corpus (blob 14,449 B; reversed-ascending 14,483, forward-descending 14,503, reversed-descending 14,562), so the baked constants are `false/false` and the decoder compiles byte-identically to before: a genuinely free addition, the unchosen branches being eliminated.

**The four schemes are not code-size-neutral, though**, measured by pinning each ordering (non-blob = total − blob): forward-asc **50,024**, forward-desc 50,066, reversed-asc 50,006, reversed-desc 50,167 — a 161 B spread.
Ascending's upper bound is the constant `26`, which `char_tot`/`char_find` fold away, while descending's is `prev[p]`, a runtime value that keeps those loops dynamic.
Worth remembering: the build minimises the *blob* as a proxy, and for this dimension the proxy is blind to ~140 B of decoder code, so on a future word list a blob-only pick could be off by that much.
Here it is moot — forward-ascending is smallest on both — but `best_model` would need a per-scheme code penalty if it ever mattered.

### 21 — Conditioned colour bit

The per-word colour bit (answer vs. valid-only) was coded by exact sampling without replacement: optimal *if the answer subset is structureless*, spending `log2 C(14853, 2339) ≈ 1166 B`.
It is not structureless — Wordle answers avoid plurals, so a word's **last letter** predicts its colour (words ending in `-s`: 0.9% answers, versus 21.9% otherwise).
Replacing SWOR with a small adaptive binary model conditioned on one build-searched letter (`USE_COLOR`/`COLOR_POS`) captures that: colour cost 1166 → ~1000 B.
The model is adaptive, so nothing is stored — its `[[u32; 2]; 26]` count table lives on the stack, not in the binary.

Which letter is used is searched, not assumed: `best_model` tries the shared model and each position, and only the last letter carries real signal (position 4: −167 B; positions 0–3: −10 to −20 B each).
Combining positions does not help — a product context over two letters over-sparsifies (`{first,last}` = 676 contexts, *worse* than last alone; all five = 26⁵ contexts seen once each, ≈1 bit/word) and a naive-Bayes mix tops out at ~11 B over last-alone for far more decoder code.

Blob 14,449 → 14,283 B (0.96 B/word, −166 B); the remaining ~10 B come from the adaptive decoder (`decode_color` plus one stack table) being *smaller* code than SWOR's `remaining` / `remaining_answers` bookkeeping and its two `Corpus` fields.
What is left in the colour partition is human-curation entropy no letter model predicts.

### 22 — `ui::center` via `saturating_sub`/`div_ceil`

`(outer - inner + 1) / 2` → `outer.saturating_sub(inner).div_ceil(2)`: cleaner *and* smaller.
It is the exception — most idiomatic cleanups measured byte-neutral (an `Ordering` import with `w.cmp(&target)`, `array::from_fn`, the named-constant colour palette in `ui.rs`, various renames), and several measured *larger*; see [Rejected experiments](#rejected-experiments).
Idiomatic ≠ smaller: measure each one.

### 23 — Panic hook: `exit(101)` instead of returning

If the hook returns, `panic = "abort"` calls `abort()`, which on Windows hands off to Windows Error Reporting — a ~1 s stall before the shell prompt returns.
`std::process::exit(101)` terminates first and skips WER (101 is Rust's conventional panic exit code).
It costs +80 B of `.text` and 0 in the `.exe`, and applies to the profiles that keep the hook at all.
See [Panic handling](#panic-handling).

### 24 — Vendored crossterm: no `io::Error` message payloads

The first find of **linker-map attribution**: building with `-Clink-arg=/MAP:…` is byte-neutral (verified: the two binaries differ only in the 6 timestamp bytes any relink changes) and lists every post-LTO/ICF symbol of the exact shipping binary with its address — sizes fall out of consecutive-address deltas, folded symbols share an address.
Unlike `cargo bloat` it cannot measure the wrong build, and folds are visible instead of misattributed.

The map showed ~6.6 KB anchored by `io::Error::new(kind, "message")` in crossterm: a `&str` payload monomorphizes `From<&str> for Box<dyn Error>`, and the boxed `StringError`'s `Debug` vtable reaches `str::escape_debug`, whose printable-class/grapheme tables cost ~3.4 KB of `.rdata` alone.
Four live sites patched (`ErrorKind` alone, no payload — nothing in the app reads error messages); details in [LOCAL_PATCH.md](vendor/crossterm/LOCAL_PATCH.md) change 4.
On Linux the same patch shrinks sections by ~430 B, but `.relro_padding` (page alignment, counted by `size -A`) grows back by almost exactly that: net −1 B across changes 24–27.

### 25 — Vendored crossterm: ASCII-only key case correction

Same map, next anchor: the Windows key parser's shift/capslock fix-up calls the full-unicode `to_lowercase()`/`to_uppercase()`, anchoring the case-mapping tables — ~5 KB of `.rdata` + ~1 KB of `.text`, the single largest non-blob `.rdata` cost.
Patched to `to_ascii_*` ([LOCAL_PATCH.md](vendor/crossterm/LOCAL_PATCH.md) change 5); only non-ASCII keyboard input behaves differently (passes through un-recased), unobservable in a game that accepts `is_ascii_alphabetic` only.

### 26 — Vendored crossterm: millisecond poll clock

`PollTimeout` stamped `Instant`, which on Windows is `QueryPerformanceCounter` behind a `Once`-cached frequency plus 128-bit `Duration` arithmetic — for a value that ends up as the millisecond argument of `WaitForMultipleObjects`.
`GetTickCount64` (u64 milliseconds) replaces it on Windows; Unix keeps `Instant` ([LOCAL_PATCH.md](vendor/crossterm/LOCAL_PATCH.md) change 6).

### 27 — `App` message in a fixed buffer

The footer message was the binary's only `String` writer; it is now a fixed `[u8; 48]` + length (`len == 0` = no message), dropping the `String::push`/`push_str` monomorphizations.
Only −176 B: the map shows most of the `String`/`RawVec` machinery is *shared* with the stuck OS-error path below (`error_string` builds a `String`), so the app's usage was riding on already-paid code.
Kept: the code is no worse, and it decouples the app from `String` should the stuck path ever fall.

### 28 — Vendored crossterm: event set reduced to the game's inputs

`try_read` was the largest single `.text` symbol (2,923 B, plus its 552 B `.rdata` jump table).
The Windows key/mouse parsers now deliver only what the game consumes — key presses (with the `ToUnicodeEx` layout path kept, so non-QWERTY layouts still type), left-button-down clicks, resizes; releases, Alt-codes, surrogate pairing, function/navigation `KeyCode`s, focus events and the other six mouse kinds are no longer parsed into events.
`try_read` fell to 1,485 B and the jump table disappeared; details and the behaviour costs (all invisible here) in [LOCAL_PATCH.md](vendor/crossterm/LOCAL_PATCH.md) change 7.
Windows-only files — the Linux binary is byte-identical.

### 29 — `ui::draw_board`/`draw_keyboard` outlined

`build_grid` had both drawers inlined (2,471 B as one symbol).
`#[inline(never)]` on the two splits it 2,471 → 1,388 + the two bodies, netting −56 B — less register pressure at the call sites beats the two extra calls.
The same trick measured *larger* everywhere else it was tried (see below): outlining is not a rule, it is one more knob to measure per site.

### 30 — Vendored crossterm: the same event-set reduction on Unix

[#28](#28--vendored-crossterm-event-set-reduced-to-the-games-inputs) applied to the other platform, where `parse_event` — an incremental ANSI decoder, not a WinAPI record switch — was the binary's largest symbol at 4,851 B.
The same principle (deliver only presses, left-clicks and resizes) is worth **−8,184 B** here versus −1,716 B on Windows, because the dropped parsers also anchored `str::parse`'s `FromStr` machinery, `str::split` and `core::fmt` integer padding, and because `parse_event`'s Alt+key branch made it *recursive*.
Full breakdown, including the Unix-only `char::is_uppercase` → `is_ascii_uppercase` fix, in [LOCAL_PATCH.md](vendor/crossterm/LOCAL_PATCH.md) change 7.

The subtlety is **framing**, and it has no Windows analogue: the caller feeds bytes one at a time, so a sequence being dropped must still be consumed to its end (`Ok(None)` until a CSI final byte) before it is refused, or its tail resurfaces as ordinary keystrokes — an arrow key would type letters into the board.
That, and every other behaviour claim above, is checked by driving the built binary through a real PTY (see below) rather than by reading the diff.

**Behaviour is verified against a control binary, not asserted.**
The parser changes are not covered by `cargo test` — no test drives a terminal — so `tools/pty_test.sh` runs the built binary under a PTY (`script -qec`, 80×30) and asserts on the bytes it renders: typing and submitting a word, an invalid word, backspace, Ctrl+C and Esc quitting, arrows/F-keys leaving the draft untouched, and clicks on the ENTER button and on a letter key in **both** mouse encodings driving the game.

```bash
bash tools/pty_test.sh target/x86_64-unknown-linux-gnu/release/wordle_tui
```

Run it against a **control** binary built from the previous commit too — that is what makes the output readable.
On a first attempt three checks failed on *both* binaries: the pty line discipline was eating CR (`ICRNL`) and `0x03` (`ISIG`) before the app switched to raw mode — an artefact of the harness, not a regression, fixed by delaying the input.
A patch is a regression only when the two runs differ.


### 31 — VT enabling doubles as a startup probe

`enable_vt` ignored its result and reached the console through `crossterm_winapi`'s `Handle`/`ConsoleMode` (an `Arc`, two `Drop` flavours, and `CreateFileW("CONOUT$")` with its UTF-16 path).
It is now a three-function `extern "system"` block on `GetStdHandle`/`GetConsoleMode`/`SetConsoleMode`, and its `bool` gates `init_terminal`.
The handle choice is the point: `CONOUT$` succeeds even when stdout is redirected to a file or a pipe, `GetStdHandle` does not — so the same call that enables ANSI also answers "is there a screen to draw on?".
Failing it, `main` exits silently instead of painting escape bytes into a terminal that will not honour them, or running invisibly against a redirected stdout. Going direct pays for itself: **−142 B**.
Unix has no equivalent probe here; a redirected stdout still runs blind there, since `enable_raw_mode` goes through `/dev/tty`.

### 32 — Vendored crossterm: blocking event source, no queue

`poll` + `read` is a pump: `poll` pulls an event from the source, queues it, and reports that one exists; `read` takes it back out. The `VecDeque`, the `Vec` of filtered-out events, the `Filter` trait, `InternalEventReader` and the `Box<dyn EventSource>` all exist only to carry the event between those two calls, and `EventSource::try_read` already returns `Result<Option<InternalEvent>>` — one call, one event.
`event::next()` blocks on the source held in a static of its concrete type and returns the event ([LOCAL_PATCH.md](vendor/crossterm/LOCAL_PATCH.md) change 8).
The 200 ms poll interval went with it: `run` renders on change and only an event changes anything, so the wakeups did nothing. The loop now blocks, and the process is idle between keystrokes.
Crossterm's own out-of-line symbols drop from 3,235 B to 59 B; the rest is inlined into `run`.

Reverting `event/read.rs` to the upstream file with change 32 in place leaves the binary byte-identical, so the fork carries no queue patch at all.


### 33 — Windows raw mode is already set by mouse capture

`init_terminal` called `enable_raw_mode()` and then, three statements later, `EnableMouseCapture`.
On Windows both write the console **input** mode, and the second one *assigns* it rather than OR-ing into it: `set_mode(ENABLE_MOUSE_INPUT | ENABLE_WINDOW_INPUT | ENABLE_EXTENDED_FLAGS)` clears every other bit, raw mode's `ENABLE_LINE_INPUT`/`ENABLE_ECHO_INPUT`/`ENABLE_PROCESSED_INPUT` included.
So `enable_raw_mode()`'s write never survived: mouse capture is what puts the console in raw mode. The same holds in reverse on the way out — `DisableMouseCapture` restores the mode captured before capture, overwriting whatever `disable_raw_mode()` just wrote.

Both calls are now `#[cfg(unix)]`. Unix raw mode is termios, which the mouse sequences do not touch, so there the pair is load-bearing.

Verified by reading the input mode out of a running instance: `0x98` with and without the calls — `PROCESSED`, `LINE` and `ECHO` all clear.

### 34 — Vendored crossterm: key characters without the keyboard layout

A key press whose `u_char` is a control code carries no character, so crossterm reconstructs one from the active keyboard layout: `GetForegroundWindow` → `GetWindowThreadProcessId` → `GetKeyboardLayout`, `ToUnicodeEx` against a 256-byte key-state buffer, a UTF-16 decode, a case correction.
This app reads one key from that range, `Ctrl+C`, and control codes `0x01..=0x1a` *are* `Ctrl`+the *n*-th Latin letter by definition — the layout does not enter into it ([LOCAL_PATCH.md](vendor/crossterm/LOCAL_PATCH.md) change 10).
Plain letters never took that path; they arrive with their character already in `u_char`.
Removes the binary's only `user32` imports.

### 35 — Vendored crossterm: blocking console read, no poll

`ReadConsoleInputW` blocks until a record is available. Crossterm still wraps it in `WaitForMultipleObjects` + `GetNumberOfConsoleInputEvents`, driven by a `PollTimeout`, for the sole purpose of giving up early — which [#32](#32--vendored-crossterm-blocking-event-source-no-queue) established this app never wants.
`event::next` now calls a blocking read directly ([LOCAL_PATCH.md](vendor/crossterm/LOCAL_PATCH.md) change 9). Off with it come `WinApiPoll`, `PollTimeout` and its `GetTickCount64` clock, three `kernel32` imports, and a `CreateFileW("CONIN$")` + `Arc` allocation + `CloseHandle` **per event**.

### 36 — Vendored crossterm: console geometry without `CONOUT$`

Two sites opened a private handle on the console screen buffer to read its window rectangle: the mouse parser, on every event, to convert an absolute y into a window-relative one; and `terminal::size()`.
The app runs in the alternate screen, whose buffer is exactly window-sized, so the y correction is the identity and the parser's call goes entirely; `size()` reads the same rectangle off the standard output handle the process already owns ([LOCAL_PATCH.md](vendor/crossterm/LOCAL_PATCH.md) change 11).
This unlinks `ScreenBuffer`, `Handle::current_out_handle` and their `Arc`/`Drop` glue.

Changes 33–36 were validated against a control binary built without them, driven through a real console (`WriteConsoleInputW` in, `ReadConsoleOutputCharacterW` out): typing, `Enter`, `Backspace`, an inert `Tab`, a click on an on-screen key and `Ctrl+C` give byte-identical screens on both. Linux is untouched by all four — same binary, and `tools/pty_test.sh` passes.


### 37 — `#![no_main]`: the process starts without Rust's runtime

An empty `fn main() {}` on this profile is **19,456 B**. Nothing in it is the program: it is `std::rt`'s entry path, `lang_start`, which wraps the real `main` in `rt::init` and `rt::cleanup` — and on MSVC the C runtime's startup underneath that.

Neither does anything this game needs. `rt::init` installs a stack-guard page for the main thread, records thread identity, and on Linux stores the `argc`/`argv` that back `env::args`; `rt::cleanup` flushes stdout at exit. There is no recursion in the tree and no live `thread::spawn` (crossterm's is behind the disabled `event-stream` feature), so nothing can overflow the stack or ask for a thread name; no code reads an argument; and `restore_terminal` already flushes.

`#![no_main]` replaces that entry, and the two platforms sit at different depths:

- **Unix** — `main` is defined as `extern "C"`, so glibc's `_start` and `__libc_start_main` still run and call it. Only `lang_start` is skipped. **−4,130 B.**
- **Windows** — `mainCRTStartup` *is* the MSVC CRT's entry, and defining it replaces the CRT startup outright: `__scrt_common_main_seh`, `__isa_available_init`, the security-cookie init and the `onexit` tables all go, and `ExitProcess` becomes ours to call. **−7,528 B**, roughly 6 KB more than the Unix-depth change would have bought here.

The Windows depth has three consequences, all checked rather than assumed:

- The linker reads the entry point and the subsystem off the names `main`/`WinMain`, and neither exists now, so `/ENTRY:mainCRTStartup` and `/SUBSYSTEM:CONSOLE` must be stated, and `/defaultlib:vcruntime` must supply the `memcpy`/`memmove` the CRT startup object would have brought in. These are emitted by `build/main.rs` as `rustc-link-arg-bins`, not put in `.cargo/config.toml`: rustflags reach every crate built for the triple and `/ENTRY` breaks the proc-macro DLLs' link. As a build-script directive it also survives an env `RUSTFLAGS` that overrides the `[target]` block, so no build command in BUILD.md changes.
- `memcpy` from `vcruntime` dispatches on `__isa_available`, which the CRT startup would have set. Left at 0 it takes the baseline SSE2 path — correct on any x86-64, marginally slower on a machine with AVX. The corpus decode, the heaviest user of it, is one pass at startup.
- The link warns LNK4210, that `.CRT`'s static initializers and terminators may go unrun. Here the section is empty — the map shows only `tlssup.obj`'s bounds markers `__xl_a` and `__xl_z` with no entry between them, so no C/C++ initializer and no TLS callback exists to miss. The warning is suppressed with `/IGNORE:4210` at that one site; the comment there names the condition that would make it real again (a `thread_local!` with a destructor reaching the binary).

`no_main` and both entry points are `cfg(not(test))`: a `cargo test` build of a bin target needs the harness's own `main`, and suppressing it makes the test link fail. The link args stay unconditional — with the CRT startup back in the picture, `/ENTRY:mainCRTStartup` simply names the CRT's own.

Verified on both platforms against a control binary: Windows through the console harness (typing, `Enter`, `Backspace`, an inert `Tab`, a click on an on-screen key, `Ctrl+C`) with byte-identical screens, plus the redirected-stdout and no-console startups still exiting 0 without writing; Linux through `tools/pty_test.sh`, 11/11. Both the stable and ship profiles build on both platforms.


## Rejected experiments

Measured, then reverted.
Recorded so they are not retried.

| Experiment | Measured | Why it was dropped |
|------------|---------:|--------------------|
| `ORDER` as a const generic on `Model` | −48 B `.text`, **0** in the `.exe` | The compiler already const-propagates `ORDER` (single call site, value baked into `constants.rs`); the const generic recovers only the residue, which hides in padding. Not worth threading a generic through `Model`/`decode_word`/`encode_*` and the build's search dispatch. |
| Generating the decoder as source in the build | not measured | Nothing in the decoder's *shape* varies with the word data — only `ORDER`/`INC`/`WORD_LEN`, already compile-time constants. Would buy at most what const generics buy (≈0) while sacrificing the single shared `codec.rs` that makes encode/decode provably agree. |
| Drop `Model.pref_total`, recompute `sum(pref)` | **+16 B** | The invariant holds, but `pref` is summed in two hot spots (`pref_tot`, `pref_update`). The cached `u32` stays — unlike the parallel `total` `Vec` ([#17](#17--codecmodel-without-the-parallel-total-vec)), which was a whole monomorphization. |
| `Model.counts`/`pref`: `Vec` → `Box<[T]>` | **+156 B** | `into_boxed_slice()` pulls in the shrink/realloc path. |
| `App::submit`: array copy → `guess == &self.target` | **+48 B** | Cleaner, but the comparison costs more than the copy. |
| `ui::gaps`: indexed loop → `iter_mut().take(d).enumerate()` | **+16 B** | Clippy's suggestion; the `.take` iterator machinery costs more. Kept under `#[allow(needless_range_loop)]`. |
| Raw-handle stdout on Windows (write to the stdout `HANDLE` via `File`/`WriteFile` to skip std's console UTF-16 path) | −314 B | The `File`/`WriteFile` path pulls back most of what the UTF-16 path freed. **The experiment also measured the wrong thing**: `File::write` calls `io::Error::last_os_error()` just as std's `Stdout::write` does, so it kept the anchor that makes the error machinery unremovable (see [Stuck costs](#stuck-costs)). A retry would have to drop `io::Error`-returning writes entirely. |
| `run()`: key and click paths merged through one `Option<KeyEvent>` | **+196 B** | The intermediate `Option<KeyEvent>` (a large crossterm struct) spills more than the duplicated `handle_key` call sites cost. |
| `run()`: first frame from a `Grid::empty()` instead of a thrown-away `build_grid` call | **+40 B** | Two identical call sites compile smaller than one call site plus an empty-grid constructor. |
| `draw_footer` outlined | **+60 B** | Unlike [#29](#29--uidraw_boarddraw_keyboard-outlined): too small to win back its call overhead. |
| `App::submit` outlined | **+60 B** | Same. |

## Stuck costs

- **Stable profile: the panic runtime.**
  Backtrace machinery (~12 KB), `io::error` formatting (~3 KB) and `env` (~2 KB) are linked unavoidably; the `backtrace` lever ([#10](#10--build-std-without-stds-backtrace)) exists only on nightly.
- **Linux `.eh_frame` + `.eh_frame_hdr` (11.9 KB).**
  Unwind tables from the *prebuilt* std, dead under `panic=abort` and immediate-abort since nothing unwinds, but this profile links prebuilt std, so no build flag drops them.
  Reclaimable via build-std (`-Cforce-unwind-tables=no`) or a post-link `objcopy --remove-section .eh_frame --remove-section .eh_frame_hdr` (measured on a 118,608 B binary: 118,608 → 99,956, −18,652 B) — the latter leaves a dangling `PT_GNU_EH_FRAME` program header, so validate at runtime before relying on it.
  About half the Windows/Linux gap; most of the rest is the Unix input stack (mio, signal-hook, the signal-driven resize path), which Windows does not have.
- **Dependency features: nothing left to trim.**
  crossterm is at its floor ([#3](#3--crossterm-feature-floor)) — `events` is required for input, `windows` for raw mode/console, and crossterm has no `no_std` mode.
  On Windows, `events` pulls `mio`/`signal-hook`/`signal-hook-mio` only under `cfg(unix)`, so they are not compiled.
  And cargo *unions* features across the graph, so from our manifest we can only ever *add* a transitive dependency's features, never remove one crossterm's own edge requests — which is why the only lever that works is `[patch]`-ing crossterm itself ([#13](#13--vendored-crossterm-ioctl-only-size), [#15](#15--vendored-crossterm-single-threaded-parking_lot-dropped)).
- **The OS-error subtree (~3.5 KB `.text`), anchored by `io::Error::last_os_error()` alone.**
  `core::io::Error` cannot call the OS itself, so `std::io::Error::from_raw_os_error` hands it a `&'static OsFunctions` — three function pointers, `format_os_error`, `decode_error_kind`, `is_interrupted` — which `set_functions` stores in an `AtomicPtr`. Storing their address is what makes all three unremovable, so **one `last_os_error()` anywhere** keeps the errno→`ErrorKind` table (`decode_error_kind`, 640 B) *and* the message formatter (`error_string` → `FormatMessageW`, plus `String`/`alloc::fmt::format` and `<i32 as Display>::fmt` for the `os error N` fallback), even though nothing in this program ever reads an error message.
  Reading the pointer is free; only creating an OS error anchors it. Two call sites do, and neither is ours: std's `Stdout::write` on every `write_all`, and `crossterm_winapi`'s `result()`/`handle_result()`.
  Measured with a probe on this profile: `#![no_main]` plus a single `io::stdout().write_all(b"hi")` and no other code pulls the whole cluster. Freeing it means giving up `io::stdout()` *and* `crossterm_winapi` — our own `WriteConsoleW` writer returning `io::Error::from(ErrorKind)`, and direct FFI for the console read. That would also shed std's stdout stack (`LineWriter`, `BufWriter`, `ReentrantLock`, `OnceLock`, the `EncodeUtf16 → Vec<u16>` console conversion), another ~2 KB. Noted, not done.
- Everything else in `.text` is either ours (`main`, `ui::build_grid`, `app::submit`, `words::decode_word`) or genuinely-used std and crossterm — on Linux, notably the Unix input stack (`parse_event`, signal-hook, mio).

## Panic handling

### The hook

`main` installs a minimal hook that restores the terminal and then exits.
It deliberately does **not delegate to the default hook**: printing the panic message would pull in `io::stderr` and the `writeln!` machinery that the default path does not otherwise share, and every message-printing variant measured *larger* (delegating to `default_hook`: +416 B; `writeln!("{info}")`: more).
On a bug-panic, the restored screen is what matters, not the text.
For why it exits rather than returns, see [#23](#23--panic-hook-exit101-instead-of-returning).

### Where panics come from

- **Explicit (ours):** one on the runtime path — `unreachable!(..)` in `pick_target`.
  There is no `unwrap`/`expect` at runtime; `main` handles every `Result` with `if let Ok`.
- **Implicit (~99%):** bounds checks on every index, the range-coder divisions (`range /= tot`, `div_round`), slice ranges.

Removing our explicit panics would save almost nothing: the implicit bounds checks keep `panic_bounds_check` → the panic runtime → the message formatting (`panic_with_hook`, the still reachable `default_hook`, `core::fmt`) all linked.
The lever that works is [#11](#11---cpanicimmediate-abort), which removes the call sites themselves.

### immediate-abort safety (manual audit)

immediate-abort's cost is a dirty terminal on panic — which only bites if a panic is actually *reachable*.
It is not.
Every runtime panic site was traced by hand.
The release profile sets `overflow-checks = false`, so integer overflow and underflow **wrap** and are not panics; the families left are out-of-bounds index/slice, divide-by-zero, `copy_from_slice` length mismatch, and explicit panics.
**Verdict: no panic is reachable while the embedded `corpus.bin` is intact**, so the dirty-terminal case requires a corrupted binary — at which point terminal state is moot.

Locally guaranteed, with no external assumption:

- **`Grid` rendering (`ui.rs`)** — every write and read goes through `set`/`text`/`hit_rect`/`hit_test`, all guarded by `px < self.w && py < self.h`.
  Off-screen access is a silent no-op, so the renderer cannot panic at any terminal size (even a `w == 0` underflow wraps to a huge index the bound rejects).
- **`app.rs` indexing** — `history[input_idx]` is safe because `input_idx ∈ 0..=5` while `Playing` (`submit` flips to `Lost` as soon as `input_idx >= 6`) and every draft/typing path runs only in `Playing`; `keyboard_letter_states` slices `history[..input_idx]` with `input_idx <= 6 = history.len()`; `type_letter`/`backspace` are guarded on `input_len`; `copy_from_slice` copies two `[u8; 5]`; the lost-message pushes the target's uppercased bytes as `char`s (all ASCII, infallible).
- **`ui.rs` layout math** — the sole runtime division (`div_round`) is called only from `gaps` with divisor `d = n - 1 >= 1` (guarded `n <= 1`); `gaps`/`stack_sizes` run with `n <= SECTION_COUNT = 4 < MAX_GUESSES`, so their fixed arrays never overflow; `col_budget` steps `cell` down from an odd `full` and stops at 1; `saturating_sub`/`div_ceil` guard every possible underflow.
- **`game::check`** — bounds `0..WORD_LEN` over the `[u8; 5]` arrays `submit` passes.
- **Range-coder divisions** (`decode_freq`: `range /= tot`, then `code / range`) — `range >= TOP = 2^24` is held by the renorm loops and every `tot <= WORD_COUNT = 14853 < 2^24`, so `range / tot >= 1` and `code / range` can never divide by zero.

Two sites are **not** locally guarded; both reduce to the same assumption — that `corpus.bin` is exactly the encoder's output, fixed at build time and checked by the `corpus_round_trips` test.
Both carry a source comment pointing here:

1. **`unreachable!` in `pick_target` (`words.rs`)** — reached only if the stream holds fewer than `ANSWER_COUNT` colour-A words.
   The encoder emits exactly `ANSWER_COUNT`.
2. **Divide-by-zero in `decode_freq` via `tot == 0`** — the only zero total is `char_tot(ctx, lo)` with `lo == 26` (empty sum), which needs a word whose first differing character exceeds `'z'`.
   The sort proves `w[p] > prev[p]` with `w[p] <= 25`, so `prev[p] <= 24` ⟹ `lo <= 25` ⟹ at least the `s = 25` term ⟹ `char_tot >= 1`.
   The colour decode feeds `decode_freq(f0 + f1)` with `f0, f1 >= 1` (add-one smoothing), so its total is always `>= 2`.

No runtime `assert!` locks these: an assert would create panic sites and grow the binary — the opposite of the goal.
The invariant is enforced where it belongs, at build time.
