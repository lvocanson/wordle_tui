# Building

Three profiles, each with **one command per platform that produces the smallest binary for that profile** (link-time identical-code folding is folded in wherever the toolchain allows):

- **Stable** — full std, no nightly, no `rust-src`.
- **build-std** — nightly; std recompiled without its backtrace machinery.
  The recommended build.
- **immediate-abort** — nightly; most aggressive (every panic → bare abort).

The `build-std` and `immediate-abort` profiles use a **pinned** nightly (`nightly-2026-08-25`, same pin as `tools/validate.sh` and CI): binary sizes are only comparable at equal rustc, so the measured numbers in OPTIMIZATION.md are tied to this toolchain.
Install it once:

```
rustup toolchain install nightly-2026-08-25 --profile minimal --component rust-src
```

To bump the pin, update it here, in `tools/validate.sh` and in `.github/workflows/ci.yml`, then re-measure the reference totals.

### Vendored crossterm (opt-in, size)

Appending **`--config .cargo/crossterm-patch.toml`** to any build command swaps in our vendored crossterm.
See `vendor/crossterm/LOCAL_PATCH.md` for more information.

---

## Windows (PowerShell, MSVC)

The MSVC link optimizations (`/OPT:ICF`, `/DEBUG:NONE`) are already in `.cargo/config.toml`, so they apply to every profile automatically — no extra flags below.
Output: stable → `target\release\wordle_tui.exe`; nightly → `target\x86_64-pc-windows-msvc\release\wordle_tui.exe`.

**Stable:**

```powershell
cargo build --release
```

**build-std:**

```powershell
cargo +nightly-2026-08-25 build --release --target x86_64-pc-windows-msvc
```

**immediate-abort:**

```powershell
$env:RUSTFLAGS = '-Zunstable-options -Cpanic=immediate-abort --cfg immediate_abort -Clink-arg=/OPT:ICF -Clink-arg=/DEBUG:NONE -Clink-arg=/MAP:target/wordle_tui.map'; cargo +nightly-2026-08-25 build --release --target x86_64-pc-windows-msvc --config .cargo/crossterm-patch.toml; Remove-Item Env:RUSTFLAGS
```

> `RUSTFLAGS` **overrides** the `[target]` block (it does not merge), so `/OPT:ICF` and `/DEBUG:NONE` are repeated in the immediate-abort line.
> `/MAP:…` makes the link also emit its symbol map for `cargo run --example bloat` — it does not change a byte of the binary (see OPTIMIZATION.md "Symbol attribution").

---

## Linux — glibc (bash)

ICF has no default-linker equivalent, so it is passed on the command line: the **stable** profile uses the system `lld` (`-fuse-ld=lld`, needs the `lld` package); the **nightly** profiles use the toolchain's bundled `rust-lld` (`-Clinker-features=+lld`, no install).
Output: stable → `target/release/wordle_tui`; nightly → `target/x86_64-unknown-linux-gnu/release/wordle_tui`.

**Stable:**

```bash
RUSTFLAGS="-Clink-arg=-fuse-ld=lld -Clink-arg=-Wl,--icf=all -Clink-arg=-Wl,--build-id=none" \
  cargo build --release --target x86_64-unknown-linux-gnu
```

**build-std:**

```bash
RUSTFLAGS="-Zunstable-options -Clinker-features=+lld -Clink-arg=-Wl,--icf=all -Clink-arg=-Wl,--build-id=none" \
  cargo +nightly-2026-08-25 build --release --target x86_64-unknown-linux-gnu
```

**immediate-abort:**

```bash
RUSTFLAGS="-Zunstable-options -Cpanic=immediate-abort --cfg immediate_abort -Clinker-features=+lld -Clink-arg=-Wl,--icf=all -Clink-arg=-Wl,--build-id=none -Clink-arg=-Wl,-Map=target/wordle_tui-linux.map" \
  cargo +nightly-2026-08-25 build --release --target x86_64-unknown-linux-gnu --config .cargo/crossterm-patch.toml
```

> `RUSTFLAGS` **overrides** the `[target]` block, so `--build-id=none` is repeated in every line.
> `-Wl,-Map=…` emits the symbol map for `cargo run --example bloat`, byte-neutral like the MSVC `/MAP`.
> Without a system `lld`, drop `-Clink-arg=-fuse-ld=lld -Clink-arg=-Wl,--icf=all` from the stable line (it falls back to `cargo build --release`, slightly larger).

---

## Linux — musl (bash, fully static)

Produces a static binary with no runtime dependencies.
Install the target once **on each toolchain you build with** — the nightly profiles need it on nightly for its prebuilt CRT:

```bash
rustup target add x86_64-unknown-linux-musl                     # stable profile
rustup target add --toolchain nightly-2026-08-25 x86_64-unknown-linux-musl # build-std / immediate-abort
```

Output: `target/x86_64-unknown-linux-musl/release/wordle_tui`.

**Stable:**

```bash
RUSTFLAGS="-Clink-arg=-fuse-ld=lld -Clink-arg=-Wl,--icf=all -Clink-arg=-Wl,--build-id=none" \
  cargo build --release --target x86_64-unknown-linux-musl
```

**build-std:**

```bash
RUSTFLAGS="-Zunstable-options -Clinker-features=+lld -Clink-arg=-Wl,--icf=all -Clink-arg=-Wl,--build-id=none" \
  cargo +nightly-2026-08-25 build --release --target x86_64-unknown-linux-musl
```

**immediate-abort:**

```bash
RUSTFLAGS="-Zunstable-options -Cpanic=immediate-abort --cfg immediate_abort -Clinker-features=+lld -Clink-arg=-Wl,--icf=all -Clink-arg=-Wl,--build-id=none" \
  cargo +nightly-2026-08-25 build --release --target x86_64-unknown-linux-musl --config .cargo/crossterm-patch.toml
```
