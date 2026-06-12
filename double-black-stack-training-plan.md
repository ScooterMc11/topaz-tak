# Plan: "Black stack setup" opening rule

## Context

This engine currently offsets White's first-player advantage with **komi** (a flat-count
bonus for Black; the README/playtak default is 2 flats = 4 half-flats). The user wants to
experiment with a different offset mechanism on the local `double-black-stack` branch:

- **New opening rule.** On **White's first move only**, instead of placing a single black
  flat (the standard swap), White places a **stack of two black flats** on one empty square.
  This is the only time in the game a 2-piece stack is placed in a single action. Both flats
  are drawn from **Black's reserve** (the swapped pieces are Black's, consistent with how the
  standard first move already debits Black). Black's first move is **unchanged** (normal swap:
  one white flat).
- **Komi = 0** everywhere.
- **PTN notation.** The double placement is written `2<square>` (e.g. `2a1`) — a leading `2`,
  no piece letter (flats), no direction.

The goal is to test whether a controlled height-2 black stack + 0 komi is a better FPA offset
than komi. The change is mechanically small: it only alters ply 0; afterward the position is a
normal, fully-representable Tak position (a height-2 black stack early).

## Encoding approach (validated)

Represent the double placement as a **place move** (`GameMove`, `src/move_gen.rs:178`) with the
placement-piece nibble = `WhiteFlat` **and** the `number()` field set to `2`
(`set_number(2)`). This keeps `is_place_move() == true`, `is_stack_move() == false`, and yields a
u32 distinct from a single placement. Verified safe: every `.number()` read in the codebase is
gated behind the stack-move branch (`board.rs` do/reverse, `move_order.rs:193-214`,
`ptn.rs` to_ptn stack branch), `is_valid` (`move_gen.rs:351`) only reads number for stack moves,
the symmetry transforms (`rotate`/`flip_*`, mask `0xFFFF_FF00`) preserve bits 8-11, and the TT
stores the full 4-byte move (no truncation). No collisions in move-ordering history (the double
placement only occurs at the unique ply-0 position).

## Implementation steps

