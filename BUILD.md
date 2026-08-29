# Building

Two profiles, each with **one command per platform that produces the smallest binary for that profile** (link-time identical-code folding is folded in wherever the toolchain allows):

- **Stable** — no prerequisites: whatever toolchain you already have, no nightly, no `rust-src`, full std.
- **ship** — the shipping profile, everything on: pinned nightly, std recompiled without its backtrace machinery, every panic compiled to a bare abort.

## One command for the `ship` profile

`./wtui-ship.sh` runs the shipping build with the toolchain, target and flags that fit the machine it is on — the same command as below, with the prerequisites installed on demand:

```bash
./wtui-ship.sh                                    # build for the host
./wtui-ship.sh run                                # build it and run the game
./wtui-ship.sh --target x86_64-unknown-linux-musl
./wtui-ship.sh -n                                 # print the cargo command, run nothing
```

It installs the pinned nightly **with `rust-src`** and adds a missing `--target`, and forwards anything it does not recognise to cargo. `./wtui-ship.sh --help` for the rest.

The flags reach cargo through `--config target.<triple>.rustflags=[…]`, which **merges** with the `[target]` blocks of `.cargo/config.toml` — the reason the script never repeats `/OPT:ICF` & co. while the hand-typed `RUSTFLAGS` lines below have to.

There is no script for the **stable** profile, and no flag to ask `wtui-ship.sh` for it: stable *is* a plain `cargo build --release`, which is the whole point of it.

The rest of this file spells out both, platform by platform — the `ship` lines are what the script runs.

---

The `ship` profile uses a **pinned** nightly (`nightly-2026-08-25`, declared in `wtui-ship.sh`, which `tools/validate.sh` and CI both build through): binary sizes are only comparable at equal rustc, so the measured numbers in OPTIMIZATION.md are tied to this toolchain.
Install it once:

```
rustup toolchain install nightly-2026-08-25 --profile minimal --component rust-src
```

To bump the pin, change `NIGHTLY` in `wtui-ship.sh` and the version quoted here, then re-measure the reference totals.

### Entry point

`src/main.rs` is `#![no_main]` and defines the process entry itself — `main` as `extern "C"` on Unix, `mainCRTStartup` on MSVC — so Rust's `lang_start` (and, on Windows, the CRT startup) never runs. See OPTIMIZATION.md change 37 for what that buys and what it costs.
No build command changes because of it: the three MSVC link args it needs come from `build/main.rs` as `rustc-link-arg-bins`, which applies to binaries only and is not overridden by an env `RUSTFLAGS`.

### Vendored crossterm

Every profile links the in-tree crossterm from `vendor/crossterm/`, patched for size.
See `vendor/crossterm/LOCAL_PATCH.md` for more information.

---

## Windows (PowerShell, MSVC)

The MSVC link optimizations (`/OPT:ICF`, `/DEBUG:NONE`) are already in `.cargo/config.toml`, so they apply to every profile automatically — no extra flags below.
Output: stable → `target\release\wordle_tui.exe`; ship → `target\x86_64-pc-windows-msvc\release\wordle_tui.exe`.

**Stable:**

```powershell
cargo build --release
```

**ship:**

```powershell
$env:RUSTFLAGS = '-Zunstable-options -Cpanic=immediate-abort --cfg immediate_abort -Clink-arg=/OPT:ICF -Clink-arg=/DEBUG:NONE -Clink-arg=/MAP:target/wordle_tui.map'; cargo +nightly-2026-08-25 build --release --target x86_64-pc-windows-msvc; Remove-Item Env:RUSTFLAGS
```

> `RUSTFLAGS` **overrides** the `[target]` block (it does not merge), so `/OPT:ICF` and `/DEBUG:NONE` are repeated in the ship line.
> `/MAP:…` makes the link also emit its symbol map for `cargo run --example bloat` — it does not change a byte of the binary (see OPTIMIZATION.md "Symbol attribution").

---

## Linux — glibc (bash)

ICF has no default-linker equivalent, so it is passed on the command line: the **stable** profile uses the system `lld` (`-fuse-ld=lld`, needs the `lld` package); **ship** uses the toolchain's bundled `rust-lld` (`-Clinker-features=+lld`, no install).
Output: stable → `target/release/wordle_tui`; ship → `target/x86_64-unknown-linux-gnu/release/wordle_tui`.

**Stable:**

```bash
RUSTFLAGS="-Clink-arg=-fuse-ld=lld -Clink-arg=-Wl,--icf=all -Clink-arg=-Wl,--build-id=none" \
  cargo build --release --target x86_64-unknown-linux-gnu
```

**ship:**

```bash
RUSTFLAGS="-Zunstable-options -Cpanic=immediate-abort --cfg immediate_abort -Clinker-features=+lld -Clink-arg=-Wl,--icf=all -Clink-arg=-Wl,--build-id=none -Clink-arg=-Wl,-Map=target/wordle_tui-linux.map" \
  cargo +nightly-2026-08-25 build --release --target x86_64-unknown-linux-gnu
```

> `RUSTFLAGS` **overrides** the `[target]` block, so `--build-id=none` is repeated in every line.
> `-Wl,-Map=…` emits the symbol map for `cargo run --example bloat`, byte-neutral like the MSVC `/MAP`.
> Without a system `lld`, drop `-Clink-arg=-fuse-ld=lld -Clink-arg=-Wl,--icf=all` from the stable line (it falls back to `cargo build --release`, slightly larger).

---

## Linux — musl (bash, fully static)

Produces a static binary with no runtime dependencies.
Install the target once **on each toolchain you build with** — the `ship` profile needs it on nightly for its prebuilt CRT:

```bash
rustup target add x86_64-unknown-linux-musl                     # stable profile
rustup target add --toolchain nightly-2026-08-25 x86_64-unknown-linux-musl # ship
```

Output: `target/x86_64-unknown-linux-musl/release/wordle_tui`.

**Stable:**

```bash
RUSTFLAGS="-Clink-arg=-fuse-ld=lld -Clink-arg=-Wl,--icf=all -Clink-arg=-Wl,--build-id=none" \
  cargo build --release --target x86_64-unknown-linux-musl
```

**ship:**

```bash
RUSTFLAGS="-Zunstable-options -Cpanic=immediate-abort --cfg immediate_abort -Clinker-features=+lld -Clink-arg=-Wl,--icf=all -Clink-arg=-Wl,--build-id=none" \
  cargo +nightly-2026-08-25 build --release --target x86_64-unknown-linux-musl
```
