#!/usr/bin/env bash
# One-shot validation for the size work: tests + Windows size + Linux size + symbol bloat
# (tools/bloat.rs over the linker maps), all in a single run so there are no back-and-forth
# invocations.
#
# Run from anywhere (it cd's to the repo root). Use Git Bash on Windows:
#   bash tools/validate.sh                 # everything (tests, Win, Linux/WSL, bloat)
#   bash tools/validate.sh --no-linux      # skip the WSL/Linux build
#   bash tools/validate.sh --no-bloat      # skip the symbol-bloat report
#   bash tools/validate.sh --quick         # tests + Windows size only (= --no-linux --no-bloat)
#
# Both platforms are measured with the same reporter (tools/stats.rs), so the section totals and
# the previous-run deltas read the same everywhere. Exits non-zero if any step failed, with a
# summary at the end.
#
# Profile = ship (the shipping/measurement profile; see BUILD.md). Both builds go
# through ./wtui-ship.sh, so the toolchain pin and the per-platform flags are defined once,
# there, and never duplicated here.

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

# Drop the log no matter how we exit (build error, Ctrl-C, success).
LOG="$(mktemp)"
trap 'rm -f "$LOG"' EXIT

# The measurement toolchain is pinned in wtui-ship.sh (totals are only comparable at equal rustc);
# bump it there and re-measure the reference totals. Same for the ship flags, including
# the /MAP and -Map args that make the link emit the symbol map tools/bloat.rs reads below
# (byte-neutral, verified — see OPTIMIZATION.md "Symbol attribution"). Only the paths those maps
# and binaries land in are spelled out here.
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
section "WINDOWS BUILD — ship"
if try "windows build" bash wtui-ship.sh build; then
  grep -iE 'warning|Finished' "$LOG" || true
  # Cargo may print "patch ... was not used in the crate graph" even when the vendored crossterm
  # IS linked (it fires whenever the patched version equals the registry one), so that warning is
  # not a signal either way. Ground truth: upstream crossterm's NO_COLOR/COLORTERM env-var
  # strings, verified absent from the vendored copy (vendor/crossterm/LOCAL_PATCH.md change 2).
  if LC_ALL=C grep -qE 'NO_COLOR|COLORTERM' "$WIN_BIN"; then
    echo "!! upstream-crossterm marker (NO_COLOR/COLORTERM) found in the binary — vendor/crossterm NOT linked"
    FAILED+=("vendored crossterm check")
  else
    echo "vendored crossterm confirmed (no upstream marker in the binary)"
  fi

  section "WINDOWS SIZE — stats.rs"
  # Explicit path so we always measure the ship exe we just built; the sed drops the
  # stats example's own build noise and compression report.
  if try "windows size" cargo run --example stats -- "$WIN_BIN"; then
    sed -n '/wordle_tui.exe/,$p' "$LOG"
  fi
fi

# ---------------------------------------------------------------------------
if [ "$DO_LINUX" = 1 ]; then
  section "LINUX BUILD + SIZE — WSL, ship"
  # Translate the current Windows path to its WSL /mnt/... form so this isn't pinned to one checkout.
  WIN_PATH="$(pwd -W 2>/dev/null || cygpath -w . 2>/dev/null || pwd)"
  WSL_PATH="$(wsl.exe wslpath -u "$WIN_PATH" 2>/dev/null | tr -d '\r')"
  if [ -z "$WSL_PATH" ]; then
    echo "!! could not resolve WSL path for '$WIN_PATH' — is WSL installed? (skip with --no-linux)"
    FAILED+=("linux (wsl path)")
  else
    # Same structure as the Windows steps, run inside WSL: the same wtui-ship.sh build (it picks the
    # Linux triple and flags up from the host it now runs on), then the same stats.rs reporter.
    wsl.exe -e bash -lc "
      set -u
      cd '$WSL_PATH' || exit 1
      export CARGO_TARGET_DIR=/tmp/wordle_target
      BIN=/tmp/wordle_target/$LIN_TARGET/release/wordle_tui
      LOG=\$(mktemp); trap 'rm -f \"\$LOG\"' EXIT
      if bash wtui-ship.sh build >\"\$LOG\" 2>&1; then
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
  # Symbol attribution from the linker maps the builds above just emitted (tools/bloat.rs).
  # Post-LTO/ICF ground truth on the exact shipping binary — replaces cargo-bloat, whose two
  # failure modes (measuring upstream crossterm, misattributing ICF folds) are documented in
  # OPTIMIZATION.md "Symbol attribution".
  section "SYMBOL BLOAT — Windows (tools/bloat.rs over target/wordle_tui.map)"
  if try "bloat (windows)" cargo run --example bloat -- target/wordle_tui.map -n 25; then
    sed -n '/symbol bloat/,$p' "$LOG"
  fi
  if [ "$DO_LINUX" = 1 ]; then
    section "SYMBOL BLOAT — Linux (tools/bloat.rs over target/wordle_tui-linux.map)"
    if try "bloat (linux)" cargo run --example bloat -- target/wordle_tui-linux.map -n 25; then
      sed -n '/symbol bloat/,$p' "$LOG"
    fi
  fi
fi

# ---------------------------------------------------------------------------
section "SUMMARY"
if [ ${#FAILED[@]} -eq 0 ]; then
  echo "all steps passed"
else
  printf 'FAILED: %s\n' "${FAILED[@]}"
  exit 1
fi