1. **`generate_all_place_moves`** — `src/move_gen.rs:62-75`. In the `move_num()==1` block, split by
   side: for **White** emit `GameMove::from_placement(Piece::WhiteFlat, index).set_number(2)` per
   empty tile; for **Black** keep the existing single `BlackFlat` placement. This is the only ply-0
   generation site (road/threat generators never select White's first move).

2. **`do_move`** place branch — `src/board.rs:586-598`. After computing the swapped `piece`, if
   `swap_pieces && m.number() == 2`: `push` the swapped flat **twice** onto `board[src_index]` and
   decrement `flats_left[piece.owner()]` (Black) by **2**. Otherwise keep the single push / −1.
   Reuse `Stack::push` (`stack.rs:113`).

3. **`reverse_move`** place branch — `src/board.rs:551-557`. Discriminate on
   `m.is_place_move() && m.number() == 2` (NOT on `move_num`, which reverse_move has already mutated).
   When matched: `Stack::pop` (`stack.rs:180`) **twice** and increment the popped owner's (Black)
   reserve by **2**. Symmetric with step 2.

4. **`legal_move`** — `src/board.rs:151-167`. The double placement (`WhiteFlat`, owner == side_to_move
   at ply 0) already passes the existing guards. Add a defensive check: if the move is a flat placement
   with `number() == 2`, also require `pieces_reserve(Color::Black) >= 2`.

5. **`try_from_ptn_m`** placement branch — `src/move_gen/ptn.rs:197-208`. When there is no direction:
   if `pieces == Some(2)` return `Self::from_placement(Piece::flat(active_player), square).set_number(2)`;
   if `Some(1)`/`None` keep the existing single placement (incl. S/C arms); for any other leading count,
   or count==2 combined with S/C, return `None`.

6. **`to_ptn`** place branch — `src/move_gen/ptn.rs:9-15`. In the flat arm, if `self.number() == 2`
   return `format!("2{}", square)`; leave wall/cap arms unchanged.

7. **Komi → 0** — `src/topaz.rs`: `const PLAYTAK_KOMI: u8 = 4` → `0` (line ~771; flows to the
   playtak board setups and the Seek string); `with_komi(4)` → `with_komi(0)` at lines ~76, ~110, ~222.
   Leave `src/lib.rs:82` (GameInitializer default, already 0) and `board.rs` `komi_flat_game` test
   (`with_komi(5)`) untouched.

## Critical files

- `src/move_gen.rs` — move generation + `GameMove` encoding
- `src/board.rs` — `do_move` / `reverse_move` / `legal_move`
- `src/move_gen/ptn.rs` — PTN parse/print
- `src/topaz.rs` — komi constants
- `src/board/stack.rs` — `push`/`pop` (reused, no change expected)

## Edge cases / caveats

- **TEI `position ... moves 2a1`** replays through `try_from_ptn` (`topaz.rs:638` et al.) → covered by
  steps 5 + 2. **No code change needed** beyond those steps.
- **Opening book** (`src/search/book.rs`): existing first-move keys are single moves like `a1`, so they
  won't match the new `2a1` opening. Lookup simply returns `None` and the engine searches — correctness
  is fine, but the bundled book is effectively unused for the new opening until regenerated. No change.
- **NNUE eval**: trained on standard openings; its read of an early height-2 black stack / 0 komi may be
  miscalibrated. The position is fully representable (no crash) — accuracy caveat only, inherent to the
  experiment.
- **Flat scoring** (`bitboard.rs` `flat_score`) counts only top-of-stack flats, so the 2-stack
  contributes 1 to Black's flat score (correct Tak); both flats leaving Black's reserve shortens the
  game — the intended FPA-offset mechanism.

## Verification

- **`cargo build --release`** must succeed (note: requires `src/quantised.bin`, which is present).
- **Existing tests expected to still pass** (`cargo test`): `ptn_equivalence`, `basic_perft`
  (starts from a mid-game position, never hits ply 0), `komi_flat_game` (explicit `with_komi(5)`),
  `playtak_move`, `rotation`, `book::get_book_move`.
- **New tests** to add in `ptn.rs` test module:
  - Round-trip: `try_from_ptn_m("2a1", 6, Color::White)` → `is_place_move() && !is_stack_move()
    && number()==2 && place_piece()==WhiteFlat`; and `to_ptn::<Board6>() == "2a1"`.
  - Negative: `try_from_ptn_m("3a1", 6, Color::White)` → `None`.
  - do/reverse symmetry from `Board6::new()`: every generated White first move has `number()==2`
    (no single flat present); after `do_move`, `flats_left[Black] == FLATS-2`, `flats_left[White]
    == FLATS`, and `board[a1]` is height 2 of `BlackFlat`; after `reverse_move`, reserves and board
    are restored.
- **Manual end-to-end** (human-run): in TEI mode send `position startpos` + `go depth 6`, confirm
  `bestmove` is a `2xx` double placement; then `position startpos moves 2d3` + `go` yields a legal
  single white-flat Black reply.

---

# Part 2 — Calibrating the NNUE for the black-stack rule (data-gen tooling + retrain roadmap)

> Part 1 (above) is **implemented and verified**. Part 2 is a forward-looking plan; the full
> retrain itself is intended for a separate session.

## Context

The net (`src/quantised.bin`) was trained on standard-opening, komi-2 self-play, so it is
miscalibrated for the new rule. Two findings make this cheap conceptually:
- **Komi is not a network input** — `TakSimple6::handle_features` uses only piece-squares +
  reserves + side-to-move; komi is applied separately in `src/eval.rs` flat-scoring/endgame. So
  komi=0 changes only the **WDL training labels**, not the inputs.
- **The new opening only changes ply 0.** Every later position is a normal, in-distribution Tak
  position. So **no architecture or feature change is needed** — calibration = retrain/fine-tune on
  self-play data generated under the new rules (forced `2xx` opening + komi 0).

The matching trainer is confirmed at `C:\Users\lance\Desktop\Tak\Bots\bulletTrainer\bullet`
(`examples/tak.rs` + `examples/tak_utils.rs`): identical feature map, `Network` layout, QA/QB/SCALE,
SCReLU, dual-perspective + PSQT, and `save_format` that emits exactly `quantised.bin`. The only
missing piece is a data **generator** in this engine that writes the trainer's input format.

## What exists vs. missing (engine side)

- Have: `selfplay` subcommand (deterministic, stdout-only), `build_nn_repr` (`src/eval.rs` → `BoardData`),
  `do_random_move` (`src/board.rs:791`, unused), the `random` cargo feature, `SearchOutcome.score()`.
- Missing: multi-game loop, randomized openings wired in, WDL labeling, and **file export in the
  trainer's binpack format**.

## Target output: the "TAK6" binpack format (from `tak_utils.rs`)

- File = sequence of **Chunks**. Chunk = `ChunkHeader { magic: u32 BE "TAK6", chunk_size: u32 }`
  followed by `chunk_size` body bytes.
- Body = sequence of **Entries**. Entry = `EntryHeader` + `data_len`×`PSquare(u8)` +
  `plys_len`×`MoveList(4B)`.
- `EntryHeader` (`#[repr(C)]`, 10 bytes): `caps:[u8;2], white_to_move:u8, extra_score:u8,
  score:i16(LE), result:u8, komi:u8, data_len:u8, plys_len:u8`.
- `PSquare(u8)` = `square | (validpiece<<6)` (validpiece 2-bit: 0 WhiteFlat, 1 BlackFlat, 2 WhiteWall,
  3 BlackWall; caps stored as flat with the square recorded in `caps[]`) — identical packing to the
  engine's `build_piece_square`/`PieceSquare`.
- Labels: `result` 0/1/2 → WDL 0/0.5/1 (÷2), STM-relative (reader flips by ply parity); `score` is the
  STM-relative eval. Trainer filters `|score| > 9999`.
- **Key limitation:** the reader's multi-ply (`MoveList`) replay only supports *placement* moves (no
  spreads). → **Emit one Entry per position with `plys_len = 0`** (full board snapshot via PSquares).
  Robust and simplest.

