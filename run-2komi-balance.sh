#!/usr/bin/env bash
#
# 2-komi balance measurement (standard Tak at komi 2), for comparison against the black-stack rule.
#
# Uses a prebuilt engine binary (BIN) that embeds the *2-komi* net. The book is standard-openings
# (single-flat ply 1; diagonal/adjacent/hug archetypes) generated with `openings --standard`, and
# games are played book-based at half-komi 4 (= 2 komi). Random-ply balance can't be used here: the
# black-stack rule in the source forces a `2xx` at move_num 1, so standard openings must come from a
# TPS book that starts past move_num 1 (where the rule no longer fires). For the deeper-search
# strength check, run run-strength-check.sh separately AFTER this.
#
# Tunables (env): GAMES (default 20000), NODES (15000), THREADS (all cores),
#                 BIN (./target/release/topaz), KOMI (4 half-komi), OUT (balance-2komi-results.txt).
#
# Example (26-core pod, 2-komi net binary):
#   BIN=./topaz-2komi GAMES=20000 THREADS=26 ./run-2komi-balance.sh
set -euo pipefail

GAMES="${GAMES:-20000}"
NODES="${NODES:-15000}"
THREADS="${THREADS:-$(nproc)}"
BIN="${BIN:-./target/release/topaz}"
KOMI="${KOMI:-4}"
OUT="${OUT:-balance-2komi-results.txt}"

[ -x "$BIN" ] || { echo "Engine binary not found/executable: $BIN (build it with the 2-komi net first)"; exit 1; }

book="2komi_book_${GAMES}.tps"
echo "Generating ${GAMES}-position standard book -> ${book}"
"$BIN" openings --standard -n "$GAMES" -t "$book" -p "2komi_book_${GAMES}.ptn" >/dev/null

: > "$OUT"
{ echo "2-komi balance (standard Tak, half-komi $KOMI) - $(date)"; echo "Bin=$BIN  Games=$GAMES  Threads=$THREADS"; echo; } | tee -a "$OUT"

run_cfg() {
  local name="$1"; shift
  printf '\n=== %s ===\n' "$name" | tee -a "$OUT"
  "$BIN" "$@" | tee -a "$OUT"
}

run_cfg "book (n=$NODES, half-komi $KOMI)" balance --book "$book" --komi "$KOMI" -n "$NODES" -t "$THREADS"

printf '\n===== SUMMARY (regime -> White-Black gap) =====\n' | tee -a "$OUT"
grep -E "^=== |White-Black" "$OUT"
echo
echo "Full output written to $OUT"
