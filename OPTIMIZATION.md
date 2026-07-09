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

**Goal (308,224) beaten at step 2** — mostly by removing ratatui (which dragged in kasuari,
hashbrown, lru, compact_str, unicode-*, parking_lot, …) and rendering directly through
crossterm. The ratatui cassowary layout algorithm was reimplemented by hand; its exact
integer rounding is reproduced by the unified spacer model
`spacer[i] = round((i+1)·E/g) − round(i·E/g)`.

### Visual fidelity

Before touching rendering, reference snapshots of the original ratatui output were generated
via `TestBackend`. The crossterm renderer is then checked **byte-for-byte** against them across
15 scenarios (vertical/horizontal, playing/won/lost, too-small, even/odd widths) — glyphs and
colors. The ratatui oracle is kept, isolated behind the `ratatui-ref` feature (zero impact on
the release binary).

The snapshot `.txt` files are generated locally and **not committed**. Regenerate them with:

```
cargo test --release --features ratatui-ref write_reference_snapshots -- --ignored
```

The `crossterm_matches_reference` test then verifies fidelity; it skips gracefully when the
`snapshots/` directory is absent (e.g. a fresh clone).

### Data note

valid.bin = 17,079 bytes for 12,514 words (~1.37 B/word) — close to the information-theoretic
limit (~17.7 KB). Compression is essentially optimal; nothing more to shave on the data side.

The remainder (std, essentially incompressible on stable): panic/backtrace machinery (~12 KB),
io::error fmt (3 KB), env (2 KB) — unavoidably linked by the panic runtime on stable std.

## build-std levers (nightly, optional — configured in `.cargo/config.toml`)

Recompiling std from source with our profile (opt-level=z, LTO). These are NOT source changes;
the program stays identical. Requires `rustup toolchain install nightly --component rust-src`.
Everything lives in `.cargo/config.toml` (the `[unstable]` table is ignored by stable cargo,
so the stable build is untouched):

| Variant | Command | Size | Behavior |
|---------|---------|------|----------|
| Stable (default) | `cargo build --release` | **214,528** | 100% preserved, no prerequisites |
| build-std | `cargo +nightly build --release` | **157,696** | **identical** (panic hook restores the terminal) |
| + immediate-abort | `RUSTFLAGS="-Zunstable-options -Cpanic=immediate-abort -Clink-arg=/OPT:ICF" cargo +nightly build --release` | **124,928** | panic → immediate abort: the terminal is no longer restored on a panic (edge case; game unchanged) |

Recommended no-compromise build: **build-std (157,696)** — functionally the same binary, just a
smaller std. Binary lands in `target/x86_64-pc-windows-msvc/release/`.

## Summary

- Baseline: 396,288
- Stable optimized (default, no prerequisites): **214,528** (−45.9%)
- build-std nightly, behavior identical: **157,696** (−60.2%)
- build-std + immediate-abort: **124,928** (−68.5%)
