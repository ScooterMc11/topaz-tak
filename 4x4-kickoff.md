# 4×4 NNUE + Balance Effort — Kickoff

## Goal
Repeat, for **4×4**, the full arc we completed for 5×5 (and earlier 6×6): build a 4×4 NNUE net for
the topaz engine, then measure and compare **opening/rule balance** (standard vs. a first-player-offset
variant) under strong self-play, and render per-archetype result **PNG sheets**. Plain PNGs only —
do *not* build a combined results.html (that was tried and dropped).

## Read these first (the playbook already exists)
This is largely a **port of the 5×5 work**. Before designing anything, read:
- Auto-memory **`5x5-nnue-effort`** — the detailed, step-by-step record of the whole 5×5 pipeline,
  the decisions made, and every gotcha. This is your primary reference.
- Auto-memories `black-stack-setup-experiment`, `nnue-trainer-and-format`, `racetrack-tps-book-requirement`,
  `move-generation-two-paths`.
- `AGENTS.md` (engine orientation) and the existing **size-5 modules**, which are the templates to mirror:
  `src/eval/incremental5.rs`, `src/datagen5.rs`, `src/openings5.rs`, `src/balance5.rs`,
  the trainer's `bulletTrainer/bullet/examples/tak5.rs` + `tak_utils5.rs`, and the
  `run-datagen5.sh` / `run-sprt5.sh` / `run-balance5.sh` / `gen_sheets.py` scripts.

The working principle throughout was **duplicate-and-isolate** per size (a `*5.rs` set that never
perturbs the proven 6×6 path). With a third size, consider whether a small generic refactor is now
worth it — but isolation is the safe default and matches the existing structure.

## ✅ Resolved 4×4-specific findings (verified against code, 2026-06-23)
The four prerequisites were investigated against the actual source before committing. Results:
1. **No `Board4` / `Bitboard4` exists** — `board.rs:716-718` and `bitboard.rs:782-784` instantiate
   only sizes 5/6/7. **Real prerequisite.** Gotcha: the per-size bitboard magic constants
   (`TOP/BOTTOM/LEFT/RIGHT/INNER/LEFT_TOP/NS[..]/EW[..]` + `build_bit_to_index_table` /
   `build_index_to_bit_table`) are **hand-written per size in separate `impl BitboardN` blocks**
   (`bitboard.rs:239-420`), NOT generated from `$sz` by `bitboard_impl!`. So Board4 needs a
   hand-authored `impl Bitboard4` block. Layout = 8-wide padded u64 with a 1-cell border: 5×5 uses
   rows 2-6 / cols 1-5 (`TOP=0x3e0000`, `INDEX_TO_BIT` rows `17,18,19,20,21`…). For 4×4, mirror it
   into rows 2-5 / cols 1-4: `TOP=0x1e0000`, `BOTTOM=TOP<<24`, `NS`=4 entries, `EW`=4 entries,
   `INDEX_TO_BIT` = `17,18,19,20 / 25.. / 33.. / 41..`, `TABLE_LENGTH=16`. Then
   `bitboard_impl![Bitboard4, 4]`, a `Board4` struct + `board_impl![Board4, Bitboard4, 4, 15, 0]`,
   zobrist for 16 squares, and dispatch wiring. The black-stack rule is **size-generic** (lives in
   `generate_all_place_moves` keyed off `move_num()`, bookkeeping in the `board_impl!` macro), so
   **Board4 plays the variant for free** once it exists — exactly as Board5 did.
2. **15 flats, 0 capstones** — confirmed by the macro arg pattern `board_impl![Name,Bits,SIZE,FLATS,CAPS]`
   (Board5 = `21, 1`). So `board_impl![Board4, Bitboard4, 4, 15, 0]`. **0 caps is the real semantic
   change:** the NNUE size-N feature mapper encodes cap pieces, so the size-4 layout must *drop* the cap
   encoding (not just rescale). Also audit move-gen for any path that assumes `caps_left ≥ 1`
   (capstone placement, wall-crush via cap) — must no-op cleanly with an empty cap reserve.
3. **tiltak fully supports `S=4` — with trained weights.** Not just const-generic: real
   `VALUE_PARAMS_4S_{0,2}KOMI` + `POLICY_PARAMS_4S_*` arrays (`parameters.rs:703-717, 752+`) and full
   dispatch (`main.rs`/`playtak.rs`/`aws`). Allowed `HalfKomi` = **0 and 4** (= 0 and 2 full), same as
   5×5. ⇒ the entire 5×5 tiltak playbook transfers unchanged: **bootstrap data source**,
   **net-vs-tiltak SPRT benchmark**, and **balance cross-check** all available.
