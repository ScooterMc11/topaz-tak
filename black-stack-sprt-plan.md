# Black-Stack SPRT — Continuation Plan

## Project context
Experiment in **Topaz** (6x6 Tak): replace komi as the first-player-advantage offset with a
**"black stack" opening** — White's first move places a 2-high stack of black flats instead of the
standard single swapped flat, played at **komi 0**. Implemented on the local `double-black-stack`
branch (not pushed). Engine repo: `C:\Users\lance\Desktop\Tak\Bots\topaz-tak`.

A black-stack NNUE was trained by bootstrapping off Topaz's original net, generating self-play data
under the new rule, training in **bullet** (`../bulletTrainer/bullet`), and iterating. Balance was
assessed via self-play (`topaz balance`, in-distribution: `-g 20000 -n 15000 -r 6`):
- iter-1: **−2.6%** White−Black, iter-2: **−4.5%** — stable, near-balanced, small (~+10–15 Elo) Black lean.
- Earlier alarming numbers (−53%, −33%) were measurement artifacts (odd/asymmetric random opening
  plies; out-of-distribution corner testing), since corrected by using even random plies and testing
  in the net's training distribution.

## Balance result history (for reference)
All runs: `topaz balance`, 20,000 games, 15k nodes/move. "W−B" = White% − Black% (negative = Black favored).

| Net | Opening settings | White | Black | Draw | W−B |
|-----|------------------|-------|-------|------|-----|
| iter-1 | `-r 3` (random, **odd → asymmetric** plies) | 15.5% | 68.4% | 16.0% | **−52.9%** |
| iter-1 | `--corner -r 4` (**out-of-distribution**) | 24.1% | 57.1% | 18.9% | **−33.0%** |
| iter-1 | `-r 6` (balanced, **in-distribution**) | 38.0% | 40.6% | 21.4% | **−2.6%** |
| iter-2 | `-r 6` (balanced, in-distribution) | 37.2% | 41.7% | 21.0% | **−4.5%** |

Takeaways: the first two numbers were measurement artifacts — an odd random-ply count hands one side
an extra opening move, and testing corner openings with a net trained on random/central openings is
out-of-distribution. The valid measurement is **in-distribution with an even random-ply count**. Win
types run ~70% flat throughout. The figure is stable across iterations at a small Black lean (~3–5%,
≈ +10–15 Elo) — i.e. essentially balanced, with a slight Black tilt.

## Current goal: SPRT (new net vs original net)
A Topaz dev recommended validating the trained net with an **SPRT** — confirm the black-stack net is
genuinely *stronger* at the black-stack variant than the original net (i.e., training actually
improved it, not just changed it). This is **strength** testing (net vs net), distinct from the
**balance** measurement (White vs Black). It also serves as the convergence check for any future
training iterations.

- **Runner: `racetrack`** — the Tak/TEI match tool (cloned as a sibling of `topaz-tak`). cutechess is
  chess-only and cannot be used.
- **Two nets to compare** (both in `topaz-tak/src/`, each exactly **1,053,760 bytes** =
  `size_of::<Network>()`):
  - `quantised-2-komi.bin` — original standard net (komi-2 trained).
  - `quantised-black-stack-iter-2.bin` — the black-stack iteration-2 net.

## How the engine works for this (already verified)
- The black-stack rule lives in the **source**, so **every build plays it** — only the embedded net
  differs. The net is embedded at compile time via `include_bytes!("../quantised.bin")` in
  `src/eval/incremental.rs`. To build with a specific net: **copy it to `src/quantised.bin`, then
  `cargo build --release`.** If cargo doesn't detect the `.bin` change, `touch src/eval/incremental.rs`
  first to force re-embed.
- **TEI mode is runner-ready** (verified end-to-end): `tei`→`teiok`, `isready`→`readyok`,
  `teinewgame 6`, `position startpos [moves ...]`, timed `go` (wtime/btime/winc/binc or movetime),
  `bestmove`. It plays the black-stack rule from startpos (e.g. `bestmove 2f1`) and replays
  black-stack opening lines via `position startpos moves 2a1 ...`. Komi via
  `setoption name HalfKomi value 0` (0 is already the default).
- **Gotcha:** `position tps <bad-tps>` panics the engine thread (`get_board_tps` unwraps). The runner
  must send valid positions — prefer **move-list openings** over TPS.

## Plan for the new session

### Step 1 — Analyze the racetrack repo
Read racetrack's source (sibling of topaz-tak). Determine concretely:
- How it launches/configures engines (the TEI invocation, working dir, and how to pass per-engine
  options — especially setting **HalfKomi/komi to 0**).
- Whether/how it runs **SPRT** (bounds, alpha/beta, output) and the exact CLI/flags.
- Its **opening-book format** (move-list lines? TPS? a specific file format?) and whether each opening
  is played twice with colors reversed.
- Time-control configuration.

### Step 2 — Build a black-stack opening-book generator
**STOP — do not start building this until the user has given specific guidelines** for the positions
to generate (how many plies deep, how openings are chosen/constrained, any distribution or
de-duplication requirements, output format details, etc.). Ask for those guidelines first.

Once the user has specified the guidelines: add a small engine subcommand (e.g.
`topaz openings -n N -r K`) that emits N varied, **legal black-stack** opening lines in racetrack's
format. **CRITICAL:** a standard Tak opening book is *illegal* here — every line must begin with the
forced `2xx` double placement. Reuse the random-opening logic already in `src/balance.rs` /
`src/datagen.rs` (`do_random_move` + the forced double placement at ply 0). Default to an **even**
number of random plies so colors get equal random moves (unless the user's guidelines say otherwise).

### Step 3 — Produce the two builds
Script it: copy each net → `src/quantised.bin`, build, save the binary under a distinct name (e.g.
`topaz-blackstack` and `topaz-2komi`). Both play the black-stack rule; only the net differs. (Restore
`src/quantised.bin` to a known net afterward — the working copy is gitignored and not in version
control.)

### Step 4 — Run the SPRT
Configure racetrack with the two binaries, **komi 0**, the black-stack opening book, alternating
colors (each opening both ways), a fast time control, and SPRT bounds `[0, 5]` or `[0, 10]` Elo at
α = β = 0.05. "Pass" = the black-stack net is stronger at the variant.

## References
- Engine: `C:\Users\lance\Desktop\Tak\Bots\topaz-tak` (branch `double-black-stack`, not pushed)
- Trainer: `C:\Users\lance\Desktop\Tak\Bots\bulletTrainer\bullet`
- racetrack: sibling of `topaz-tak`
- Nets: `topaz-tak/src/quantised-2-komi.bin`, `topaz-tak/src/quantised-black-stack-iter-2.bin`
- Existing tooling: `topaz datagen | checkdata | trimdata | balance`; `run-training.{sh,ps1} --shards`
- Build: `cargo build --release` (needs `src/quantised.bin` present); Windows binary `target/release/topaz.exe`
- Auto-memory files cover deeper details: `black-stack-setup-experiment`, `nnue-trainer-and-format`,
  `move-generation-two-paths`.
