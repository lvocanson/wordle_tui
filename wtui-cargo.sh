#!/usr/bin/env bash
# wtui-cargo.sh — the BUILD.md build commands, with the ceremony detected instead of typed.
#
#   ./wtui-cargo.sh run                        # stable, host-native (a plain `cargo run --release`)
#   ./wtui-cargo.sh build --build-std          # std recompiled without its backtrace machinery
#   ./wtui-cargo.sh run --immediate-abort      # every panic -> bare abort (the shipping profile)
#   ./wtui-cargo.sh build --immediate-abort --target x86_64-unknown-linux-musl
#   ./wtui-cargo.sh build --build-std -n       # print the cargo command it would run, run nothing
#
# The three profiles are exactly the ones documented in BUILD.md; the script only removes the
# manual steps around them:
#
#   * platform — the target triple defaults to the host's (from `rustc -vV`), and the linker
#     flags follow from it: MSVC folds through /OPT:ICF (already in .cargo/config.toml) and emits
#     /MAP, ELF needs an explicit lld + --icf=all and emits -Wl,-Map.
#   * toolchain — the nightly profiles install the pinned nightly AND its rust-src component when
#     missing. `cargo +nightly-…` on its own does auto-install a toolchain, but with the default
#     profile, i.e. without rust-src, which is exactly what build-std needs. Any --target you ask
#     for that is not installed is added too (that is the musl prerequisite in BUILD.md).
#   * flags — passed as `--config target.<triple>.rustflags=[…]`, which MERGES with the [target]
#     blocks of .cargo/config.toml. An env RUSTFLAGS OVERRIDES them instead, which is why the
#     BUILD.md lines have to repeat /OPT:ICF, /DEBUG:NONE and --build-id=none, and this does not.
#
# Options: --stable (default) | --build-std | --immediate-abort, --target <triple>, -n/--dry-run.
# Anything else is forwarded to cargo untouched: `-v`, `--features …`, `-- --args-for-the-game`.
#
# Run from anywhere (it cd's to the repo root). Use Git Bash on Windows, like tools/validate.sh:
#   bash wtui-cargo.sh run --immediate-abort

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$REPO" || exit 1

# Pinned measurement toolchain: binary sizes are only comparable at equal rustc, so the nightly
# profiles use this dated one (same pin as tools/validate.sh, .github/workflows/ci.yml and
# BUILD.md). Bump it in all four places, then re-measure the reference totals.
NIGHTLY='nightly-2026-08-25'

say()  { printf '\033[1;36m%s\033[0m\n' "$*"; }
note() { printf '%s\n' "$*"; }
die()  { printf '%s\n' "$*" >&2; exit 2; }
have() { command -v "$1" >/dev/null 2>&1; }
usage(){ awk 'NR > 1 && !/^#/ { exit } NR > 1 { sub(/^# ?/, ""); print }' "$0"; }

# --- arguments ---------------------------------------------------------------------------------
PROFILE=stable
CMD=''
TARGET=''
DRY=0
EXTRA=()

while [ $# -gt 0 ]; do
  case "$1" in
    --stable)             PROFILE=stable ;;
    --build-std)          PROFILE=build-std ;;
    --immediate-abort)    PROFILE=immediate-abort ;;
    --target)             [ $# -ge 2 ] || die "--target needs a triple"; TARGET="$2"; shift ;;
    --target=*)           TARGET="${1#*=}" ;;
    -n|--dry-run)         DRY=1 ;;
    -h|--help)            usage; exit 0 ;;
    --)                   EXTRA+=("$@"); break ;;              # the rest belongs to the program
    -*)                   EXTRA+=("$1") ;;                     # unknown flag -> cargo's problem
    *)                    if [ -z "$CMD" ]; then CMD="$1"; else EXTRA+=("$1"); fi ;;
  esac
  shift
done
CMD="${CMD:-build}"

# --- platform ----------------------------------------------------------------------------------
have rustup || die "rustup not found — install it from https://rustup.rs"
HOST="$(rustc -vV | sed -n 's/^host: //p')"
[ -n "$HOST" ] || die "could not read the host triple from 'rustc -vV'"
TARGET="${TARGET:-$HOST}"

case "$TARGET" in
  *-windows-msvc) LINKER=msvc ;;
  *-linux-*)      LINKER=elf ;;
  *)              LINKER=other ;;   # everything else builds fine, just without the ICF extras
esac

# --- toolchain ---------------------------------------------------------------------------------
if [ "$PROFILE" = stable ]; then TOOLCHAIN=''; else TOOLCHAIN="$NIGHTLY"; fi
TC=()
[ -n "$TOOLCHAIN" ] && TC=(--toolchain "$TOOLCHAIN")

