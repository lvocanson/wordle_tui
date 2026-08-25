#!/usr/bin/env bash
# Behavioural test for the input path, which `cargo test` cannot reach: it drives a built binary
# through a real PTY and asserts on the bytes it renders. This is what validates the vendored
# crossterm parser patches (LOCAL_PATCH.md change 7) — nothing else exercises them.
#
#   bash tools/pty_test.sh <path-to-binary>
#
# Linux/WSL only (needs `script -qec`). Run it against a CONTROL binary built from the previous
# commit as well: a bare failure list is not readable on its own, because the pty line discipline
# mangles some input on its own (see the sleep below). A patch is a regression only when the two
# runs differ.
#
# Geometry assumes an 80x30 terminal: ENTER spans x=21..25, y=27 and the 'z' key x=27..29, y=27,
# so the SGR (1-based) coordinates below are col 23 / col 29, row 28.

BIN="${1:?usage: pty_test.sh <path-to-binary>}"
OUT="$(mktemp)"
trap 'rm -f "$OUT"' EXIT
FAILED=0

# Input is sent after a delay so the app has already switched the tty to raw mode; otherwise the
# line discipline eats CR (ICRNL) and 0x03 (ISIG) before the app can ever see them, and the test
# reads as a failure that has nothing to do with the binary.
run() { # run <name> <printf-format-of-input>
  (sleep 0.6; printf "$2"; sleep 0.6) \
    | timeout 8 script -qec "bash -c 'stty cols 80 rows 30; $BIN'" /dev/null >"$OUT" 2>&1
  printf '\n== %s\n' "$1"
}
check() { # check <grep-pattern> <what it proves>
  if grep -aoqE -- "$1" "$OUT"; then echo "  ok   $2"; else echo "  FAIL $2"; FAILED=1; fi
}
reject() { # reject <grep-pattern> <what must not have happened>
  if grep -aoqE -- "$1" "$OUT"; then echo "  FAIL $2"; FAILED=1; else echo "  ok   $2"; fi
}

run "type a word and submit it" 'crane\r\033'
check "C" "typed letters are shown uppercased"
check "48;2;(83;141;78|181;159;59|58;58;60)m" "the guess was scored (cell colours appeared)"

run "submit a non-word" 'zzzzz\r\033'
check "Invalid word\." "the invalid-word message appeared"

run "backspace shortens the draft" 'crane\177\177\r\033'
check "Incorrect size\." "the shortened draft was rejected as too short"

# The framing check: dropped escape sequences must be consumed whole. If one were refused a byte
# early its tail would be typed into the board, and the trailing Enter would then report a bad
# word instead of an empty draft.
run "arrows and F-keys are swallowed whole" '\033[A\033[B\033[C\033[D\033[15~\033OP\033[5~\r\033'
reject "Invalid word\." "no dropped sequence leaked letters into the board"
check "Incorrect size\." "Enter still works and the draft was still empty"

run "SGR click on the ENTER button" '\033[<0;23;28M\033[<0;23;28m\033'
check "Incorrect size\." "the click submitted, so it hit-tested to Enter"

run "SGR clicks on the 'z' key, five times, then Enter" \
  '\033[<0;29;28M\033[<0;29;28m\033[<0;29;28M\033[<0;29;28m\033[<0;29;28M\033[<0;29;28m\033[<0;29;28M\033[<0;29;28m\033[<0;29;28M\033[<0;29;28m\r\033'
check "Invalid word\." "five mouse-typed letters submitted as zzzzz"

# The other mouse encoding kept by the patch: ESC [ M <32+cb> <32+1+x> <32+1+y>.
run "X10-encoded click on the ENTER button" '\033[M\040\067\074\033'
check "Incorrect size\." "the X10-encoded click submitted too"

run "Ctrl+C quits" '\003'
check "Wordle" "the game rendered, then exited on Ctrl+C"

run "Esc quits" '\033'
check "Wordle" "the game rendered, then exited on Esc"

echo
if [ "$FAILED" = 0 ]; then echo "all checks passed"; else echo "SOME CHECKS FAILED"; fi
exit "$FAILED"
