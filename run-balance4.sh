#!/usr/bin/env bash
#
# 4x4 balance comparison (for the info sheets). Runs `balance4 --archetypes` for BOTH the black-stack
# and the standard variant at komi 0, using the SAME black-stack-trained net for both (standard 4x4 is
# too White-imbalanced to train a usable native net — same finding as 5x5, where the black-stack net
# was the strongest player at both variants and thus the cleanest measuring instrument).
#
# balance4 evaluates with the net embedded in the build, so this swaps src/quantised4.bin + rebuilds
# ONCE. The black-stack run uses the internal black-stack book (207 openings); the standard run uses
# the internal standard book (146 openings); both deterministic, each opening played once.
#
# Optionally, set TILTAK to also cross-check the standard variant with native tiltak self-play
# (-a -s -e tiltak), the way 5x5 did, written to balance_standard_tiltak.txt.
#
# Usage:
#   BS_NET=nets/quantised4-bs-v1.bin ./run-balance4.sh
#   BS_NET=nets/quantised4-bs-v1.bin TILTAK=/workspace/tiltak/target/release/tei ./run-balance4.sh
set -euo pipefail

NODES=${NODES:-60000}     # search nodes/move (-a caps at the book ceiling 207/146, so high nodes are cheap)
THREADS=${THREADS:-26}
KOMI=${KOMI:-0}           # 0 for both variants (the comparison is at 0 komi)
PLYCAP=${PLYCAP:-120}     # tiltak-mode draw cap
OUTDIR=${OUTDIR:-balance_results}

ENGINE_DIR=${ENGINE_DIR:-/workspace/topaz-tak}
BS_NET=${BS_NET:-}        # black-stack net (e.g. nets/quantised4-bs-v1.bin); used for BOTH variants
TILTAK=${TILTAK:-}        # optional: tiltak tei binary for the standard cross-check

say() { printf '\n=== %s ===\n' "$*"; }
cd "$ENGINE_DIR"
mkdir -p "$OUTDIR"

[ -n "$BS_NET" ] || { echo "ERROR: set BS_NET to the SPRT-validated black-stack net"; exit 1; }
[ -f "$BS_NET" ] || { echo "ERROR: BS_NET not found: $BS_NET"; exit 1; }

say "Build topaz with the black-stack net ($BS_NET)"
cp "$BS_NET" src/quantised4.bin
touch src/eval/incremental4.rs      # force cargo to re-embed the swapped net
cargo build --release

bs_out="$OUTDIR/balance_blackstack.txt"
say "balance4 [black-stack] --archetypes, komi $KOMI, $NODES nodes -> $bs_out"
./target/release/topaz balance4 -a -n "$NODES" -t "$THREADS" -k "$KOMI" | tee "$bs_out"

std_out="$OUTDIR/balance_standard.txt"
say "balance4 [standard] --archetypes -s (same black-stack net), komi $KOMI, $NODES nodes -> $std_out"
./target/release/topaz balance4 -a -s -n "$NODES" -t "$THREADS" -k "$KOMI" | tee "$std_out"

if [ -n "$TILTAK" ]; then
  [ -x "$TILTAK" ] || { echo "ERROR: TILTAK not executable: $TILTAK"; exit 1; }
  tlt_out="$OUTDIR/balance_standard_tiltak.txt"
  say "balance4 [standard, tiltak cross-check] -a -s -e $TILTAK, komi $KOMI, $NODES nodes -> $tlt_out"
  ./target/release/topaz balance4 -a -s -e "$TILTAK" -n "$NODES" -p "$PLYCAP" -t "$THREADS" -k "$KOMI" | tee "$tlt_out"
fi

say "Done. Per-archetype balance results in $OUTDIR/ (feed these to gen_sheets.py)"