4. **Variant mechanism — decided (see "Decisions made" below):** double black stack, no komi.

## Milestone sequence (mirrors 5×5; tiltak 4×4 confirmed, so the path is the full 5×5 one)
0. **Prereqs — Board4 (the big one):** `Bitboard4` const block + `bitboard_impl![Bitboard4,4]`,
   `Board4` struct + `board_impl![Board4,Bitboard4,4,15,0]`, zobrist (16 sq), dispatch wiring; audit
   move-gen for `caps_left ≥ 1` assumptions (0-cap). Get Board4 playing legal black-stack games (perft
   / TEI smoke) before any NNUE work.
1. **Engine plumbing:** `incremental4.rs` (size-4 NNUE mapper, **0-cap: drop the cap encoding** — recompute
   `SQUARE_INPUTS`/offsets/`Network4` size), `build_nn_repr4`, `NNUE4` evaluator, zeroed placeholder
   `src/quantised4.bin` (compile-time size assert validates it), wiring + a net-agnostic smoke test.
2. **Trainer fork:** `examples/tak4.rs` + `tak_utils4.rs` in the bullet trainer (size-4 feature map +
   `Network`), registered as an example. Strip the vestigial inference code like tak5 did. Feature
   layout must be byte-identical to `incremental4.rs` (0-cap).
3. **Datagen + openings:** `datagen4` (tiltak-driven, book-mode + self-play) and `openings4`
   (corners a1/a4/d1/d4). **4×4 book design differs from 5×5 — see "Opening book design" below:**
   **4 plies** (not 6), **4 archetypes** (center-stack dropped), plies 3-4 = proximity-constrained
   flats (≤3 orthogonal steps from an own-colour piece).
4. **Train → SPRT (lighter first):** tiltak-bootstrap one net → train → **SPRT vs tiltak** + a sanity
   net-vs-net. Swap 4s dispatch to `NNUE4`. **Iterate to self-play v2+ ONLY if the bootstrap net is
   weak / tiltak's 4×4 eval proves limiting** (decision below). The self-play `datagen4self` loop is
   built but not necessarily run.
5. **Balance + sheets:** `balance4` (+ `--tiltak` cross-check) → `balance4 -a` for **both** variants at
   komi 0, **both using the black-stack net** (standard = play from a standard book at move_num≥2;
   standard rules are too imbalanced to train a usable native net — same finding as 5×5) →
   `gen_sheets.py` 4×4 configs → PNGs.

## Gotchas already paid for (don't re-discover these)
- **Key insight:** training on a *balanced* variant yields a better evaluator than training on the
  *imbalanced* one — even for playing the imbalanced one (saturated WDL labels starve the net). So the
  **strongest available net** (whatever it was trained on) tends to be the best instrument for
  measuring *both* variants' balance; a native "standard" net underperformed badly.
- **Net embedding:** after `cp <net> src/quantised4.bin`, run `touch src/eval/incremental4.rs` before
  rebuilding or cargo may keep the stale embedded net.
- **Pod tar extraction:** after `tar -xzf` on the pod, run `find topaz-tak -name '*.rs' -exec touch {} +`
  — tar restores old mtimes and cargo will otherwise reuse the old binary (this bit us as a silent
  "feature missing" failure).
- **Trainer `net_id` is hardcoded** → every training run overwrites `checkpoints/<net_id>-<sb>/`; `cp`
  the net out to a versioned name immediately after each run.
- **racetrack** lists the FIRST `--engine` as "base", the SECOND as "under test"; put the *baseline*
  first and the *candidate* second so positive Elo / "SPRT passed" = improvement. Komi is set ONLY via
  `--komi` (`option.HalfKomi` is rejected). It plays standard rules and **rejects the `2xx` opening**,
  so any variant match needs a **TPS book at move_num≥2** (the rule only fires at move 1).
- **tiltak HalfKomi** is a combo with limited allowed values (0 and 4 at 5×5) — verify for 4×4.
- **Opening ceiling:** `balance -a` plays each D4-unique opening once (deterministic), so the game
  count caps at the number of distinct openings (tiny on 4×4 — could be very small). Don't request
  more games than exist.
