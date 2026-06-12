#!/usr/bin/env bash
#
# End-to-end NNUE retraining for the "black stack setup" ruleset.
# Linux/cloud port of run-training.ps1. Run once and walk away.
#
# Pipeline: build engine -> datagen -> checkdata -> train (bullet) -> install net -> rebuild engine.
#
#   ./run-training.sh --smoke --backend cpu                       # tiny end-to-end validation, no GPU
#   ./run-training.sh --games 2000000 --backend cuda              # full from-scratch run on a GPU box
#   ./run-training.sh --skip-train                                # just build + generate + validate
#   ./run-training.sh --shards datagenShards/doubleBlackStack     # clean+combine shards, auto-size, train
#
# Repo locations default to this script's dir (engine) and a sibling bullet checkout; override with
# the ENGINE_DIR / BULLET_DIR env vars or --bullet-dir.
set -euo pipefail

ENGINE_DIR="${ENGINE_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
BULLET_DIR="${BULLET_DIR:-$ENGINE_DIR/../bulletTrainer/bullet}"

GAMES=1000000
THREADS="$(nproc 2>/dev/null || echo 8)"
NODES=5000
RANDOM_PLIES=6
DATA_FILE="$ENGINE_DIR/data.bin"
BACKEND=cuda
SUPERBATCHES=""        # "" -> "auto" when --shards is used, else 240
BATCHES=6104
SHARDS_DIR=""
SMOKE=0
SKIP_DATAGEN=0
SKIP_TRAIN=0

usage() {
  awk 'NR>1 && /^#/ {sub(/^# ?/, ""); print; next} NR>1 {exit}' "${BASH_SOURCE[0]}"
  cat <<'EOF'

Options:
  --games N           self-play games (default 1000000)
  --threads N         datagen worker threads (default: nproc)
  --nodes N           search node cap per move (default 5000; lower = faster datagen)
  --random-plies N    random opening plies (default 6)
  --data PATH         dataset output path (default ENGINE_DIR/data.bin)
  --bullet-dir PATH   bullet trainer checkout (default ENGINE_DIR/../bulletTrainer/bullet)
  --backend NAME      cuda | hip | hip-cuda | cpu (default cuda)
  --superbatches N    training superbatches ("auto" sizes from the dataset for ~24 epochs;
                      default 240, or auto when --shards is used)
  --batches N         batches per superbatch (default 6104)
  --shards DIR        clean (trim) + concatenate every *.bin in DIR into --data, then train
  --smoke             tiny end-to-end run; does NOT install the toy net
  --skip-datagen      reuse an existing dataset
  --skip-train        stop after datagen + validation
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --games) GAMES="$2"; shift 2;;
    --threads) THREADS="$2"; shift 2;;
    --nodes) NODES="$2"; shift 2;;
    --random-plies) RANDOM_PLIES="$2"; shift 2;;
    --data) DATA_FILE="$2"; shift 2;;
    --bullet-dir) BULLET_DIR="$2"; shift 2;;
    --backend) BACKEND="$2"; shift 2;;
    --superbatches) SUPERBATCHES="$2"; shift 2;;
    --batches) BATCHES="$2"; shift 2;;
    --shards) SHARDS_DIR="$2"; shift 2;;
    --smoke) SMOKE=1; shift;;
    --skip-datagen) SKIP_DATAGEN=1; shift;;
    --skip-train) SKIP_TRAIN=1; shift;;
    -h|--help) usage; exit 0;;
    *) echo "unknown arg: $1" >&2; usage; exit 1;;
  esac
done

# shellcheck disable=SC1090
source "$HOME/.cargo/env" 2>/dev/null || true
ENGINE_EXE="$ENGINE_DIR/target/release/topaz"

step() { printf '\n=== %s ===\n' "$*"; }
die()  { printf 'FAILED: %s\n' "$*" >&2; exit 1; }

# Resolve the superbatch default: auto-size when assembling shards, else the classic 240.
if [ -z "$SUPERBATCHES" ]; then
  if [ -n "$SHARDS_DIR" ]; then SUPERBATCHES="auto"; else SUPERBATCHES=240; fi
fi

if [ "$SMOKE" -eq 1 ]; then
  echo "SMOKE MODE: tiny end-to-end validation run."
  GAMES=200; SUPERBATCHES=2; BATCHES=50
fi

start=$(date +%s)

# 1. Build the engine ---------------------------------------------------------
step "1/5 Build engine (release)"
( cd "$ENGINE_DIR" && cargo build --release )
[ -x "$ENGINE_EXE" ] || die "engine binary not found at $ENGINE_EXE"

