#!/usr/bin/env bash
#
# 5x5 balance comparison (for the info sheets). Runs `balance5 --archetypes` for the black-stack
# and/or standard variant(s), each with its OWN trained net embedded, at komi 0. Always uses the
# per-archetype breakdown (-a), so every result file carries archetype data. Writes one results file
# per variant under $OUTDIR for `gen_sheets.py`.
#
# balance5 evaluates with the net embedded in the build, so this swaps src/quantised5.bin + rebuilds
# per variant. NOTE: it leaves src/quantised5.bin set to the LAST variant's net — re-`cp` your working
# net afterwards if needed. Only run this once both nets are SPRT-validated (don't compare weak nets).
#
# Usage (set the SPRT-validated nets; leave a var empty to skip that variant):
#   BS_NET=nets/quantised5-bs-v3.bin STD_NET=nets/quantised5-std-v2.bin ./run-balance5.sh
set -euo pipefail

GAMES=${GAMES:-20000}     # archetype openings generated internally by balance5 -a
NODES=${NODES:-15000}     # search nodes/move (use a strong setting so balance reflects good play)
THREADS=${THREADS:-26}
KOMI=${KOMI:-0}           # 0 for both variants (the comparison is at 0 komi)
OUTDIR=${OUTDIR:-balance_results}

ENGINE_DIR=${ENGINE_DIR:-/workspace/topaz-tak}
BS_NET=${BS_NET:-}        # black-stack net (e.g. nets/quantised5-bs-v3.bin)
STD_NET=${STD_NET:-}      # standard net   (e.g. nets/quantised5-std-v2.bin)

say() { printf '\n=== %s ===\n' "$*"; }
cd "$ENGINE_DIR"
mkdir -p "$OUTDIR"

# $1 = label, $2 = net path, $3 = extra balance5 flags ("-s" selects standard archetypes)
run_variant() {
  local label="$1" net="$2" flags="$3"
  if [ -z "$net" ]; then echo "skip $label (no net set)"; return; fi
  [ -f "$net" ] || { echo "ERROR: $label net not found: $net"; exit 1; }
  say "Build topaz with $label net ($net)"
  cp "$net" src/quantised5.bin
  touch src/eval/incremental5.rs      # force cargo to re-embed the swapped net
  cargo build --release
  local out="$OUTDIR/balance_${label}.txt"
  say "balance5 [$label] --archetypes, komi $KOMI, $GAMES games, $NODES nodes -> $out"
  ./target/release/topaz balance5 -a $flags -g "$GAMES" -n "$NODES" -t "$THREADS" -k "$KOMI" | tee "$out"
}

run_variant blackstack "$BS_NET" ""
run_variant standard   "$STD_NET" "-s"

say "Done. Per-archetype balance results in $OUTDIR/ (feed these to gen_sheets.py)"
