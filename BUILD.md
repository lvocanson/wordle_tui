# Building

Three profiles, each with **one command per platform that produces the smallest binary for that profile** (link-time identical-code folding is folded in wherever the toolchain allows):

- **Stable** — full std, no nightly, no `rust-src`.
- **build-std** — nightly; std recompiled without its backtrace machinery.
- **immediate-abort** — nightly; most aggressive (every panic → bare abort).

## One command

`./wtui-cargo.sh` runs any of these profiles with the toolchain, target and flags that fit the machine it is on — the same commands as below, with the prerequisites installed on demand:

```bash
./wtui-cargo.sh run                       # stable, host-native (= cargo run --release)
./wtui-cargo.sh build --build-std
./wtui-cargo.sh run --immediate-abort     # the shipping profile
./wtui-cargo.sh build --immediate-abort --target x86_64-unknown-linux-musl
./wtui-cargo.sh build --build-std -n      # print the cargo command it would run, run nothing
```

It installs the pinned nightly **with `rust-src`** and adds a missing `--target`, and forwards anything it does not recognise to cargo. `./wtui-cargo.sh --help` for the rest.

The flags reach cargo through `--config target.<triple>.rustflags=[…]`, which **merges** with the `[target]` blocks of `.cargo/config.toml` — the reason the script never repeats `/OPT:ICF` & co. while the hand-typed `RUSTFLAGS` lines below have to.

The rest of this file is what the script runs, profile by profile.

---

The `build-std` and `immediate-abort` profiles use a **pinned** nightly (`nightly-2026-08-25`, declared in `wtui-cargo.sh`, which `tools/validate.sh` and CI both build through): binary sizes are only comparable at equal rustc, so the measured numbers in OPTIMIZATION.md are tied to this toolchain.
Install it once:

```
rustup toolchain install nightly-2026-08-25 --profile minimal --component rust-src
```

To bump the pin, change `NIGHTLY` in `wtui-cargo.sh` and the version quoted here, then re-measure the reference totals.

### Vendored crossterm

Every profile links the in-tree crossterm from `vendor/crossterm/`, patched for size.
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
$env:RUSTFLAGS = '-Zunstable-options -Cpanic=immediate-abort --cfg immediate_abort -Clink-arg=/OPT:ICF -Clink-arg=/DEBUG:NONE -Clink-arg=/MAP:target/wordle_tui.map'; cargo +nightly-2026-08-25 build --release --target x86_64-pc-windows-msvc; Remove-Item Env:RUSTFLAGS
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
  cargo +nightly-2026-08-25 build --release --target x86_64-unknown-linux-gnu
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
  cargo +nightly-2026-08-25 build --release --target x86_64-unknown-linux-musl
```
