#!/usr/bin/env bash
# startPositionEvaluator: evaluate each 6x6 opening archetype across search depths (0..maxDepth) for
# both the black-stack ruleset (double-black-stack opening, komi 0) and the 2-komi ruleset (1-1 swap,
# 2 komi), then render one PNG comparing the eval-vs-depth curves + a per-depth value table.
#
# Usage:  ./startPositionEvaluator.sh [-d|--maxDepth N] [-t|--threads N]
#   -d, --maxDepth N   Maximum (ceiling) search depth; the curve runs 0..N. Default 30.
#   -t, --threads  N   Worker threads (default: auto, capped at the 7 positions).
#   -h, --help         Show this help.
#
# Output: startPositionEvaluator/results/start_eval_d<N>_<stamp>.{json,png}
#
# Notes:
#   * Each of the 7 positions is ONE single-threaded iterative-deepening search; deep runs take hours
#     and print live progress (depth / score / nodes-per-second) to the terminal.
#   * The engine is built into a private target dir (.build/) so this never disturbs a running bot
#     ladder that holds a lock on the main target/release/topaz.exe.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BUILD_DIR="$SCRIPT_DIR/.build"
RESULTS_DIR="$SCRIPT_DIR/results"
RENDERER="$SCRIPT_DIR/render_start_eval.py"

MAX_DEPTH=30
THREADS=""

is_uint() { [[ "$1" =~ ^[0-9]+$ ]]; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    -d|--maxDepth)
      [[ $# -ge 2 ]] || { echo "error: $1 needs a value" >&2; exit 2; }
      is_uint "$2" || { echo "error: --maxDepth must be a non-negative integer, got '$2'" >&2; exit 2; }
      MAX_DEPTH="$2"; shift 2 ;;
    -t|--threads)
      [[ $# -ge 2 ]] || { echo "error: $1 needs a value" >&2; exit 2; }
      is_uint "$2" || { echo "error: --threads must be a non-negative integer, got '$2'" >&2; exit 2; }
      THREADS="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)
      echo "error: unknown argument '$1' (see --help)" >&2; exit 2 ;;
  esac
done

# Pick a python interpreter.
PYTHON=""
for cand in python python3; do
  if command -v "$cand" >/dev/null 2>&1; then PYTHON="$cand"; break; fi
done
[[ -n "$PYTHON" ]] || { echo "error: no python interpreter found (need python or python3)" >&2; exit 1; }

echo "==> Building topaz (isolated build dir, won't touch a running bot ladder) ..."
( cd "$REPO_ROOT" && cargo build --release --bin topaz --target-dir "$BUILD_DIR" )

BIN="$BUILD_DIR/release/topaz.exe"
[[ -f "$BIN" ]] || BIN="$BUILD_DIR/release/topaz"
[[ -f "$BIN" ]] || { echo "error: built binary not found under $BUILD_DIR/release" >&2; exit 1; }

mkdir -p "$RESULTS_DIR"
STAMP="$(date +%Y%m%d_%H%M%S)"
BASE="start_eval_d${MAX_DEPTH}_${STAMP}"
JSON="$RESULTS_DIR/$BASE.json"
PNG="$RESULTS_DIR/$BASE.png"

echo "==> Running starteval to depth $MAX_DEPTH (live progress below) ..."
ARGS=(starteval -d "$MAX_DEPTH" -o "$JSON")
[[ -n "$THREADS" ]] && ARGS+=(-t "$THREADS")
"$BIN" "${ARGS[@]}"

echo "==> Rendering PNG ..."
"$PYTHON" "$RENDERER" "$JSON" "$PNG"

echo ""
echo "Done."
echo "  JSON: $JSON"
echo "  PNG:  $PNG"