# A dry run stays side-effect free: with -n, nothing below installs anything.
if [ -n "$TOOLCHAIN" ] && [ "$DRY" -eq 0 ]; then
  if ! rustup toolchain list | grep -q "^${TOOLCHAIN}"; then
    say "installing $TOOLCHAIN + rust-src (build-std needs it)"
    rustup toolchain install "$TOOLCHAIN" --profile minimal --component rust-src \
      || die "could not install $TOOLCHAIN"
  elif ! rustup component list "${TC[@]}" --installed | grep -q '^rust-src'; then
    say "adding rust-src to $TOOLCHAIN (build-std needs it)"
    rustup component add rust-src "${TC[@]}" || die "could not add rust-src to $TOOLCHAIN"
  fi
fi

if [ "$DRY" -eq 0 ] && ! rustup target list ${TC[@]+"${TC[@]}"} --installed | grep -qx "$TARGET"; then
  say "adding target $TARGET to ${TOOLCHAIN:-the default toolchain}"
  rustup target add ${TC[@]+"${TC[@]}"} "$TARGET" || die "could not add target $TARGET"
fi

# --- flags -------------------------------------------------------------------------------------
# Only what .cargo/config.toml does not already carry: its [target] blocks stay in force because
# these go through --config (merge) and not RUSTFLAGS (override).
FLAGS=()
if [ "$PROFILE" = stable ]; then
  # MSVC folds through /OPT:ICF from .cargo/config.toml. ELF has no default-linker equivalent and
  # stable ships no lld, so this is the one profile that needs a SYSTEM lld; without it the build
  # still works, it just links with bfd and comes out slightly larger.
  if [ "$LINKER" = elf ]; then
    if have ld.lld || have lld; then
      FLAGS+=(-Clink-arg=-fuse-ld=lld -Clink-arg=-Wl,--icf=all)
    else
      note "no system lld found — linking with the default linker (no ICF, slightly larger)"
    fi
  fi
else
  # The nightly toolchain ships its own lld (rust-lld), so ELF ICF needs no package here.
  [ "$LINKER" = elf ] && FLAGS+=(-Clinker-features=+lld -Clink-arg=-Wl,--icf=all)

  if [ "$PROFILE" = immediate-abort ]; then
    FLAGS+=(-Cpanic=immediate-abort --cfg immediate_abort)
    # Byte-neutral: makes the link also emit its symbol map for `cargo run --example bloat`
    # (OPTIMIZATION.md "Symbol attribution"). Only for the two triples tools/validate.sh reads.
    case "$TARGET" in
      *-windows-msvc)      FLAGS+=(-Clink-arg=/MAP:target/wordle_tui.map) ;;
      *-unknown-linux-gnu) FLAGS+=(-Clink-arg=-Wl,-Map=target/wordle_tui-linux.map) ;;
    esac
  fi
  # Every flag added above is unstable; -Zunstable-options gates them. build-std on MSVC needs
  # none of them, and then the command is a bare `cargo +nightly build --release --target …`.
  [ ${#FLAGS[@]} -gt 0 ] && FLAGS=(-Zunstable-options "${FLAGS[@]}")
fi

# --- command -----------------------------------------------------------------------------------
toml_array(){ local out='[' sep=''; for f; do out="$out$sep\"$f\""; sep=', '; done; printf '%s]' "$out"; }

ARGS=(cargo)
[ -n "$TOOLCHAIN" ] && ARGS+=("+$TOOLCHAIN")
ARGS+=("$CMD" --release)

# build-std REQUIRES an explicit --target; the stable profile stays host-native without one, so a
# `./wtui-cargo.sh run` lands in target/release just like the plain cargo command it replaces.
USE_TARGET=0
if [ "$PROFILE" != stable ] || [ "$TARGET" != "$HOST" ]; then
  USE_TARGET=1
  ARGS+=(--target "$TARGET")
fi
[ ${#FLAGS[@]} -gt 0 ] && ARGS+=(--config "target.$TARGET.rustflags=$(toml_array "${FLAGS[@]}")")
ARGS+=(${EXTRA[@]+"${EXTRA[@]}"})

EXT=''; case "$TARGET" in *windows*) EXT='.exe' ;; esac
DIR="${CARGO_TARGET_DIR:-target}"                     # tools/validate.sh redirects it under WSL
if [ "$USE_TARGET" -eq 1 ]; then OUT="$DIR/$TARGET/release/wordle_tui$EXT"
else                             OUT="$DIR/release/wordle_tui$EXT"; fi

show(){ local a out=''; for a; do case "$a" in *[\ \"\[]*) out="$out '$a'" ;; *) out="$out $a" ;; esac; done; printf '%s\n' "${out# }"; }
say "profile: $PROFILE | target: $TARGET | toolchain: ${TOOLCHAIN:-default}"
show "${ARGS[@]}"
[ "$DRY" -eq 1 ] && exit 0

"${ARGS[@]}"; RC=$?
if [ "$RC" -eq 0 ] && [ "$CMD" = build ] && [ -f "$OUT" ]; then
  say "$OUT — $(wc -c < "$OUT" | tr -d ' ') bytes"
fi
exit "$RC"
