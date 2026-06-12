#!/usr/bin/env bash
#
# Black-stack balance consistency sweep (Linux twin of run-balance-sweep.ps1).
#
# Runs `topaz balance` under several opening regimes (all at the standard node count) and tabulates
# the White-Black gap. The point is robustness: if the lean is stable across opening depth and
# opening source, it is a property of the RULE, not an artifact. For the deeper-search strength
# check, run run-strength-check.sh separately AFTER this.
#
#   Regimes: random flat plies r=4 / r=6 / r=8 (opening-depth), and a fixed TPS book played one
#            deterministic game per position (reproducible).
#
# Tunables (env vars):
#   GAMES          games per regime         (default 5000; book regime uses a book of this size)
#   NODES          search nodes/move        (default 15000)
#   THREADS        worker threads           (default: all cores, $(nproc))
#   BIN            engine binary            (default ./target/release/topaz)
#   OUT            results file             (default balance-sweep-results.txt)
#
# Example (26-core pod, definitive run):
#   GAMES=20000 THREADS=26 ./run-balance-sweep.sh
set -euo pipefail

GAMES="${GAMES:-5000}"
NODES="${NODES:-15000}"
THREADS="${THREADS:-$(nproc)}"
BIN="${BIN:-./target/release/topaz}"
OUT="${OUT:-balance-sweep-results.txt}"

[ -x "$BIN" ] || { echo "Engine binary not found/executable: $BIN (run 'cargo build --release')"; exit 1; }

book="sweep_book_${GAMES}.tps"
echo "Generating ${GAMES}-position book -> ${book}"
"$BIN" openings -n "$GAMES" -t "$book" -p "sweep_book_${GAMES}.ptn" >/dev/null

: > "$OUT"
{ echo "Balance consistency sweep - $(date)"; echo "Bin=$BIN  Games=$GAMES  Threads=$THREADS"; echo; } | tee -a "$OUT"

run_cfg() {
  local name="$1"; shift
  printf '\n=== %s ===\n' "$name" | tee -a "$OUT"
  local t0=$SECONDS
  # stdout carries the summary; stderr (per-1000-game progress) streams to the console.
  "$BIN" "$@" | tee -a "$OUT"
  printf '(%.1f min)\n' "$(echo "scale=2; ($SECONDS - $t0)/60" | bc 2>/dev/null || echo $(( (SECONDS - t0) / 60 )))" | tee -a "$OUT"
}

run_cfg "random r=4  (n=$NODES)"          balance -g "$GAMES" -n "$NODES"          -t "$THREADS" -r 4
run_cfg "random r=6  (n=$NODES)"          balance -g "$GAMES" -n "$NODES"          -t "$THREADS" -r 6
run_cfg "random r=8  (n=$NODES)"          balance -g "$GAMES" -n "$NODES"          -t "$THREADS" -r 8
run_cfg "book        (n=$NODES)"          balance --book "$book" -n "$NODES"        -t "$THREADS"

printf '\n===== SUMMARY (regime -> White-Black gap) =====\n' | tee -a "$OUT"
grep -E "^=== |White-Black" "$OUT"
echo
echo "Full output written to $OUT"
