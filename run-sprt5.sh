#!/usr/bin/env bash
#
# 5x5 strength test via racetrack, on the black-stack TPS opening book (komi 0).
#
# Default mode: a fixed round-robin **Elo run, topaz (NNUE5) vs tiltak** — measures how close the new
# 5x5 net is to the strongest 5x5 engine. Each opening is played twice (both colours), so games =
# 2 x openings.
#
# net-vs-net regression SPRT (for iterating): set ENGINE_B=/path/to/other/topaz and SPRT=1. Build the
# "other" topaz by copying the comparison net to src/quantised5.bin, `cargo build --release`, and
# saving the binary under a distinct name first.
#
# racetrack speaks TEI: topaz's default mode is TEI; tiltak's `tei` binary is TEI. Komi is set ONLY
# via --komi (racetrack forbids option.HalfKomi). Both engines play standard Tak from the TPS book
# positions (move_num >= 2), so the forced 2xx opening is never replayed.
set -euo pipefail

OPENINGS=${OPENINGS:-2000}        # distinct openings in the book
TC=${TC:-8+0.8}                   # time+increment seconds, per game per engine
CONCURRENCY=${CONCURRENCY:-24}    # parallel games (only one engine thinks per game; leave a core or two)
KOMI=${KOMI:-0}                   # 0 for both the black-stack variant and standard-0-komi
PTNOUT=${PTNOUT:-sprt5-games.ptn}

# STANDARD=1 switches to the standard-Tak opening book (single-flat, 3 archetypes); default is the
# black-stack book (forced 2xx, 5 archetypes). This sets the default BOOK and the openings5 flag used
# to (re)generate it, so net-vs-net SPRT for the standard net auto-uses the right book.
STANDARD=${STANDARD:-0}
if [ "$STANDARD" = "1" ]; then
  OPENING_FLAG="--standard"; VARIANT="standard"; BOOK_DEFAULT="5s_standard_openings.tps"
else
  OPENING_FLAG="";           VARIANT="black-stack"; BOOK_DEFAULT="5s_black_stack_openings.tps"
fi
BOOK=${BOOK:-$BOOK_DEFAULT}

ENGINE_DIR=${ENGINE_DIR:-/workspace/topaz-tak}
TOPAZ=${TOPAZ:-$ENGINE_DIR/target/release/topaz}
TILTAK=${TILTAK:-/workspace/tiltak/target/release/tei}
RACETRACK_SRC=${RACETRACK_SRC:-/workspace/racetrack}
RACETRACK=${RACETRACK:-$RACETRACK_SRC/target/release/racetrack}

# net-vs-net SPRT (optional)
ENGINE_B=${ENGINE_B:-}
SPRT=${SPRT:-0}
ELO0=${ELO0:-0}
ELO1=${ELO1:-10}

say() { printf '\n=== %s ===\n' "$*"; }

cd "$ENGINE_DIR"

say "Build release topaz"
cargo build --release

if [ ! -x "$RACETRACK" ]; then
  say "Building racetrack from $RACETRACK_SRC"
  ( cd "$RACETRACK_SRC" && cargo build --release )
fi
[ -x "$RACETRACK" ] || { echo "ERROR: racetrack binary not found at $RACETRACK"; exit 1; }
[ -x "$TILTAK" ]   || { echo "ERROR: tiltak tei binary not found at $TILTAK (build: cd ../tiltak && cargo build --release --features smol --bin tei)"; exit 1; }

# (Re)generate the opening book if missing or too small. openings5 is random, so a fresh book is fine.
if [ ! -s "$BOOK" ] || [ "$(wc -l < "$BOOK")" -lt "$OPENINGS" ]; then
  say "Generating $OPENINGS-opening $VARIANT book -> $BOOK"
  ./target/release/topaz openings5 $OPENING_FLAG -n "$OPENINGS" -t "$BOOK" -p "${BOOK%.tps}.ptn"
fi
book_count=$(wc -l < "$BOOK")
GAMES=$(( book_count * 2 ))   # both colours per opening
say "book = $BOOK ($book_count openings) | tc=$TC | concurrency=$CONCURRENCY | $GAMES games"

if [ "$SPRT" = "1" ] && [ -n "$ENGINE_B" ]; then
  # Baseline (ENGINE_B) is listed FIRST so racetrack treats it as "base"; the current candidate
  # ($TOPAZ) is listed second as "under test". A positive Elo / "SPRT passed" then means the
  # candidate improved over the baseline.
  say "net-vs-net SPRT: baseline $ENGINE_B  vs  candidate $TOPAZ   (elo0=$ELO0 elo1=$ELO1)"
  "$RACETRACK" -s 5 --komi "$KOMI" -c "$CONCURRENCY" -g "$GAMES" \
    --book "$BOOK" --book-format tps --ptnout "$PTNOUT" \
    --format sprt --sprt elo0="$ELO0" elo1="$ELO1" alpha=0.05 beta=0.05 \
    --engine path="$ENGINE_B" \
    --engine path="$TOPAZ" \
    --all-engines tc="$TC"
else
  say "net-vs-tiltak Elo run: $TOPAZ  vs  $TILTAK"
  "$RACETRACK" -s 5 --komi "$KOMI" -c "$CONCURRENCY" -g "$GAMES" \
    --book "$BOOK" --book-format tps --ptnout "$PTNOUT" \
    --engine path="$TOPAZ" \
    --engine path="$TILTAK" \
    --all-engines tc="$TC"
fi
