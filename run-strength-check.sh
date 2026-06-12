#!/usr/bin/env bash
#
# Deep-search strength-robustness check for the balance experiment. Re-runs the BOOK balance for
# both the black-stack and 2-komi cases at a higher node count, REUSING the same book files the main
# balance runs produced -- so the only difference vs the 15k-node results is search depth. If the
# White-Black gap stays put at deeper search, the lean isn't a shallow-search artifact.
#
# Run this AFTER run-balance-sweep.sh and run-2komi-balance.sh (with the SAME GAMES), so the books
# sweep_book_<GAMES>.tps and 2komi_book_<GAMES>.tps already exist.
#
# Tunables (env): GAMES (default 20000, must match the main runs), NODES (default 60000),
#                 THREADS (all cores), BS_BIN (./topaz-blackstack), K2_BIN (./topaz-2komi),
#                 OUT (balance-strength-results.txt).
#
# Example (26-core pod):
#   GAMES=20000 NODES=60000 THREADS=26 ./run-strength-check.sh
set -euo pipefail

GAMES="${GAMES:-20000}"
NODES="${NODES:-60000}"
THREADS="${THREADS:-$(nproc)}"
BS_BIN="${BS_BIN:-./topaz-blackstack}"
K2_BIN="${K2_BIN:-./topaz-2komi}"
OUT="${OUT:-balance-strength-results.txt}"

bs_book="sweep_book_${GAMES}.tps"
k2_book="2komi_book_${GAMES}.tps"
[ -x "$BS_BIN" ] || { echo "Black-stack binary not found: $BS_BIN"; exit 1; }
[ -x "$K2_BIN" ] || { echo "2-komi binary not found: $K2_BIN"; exit 1; }
[ -f "$bs_book" ] || { echo "Missing $bs_book -- run run-balance-sweep.sh first with GAMES=$GAMES"; exit 1; }
[ -f "$k2_book" ] || { echo "Missing $k2_book -- run run-2komi-balance.sh first with GAMES=$GAMES"; exit 1; }

: > "$OUT"
{ echo "Deep-search strength check (n=$NODES) - $(date)"; echo "Games=$GAMES  Threads=$THREADS"; echo; } | tee -a "$OUT"

run_cfg() {
  local name="$1"; shift
  printf '\n=== %s ===\n' "$name" | tee -a "$OUT"
  "$@" | tee -a "$OUT"
}

run_cfg "black-stack book (n=$NODES, half-komi 0)" "$BS_BIN" balance --book "$bs_book" --komi 0 -n "$NODES" -t "$THREADS"
run_cfg "2-komi book      (n=$NODES, half-komi 4)" "$K2_BIN" balance --book "$k2_book" --komi 4 -n "$NODES" -t "$THREADS"

printf '\n===== SUMMARY (deep-search White-Black gap) =====\n' | tee -a "$OUT"
grep -E "^=== |White-Black" "$OUT"
echo
echo "Compare these against the n=15000 'book' lines in balance-sweep-results.txt / balance-2komi-results.txt"
echo "Full output written to $OUT"