## In-repo tooling (recommended) — `topaz-tak`

Add a `datagen` subcommand in `src/topaz.rs` (mirror the `tinue` getopts pattern), backed by a new
`src/datagen.rs`:
1. Loop over N games (multithreaded, one game per thread). Each game starts from
   `Board6::new().with_komi(0)`; White's forced first move is already the `2xx` double placement.
2. **Randomized opening:** for the first k plies (e.g. 2–8, after ply 0), call `board.do_random_move(rng)`
   for diversity, then play to completion with a **fast fixed search** (low fixed depth or
   `SearchInfo::set_max_nodes`, e.g. a few thousand nodes). Record each non-random position's
   STM-relative `outcome.score()`.
3. On game end, map `game.game_result()` to a white-relative {0,1,2}; set each stored position's
   `result` from its STM perspective (match the reader's parity convention).
4. Serialize per position: `build_nn_repr(&board)` → `BoardData`; write `EntryHeader{ caps,
   white_to_move, extra_score:0, score, result, komi:0, data_len, plys_len:0 }` then the `data_len`
   PSquare bytes. Batch entries into ~8 KB chunks with the `TAK6` `ChunkHeader`; append to the output
   `.bin`. Copy the `EntryHeader`/`ChunkHeader`/`PSquare` struct definitions verbatim from
   `tak_utils.rs` so the byte layout matches exactly (reuse the existing `bytemuck` dep or manual LE writes).
- Critical files: `src/topaz.rs` (subcommand), `src/eval.rs` (`build_nn_repr`, reuse), `src/board.rs`
  (`do_random_move`), new `src/datagen.rs` (structs + serializer).

## Training roadmap (external — `bulletTrainer/bullet`)

1. In `examples/tak.rs`, point `file_path` (currently a hardcoded Linux path
   `/media/justin/SSD.../data.bin`) at the generated `.bin`. Leave inputs/arch/`save_format` unchanged.
2. Tune `TrainingSchedule` to dataset size. Full retrain: from-scratch init, superbatches scaled to
   data volume. (Fine-tune variant: init from the current net, fewer superbatches, lower LR.)
3. Run `cargo run --release --example tak` (needs the configured CUDA/HIP or CPU backend). Output:
   `checkpoints/<net_id>/quantised.bin`.
4. Deploy: copy that file → `topaz-tak/src/quantised.bin`; `cargo build --release`. Sanity-check via
   TEI and an A/B match vs. the old net.

## Efficiency options (for the eventual full retrain)

- **Fine-tune first (cheapest win):** since only ply 0 + WDL labels changed, fine-tuning the existing
  net on a modest new-rules dataset (tens of millions of positions) likely closes most of the gap —
  good ROI before committing to a ~1B-position from-scratch run.
- **Datagen dominates cost:** use fixed shallow node counts (not deep search), many parallel game
  threads, and a few random opening plies instead of building a book.
- **Re-use & re-label old data:** positions from ~move 2 on are rule-agnostic except WDL under 0 komi;
  much existing standard-rule self-play can be reused if terminal/flat-decided results are **re-scored
  for komi 0**. This can cut from-scratch datagen drastically.
- **Don't pre-augment:** the loader already applies random 8-fold board symmetry (`shuffle_and_rotate`).

## Verification

- **Format round-trip test:** generate a handful of positions, decode them with `tak_utils.rs`'s
  `CompressedTrainingDataEntryReader` (or a port), assert the decoded `BoardData` matches
  `build_nn_repr` of the source board — a cheap guard against byte-layout drift.
- **Pipeline smoke test:** generate a small `.bin` (~10k positions), train 1–2 superbatches, confirm
  loss decreases and a `quantised.bin` of exactly `size_of::<Network>()` bytes is produced; drop it
  into the engine and confirm it loads (the `include_bytes!` size assert) and plays.
