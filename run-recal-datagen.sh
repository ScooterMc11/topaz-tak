#!/usr/bin/env bash
#
# Recalibration datagen launcher (Linux / RunPod) — 6x6 or 5x5.
#
# Generates CORNER-OPEN self-play training data with Topaz's OWN embedded net (no tiltak): ply-1's
# 2xx black stack is pinned to a random corner and the remaining opening plies are random FLATS
# (no walls/caps/spreads). This is the corrected opening distribution that fixes the Black-favoured
# eval skew (a uniformly random stack square lands off-corner ~89% of the time on 6x6 / ~88% on 5x5,
# and off-corner stacks over-influence the board toward Black). Center-stack never occurs (the stack
# is always cornered). See auto-memory `black-stack-setup-experiment`.
#
# SAME command serves two purposes, differing only in GAMES:
#   * DIAGNOSTIC  — a small run (e.g. GAMES=4000); the script auto-runs `topaz calibrate` on the
#                   output, printing the net's White/Black calibration (the crossover cp) so you can
#                   see the skew before committing GPU hours.
#   * FULL REGEN  — a large run (the training dataset for the from-scratch retrain).
#
# The player is whatever net is EMBEDDED in the built binary, so cp the intended best net into
# src/quantised{,5}.bin and `touch` the matching incremental*.rs BEFORE running (see command sheet).
#
# Output is SHARDED + RESUMABLE (killed run skips finished shards). Datagen is CPU-only (no GPU).
#
# Launch detached so it survives a Jupyter kernel restart, e.g.:
#   setsid nohup env SIZE=6 GAMES=280000 NODES=25000 THREADS=26 ./run-recal-datagen.sh \
#     > recal6.log 2>&1 &
set -euo pipefail

SIZE=${SIZE:-6}                 # 6 or 5
GAMES=${GAMES:-280000}         # total self-play games (~70 pos/game on 6x6, ~55 on 5x5)
NODES=${NODES:-25000}          # Topaz search node cap per move (label quality)
THREADS=${THREADS:-26}         # worker threads; leave a core for the OS/Jupyter
SHARDS=${SHARDS:-20}           # output split for resumability
RANDOM_PLIES=${RANDOM_PLIES:-6}
OUTDIR=${OUTDIR:-recalShards${SIZE}}
CONCAT=${CONCAT:-data_recal_${SIZE}.bin}
CALIBRATE=${CALIBRATE:-1}      # run `topaz calibrate` on the concatenated output

case "$SIZE" in
  6) SUBCMD=datagen ;;          # NNUE6 self-play, corner-open
  5) SUBCMD=datagen5self ;;     # NNUE5 self-play, corner-open
  *) echo "ERROR: SIZE must be 6 or 5 (got '$SIZE')"; exit 1 ;;
esac

say() { printf '\n=== %s ===\n' "$*"; }

# rustup installs cargo into ~/.cargo/bin but only edits your shell PROFILE; a fresh/non-login shell
# (and this script's subshell) won't have it on PATH. Pull it in if missing so `cargo` resolves.
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env" 2>/dev/null || true

# The build embeds all three nets via include_bytes! (size-asserted, content-agnostic). The TARGET
# size's net must be the REAL best net (cp'd in by the caller — it is the self-play player); create
# zero placeholders only for any OTHER missing net so the build still links. Never clobber existing.
[ -f src/quantised.bin ]  || { say "src/quantised.bin missing -> zero placeholder";  head -c 1053760 /dev/zero > src/quantised.bin; }
[ -f src/quantised5.bin ] || { say "src/quantised5.bin missing -> zero placeholder"; head -c 760320  /dev/zero > src/quantised5.bin; }
[ -f src/quantised4.bin ] || { say "src/quantised4.bin missing -> zero placeholder"; head -c 520192  /dev/zero > src/quantised4.bin; }

say "Build release topaz (SIZE=$SIZE, subcommand '$SUBCMD')"
cargo build --release
ENGINE=./target/release/topaz

mkdir -p "$OUTDIR"
per_shard=$(( (GAMES + SHARDS - 1) / SHARDS ))
say "$GAMES games / $SHARDS shards ($per_shard each) | $NODES nodes | $THREADS threads | corner-open -> $OUTDIR/"

start=$(date +%s)
for i in $(seq 0 $((SHARDS - 1))); do
  out=$(printf "%s/shard_%03d.bin" "$OUTDIR" "$i")
  if [ -s "$out" ]; then
    echo "shard $i already complete ($out), skipping"
    continue
  fi
  tmp="$out.partial"
  echo "--- shard $i/$((SHARDS - 1)) -> $out ---"
  "$ENGINE" "$SUBCMD" -c -g "$per_shard" -t "$THREADS" -n "$NODES" -r "$RANDOM_PLIES" -o "$tmp"
  mv "$tmp" "$out"   # atomic promote: only a completed shard becomes shard_NNN.bin
done

say "Concatenate shards -> $CONCAT and validate"
cat "$OUTDIR"/shard_*.bin > "$CONCAT"
"$ENGINE" checkdata "$CONCAT"

if [ "$CALIBRATE" = "1" ]; then
  say "Calibration of the embedded net on this (corrected) distribution"
  "$ENGINE" calibrate "$CONCAT"
fi

secs=$(( $(date +%s) - start ))
echo "recal datagen (SIZE=$SIZE) complete in ${secs}s -> $CONCAT"
