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
# Both platforms are measured with the same reporter (tools/stats.rs), so the section totals and
# the previous-run deltas read the same everywhere. Exits non-zero if any step failed, with a
# summary at the end.
#
# Profile = immediate-abort + vendored crossterm (the shipping/measurement profile; see BUILD.md).
# Cargo.lock is rewritten by the `--config` patch build (registry -> path); this script restores it
# on exit (caveat in vendor/crossterm/LOCAL_PATCH.md).

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

# Restore the lock and drop the log no matter how we exit (build error, Ctrl-C, success).
LOG="$(mktemp)"
trap 'rm -f "$LOG"; git checkout -- Cargo.lock 2>/dev/null || true' EXIT

# Pinned measurement toolchain: totals are only comparable at equal rustc, so every nightly
# command below uses this dated toolchain (same pin as CI and BUILD.md). Bump deliberately, and
# re-measure the reference totals when you do.
NIGHTLY='nightly-2026-08-25'

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
    -h|--help)  awk 'NR > 1 && !/^#/ { exit } NR > 1 { sub(/^# ?/, ""); print }' "$0"; exit 0 ;;
    *) echo "unknown flag: $a (try --help)" >&2; exit 2 ;;
  esac
done

FAILED=()
section(){ printf '\n\033[1;36m========== %s ==========\033[0m\n' "$1"; }
# try <label> <cmd...>: run the command with its output captured in $LOG; on failure record the
# label and print the log tail (the callers print the interesting lines on success).
try(){
  local label="$1"; shift
  if "$@" >"$LOG" 2>&1; then return 0; fi
  FAILED+=("$label"); tail -n 40 "$LOG"; return 1
}

# ---------------------------------------------------------------------------
section "TESTS (host toolchain)"
if try "tests" cargo test --quiet; then
  grep -E 'test result' "$LOG" || echo '(no test summary captured)'
fi

# ---------------------------------------------------------------------------
section "WINDOWS BUILD — immediate-abort"
if try "windows build" env RUSTFLAGS="$WIN_FLAGS" cargo "+$NIGHTLY" build --release --target "$WIN_TARGET" $CONFIG; then
  grep -iE 'warning|Finished' "$LOG" || true
  # Cargo may print "patch ... was not used in the crate graph" even when the vendored crossterm
  # IS used (it fires whenever the patched version equals the registry one), so that warning is
  # not a signal either way. Ground truth: upstream crossterm's NO_COLOR/COLORTERM env-var
  # strings, verified absent from a patched binary (vendor/crossterm/LOCAL_PATCH.md change 2).
  if LC_ALL=C grep -qE 'NO_COLOR|COLORTERM' "$WIN_BIN"; then
    echo "!! upstream-crossterm marker (NO_COLOR/COLORTERM) found in the binary — vendored patch NOT applied"
    FAILED+=("vendored crossterm check")
  else
    echo "vendored crossterm confirmed (no upstream marker in the binary)"
  fi

  section "WINDOWS SIZE — stats.rs"
  # Explicit path so we always measure the immediate-abort exe we just built; the sed drops the
  # stats example's own build noise and compression report.
  if try "windows size" cargo run --example stats -- "$WIN_BIN"; then
    sed -n '/wordle_tui.exe/,$p' "$LOG"
  fi
fi

# ---------------------------------------------------------------------------
if [ "$DO_LINUX" = 1 ]; then
  section "LINUX BUILD + SIZE — WSL, immediate-abort"
  # Translate the current Windows path to its WSL /mnt/... form so this isn't pinned to one checkout.
  WIN_PATH="$(pwd -W 2>/dev/null || cygpath -w . 2>/dev/null || pwd)"
  WSL_PATH="$(wsl.exe wslpath -u "$WIN_PATH" 2>/dev/null | tr -d '\r')"
  if [ -z "$WSL_PATH" ]; then
    echo "!! could not resolve WSL path for '$WIN_PATH' — is WSL installed? (skip with --no-linux)"
    FAILED+=("linux (wsl path)")
  else
    # Same structure as the Windows steps, run inside WSL: build (log tail on failure), then the
    # same stats.rs reporter. The --config build in WSL rewrites the same Cargo.lock; the EXIT
    # trap restores it.
    wsl.exe -e bash -lc "
      set -u
      cd '$WSL_PATH' || exit 1
      export CARGO_TARGET_DIR=/tmp/wordle_target
      BIN=/tmp/wordle_target/$LIN_TARGET/release/wordle_tui
      LOG=\$(mktemp); trap 'rm -f \"\$LOG\"' EXIT
      if RUSTFLAGS='$LIN_FLAGS' cargo +$NIGHTLY build --release --target $LIN_TARGET $CONFIG >\"\$LOG\" 2>&1; then
        grep -iE 'warning|Finished' \"\$LOG\" || true
      else
        tail -n 40 \"\$LOG\"; exit 1
      fi
      if cargo run --example stats -- \"\$BIN\" >\"\$LOG\" 2>&1; then
        sed -n '/wordle_tui/,\$p' \"\$LOG\"
      else
        tail -n 40 \"\$LOG\"; exit 1
      fi
    " || FAILED+=("linux")
  fi
fi

# ---------------------------------------------------------------------------
if [ "$DO_BLOAT" = 1 ]; then
  # WARNING: cargo-bloat is only a rough HINT generator for this binary, for two reasons:
  #   1. The `--config` crossterm patch is NOT reliably applied under `cargo bloat`, so bloat
  #      often measures UPSTREAM crossterm. Symptom: it still lists parking_lot / Once / env::var
  #      even though the shipping binary has dropped them (the marker check above is the proof).
  #      (Cargo's "patch ... was not used" warning is NOT a signal either way — see that check.)
  #   2. `/OPT:ICF` folds identical functions, so bloat attributes a folded body to an arbitrary,
  #      often-dead symbol name.
  # Ground truth is always the measured `total` delta from stats.rs, never a bloat number. Confirm
  # any lead by string-probing the binary and a measured rebuild. See OPTIMIZATION.md "Stuck costs".
  section "CARGO BLOAT — by crate (HINTS ONLY — see caveats above)"
  RUSTFLAGS="$WIN_FLAGS" cargo "+$NIGHTLY" bloat --release --target "$WIN_TARGET" $CONFIG --crates -n 15 2>&1 \
    | sed -n '/File .*Crate/,$p'
  section "CARGO BLOAT — by function, top 30 (HINTS ONLY — see caveats above)"
  RUSTFLAGS="$WIN_FLAGS" cargo "+$NIGHTLY" bloat --release --target "$WIN_TARGET" $CONFIG -n 30 2>&1 \
    | sed -n '/File .*Name/,$p'
fi

# ---------------------------------------------------------------------------
section "SUMMARY"
if [ ${#FAILED[@]} -eq 0 ]; then
  echo "all steps passed"
else
  printf 'FAILED: %s\n' "${FAILED[@]}"
  exit 1
fi
