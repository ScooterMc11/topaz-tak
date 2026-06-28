#!/usr/bin/env bash
#
# 5x5 tiltak-driven self-play datagen launcher (Linux / RunPod).
#
# Drives tiltak via `topaz datagen5` to produce a "TAK6" binpack for the double-black-stack 5x5
# variant (komi 0), consumable by the bullet trainer's `examples/tak5.rs`. Datagen is CPU-only
# (no GPU / no bullet needed).
#
# Output is SHARDED and RESUMABLE: each shard is written to `<out>.partial` and atomically promoted
# to `shard_NNN.bin` only on completion, so a killed/restarted run skips finished shards and redoes
# only the incomplete one. Shards are concatenated at the end (TAK6 chunks are self-delimiting).
#
# Needs the engine repo (this dir) and the tiltak repo, built from source on the box. tiltak is
# self-contained (params compiled in — nothing to transfer beyond Cargo.toml/Cargo.lock/src/).
#
# Cloud usage (RunPod; see setup-cloud.sh for toolchain install). Launch DETACHED so it survives a
# Jupyter kernel restart, e.g.:
#   setsid nohup env GAMES=20000 NODES=25000 THREADS=26 ./run-datagen5.sh > datagen5.log 2>&1 &
#
# Defaults target ~1M positions (~20k games at ~50 pos/game), ~4h at 26 threads / 25k nodes.
set -euo pipefail

GAMES=${GAMES:-20000}        # total games (~50 positions each)
NODES=${NODES:-25000}        # tiltak go-nodes per move (label quality)
THREADS=${THREADS:-26}       # one tiltak subprocess per thread; leave a core for the OS/Jupyter
SHARDS=${SHARDS:-10}         # output split for resumability
RANDOM_PLIES=${RANDOM_PLIES:-6}
KOMI=${KOMI:-0}              # half-komi; 0 = double-black-stack variant
OUTDIR=${OUTDIR:-datagenShards5}
CONCAT=${CONCAT:-data5.bin}
TILTAK=${TILTAK:-../tiltak/target/release/tei}   # prebuilt tei binary, if present
TILTAK_SRC=${TILTAK_SRC:-../tiltak}              # else build from here

say() { printf '\n=== %s ===\n' "$*"; }

# datagen5 evaluates via tiltak and uses NONE of Topaz's own nets — but the engine still embeds both
# via include_bytes! at build time (size-asserted, content-agnostic). Create zero placeholders for
# any missing net so a minimal datagen box builds without transferring real nets.
[ -f src/quantised.bin ]  || { say "src/quantised.bin missing -> zero placeholder (unused by datagen5)";  head -c 1053760 /dev/zero > src/quantised.bin; }
[ -f src/quantised5.bin ] || { say "src/quantised5.bin missing -> zero placeholder (unused by datagen5)"; head -c 760320  /dev/zero > src/quantised5.bin; }

say "Build release topaz"
cargo build --release

# Build tiltak if its binary is missing but we have the source.
if [ ! -x "$TILTAK" ]; then
  if [ -f "$TILTAK_SRC/Cargo.toml" ]; then
    say "Building tiltak (release) from $TILTAK_SRC"
    # tiltak's `tei` bin is gated behind required-features = ["smol"] (default features are just
    # mimalloc), so a plain `cargo build --release` skips it. Build the bin with the feature.
    ( cd "$TILTAK_SRC" && cargo build --release --features smol --bin tei )
    TILTAK="$TILTAK_SRC/target/release/tei"
  fi
fi
if [ ! -x "$TILTAK" ]; then
  echo "ERROR: tiltak tei binary not found at '$TILTAK' and no source at '$TILTAK_SRC'."
  echo "Upload the tiltak repo and/or set TILTAK (prebuilt) or TILTAK_SRC (to build)."
  exit 1
fi
say "tiltak = $TILTAK"

mkdir -p "$OUTDIR"
per_shard=$(( (GAMES + SHARDS - 1) / SHARDS ))
say "$GAMES games / $SHARDS shards ($per_shard each) | $NODES nodes | $THREADS threads | komi $KOMI -> $OUTDIR/"

start=$(date +%s)
for i in $(seq 0 $((SHARDS - 1))); do
  out=$(printf "%s/shard_%03d.bin" "$OUTDIR" "$i")
  if [ -s "$out" ]; then
    echo "shard $i already complete ($out), skipping"
    continue
  fi
  tmp="$out.partial"
  echo "--- shard $i/$((SHARDS - 1)) -> $out ---"
  ./target/release/topaz datagen5 -g "$per_shard" -t "$THREADS" -n "$NODES" \
    -r "$RANDOM_PLIES" -k "$KOMI" -e "$TILTAK" -o "$tmp"
  mv "$tmp" "$out"   # atomic promote: only a completed shard becomes shard_NNN.bin
done

say "Concatenate shards -> $CONCAT and validate"
cat "$OUTDIR"/shard_*.bin > "$CONCAT"
./target/release/topaz checkdata "$CONCAT"
secs=$(( $(date +%s) - start ))
echo "datagen5 complete in ${secs}s -> $CONCAT"