- **Balance score scale:** datagen's tiltak path maps tiltak `score cp` via `(cp+100)/200`→logit;
  topaz self-play stores its native eval directly (already on the trainer's eval_scale). Keep them
  distinct.

## Conventions
- **Repo org:** artifacts live under `topaz-tak/{5x5,6x6}/{nets,data,books,bins,results}` (add a
  parallel `4x4/`). **Source + all `run-*.sh`/`gen_sheets.py` stay in the repo root** (scripts derive
  the repo root from their own location). Naming: `<variant>-vN` (e.g. `quantised4-bs-v1.bin`,
  `topaz-4s-bs-v1`). `src/quantised*.bin` stay in `src/`.
- **Pod (RunPod):** everything under `/workspace`; repos as siblings (`topaz-tak`, `tiltak`,
  `bulletTrainer/bullet`, `racetrack`); persistent Rust toolchain in `/workspace`. Bundle to ship:
  `tar … --exclude=target --exclude=.git --exclude=topaz-tak/5x5 --exclude=topaz-tak/6x6
  --exclude='*.bin' --exclude='*.exe' --exclude='*.tps' --exclude='*.png' topaz-tak` (add `--exclude
  topaz-tak/4x4`). Extract recipe: `cd /workspace && tar --no-same-owner -xzf pod-topaz.tgz &&
  find topaz-tak -name '*.rs' -exec touch {} +`.
- `gen_sheets.py` runs locally (Windows + Pillow); add 4×4 config dicts with an `"engine"` badge field.

## Decisions made (2026-06-23, with the user)
- **Balancing mechanism = double black stack, NO komi.** User is certain komi is too weak an FPA
  balancer here. From their playtesting the double black stack is the most effective: Black's advantage
  grows on smaller boards at ~the same rate White's first-player advantage does, so the 2-flat offset
  *scales* with board size rather than overcorrecting. (So we mirror 5×5/6×6 exactly — disregard the
  earlier "may overcorrect" worry.)
- **"Standard" (no balancer) is overwhelmingly White-favored** and too imbalanced to train a usable net
  — so, exactly as at 5×5, **simulate standard games with the black-stack-trained net** (play from a
  standard opening book at move_num≥2; the black-stack rule only fires at move 1). Both balance
  measurements share the one strong black-stack net.
- **Net ambition = lighter first.** One tiltak-bootstrapped net → SPRT vs tiltak, then measure balance.
  Iterate to self-play v2+ only if that net is weak (tiltak's 4×4 eval strength is unproven).
- **No komi in the comparison** — both variants measured at komi 0 (so the only standard-4×4 komi
  question from the old plan is moot).

## Opening book design — 4×4 (decided 2026-06-23, with the user)
The 5×5 book (6 plies, 5 archetypes) does NOT port directly — 4×4 is too small (16 squares, fills
fast; 6 flats already decides positions and leaves almost no game). New 4×4 scheme:
- **4 plies, not 6.** Keeps openings neutral / undecided on a tiny board. ply 1 = forced `2xx`
  black-stack at `w1` (archetype); ply 2 = swapped white flat at `b2` (archetype); plies 3-4 =
  proximity-constrained flats.
- **4 archetypes, center-stack DROPPED.** Keep: diagonal corners, adjacent corners, hug, gap-hug.
- **Plies 3-4 = proximity rule kept (for now):** ply 3 (white) ≤3 orthogonal (Manhattan) steps from a
  white piece (= `b2`); ply 4 (black) ≤3 steps from a black piece (= `w1`). No interior-only plies
  (the 5×5 plies 3-4) — at 4×4 the interior is just the 2×2 `b2..c3`, too restrictive.
- Colour swap (same as 5×5): `w1` = black 2-stack, `b2`/p3 = white flats, p4 = black flat. TPS at
  move_num≥2 so racetrack/tiltak (standard rules) accept it.

**Enumerated D4-unique ceiling for this exact scheme = 207 positions** (verified by full enumeration).
Per-archetype standalone: diagonal 39, adjacent 60, hug 70, gap-hug 74 (overlapping; disjoint
round-robin shares sum to ≤207, ~50 each). Sensitivity: dropping the proximity rule → 506; within-2 →
67; within-1 → 10; the 6-ply port would have been 3319. **207 is the `balance -a` game cap** (each
D4-unique opening played once, deterministically). Implications: **ample for the headline
standard-vs-black-stack comparison** (5×5 effect was +83.6% vs +0.4% White — a ±7% CI on 207 games
trivially separates that), **thin for per-archetype CIs** (~50 games/bucket ≈ ±14%) and **coarse for
SPRT** (207 openings ×2 colours = 414 games vs 5×5's 4000). If more headroom is wanted later, the cheap
lever is dropping the proximity rule (→506, since at 4×4 within-3 already covers 10/16 squares from a
corner and barely constrains), NOT adding plies.

## Still to decide as we build
- Whether to keep or drop the within-3 proximity rule once we see real book/balance behaviour
  (kept for now; dropping ≈2.4× more positions at little cost to neutrality).

Board4 (milestone 0) is the critical path and has no 5×5 template — build and validate it first
(perft + a legal-black-stack TEI smoke), then the rest follows the size-5 modules line-by-line.