# 2. Generate self-play data (or assemble shards) -----------------------------
if [ -n "$SHARDS_DIR" ]; then
  step "2/5 Assemble shards from $SHARDS_DIR -> $DATA_FILE"
  [ -d "$SHARDS_DIR" ] || die "shards dir not found: $SHARDS_DIR"
  shopt -s nullglob
  shards=("$SHARDS_DIR"/*.bin)
  shopt -u nullglob
  [ "${#shards[@]}" -gt 0 ] || die "no .bin shards found in $SHARDS_DIR"
  : > "$DATA_FILE"   # start fresh so re-runs don't append to a stale dataset
  count=0
  for s in "${shards[@]}"; do
    # Don't fold the output file into itself if it happens to live in the shards dir.
    if [ "$(readlink -f "$s")" = "$(readlink -f "$DATA_FILE")" ]; then continue; fi
    echo "  + $(basename "$s"): $("$ENGINE_EXE" trimdata "$s")"
    cat "$s" >> "$DATA_FILE"
    count=$((count + 1))
  done
  echo "  assembled $count shard(s) into $DATA_FILE"
elif [ "$SKIP_DATAGEN" -eq 1 ]; then
  step "2/5 Generate data (SKIPPED, using $DATA_FILE)"
  [ -f "$DATA_FILE" ] || die "--skip-datagen set but $DATA_FILE does not exist"
else
  step "2/5 Generate data ($GAMES games -> $DATA_FILE)"
  "$ENGINE_EXE" datagen -g "$GAMES" -t "$THREADS" -n "$NODES" -r "$RANDOM_PLIES" -o "$DATA_FILE"
fi

# 3. Validate the dataset -----------------------------------------------------
step "3/5 Validate dataset"
summary=$("$ENGINE_EXE" checkdata "$DATA_FILE")
echo "$summary"
grep -q "invalid=0" <<<"$summary" || die "dataset reported invalid entries: $summary"

# Suggest (and, if requested, apply) a superbatch count sized for ~24 epochs.
positions=$(printf '%s' "$summary" | sed -n 's/.*positions=\([0-9]\+\).*/\1/p')
if [ -n "$positions" ] && [ "$positions" -gt 0 ]; then
  suggested=$(( (positions * 24 + 50000000) / 100000000 ))
  [ "$suggested" -lt 1 ] && suggested=1
  echo "  ~$positions positions -> suggested --superbatches $suggested (~24 epochs)"
  if [ "$SUPERBATCHES" = "auto" ]; then
    SUPERBATCHES=$suggested
    echo "  using --superbatches $SUPERBATCHES (auto)"
  fi
fi
if [ "$SUPERBATCHES" = "auto" ]; then
  echo "  (could not parse position count; defaulting --superbatches to 240)"
  SUPERBATCHES=240
fi

# 4. Train --------------------------------------------------------------------
if [ "$SKIP_TRAIN" -eq 1 ]; then
  step "4/5 Train (SKIPPED)"
  echo "To train later, from '$BULLET_DIR':"
  echo "  TAK_DATA='$DATA_FILE' TAK_SUPERBATCHES=$SUPERBATCHES TAK_BATCHES=$BATCHES \\"
  echo "    cargo run -p bullet_lib --release --example tak   # add backend feature flags as needed"
  echo
  echo "Pipeline (steps 1-3) complete."
  exit 0
fi

step "4/5 Train ($SUPERBATCHES superbatches, backend=$BACKEND)"

# The trainer's tak_utils.rs embeds checkpoints/test-240b/quantised.bin via include_bytes! (for its
# unused inference helpers), so that file must exist for the trainer to *build*. Same arch/size as
# the engine's net, so bootstrap it from there if absent.
embed="$BULLET_DIR/checkpoints/test-240b/quantised.bin"
if [ ! -f "$embed" ]; then
  [ -f "$ENGINE_DIR/src/quantised.bin" ] || die "trainer needs $embed but engine net is missing too"
  mkdir -p "$(dirname "$embed")"
  cp "$ENGINE_DIR/src/quantised.bin" "$embed"
  echo "Bootstrapped trainer embed net: $embed"
fi

case "$BACKEND" in
  cuda)     FEATURES=(--no-default-features --features cuda);;
  hip)      FEATURES=(--no-default-features --features hip);;
  hip-cuda) FEATURES=();;   # trainer default
  cpu)      FEATURES=(--no-default-features --features cpu);;
  *) die "unknown backend: $BACKEND";;
esac

( cd "$BULLET_DIR" \
  && TAK_DATA="$DATA_FILE" TAK_SUPERBATCHES="$SUPERBATCHES" TAK_BATCHES="$BATCHES" \
     cargo run -p bullet_lib --release --example tak ${FEATURES[@]+"${FEATURES[@]}"} )

# 5. Install the new net and rebuild -----------------------------------------
step "5/5 Install new network"
net=$(ls -t "$BULLET_DIR"/checkpoints/*/quantised.bin 2>/dev/null | head -n1 || true)
[ -n "$net" ] || die "no quantised.bin produced under $BULLET_DIR/checkpoints (quantisation may have overflowed)"
echo "Newest checkpoint: $net"

if [ "$SMOKE" -eq 1 ]; then
  echo "SMOKE MODE: not installing the toy net. Pipeline validated end-to-end."
  exit 0
fi

dest="$ENGINE_DIR/src/quantised.bin"
if [ -f "$dest" ]; then
  cp "$dest" "$dest.prev"
  echo "Backed up previous net to $dest.prev"
fi
cp "$net" "$dest"

step "Rebuild engine with new network"
# touch the file that embeds the net so cargo re-runs include_bytes! on the new quantised.bin.
( cd "$ENGINE_DIR" && touch src/eval/incremental.rs && cargo build --release )

mins=$(( ($(date +%s) - start) / 60 ))
echo
echo "DONE in ${mins} min. New net installed at src/quantised.bin (previous saved as quantised.bin.prev)."
echo "Sanity-check: printf 'tei\nteinewgame 6\nposition startpos\ngo depth 8\n' | $ENGINE_EXE"
