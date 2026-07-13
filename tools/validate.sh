#!/usr/bin/env bash
# One-shot validation for the size work: tests + Windows size + Linux size + cargo-bloat, all in a
# single run so there are no back-and-forth invocations.
#
# Run from anywhere (it cd's to the repo root). Use Git Bash on Windows:
#   bash tools/validate.sh                 # everything (tests, Win, Linux/WSL, bloat)
#   bash tools/validate.sh --no-linux      # skip the WSL/Linux build
#   bash tools/validate.sh --no-bloat      # skip cargo bloat (the slow part)
#   bash tools/validate.sh --quick         # tests + Windows size only (= --no-linux --no-bloat)
#
# Inside a Claude Code session you can trigger it with:  ! bash tools/validate.sh
#
# Profile = immediate-abort + vendored crossterm (the shipping/measurement profile; see BUILD.md).
# Cargo.lock is rewritten by the `--config` patch build (registry -> path); this script restores it
# on exit (caveat in vendor/crossterm/LOCAL_PATCH.md).

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

# Restore the lock no matter how we exit (build error, Ctrl-C, success).
trap 'git checkout -- Cargo.lock 2>/dev/null || true' EXIT

WIN_FLAGS='-Zunstable-options -Cpanic=immediate-abort --cfg immediate_abort -Clink-arg=/OPT:ICF -Clink-arg=/DEBUG:NONE'
LIN_FLAGS='-Zunstable-options -Cpanic=immediate-abort --cfg immediate_abort -Clinker-features=+lld -Clink-arg=-Wl,--icf=all -Clink-arg=-Wl,--build-id=none'
CONFIG='--config .cargo/crossterm-patch.toml'
WIN_TARGET='x86_64-pc-windows-msvc'
LIN_TARGET='x86_64-unknown-linux-gnu'
WIN_BIN="target/${WIN_TARGET}/release/wordle_tui.exe"

DO_LINUX=1; DO_BLOAT=1
for a in "$@"; do
  case "$a" in
    --no-linux) DO_LINUX=0 ;;
    --no-bloat) DO_BLOAT=0 ;;
    --quick)    DO_LINUX=0; DO_BLOAT=0 ;;
    -h|--help)  sed -n '2,17p' "$0"; exit 0 ;;
    *) echo "unknown flag: $a (try --help)" >&2; exit 2 ;;
  esac
done

section(){ printf '\n\033[1;36m========== %s ==========\033[0m\n' "$1"; }

# ---------------------------------------------------------------------------
section "TESTS (host toolchain)"
cargo test --quiet 2>&1 | grep -E 'test result|error|FAILED|panicked' || echo '(no test summary captured)'

# ---------------------------------------------------------------------------
section "WINDOWS BUILD — immediate-abort (warnings/errors only)"
RUSTFLAGS="$WIN_FLAGS" cargo +nightly build --release --target "$WIN_TARGET" $CONFIG 2>&1 \
  | grep -iE 'error|warning|Finished' || echo '(build produced no notable lines)'

section "WINDOWS SIZE — stats.rs"
# Explicit path so we always measure the immediate-abort exe we just built.
cargo run --example stats -- "$WIN_BIN" 2>&1 | sed -n '/wordle_tui.exe/,$p'

# ---------------------------------------------------------------------------
if [ "$DO_LINUX" = 1 ]; then
  section "LINUX BUILD + SIZE — WSL, immediate-abort"
  # Translate the current Windows path to its WSL /mnt/... form so this isn't pinned to one checkout.
  WIN_PATH="$(pwd -W 2>/dev/null || cygpath -w . 2>/dev/null || pwd)"
  WSL_PATH="$(wsl.exe wslpath -u "$WIN_PATH" 2>/dev/null | tr -d '\r')"
  if [ -z "$WSL_PATH" ]; then
    echo "!! could not resolve WSL path for '$WIN_PATH' — is WSL installed? (skip with --no-linux)"
  else
    wsl.exe -e bash -lc "
      set -uo pipefail
      cd '$WSL_PATH' || exit 1
      export CARGO_TARGET_DIR=/tmp/wordle_target
      RUSTFLAGS='$LIN_FLAGS' cargo +nightly build --release --target $LIN_TARGET $CONFIG 2>&1 \
        | grep -iE 'error|warning|Finished' || echo '(build produced no notable lines)'
      BIN=/tmp/wordle_target/$LIN_TARGET/release/wordle_tui
      echo
      echo \"section sizes (size -A):\"
      size -A \"\$BIN\" | grep -E '\\.text|\\.rodata|\\.data|\\.bss|Total'
      echo \"on-disk: \$(wc -c < \"\$BIN\") B  (compare the section Total, not this)\"
    "
    # The --config build inside WSL rewrites the same Cargo.lock; the EXIT trap restores it.
  fi
fi

# ---------------------------------------------------------------------------
if [ "$DO_BLOAT" = 1 ]; then
  # WARNING: cargo-bloat is only a rough HINT generator for this binary, for two reasons:
  #   1. The `--config` crossterm patch is NOT reliably applied under `cargo bloat` (watch for
  #      "patch ... was not used in the crate graph") — so bloat often measures UPSTREAM crossterm.
  #      Symptom: it still lists parking_lot / Once / env::var even though the shipping binary has
  #      dropped them (the stats `total` above is the proof it's gone).
  #   2. `/OPT:ICF` folds identical functions, so bloat attributes a folded body to an arbitrary,
  #      often-dead symbol name.
  # Ground truth is always the measured `total` delta from stats.rs, never a bloat number. Confirm
  # any lead by string-probing the binary and a measured rebuild. See OPTIMIZATION.md "Stuck costs".
  section "CARGO BLOAT — by crate (HINTS ONLY — see caveats above)"
  RUSTFLAGS="$WIN_FLAGS" cargo +nightly bloat --release --target "$WIN_TARGET" $CONFIG --crates -n 15 2>&1 \
    | sed -n '/File .*Crate/,$p'
  section "CARGO BLOAT — by function, top 30 (HINTS ONLY — see caveats above)"
  RUSTFLAGS="$WIN_FLAGS" cargo +nightly bloat --release --target "$WIN_TARGET" $CONFIG -n 30 2>&1 \
    | sed -n '/File .*Name/,$p'
fi

section "DONE"
