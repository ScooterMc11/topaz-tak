//! Whole-game analysis for Takana's `/analyze` game sheet.
//!
//! Walks a game's plies from the initial position, evaluates every resulting position with a fixed
//! **node** budget, and prints one JSON document with a per-ply evaluation plus the game statistics
//! the sheet's stats table needs. One process, one net, one pass — the bot shells out once per
//! `/analyze` and parses stdout.
//!
//! **Evaluation.** Every position is scored by a fresh fixed-node search whose transposition table is
//! fully cleared first, exactly as `balanced_lines::eval_white` does: the score is then a pure
//! function of the position, so the same game always produces the same sheet. Scores are reported in
//! a fixed **White perspective** (`search` returns the score for the side to move, so the sign is
//! flipped when Black is to move), as internal cp, human cp (`internal / 4`), and as a win
//! probability `sigmoid(internal / 400)` — the engine's own trained scale, shared with the trainer
//! (`datagen.rs`). Terminal positions are pinned to 1 / 0 / ½ rather than searched.
//!
//! A position's reported value is **not** its deepest completed iteration but a weighted average of
//! the last few (`DEPTH_KERNEL`), because a fixed node budget stops adjacent positions at different
//! depths and the difference between two such readings is partly the searches disagreeing rather than
//! the positions differing. `search_depth` still reports the deepest iteration that completed, so it
//! names the top of that window rather than the depth of a single reading.
//!
//! **Statistics** (one column per player in the sheet):
//!   - `calls` — plies that put the opponent *into* tak. Only transitions count: a ply is a call when
//!     the mover had no road threat coming into it and has one after it. A **game-winning move is
//!     never a call** — the opponent gets no turn, so there is no threat for them to answer. A game
//!     that only *stops* on its last ply (resignation, timeout) is not special: the opponent had
//!     their chance, so that ply counts like any other.
//!   - `flat_placements` / `wall_placements` / `movements` — capstone placements are deliberately
//!     counted separately and are *not* folded into either placement row (the sheet has no capstone
//!     row at all; `cap_placements` is reported here for completeness).
//!   - `largest_flat_lead` — the largest on-board flat lead the player held at any point, **raw**
//!     (komi is a constant offset, so it selects the same ply; the sheet renders `raw (±komi)`).
//!   - `largest_reserve_lead` — the largest flat-reserve lead the player held at any point. The
//!     lead belongs to whoever has *fewer* pieces left: 2 reserves against 6 is a lead of 4.
//!   - `largest_stack` — the tallest stack the player *controlled* at any point: over every position
//!     in the game, the height of the tallest stack they topped. Control is whatever piece is face
//!     up, wall and capstone included, so a stack a wall sits on is theirs. A lone flat is a stack
//!     of 1, and a player who never had a piece on the board scores 0.
//!
//! Each ply is also reported twice: `ptn` exactly as it was given, and `display` in conventional
//! short PTN for the sheet.
//!
//! **Starting anywhere.** `--tps` starts the walk from a given position instead of the empty board,
//! which makes this a single-position tool as well as a whole-game one: `--tps <position> --plies
//! <move>` grades exactly one move, which is what Takana's `scripts/positionAnalyzer.js`
//! uses to regression-test badge tuning against moves a human already has an opinion about. Ply
//! *numbering* is recovered from the position (`ply_offset`), not restarted at 1, so a ply analyzed
//! out of a TPS lands in the same opening/rarity bands it would have reached by replaying the game
//! to it. The ply `best` (`BEST_PLY`) stands in for a move where the caller would rather have the
//! engine's own choice than name one.
//!
//! **Which engine searches.** By default, topaz's own search. `--engine-cmd <path>` hands every
//! search to another engine over TEI instead (`crate::analyze_tei`), behind one seam,
//! `PositionSearch`: everything this module builds from a search — the board facts, the grading, the
//! JSON — is the same code whichever engine answered, so another engine's sheet is graded by exactly
//! the rules a topaz sheet is. The JSON names who searched (`engine`).
//!
//! **The opening.** `--opening swap|dbs` says what White's first move is. Without it the board
//! falls back to the original rule, which tied the opening to komi — double black stack exactly at
//! komi 0 — and was wrong for a double-black-stack game played with komi and for a swap game played
//! without. Only ply 1 depends on it, but there it decides the moves the search ranks, the move the
//! grade compares against, and how an engine's reply is read.
//!
//! **Move grading.** Every ply additionally carries the raw numbers a caller needs to award it a
//! badge — the thresholds themselves live in the bot, so they can be retuned without a rebuild:
//!   - `loss_cp` — how far the bar fell across the ply, in the mover's frame (see *How the loss is
//!     measured* below). `loss_exact` is false only when one of the two positions had no completed
//!     search to read. **A diagnostic, not the grading input** — see `cmp_played_cp` below for what
//!     to grade on.
//!   - `cmp_played_cp` / `cmp_other_cp` — the two ends of that comparison, in the mover's frame and
//!     on one footing: the played move, and the move it was measured against (the best move, or the
//!     runner-up when the ply *was* the best move). Normalise both into win probability and subtract
//!     *there*. Subtracting them as cp is the one thing they must not be used for: the bar on one
//!     side of a ply can be a proven win and on the other an evaluation, and a mate score minus an
//!     evaluation is not a quantity. See `Grade::cmp_played` for why `cp_before - loss_cp` cannot
//!     stand in.
//!   - `sharp_cp` — how much *better* than the best alternative to it. Negative whenever some other
//!     move was better, so "the only good move here" is simply a large positive value.
//!   - `fcd` — flat-count differential: how far the ply moved the mover's flat lead.
//!   - `cp_before` / `win_prob_before` — the position's value before the ply, mover's frame.
//!   - `solved_loss` — the ply handed the opponent a proven win.
//!   - `best_alt` — only on a ply that did: the best *other* move in that position, searched on the
//!     same terms the played move was, as `{display, cp, solved_loss}` (`cp` in the mover's frame).
//!     `solved_loss` on it means the alternative hands the win over as well, so the mover had no way
//!     out the search can see and the ply they chose was not the losing decision. `null` on every
//!     other ply, and where the position had only the one move. See `BestAlt` for why the position's
//!     own score cannot answer this and one alternative is enough.
//!   - `ocs_added` — of the squares the ply touched, how many the opponent controls after it (see
//!     `ocs_added` below).
//!   - `move_top` — for a stack movement, what was on top of the stack it moved: `"flat"`, `"wall"`
//!     or `"cap"`. `null` for a placement.
//!   - `defends_tak` — the mover was in tak coming into this ply and is not after it.
//!   - `completes_road` — the ply built the mover a road. Narrower than "won the game": a flat win
//!     has no road, and completing the opponent's road is their win, not this.
//!   - `largest_capture` / `largest_capture_after` — the tallest stack this ply took control of,
//!     measured before and after the move (see `largest_capture` below).
//!   - `pv` — the principal variation from the position this ply reached, in short PTN: the engine's
//!     recommended reply first, then the line it expects. This is the line the ply's own score is
//!     *about*, which is what makes it the answer to "why that evaluation?".
//!   - `runner_up` — the second-best move in that same position, `{display, cp_white, pv}`, from
//!     that same multi-PV 2 search: `pv` is its line on the same terms as the ply's own, and
//!     `display` is its first move. Every position gets one for free — it is where `sharp_cp` comes
//!     from, and multi-PV builds a line per index whether or not anyone keeps it — so unlike `alts`
//!     it is not gated. `null` on a terminal ply and in a position with one move to make.
//!   - `mate` — set when the ply's score is a *proven* win, `{winner, plies, kind}`: who has it,
//!     how many plies until it lands (counted from the position the ply reached, so 1 means the
//!     side to move wins with its next move), and `"road"` or `"flats"` for how. The kind is read
//!     by replaying the line to the end of the game, searching on from wherever the table's PV ran
//!     out (see `mate_of`), and that finished line is what `pv` then reports. A line that still
//!     stops short leaves `kind` null, and so does one whose ending disagrees with the score.
//!     `runner_up` carries one on the same terms. Every engine reports a proof as
//!     `WIN_SCORE - distance`, so this is engine-agnostic.
//!   - `search_nodes` — how many nodes that search actually spent. `nodes_per_ply` is the *budget*;
//!     this is the bill, and the two differ whenever the search stopped early — a solved position,
//!     or an iteration that finished under the cap. `null` on a terminal ply, which is pinned
//!     rather than searched.
//!
//! **How the loss is measured: off the bar.** A ply's loss is how far the sheet's own evaluation bar
//! fell across it, in the mover's frame — the position before the ply against the position it
//! reached, each read exactly as the bar draws it (`Searched::smoothed`). So a badge and the bar
//! beside it cannot disagree: a Blunder is a bar that fell by the Blunder band, and a move whose bar
//! rose gave nothing up. Every position is searched once, and that one search is both the bar for the
//! ply that reached it and the reference for the ply played from it (`bar_grade`).
//!
//! Two plies are read differently. One that **was** the engine's own best move gives up nothing by
//! definition, so its loss is pinned to 0 and it is compared against the runner-up instead — the
//! gap `sharp_cp` reports, which multi-PV 2 returns with exact scores from the same search, at the
//! same depth and for the same side to move, so it needs no extra search. And one that ended the
//! game is compared against its pinned result, which is what the bar draws for it too.
//!
//! **What this replaced: depth alignment.** The loss used to pair the parent at depth `D` against the
//! child at `D-1`, on the reasoning that a position and the next one only look equally far down the
//! tree at matched horizons, and fell back to searching the best alternative as a sibling whenever
//! that pairing disagreed with the child's own deeper reading (about a tenth of plies). It left the
//! badge and the bar on different readings — 0.65 win points apart on an average ply that was not the
//! best move, and up to 13 — and the fallback searches cost about 12% of a run's nodes. Measured on
//! eight games at 200k nodes/ply against two 800k references (one searched forward, one backward),
//! grading off the bar is as accurate: mean error 1.07 / 1.18 win points against 1.05 / 1.17, band
//! agreement 79.3% / 77.4% against 78.8% / 77.6%, and a badge mix all but unchanged.
//!
//! Plain differencing of a position against its child was rejected once, for swings of up to 27cp on
//! plies the engine itself picked: the child search sees a ply deeper, so the two horizons differ.
//! Those plies are now pinned, and on the rest the bar is read across several depths
//! (`DEPTH_KERNEL`), which mixes the two parities rather than picking one. The step it still shows
//! across the engine's own move — 1.1 win points on average — is the measurement's own noise, and is
//! now the same noise in the badge and in the bar.
//!
//! **The other moves that were available** (`--alts`). A caller grading a ply on how *hard it was to
//! see* eventually needs to know what else was on offer: a sacrifice is only remarkable if the quiet
//! moves were worse. `--alts N` asks for the position's best `N` root moves, each carrying the same
//! raw grading numbers the played ply does, so the caller can score them on its own tables. See
//! `Alternatives` for the shape and for what the list can and cannot answer.
//!
//! Enumerating them costs one extra search of the position, so it is gated: `--alts-min-decided`
//! and `--alts-max-loss` name the plies worth spending it on. Both are the *caller's* thresholds
//! handed to the engine — nothing here decides which plies matter, it only avoids searching the ones
//! the caller has already said it will ignore. Note that the first is read off the **leader's** win
//! probability rather than the mover's, so it admits both players' moves out of a settled position,
//! and off **either side of the ply**, so it also admits the move that settled the game as it was
//! played. `--alts-max-fcd` and `--alts-min-ocs` are a third gate, on the move itself: a caller
//! that only reads the list for moves it might call rare can say which moves never are, and in a
//! settled game — where the first gate admits nearly every ply — that is most of them.

use crate::analyze_tei::{TeiEngineConfig, TeiSearch};
use crate::board::{Board4, Board5, Board6, Piece, TakBoard};
use crate::eval::{Evaluator, LOSE_SCORE, NNUE4, NNUE5, NNUE6, WIN_SCORE};
use crate::search::{search, SearchInfo};
use crate::transposition_table::HashTable;
use crate::{Color, GameMove, GameResult, RevGameMove, TimeBank};

use std::io::{self, Write};
use std::time::Instant;

/// Transposition-table size for every per-ply search. Each search clears it first (see `eval_white`).
const TT_SIZE: usize = 1 << 20;

/// Depth ceiling; the node budget is what actually stops the search.
const MAX_DEPTH: usize = 32;

/// How far below `WIN_SCORE` still counts as a *proven* win. Win scores are `WIN_SCORE - distance`,
/// and the distance is bounded by the search depth, so this has plenty of room.
pub(crate) const SOLVED_MARGIN: i32 = 200;

/// Win-probability scale. `sigmoid(internal_score / EVAL_SCALE)`, matching the trainer's convention
/// (`datagen.rs`), i.e. equivalently `sigmoid(human_cp / 100)`.
const EVAL_SCALE: f64 = 400.0;

/// How a position is read off its own iterative-deepening ladder: weights for the deepest completed
/// iteration, the one below it, and so on. `&[1]` is "deepest only", which is what this module did
/// before depth averaging.
///
/// **It is also what the badges read.** A ply's loss is the fall in the bar across it (see the
/// module docs), so changing this changes the grades along with the curve, and the two cannot come
/// apart. `smooth_ladder` is where the arithmetic lives and where the edge cases (short ladders,
/// proven wins) are handled.
///
/// **Measured, not reasoned** — 2026-08-14/15, PlayTak 858074 / 812688 / 864344 / 863024 at 200k
/// nodes/ply against a 1.6M reference. The phantom step on a best ply — how far the curve moves
/// across a move the engine itself would have played, which is worth exactly what the position
/// before it was worth, so every point of it is measurement error:
///
/// | weights | phantom step |
/// |---|---|
/// | `[1]` (deepest only) | 1.33 |
/// | `[3, 2, 1]` | 0.98 |
/// | `[1, 1, 1]` | 0.94 |
/// | `[1, 1, 1, 1]` | 0.83 |
///
/// It falls because adjacent positions rarely stop at the same depth — the error runs 0.80 win
/// points when the child searched one shallower (horizons already matched), 1.57 at equal depth and
/// 1.78 when the child searched deeper — and averaging mixes the parities instead of picking one.
/// Re-measured on eight games when grading moved onto the bar (2026-10-28): 1.40 deepest-only
/// against 1.12 with this kernel.
///
/// **Window length is what matters; the shape barely does** (a flat `[1, 1, 1]` scores 0.94 against
/// this one's 0.98). Parity-cancelling shapes — `[2, 2, 1, 1]`, `[3, 3, 1, 1]`, whose alternating sum
/// is zero — were tried on the theory that they would cancel the odd/even component exactly, and they
/// are no better than flat windows of the same length. Medians instead of means were tried too, on
/// the theory that they would reject a single outlier depth, and they are worse on every measure.
/// Going past 4 starts costing curve accuracy against the reference.
const DEPTH_KERNEL: &[i64] = &[3, 2, 1];

/// A ply written as this in the ply list is not read as PTN: the engine searches the position and
/// plays **its own best move** instead, then grades it like any other ply (loss 0, by definition).
///
/// That is what turns this command into a single-position tool — `--tps <position> --plies best`
/// asks "what does the engine play here, and what is that move worth?" — and it composes, so a run
/// of them plays out the engine's own line.
pub const BEST_PLY: &str = "best";

#[derive(Clone)]
pub struct AnalyzeGameConfig {
    /// Board size (4, 5, or 6 — the sizes with nets).
    pub size: usize,
    /// Half-komi (2 komi == 4). Also selects the komi-specialized 6x6 net.
    pub half_komi: u8,
    /// Soft node budget per position. The hard cap is 4x this.
    pub nodes: u64,
    /// Start from this position rather than the empty board. The komi in the TPS, if it carries
    /// one, is overridden by `half_komi`.
    ///
    /// Ply numbering comes off the position itself (see `ply_offset`), so a ply analyzed out of a
    /// TPS is graded on the same footing as the same ply reached by replaying the game.
    pub tps: Option<String>,
    /// The game's plies in PTN, in order. `BEST_PLY` in place of a ply means "play the engine's
    /// choice here".
    pub plies: Vec<String>,
    /// JSON output path; `None` prints to stdout.
    pub output: Option<String>,
    /// Suppress the per-ply stderr progress line.
    pub quiet: bool,
    /// How many root moves to enumerate alongside a ply (see `Alternatives`). 0 turns the whole
    /// thing off, which is the default; the search caps it at `MAX_MULTI_PV`.
    pub alts: usize,
    /// Only plies with a position at least this **decided** on one side of them get an
    /// alternatives list, where decidedness is the *leader's* win probability — `max(p, 1 - p)`, so
    /// 0.5 is dead even and 1 is a proven win, whichever player holds it.
    ///
    /// Two-sided in both senses. **Whose** probability: "this game is settled" is a fact about the
    /// position, not about who is to move in it, and a caller weighing how hard a move was to find
    /// wants it for the player being ground down as much as for the one doing the grinding.
    /// **Which position**: the ply going in *and* the ply coming out, taken at whichever of the two
    /// reads more decided. A caller can hold the two players to opposite sides of the ply — the one
    /// behind to the position their move left, the one ahead to the position they moved from — and
    /// this way the list is there for either reading, rather than the engine deciding on its own
    /// which of them the caller meant. The caller's threshold, not the engine's — see the module
    /// docs.
    pub alts_min_decided: f64,
    /// ...and only those that gave up at most this many **win percentage points**, on the same
    /// normalised footing the caller grades on (`loss_cp` against `cp_before`). A ply the loss
    /// method could not grade at all never qualifies.
    pub alts_max_loss_pct: f64,
    /// ...and, when set, only **stack movements** that gave up flat count down to `max_fcd` or left
    /// at least `min_ocs` of the squares they touched with the opponent (`fcd`, `ocs_added`) —
    /// either will do. A caller that scores rarity off those two numbers and calls a move that
    /// moved nothing, or did neither, ordinary whatever else it did, has no use for that move's
    /// alternatives. `None` admits every ply. The caller's thresholds, not the engine's.
    pub alts_irregular: Option<AltsIrregular>,
    /// Attach every fully-completed iteration's score to each ply (`depth_scores`), as a diagnostic.
    /// Free — the search already records them (`SearchInfo::depth_scores`) — but off by default
    /// because nothing in the contract reads them; they exist so multi-depth grading and curve
    /// smoothing can be evaluated offline from one run.
    pub depth_scores: bool,
    /// The opening the game was played under: `Some(true)` for double black stack, `Some(false)`
    /// for the standard swap. `None` leaves it to the board's original rule, which ties the opening
    /// to komi (double black stack exactly at komi 0) — see `TakBoard::double_black_stack`. Only
    /// ply 1 reads it, but ply 1 reads it everywhere: the moves the search ranks there, the move
    /// the grade compares against, and the spelling of an engine's reply.
    pub double_black_stack: Option<bool>,
    /// Who searches the positions. `None` is topaz's own search; `Some` drives another engine over
    /// TEI (`crate::analyze_tei`). Nothing else changes with it: the board facts, the grading and
    /// the JSON are this module's whichever engine answers.
    pub engine: Option<TeiEngineConfig>,
}

impl Default for AnalyzeGameConfig {
    fn default() -> Self {
        Self {
            size: 6,
            half_komi: 0,
            nodes: 100_000,
            tps: None,
            plies: Vec::new(),
            output: None,
            quiet: false,
            alts: 0,
            alts_min_decided: 0.0,
            alts_max_loss_pct: f64::INFINITY,
            alts_irregular: None,
            depth_scores: false,
            double_black_stack: None,
            engine: None,
        }
    }
}

/// The move-shape gate on `--alts` — see `AnalyzeGameConfig::alts_irregular`.
#[derive(Debug, Clone, Copy)]
pub struct AltsIrregular {
    /// A spread that moved the mover's flat lead (`fcd`) by at most this much qualifies.
    pub max_fcd: i32,
    /// ...as does one that left at least this many of its squares topped by the opponent.
    pub min_ocs: u32,
}

impl AltsIrregular {
    fn admits(&self, move_top: Option<Piece>, fcd: i32, ocs_added: u32) -> bool {
        move_top.is_some() && (fcd <= self.max_fcd || ocs_added >= self.min_ocs)
    }
}

/// A ply's grade, in the mover's frame and in internal cp. See `bar_grade`.
#[derive(Clone, Copy, Default)]
struct Grade {
    /// How far the bar fell across the ply. Clamped at 0, and pinned to 0 for the engine's own best
    /// move. A diagnostic: grade on the endpoints (`loss_win_pct`), never on this.
    loss: i32,
    /// False when one of the two positions had no completed search to read; `loss` is then a
    /// meaningless 0.
    exact: bool,
    /// Gap back to the runner-up. `None` unless the ply was the engine's best move.
    sharp: Option<i32>,
    /// The value of the position before the ply: its best root move's score at the deepest
    /// iteration. It is not the grade's reference — that is the bar (`Searched::smoothed`) — but
    /// what decides how settled the game already was (`win_prob_before`).
    ///
    /// **Not smoothed through `DEPTH_KERNEL`, unlike the sheet's bar.** It comes from `top()`, the
    /// best *root move's* score, and only the final iteration's root list survives the search — there
    /// is no per-depth root ladder to average. Smoothing it would mean recording `root_choices` per
    /// iteration in `search`, which is a change to the search rather than to this module. So this and
    /// `sharp` are the two grading numbers still read at a single depth; the gap that leaves between
    /// `cp_before` and the previous ply's bar is a fraction of a win point and predates this.
    cp_before: i32,
    /// The two ends of the comparison this grade is about, in the mover's frame and **on one
    /// footing**.
    ///
    /// `cmp_played` is the move that was played, `cmp_other` the move it is measured against. On an
    /// ordinary ply those are the bar after it and the bar before it. A ply that was the engine's own
    /// choice is compared against the **runner-up** instead, both ends off the parent's root list, and
    /// gives up nothing; one that was not is not sharp. So one pair of endpoints carries both `loss`
    /// and `sharp`.
    ///
    /// They exist because `loss` and `cp_before` cannot be used to recover them: `cp_before` is the
    /// best root move's deepest score while the reference is the smoothed bar, so `cp_before - loss`
    /// mixes two readings. Consumers normalise these two into win probability and subtract *there*;
    /// that is the only subtraction that is meaningful when one end is a proven win and the other is
    /// an evaluation, because the sigmoid saturates instead of running off the scale.
    ///
    /// **On a `played_best` ply, `cmp_other` never sits above `cmp_played`.** It is the runner-up, so
    /// the pair runs the other way and the ply gave up nothing by definition — which is exactly what
    /// pinning `loss` to 0 says. The clamp that guarantees it is not cosmetic: each multi-PV index is
    /// a full re-search with the earlier PVs excluded at the root, and can hand back a runner-up that
    /// scored *above* the move it lost to. Left alone that reads as the best move in the position
    /// having given ground, which is how a Great move becomes an Inaccuracy.
    cmp_played: i32,
    cmp_other: i32,
}

impl Grade {
    /// How much winning chance the ply gave up, in percentage points.
    ///
    /// The endpoints normalised into win probability and subtracted *there* — never as cp. That is
    /// what lets a comparison survive one end being a proven win and the other an evaluation: the
    /// sigmoid saturates, so "you had a forced win and instead stayed 98% winning" comes out as the
    /// point or two it is, rather than as the thousands of cp the raw difference reads.
    ///
    /// Clamped at zero, which is also the answer for a ply the engine itself chose: its endpoints
    /// are the ply and the runner-up it beat, so the difference runs the other way. `None` when the
    /// method had nothing to grade against.
    fn loss_win_pct(&self) -> Option<f64> {
        self.exact.then(|| {
            let other = sigmoid(self.cmp_other as f64 / EVAL_SCALE);
            let played = sigmoid(self.cmp_played as f64 / EVAL_SCALE);
            ((other - played) * 100.0).max(0.0)
        })
    }
}

/// Everything about a move that a badge is scored from, measured by playing it. The fields mean
/// exactly what the same-named fields on `PlyAnalysis` mean — this is where the played ply gets its
/// own copies from, so a move on the alternatives list is measured by the identical code.
struct MoveFacts {
    move_top: Option<Piece>,
    terminal: Option<GameResult>,
    call: bool,
    defends_tak: bool,
    completes_road: bool,
    ocs_added: u32,
    largest_capture: u32,
    largest_capture_after: u32,
    /// On-board flat counts after the move, `[white, black]`.
    flats: [i32; 2],
    fcd: i32,
}

/// One move that was available instead of the one played, with its score and its grading numbers.
struct Alternative {
    /// Short PTN, spelled for the position it would have been played in.
    display: String,
    /// Its exact score from the alternatives search, in the **mover's** frame and internal cp.
    cp: i32,
    facts: MoveFacts,
}

/// The other moves available on a ply, from one multi-PV search of the position it was played in.
///
/// **Everything here comes from that one search**, the played move's own `played_cp` included, so
/// the scores are directly comparable to each other: same position, same depth, same side to move,
/// no horizon alignment needed. They are deliberately *not* comparable to the ply's `cp_before` or
/// `loss_cp`, which come from the main grading searches — use these to rank moves against each
/// other, not to re-derive the ply's grade.
///
/// **What a truncated list can't tell you.** `moves` holds the best `--alts N` root moves and
/// nothing below them, so it answers "was there a better move of some kind?" and not "how many were
/// there?". Two consequences worth designing around: when `played_cp` is `None` the played move did
/// not make the list at all, which says only that at least `N` moves beat it; and a move tied with
/// the last entry may have been cut, so a tie at the bottom of the list is not proof there are no
/// others.
struct Alternatives {
    /// The played move's own score in this search, or `None` if it did not make the list.
    played_cp: Option<i32>,
    /// How many nodes the multi-PV search that produced this list spent. Reported for the same
    /// reason `search_nodes` is: the budget says what was allowed, this says what was used.
    nodes: u64,
    /// The other root moves, best first, excluding the played one.
    moves: Vec<Alternative>,
}

/// The second-best move in the position a ply reached, from that position's own multi-PV 2 search —
/// the one every position gets. See `Searched::runner_up`.
///
/// **Not an `Alternative`**, despite answering a similar question. An `Alternative` comes from the
/// extra `--alts` search of the position a ply was played *in*, carries the rarity facts, and exists
/// only on the plies that search was spent on. This is one move out of the search the ply's own `pv`
/// and `cp_white` come from, so it is on exactly their footing and is there for every ply.
struct RunnerUp {
    /// The move in conventional short PTN, spelled against the position it is played in. Always the
    /// first entry of `pv`, kept separately because the line can come back empty.
    display: String,
    /// Its score in **White's frame**, so it sits directly beside the ply's `cp_white`.
    ///
    /// **Read at the final iteration only**, where `cp_white` is now averaged over the last few
    /// (`DEPTH_KERNEL`), so the two are no longer strictly the best and second-best readings of one
    /// search: only the root list of the last iteration survives, and there is no per-depth ladder to
    /// average — the same limitation `Grade::cp_before` carries. A consumer subtracting one from the
    /// other to recover a gap should use `sharp_cp` instead, which is both ends from one iteration.
    score_white: i32,
    /// The line behind it, in short PTN, on the same terms as the ply's own `pv` — replayed on the
    /// board so the spelling and crush flags are right, and truncated at anything unplayable.
    pv: Vec<String>,
    /// Set when `score_white` is a proven win, on the same terms as the ply's own `mate`.
    mate: Option<Mate>,
}

/// How a proven win ends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MateKind {
    Road,
    Flats,
}

/// A proven win, as a reader wants it said: who has it, how far away it is, and how it ends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Mate {
    winner: Color,
    /// Plies from the position the score was read in to the one the game ends in — so 1 is the
    /// side to move winning with its next move. `WIN_SCORE - |score|`, which is how every engine
    /// here encodes a proof (topaz natively, and `analyze_tei` maps a TEI `mate n` onto it).
    plies: u32,
    /// `None` when the line the search reported stops short of the end of the game, or ends it some
    /// other way than the score says — the distance is the search's own, but *how* is read off the
    /// line, and a line that doesn't get there cannot say.
    kind: Option<MateKind>,
}

/// One analyzed ply.
struct PlyAnalysis {
    /// 1-based ply number.
    index: usize,
    /// The ply exactly as it was given to us.
    ptn: String,
    /// The ply in conventional short PTN, for display (see where it is built).
    display: String,
    /// The player who made the ply (the side to move before it — on ply 1 that player places the
    /// opponent's flat, but the ply is still theirs).
    player: Color,
    move_number: usize,
    /// Fixed White-perspective internal score of the position *after* the ply.
    score_white: i32,
    win_prob_white: f64,
    /// Set when the ply ended the game.
    terminal: Option<GameResult>,
    /// This ply put the opponent into tak (see module docs).
    call: bool,
    /// The mover was in tak coming into this ply, and is not after it. The mirror of `call`, but
    /// unlike `call` it *can* be set on a game-ending ply — the threat it answers was on the board
    /// before the move, so winning by breaking it and winning the race past it are distinguished.
    defends_tak: bool,
    /// This ply completed the mover's road. Not a synonym for winning — a flat win ends the game
    /// with no road, and completing the *opponent's* road does not set this.
    completes_road: bool,
    /// On-board flat counts after the ply, `[white, black]`.
    flats: [i32; 2],
    /// Change in the mover's flat differential: +1 for a flat placement, 0 for a wall or capstone,
    /// and whatever a spread covers or buries.
    fcd: i32,
    /// How much of what this ply *reached* the opponent controls afterwards: of the squares the ply
    /// touched — the origin square, plus every square a spread dropped onto, or the single square a
    /// placement filled — how many an opponent piece tops once the move is done.
    ///
    /// Measured **after the move only**, so it is a state rather than a change. A square the
    /// opponent already topped and still tops, having been spread across, counts: the question is
    /// how much of the ground this move covered it left in their hands, and ground you slid over
    /// without taking is left to them exactly as much as ground you uncovered.
    ///
    /// "Tops" means any piece — flat, wall or capstone. A stack the opponent's wall sits on is
    /// theirs, however little it does for their flat count. A square left empty is nobody's, so a
    /// fully vacated origin does not count.
    ocs_added: u32,
    /// What was on top of the stack a movement moved, or `None` for a placement.
    move_top: Option<Piece>,
    /// The tallest stack this ply took control of: over every square the spread dropped onto and
    /// **ends up topped by the mover**, the height that square had *before* the move. 0 for a
    /// placement, and for a spread that only landed on empty squares.
    ///
    /// Who owned a square going in does not matter — burying your own tall stack is the same act as
    /// burying the opponent's. What matters is that the square is yours afterwards, which is why
    /// this is measured on both sides of the move: heights before, ownership after. Squares the
    /// spread dropped an *opponent's* piece onto (the carry keeps its order, so anything under the
    /// top of the stack you moved can land face-up) are not captures and are excluded.
    largest_capture: u32,
    /// The same measurement with the tape run *after* the move instead: same squares, same
    /// ownership filter, but each one's height once the spread has landed on it.
    ///
    /// Both are reported because they answer different questions, and which one a caller wants
    /// depends on what led the spread. Against a wall or capstone, what matters is the stack you
    /// took — the height going in. Against a flat, the leading piece ends up on top of whatever it
    /// lands on and the stack it leaves behind is what the move built, so the height coming out is
    /// the number with meaning. Takana's `src/badges.js` picks between them on `move_top`
    /// and scores each against its own table.
    ///
    /// Always at least 1 for a flat-led spread — the carry keeps its order, so the leading flat is
    /// the last piece dropped and the final square is always the mover's.
    largest_capture_after: u32,
    /// The principal variation from the position this ply reached, in short PTN — the engine's
    /// recommended reply first, then the line it expects to follow. Empty for a terminal ply, and
    /// truncated at any move the position cannot actually play (see `pv_ptn`).
    pv: Vec<String>,
    /// The second-best move in that same position, from that same search. `None` for a terminal
    /// ply, and for a position with only one move to make.
    runner_up: Option<RunnerUp>,
    /// Set when `score_white` is a proven win: who has it, how many plies off, and how it ends.
    /// `None` on a terminal ply — the game is over, not about to be.
    mate: Option<Mate>,
    /// Deepest fully-completed iteration of the search of the position this ply reached. `None` for
    /// a terminal ply, which is pinned rather than searched. A position that searched much
    /// shallower than its neighbours reads its bar — and so the grades either side of it — off a
    /// shallower window, which is worth being able to see.
    ///
    /// The **top of the reading window**, not the depth of a single reading: `score_white` averages
    /// this iteration with the ones below it (`DEPTH_KERNEL`).
    search_depth: Option<usize>,
    /// Nodes that same search spent — the companion to `search_depth`, and the number a consumer
    /// should quote when it wants to say how hard this position was actually looked at. `None` on a
    /// terminal ply for the same reason `search_depth` is.
    search_nodes: Option<u64>,
    /// The mover's win probability before this ply, from the primary grade's `cp_before`.
    win_prob_before: f64,
    /// This ply handed the opponent a proven win.
    solved_loss: bool,
    /// What the best *other* move in that position would have come to, on a ply that did. `None`
    /// everywhere else, and where there was no other move to try. See `BestAlt`.
    best_alt: Option<BestAlt>,
    /// The ply's grade, off the bar (see `bar_grade`).
    grade: Grade,
    /// Whether this ply was the engine's own best move — the one kind of ply graded against the
    /// runner-up rather than the bar.
    played_best: bool,
    /// The other moves available here, when this ply passed the `--alts` gates.
    alts: Option<Alternatives>,
    /// Every fully-completed iteration of the search of the position this ply reached, as
    /// `(depth, White-frame internal score)`, ascending. Empty unless `--depth-scores` asked for it,
    /// and on a terminal ply, which is pinned rather than searched. Diagnostic only.
    depth_scores: Vec<(usize, i32)>,
}

#[derive(Default, Clone, Copy)]
struct PlayerStats {
    calls: u32,
    flat_placements: u32,
    wall_placements: u32,
    cap_placements: u32,
    movements: u32,
    largest_flat_lead: i32,
    largest_reserve_lead: i32,
    largest_stack: u32,
}

struct Analysis {
    plies: Vec<PlyAnalysis>,
    stats: [PlayerStats; 2],
    /// Engine-determined result, if the game ended within the given plies.
    result: Option<GameResult>,
    nodes: u64,
    seconds: f64,
}

fn color_idx(c: Color) -> usize {
    match c {
        Color::White => 0,
        Color::Black => 1,
    }
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// The tallest stack `player` tops in this position, 0 if they top none. Control is whatever piece
/// is face up — `active_stacks` is the ownership bitboard, so a stack under their wall or capstone
/// counts as theirs.
fn tallest_owned<B: TakBoard>(board: &B, player: Color) -> u32 {
    board
        .active_stacks(player)
        .map(|idx| board.index(idx).len() as u32)
        .max()
        .unwrap_or(0)
}

/// Is this an evaluation, as opposed to one of the search's sentinel window bounds? Real scores never
/// leave `[LOSE_SCORE, WIN_SCORE]`; the bounds sit two orders of magnitude outside it.
fn is_real_score(score: i32) -> bool {
    score.abs() <= WIN_SCORE + SOLVED_MARGIN
}

/// Is this a *proven* win or loss rather than an evaluation? Win scores are `WIN_SCORE - distance`
/// either side of zero, so this is one test for both.
///
/// The distinction matters wherever two scores are subtracted. A mate score says "this is over in N
/// plies" and an evaluation says "this is worth so many flats"; the two are not on one scale, and a
/// difference between them measures nothing.
fn is_solved(score: i32) -> bool {
    score.abs() >= WIN_SCORE - SOLVED_MARGIN
}

/// Read a position off its depth ladder as a weighted average of the iteration ending at `top` and
/// the ones below it, per `DEPTH_KERNEL`. `None` when `top` itself never completed.
///
/// The window shortens rather than fails, in three situations, all of which occur in real games:
///
/// - **The ladder runs out.** A position whose budget bought only three iterations cannot be read
///   four deep. Weights are renormalised over what is there, so the deepest reading always dominates.
/// - **An iteration is missing from the middle.** `depth_scores` only records iterations that
///   completed, so a gap is possible in principle; the window stops at the gap rather than reaching
///   past it and silently averaging non-adjacent depths.
/// - **A proven win is in range.** Win scores are `WIN_SCORE - distance`, thousands of cp from any
///   evaluation, so averaging the two produces a number that is neither. The window stops below the
///   shallowest solved reading — but the reading *at* `top` is always kept, solved or not, so a
///   position the search solved still reports its proof rather than a blend. It catches the search's
///   window sentinels too, which sit further out still (`is_real_score`) — those are not scores at
///   all, and averaging one in would drag a whole position thousands of cp.
///
/// Rounds to the nearest internal cp, away from zero on a tie. Internal cp are quarter-human-cp, so
/// the rounding error is at most 0.125 human cp — a hundredth of a win point where the sigmoid is
/// steepest, and less everywhere else.
fn smooth_ladder(depths: &[(usize, i32)], top: usize, weights: &[i64]) -> Option<i32> {
    let score_at = |d: usize| depths.iter().find(|&&(x, _)| x == d).map(|&(_, s)| s);
    let first = score_at(top)?;
    let (mut num, mut den) = (0i64, 0i64);
    for (i, &w) in weights.iter().enumerate() {
        // `top - i` would wrap at depth 0, and there is no iteration 0 to read anyway.
        let Some(depth) = top.checked_sub(i).filter(|&d| d > 0) else { break };
        let Some(score) = score_at(depth) else { break };
        // The reading at `top` is the position's answer and is kept whatever it is; anything below
        // it that crosses into proven-win territory ends the window.
        if i > 0 && is_solved(score) {
            break;
        }
        num += w * score as i64;
        den += w;
        // A solved reading at `top` is reported alone rather than blended into the evaluations below.
        if i == 0 && is_solved(first) {
            break;
        }
    }
    if den == 0 {
        return Some(first);
    }
    // Round half away from zero: `num / den` truncates toward zero in Rust, which would bias every
    // averaged reading a fraction of a cp toward the draw.
    let rounded = if num >= 0 { (num + den / 2) / den } else { (num - den / 2) / den };
    Some(rounded as i32)
}

/// Build a search configured to spend a fixed node budget: `abort_depth(1)` lets the soft-node check
/// end an iteration from the very first one, and the time bank is only a runaway guard.
///
/// Multi-PV 2 is what makes move grading possible. Each PV index is a full re-search with the
/// earlier PVs excluded at the root, so `root_choices` comes back holding the best and second-best
/// move with *exact* scores. That is where `sharp_cp` comes from directly — same search, same depth,
/// same side to move — and it is also how a ply is recognised as the engine's own choice. The node
/// budget is shared across both PVs, so this costs depth rather than time.
fn make_info(nodes: u64, table: &HashTable) -> SearchInfo<'_> {
    SearchInfo::new(MAX_DEPTH, table)
        .quiet(true)
        .abort_depth(1)
        .set_multi_pv(2)
        .set_max_nodes(nodes, nodes.saturating_mul(4))
        .time_bank(TimeBank::flat(600_000))
}

/// A searched position: the score from the **side to move's** perspective, plus the exact scores of
/// the best root moves (best first), from the same search.
pub(crate) struct Searched {
    pub(crate) score: i32,
    pub(crate) roots: Vec<(GameMove, i32)>,
    /// The line behind each entry of `roots`, same order (`SearchInfo::multi_pv_lines`). Empty for
    /// a terminal position, and never longer than `roots`.
    pub(crate) lines: Vec<Vec<GameMove>>,
    /// `(depth, score)` for every fully-completed iteration, ascending. Empty for a terminal
    /// position, which is pinned rather than searched.
    pub(crate) depths: Vec<(usize, i32)>,
    /// The principal variation of the deepest iteration — the line the score is *about*, best move
    /// first. Empty for a terminal position.
    pub(crate) pv: Vec<GameMove>,
}

impl Searched {
    /// A position that was never searched because the game had already ended there.
    fn terminal(score: i32) -> Self {
        Self {
            score,
            roots: Vec::new(),
            lines: Vec::new(),
            depths: Vec::new(),
            pv: Vec::new(),
        }
    }
    /// The best root move's score; falls back to the position score if the root list is empty
    /// (possible when the node budget was too small to complete even one depth).
    fn top(&self) -> i32 {
        self.roots.first().map(|&(_, s)| s).unwrap_or(self.score)
    }
    /// The best root move the search found.
    fn best_move(&self) -> Option<GameMove> {
        self.roots.first().map(|&(m, _)| m)
    }
    /// The best root move *other* than `mv` — the alternative a ply is graded against.
    fn best_other_move(&self, mv: GameMove) -> Option<GameMove> {
        self.roots.iter().map(|&(m, _)| m).find(|&m| m != mv)
    }
    /// The second-best root move, its exact score, and the line behind it — the other half of what
    /// multi-PV 2 found here.
    ///
    /// Free: this is the same pair `root_gap` measures `sharp_cp` from, and multi-PV builds the line
    /// for every index whether or not anyone keeps it, so none of this costs a node.
    ///
    /// The line is empty rather than absent when the search didn't retain one (a single-PV search,
    /// or an index that produced a move but no stored line), so a caller can always fall back to the
    /// move itself.
    ///
    /// `None` when multi-PV found only one move, or when the second index came back holding a
    /// window bound rather than an evaluation (see `root_gap`).
    fn runner_up(&self) -> Option<(GameMove, i32, &[GameMove])> {
        match self.roots.as_slice() {
            [_, (mv, cp), ..] if is_real_score(*cp) => Some((
                *mv,
                *cp,
                // Index-parallel with `roots` by construction, but read defensively: a search that
                // was not multi-PV records no lines at all.
                self.lines.get(1).map(|l| l.as_slice()).unwrap_or(&[]),
            )),
            _ => None,
        }
    }
    /// How much the best root move beat the runner-up by. Both scores come from the same search at
    /// the same depth for the same side to move, so no alignment is needed. `None` when multi-PV
    /// found only one move.
    fn root_gap(&self) -> Option<i32> {
        match self.roots.as_slice() {
            // A PV that found no move at all comes back holding the search's opening alpha/beta
            // bound rather than an evaluation. That is not a score to subtract — it would report a
            // runner-up a quarter of a million cp behind — so it is rejected outright.
            [(_, best), (_, second), ..] if is_real_score(*best) && is_real_score(*second) => {
                Some(best - second)
            }
            _ => None,
        }
    }
    /// The deepest fully-completed iteration.
    fn deepest(&self) -> Option<usize> {
        self.depths.last().map(|&(d, _)| d)
    }
    /// This position's score read across `DEPTH_KERNEL`, ending at `depth`.
    fn smoothed_at(&self, depth: usize) -> Option<i32> {
        smooth_ladder(&self.depths, depth, DEPTH_KERNEL)
    }
    /// This position's value as the sheet draws it: the kernel applied at the deepest iteration that
    /// completed. Falls back to the raw search score for a position with no ladder at all — a
    /// terminal position, which is pinned, or a budget too small to finish one iteration.
    fn smoothed(&self) -> i32 {
        self.deepest().and_then(|d| self.smoothed_at(d)).unwrap_or(self.score)
    }
}

/// Search one position with the transposition table cleared first, so the result is a pure function
/// of the position (see module docs).
fn search_position<B, E>(board: &mut B, eval: &mut E, info: &mut SearchInfo) -> Searched
where
    B: TakBoard,
    E: Evaluator<Game = B>,
{
    info.clear_tt_full();
    let outcome = search(board, eval, info);
    let score = outcome.as_ref().map(|o| o.score()).unwrap_or(0);
    Searched {
        score,
        roots: info.root_choices().to_vec(),
        lines: info.multi_pv_lines().to_vec(),
        depths: info.depth_scores().to_vec(),
        pv: outcome.map(|o| o.pv).unwrap_or_default(),
    }
}

/// How many root moves a search ranks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Breadth {
    /// Best and second-best — every ordinary search, and all the grading needs (see `make_info`).
    Grading,
    /// The `--alts` list: this many root moves, every one of them searched rather than pruned.
    Alternatives(usize),
}

/// Where a position's search comes from: topaz's own search (`NativeSearch`), or another engine
/// over TEI (`crate::analyze_tei::TeiSearch`). Everything that reads the result — the board facts,
/// the grading, the JSON — is the same code whichever answers, which is the point: a sheet from
/// another engine is graded by exactly the rules a topaz sheet is.
pub(crate) trait PositionSearch<B: TakBoard> {
    /// Search the position on `board`, side to move's frame, leaving the board as it was found.
    /// Returns the search and the nodes it spent.
    fn search(&mut self, board: &mut B, breadth: Breadth) -> anyhow::Result<(Searched, u64)>;
}

/// topaz's own search: one net, one transposition table, cleared before every position.
struct NativeSearch<'a, E> {
    eval: E,
    table: &'a HashTable,
    info: SearchInfo<'a>,
    nodes: u64,
}

impl<'a, B, E> PositionSearch<B> for NativeSearch<'a, E>
where
    B: TakBoard,
    E: Evaluator<Game = B>,
{
    fn search(&mut self, board: &mut B, breadth: Breadth) -> anyhow::Result<(Searched, u64)> {
        Ok(match breadth {
            Breadth::Grading => {
                let s = search_position(board, &mut self.eval, &mut self.info);
                (s, self.info.nodes() as u64)
            }
            Breadth::Alternatives(n) => {
                let mut info = make_info(self.nodes, self.table)
                    .set_multi_pv(n)
                    .full_root_moves(true);
                let s = search_position(board, &mut self.eval, &mut info);
                (s, info.nodes() as u64)
            }
        })
    }
}

/// Conventional short PTN for a move about to be played on `board`. `to_ptn` already drops the count
/// and the drop-list wherever they are redundant (`1c2-1` -> `c2-`, `3c2-3` -> `3c2-`); the one
/// thing added back is the leading `1` on a single piece coming off a taller stack (`1b4-`), so a
/// stack being split is visible. Must be called *before* the move, while the source stack is intact.
fn short_ptn<B: TakBoard>(board: &B, mv: GameMove) -> String {
    let short = mv.to_ptn::<B>();
    if mv.is_stack_move() && mv.number() == 1 && board.index(mv.src_index()).len() > 1 {
        format!("1{short}")
    } else {
        short
    }
}

/// A principal variation as PTN, played out on `board` and then unwound, leaving it exactly as it
/// was found.
///
/// Replaying is what makes the spelling right: `short_ptn` needs the source stack to see that a lone
/// piece came off a taller one, and the crush flag is a property of the position a move is made in,
/// not of the move. It also filters — a PV table can hand back a move belonging to a line that is no
/// longer reachable — so the walk stops at the first move that is not legal here rather than
/// emitting something no reader can replay. It stops at a win, too: the moves a PV lists past the
/// end of the game are an artefact of the table, not analysis.
fn pv_ptn<B: TakBoard>(board: &mut B, pv: &[GameMove]) -> Vec<String> {
    let mut out = Vec::with_capacity(pv.len());
    let mut played: Vec<RevGameMove> = Vec::with_capacity(pv.len());
    for &mv in pv {
        if !board.legal_move(mv) {
            break;
        }
        out.push(short_ptn(board, mv));
        played.push(board.do_move(mv));
        if board.game_result().is_some() {
            break;
        }
    }
    while let Some(rev) = played.pop() {
        board.reverse_move(rev);
    }
    out
}

/// Read a proven win off a White-frame score, and how it ends off the line behind it.
///
/// The distance comes from the score, which is the search's own claim; the kind comes from playing
/// `pv` out on the board the score was read in and looking at how the game ended.
///
/// **A PV routinely stops short of the end**, because it is read back out of a transposition table
/// that has already overwritten the tail: Syntaks's line for the road in 3 of PlayTak 878681 16B is
/// two moves long, and topaz's own lines on the flat win of 812697 lose a move or two the same way.
/// So where the line runs out before the game does, `more` is asked to search the position it ran
/// out in — the engine that proved the win, continuing its own line — and the walk carries on from
/// what comes back. A proven win a few plies off is the cheapest thing a search ever solves, and this
/// only happens on plies that carry one. It is asked at most once per ply the score says is left,
/// and a walk that has gone that far without the game ending stops there.
///
/// A line that still stops short, or that ends the game in the other player's favour or in a draw,
/// leaves the kind unknown rather than guessed. The board is left as it was found.
///
/// Returns the line as walked, too — `pv` with whatever `more` added, up to where the walk stopped —
/// so a caller can show the win all the way to the end rather than the stub the table kept.
fn mate_of<B: TakBoard>(
    board: &mut B,
    score_white: i32,
    pv: &[GameMove],
    more: &mut dyn FnMut(&mut B) -> anyhow::Result<Vec<GameMove>>,
) -> anyhow::Result<(Option<Mate>, Vec<GameMove>)> {
    if !is_solved(score_white) || !is_real_score(score_white) {
        return Ok((None, pv.to_vec()));
    }
    let winner = if score_white > 0 { Color::White } else { Color::Black };
    let plies = (WIN_SCORE - score_white.abs()).max(1) as u32;
    let mut line: Vec<GameMove> = pv.to_vec();
    let mut played: Vec<RevGameMove> = Vec::with_capacity(plies as usize);
    let mut asked = 0;
    let mut kind = None;
    let walked = (|| -> anyhow::Result<()> {
        while (played.len() as u32) < plies {
            if played.len() == line.len() {
                if asked == plies {
                    break;
                }
                asked += 1;
                let rest = more(board)?;
                if rest.is_empty() {
                    break;
                }
                line.extend(rest);
            }
            let mv = line[played.len()];
            if !board.legal_move(mv) {
                break;
            }
            played.push(board.do_move(mv));
            if let Some(result) = board.game_result() {
                if result == GameResult::win_by(winner) {
                    kind = Some(if board.road(winner) { MateKind::Road } else { MateKind::Flats });
                }
                break;
            }
        }
        Ok(())
    })();
    // Past the original line, keep only what was actually played: an extension can bring back moves
    // beyond where the walk stopped. Within it, keep everything, as `pv_ptn` would have.
    line.truncate(played.len().max(pv.len()));
    while let Some(rev) = played.pop() {
        board.reverse_move(rev);
    }
    walked?;
    Ok((Some(Mate { winner, plies, kind }), line))
}

/// Grade a ply off the bar: the position before it against the position it reached, each read the
/// way the sheet draws it (`Searched::smoothed`), in the mover's frame. See the module docs.
///
/// `played` is what the position the ply reached is worth to the mover — the negated bar of that
/// position, or the pinned result of a game the ply ended — and `None` when its search completed no
/// iteration, so there is no bar to read. Either side without one leaves the ply ungraded.
fn bar_grade(before: &Searched, played: Option<i32>, played_best: bool) -> Grade {
    let cp_before = before.top();
    let Some(played) = played.filter(|_| before.deepest().is_some()) else {
        return Grade { cp_before, ..Default::default() };
    };
    let reference = before.smoothed();
    let sharp = if played_best { before.root_gap() } else { None };
    // A ply the engine chose is compared against the runner-up it beat, both ends off the parent's
    // own root list — same search, same depth, so `top()` and the gap back from it are exactly
    // `roots[0]` and `roots[1]`. Every other ply is compared against the bar it was played from.
    let (cmp_played, cmp_other) = match sharp {
        Some(gap) => (cp_before, cp_before - gap.max(0)),
        None if played_best => (cp_before, cp_before),
        None => (played, reference),
    };
    Grade {
        loss: if played_best { 0 } else { (reference - played).max(0) },
        exact: true,
        sharp,
        cp_before,
        cmp_played,
        cmp_other,
    }
}

/// What a finished game is worth to `player`, in their own frame.
fn terminal_value(result: GameResult, player: Color) -> i32 {
    match result {
        GameResult::WhiteWin => {
            if player == Color::White { WIN_SCORE } else { LOSE_SCORE }
        }
        GameResult::BlackWin => {
            if player == Color::Black { WIN_SCORE } else { LOSE_SCORE }
        }
        GameResult::Draw => 0,
    }
}

/// The best move other than the one played, searched the way the played move was — the evidence
/// behind "was this loss the mover's doing?".
///
/// **Why `cp_before` cannot answer that.** It comes from a multi-PV search of the position, spreading
/// one budget across every root move, while `solved_loss` comes from a dedicated search of one child
/// one ply deeper. So "the opponent has a forced win after your move" becomes provable long before
/// "you were lost whatever you played" does, and a position where every move loses can still report
/// an ordinary evaluation. Measured on Takana's games: in two positions reading -430 and -778cp,
/// every one of the top 16 alternatives was a *proven* loss once searched individually at the same
/// budget the played move got.
///
/// **One alternative, not the whole root list.** The best move's verdict matched the full 16-move
/// sweep in every position measured, and each further move is another full search. This is a probe,
/// not a proof: it says the best way out also loses, which is not quite "every move loses", and the
/// two come apart only where the root ordering is itself meaningless.
struct BestAlt {
    display: String,
    /// Its value in the **mover's** frame, like `Grade::cmp_played`.
    value: i32,
    /// It hands the opponent a proven win as well.
    solved_loss: bool,
}

/// Play `alt` from the position before the ply, search it on the played move's own terms, and report
/// what it would have come to. Leaves the board as it found it.
fn best_alt_outcome<B, S>(
    board: &mut B,
    searcher: &mut S,
    alt: GameMove,
    player: Color,
    nodes: &mut u64,
) -> anyhow::Result<BestAlt>
where
    B: TakBoard,
    S: PositionSearch<B>,
{
    // Spelled before the move, while the source stack is still intact.
    let display = short_ptn(board, alt);
    let rev = board.do_move(alt);
    let (value, solved_loss) = match board.game_result() {
        // A move that ends the game is pinned rather than searched, exactly as the played move is.
        Some(result) => (
            terminal_value(result, player),
            match result {
                GameResult::WhiteWin => player == Color::Black,
                GameResult::BlackWin => player == Color::White,
                GameResult::Draw => false,
            },
        ),
        None => match searcher.search(board, Breadth::Grading) {
            Ok((searched, spent)) => {
                *nodes += spent;
                // The search scores the position from the *opponent's* view — they are to move
                // there — so the mover's value is its negation, and a proven win for them is the
                // loss this is asking about. Same test `solved_loss` applies to the played move.
                (-searched.score, searched.score >= WIN_SCORE - SOLVED_MARGIN)
            }
            Err(e) => {
                board.reverse_move(rev);
                return Err(e);
            }
        },
    };
    board.reverse_move(rev);
    Ok(BestAlt { display, value, solved_loss })
}

/// Convert a side-to-move score to the fixed White-perspective frame the sheet's bars use.
fn to_white_frame<B: TakBoard>(board: &B, score: i32) -> i32 {
    if board.side_to_move() == Color::White {
        score
    } else {
        -score
    }
}

/// Does the side to move have a road-completing move available? That is exactly "the opponent is in
/// tak".
fn has_road_threat<B: TakBoard>(board: &B, storage: &mut Vec<GameMove>) -> bool {
    storage.clear();
    board.can_make_road(storage, None).is_some()
}

/// Play `mv` and measure everything a badge is scored from.
///
/// The board is **left on the position the move reached**, with the handle to undo it returned: the
/// played ply walks on from there, an alternative reverses immediately. That is the whole reason
/// this is one function — the played move and the moves it was chosen over have to be measured
/// identically or a caller comparing them is comparing two different measurements.
///
/// `threat_before` / `in_tak_before` / `flats_before` are properties of the position *before* the
/// move, so they are read once per ply and passed in rather than recomputed per move.
/// `capture_targets` is a scratch buffer, clobbered on every call.
fn play_and_measure<B: TakBoard>(
    board: &mut B,
    mv: GameMove,
    player: Color,
    flats_before: [i32; 2],
    threat_before: bool,
    in_tak_before: bool,
    storage: &mut Vec<GameMove>,
    capture_targets: &mut Vec<(usize, u32)>,
) -> (MoveFacts, RevGameMove) {
    // What the ply is about to move, read while the source stack is intact.
    let move_top = mv
        .is_stack_move()
        .then(|| board.index(mv.src_index()).top())
        .flatten();

    // Every square this spread drops onto, with the height it has right now. Which of these the
    // mover actually ends up owning can only be settled after the move.
    capture_targets.clear();
    if mv.is_stack_move() {
        capture_targets.extend(
            mv.quantity_iter(B::SIZE)
                .map(|step| (step.index, board.index(step.index).len() as u32)),
        );
    }

    let rev = board.do_move(mv);
    let terminal = board.game_result();

    // A move that *wins outright* is never a call: the opponent gets no turn, so there was never a
    // threat for them to answer. A game that merely stops here — resignation, timeout — is
    // different: the opponent did have their chance and didn't take it, so that last ply counts
    // like any other. Beyond that, a call is a *transition* into tak.
    let call = if terminal.is_some() {
        false
    } else {
        board.null_move();
        let threat_after = has_road_threat(board, storage);
        board.rev_null_move();
        threat_after && !threat_before
    };

    // Defending tak is the mirror image — but *not* in how it treats a game-ending ply, which is
    // where the symmetry with `call` stops. A move that wins outright cannot call, since the
    // opponent gets no turn to be threatened on; it can perfectly well defend, because the threat
    // it answers was already on the board when it was played. A player in tak who wins by taking
    // the road either broke that threat on the way past or won the race and left it standing, and
    // those are different moves. So the question is asked of the position the move reached, ending
    // or not: the opponent is to move there, so their road-completing move is read directly, with
    // no null move needed.
    let defends_tak = in_tak_before && !has_road_threat(board, storage);

    // The mover has a road now. Since a position with one already on it is a finished game, having
    // one *after* the move means this move built it — so this is "the move that took the road",
    // which is not the same thing as "the move that won": a flat win ends the game with no road,
    // and a spread that completes the *opponent's* road hands them the win and leaves this false.
    let completes_road = board.road(player);

    // How much of what this ply reached ended up in the opponent's hands: of the squares it
    // touched — the origin, plus every square a spread dropped onto — how many does an opponent
    // piece top now. A *state*, not a change: a square they already controlled and still control,
    // having been spread across, counts the same as one this move uncovered.
    let ocs_added = std::iter::once(mv.src_index())
        .chain(capture_targets.iter().map(|&(idx, _)| idx))
        .filter(|&idx| board.index(idx).top().map_or(false, |p| p.owner() != player))
        .count() as u32;

    // The tallest of those squares the mover came out on top of, measured both ways: the height it
    // had going in, and the height it has now. Same squares, same ownership filter — a drop that
    // left an opponent's piece face-up took nothing, so it is excluded from both — the two differ
    // only in when the tape measure comes out. See the field docs for why both exist.
    let (largest_capture, largest_capture_after) = capture_targets
        .iter()
        .filter(|&&(idx, _)| board.index(idx).top().map_or(false, |p| p.owner() == player))
        .fold((0u32, 0u32), |(before, after), &(idx, height)| {
            (before.max(height), after.max(board.index(idx).len() as u32))
        });

    let flats = [
        board.bits().flat_score(Color::White) as i32,
        board.bits().flat_score(Color::Black) as i32,
    ];

    // Flat-count differential: how far this move moved the mover's flat lead. A flat placement is
    // +1, a wall or capstone 0, a spread whatever it covers and uncovers — including negative when
    // it buries the mover's own flat.
    let mover = color_idx(player);
    let opponent = 1 - mover;
    let fcd = (flats[mover] - flats[opponent]) - (flats_before[mover] - flats_before[opponent]);

    let facts = MoveFacts {
        move_top,
        terminal,
        call,
        defends_tak,
        completes_road,
        ocs_added,
        largest_capture,
        largest_capture_after,
        flats,
        fcd,
    };
    (facts, rev)
}

fn analyze_board<B, S>(
    mut board: B,
    cfg: &AnalyzeGameConfig,
    searcher: &mut S,
) -> anyhow::Result<Analysis>
where
    B: TakBoard,
    S: PositionSearch<B>,
{
    let mut storage: Vec<GameMove> = Vec::with_capacity(128);
    // Reused across plies: (square, height before the move) for each square a spread drops onto.
    let mut capture_targets: Vec<(usize, u32)> = Vec::with_capacity(8);

    // How many plies came before the first one we analyze. Zero from the empty board; recovered
    // from the position when starting out of a TPS, since the caller's badge thresholds key on a
    // ply's number *in the game* (the opening bands, the earliest ply a rarity badge can reach).
    // White's move N is ply 2N-1, Black's is 2N.
    let ply_offset = (board.move_num() - 1) * 2
        + if board.side_to_move() == Color::Black { 1 } else { 0 };

    let mut out = Vec::with_capacity(cfg.plies.len());
    let mut stats = [PlayerStats::default(); 2];
    let mut result = None;
    let mut nodes = 0u64;
    let start = Instant::now();

    // Leads are tracked over every position in the game, the initial one included (0 for both).
    for s in stats.iter_mut() {
        s.largest_flat_lead = 0;
        s.largest_reserve_lead = 0;
    }
    // Stack heights are read off the board rather than assumed empty, for the same reason `fcd` is:
    // a walk that starts from a TPS starts with stacks already standing.
    stats[color_idx(Color::White)].largest_stack = tallest_owned(&board, Color::White);
    stats[color_idx(Color::Black)].largest_stack = tallest_owned(&board, Color::Black);

    // Every position is searched once, and that one search does double duty: its score is the bar
    // for the ply that *reached* the position, and its root move list grades the ply played *from*
    // it. So the loop carries the previous position's search forward — only the starting position
    // needs an extra one.
    // Every search reports its own node count, not a lifetime total, so each is added here.
    let mut before = {
        let (s, spent) = searcher.search(&mut board, Breadth::Grading)?;
        nodes += spent;
        s
    };
    // Read off the board rather than assumed empty, so `fcd` on the first analyzed ply is right
    // when the run starts from a TPS.
    let mut flats_before = [
        board.bits().flat_score(Color::White) as i32,
        board.bits().flat_score(Color::Black) as i32,
    ];

    for (i, ptn) in cfg.plies.iter().enumerate() {
        if result.is_some() {
            anyhow::bail!("ply {} ({}) comes after the game already ended", i + 1, ptn);
        }
        let player = board.side_to_move();
        let move_number = board.move_num();
        // `best` hands the choice to the search that has already been run on this position;
        // otherwise the ply is read as PTN. Board-aware on purpose: that fills in the crush flag
        // for a wall smash written without the optional `*`, which is how every PlayTak-derived
        // move list writes them.
        let engine_chose = ptn.eq_ignore_ascii_case(BEST_PLY);
        let mv = if engine_chose {
            before.best_move().ok_or_else(|| {
                anyhow::anyhow!("ply {}: the search returned no move to play here", i + 1)
            })?
        } else {
            GameMove::try_from_ptn(ptn, &board)
                .ok_or_else(|| anyhow::anyhow!("ply {} is not valid PTN: {}", i + 1, ptn))?
        };
        if !board.legal_move(mv) {
            anyhow::bail!("ply {} is not legal here: {}", i + 1, ptn);
        }

        // "Was the opponent already in tak coming into this ply?" — the mover is to move, so their
        // own road-completing move is the threat.
        let threat_before = has_road_threat(&board, &mut storage);

        // The same question pointed the other way: is the *mover* in tak? That is the opponent's
        // road-completing move, so it takes a null move to hand them the turn and ask.
        let in_tak_before = {
            board.null_move();
            let t = has_road_threat(&board, &mut storage);
            board.rev_null_move();
            t
        };

        // Was this the engine's own choice? Such a ply gives up nothing by definition, and is
        // compared against the runner-up rather than against the bar (see `bar_grade`).
        let played_best = before.best_move() == Some(mv);

        // Display PTN, computed before the move while the source stack is still intact.
        let display = short_ptn(&board, mv);

        // Move classification. On ply 1 the placed piece is the opponent's colour, but the ply still
        // belongs to the mover.
        let ps = &mut stats[color_idx(player)];
        if mv.is_place_move() {
            let piece = mv.place_piece();
            if piece.is_cap() {
                ps.cap_placements += 1;
            } else if piece.is_wall() {
                ps.wall_placements += 1;
            } else {
                ps.flat_placements += 1;
            }
        } else {
            ps.movements += 1;
        }

        // Play it and measure it. `rev_played` is kept so the alternatives below, and the best
        // alternative on a ply that lost the game, can be played from the position before it.
        let (facts, rev_played) = play_and_measure(
            &mut board,
            mv,
            player,
            flats_before,
            threat_before,
            in_tak_before,
            &mut storage,
            &mut capture_targets,
        );
        let MoveFacts {
            move_top,
            terminal,
            call,
            defends_tak,
            completes_road,
            ocs_added,
            largest_capture,
            largest_capture_after,
            flats,
            fcd,
        } = facts;
        if call {
            stats[color_idx(player)].calls += 1;
        }

        let reserves = [
            board.pieces_reserve(Color::White) as i32,
            board.pieces_reserve(Color::Black) as i32,
        ];
        let tallest = [
            tallest_owned(&board, Color::White),
            tallest_owned(&board, Color::Black),
        ];
        for (idx, s) in stats.iter_mut().enumerate() {
            let other = 1 - idx;
            s.largest_flat_lead = s.largest_flat_lead.max(flats[idx] - flats[other]);
            // A reserve *lead* belongs to whoever has spent more, i.e. has fewer left.
            s.largest_reserve_lead = s.largest_reserve_lead.max(reserves[other] - reserves[idx]);
            s.largest_stack = s.largest_stack.max(tallest[idx]);
        }

        // Search the position this ply reached. Its score is this ply's bar; its root moves grade
        // the next ply. A game-ending ply is pinned instead of searched.
        // `search_nodes` is what that search cost, kept per ply rather than only summed into the
        // run's total: it is the honest answer to "how hard was this position looked at?", which
        // the budget alone cannot give once a search finishes early.
        let mut search_nodes = None;
        let after = match terminal {
            Some(GameResult::WhiteWin) => {
                Searched::terminal(if player == Color::White { WIN_SCORE } else { LOSE_SCORE })
            }
            Some(GameResult::BlackWin) => {
                Searched::terminal(if player == Color::Black { WIN_SCORE } else { LOSE_SCORE })
            }
            Some(GameResult::Draw) => Searched::terminal(0),
            None => {
                let (s, spent) = searcher.search(&mut board, Breadth::Grading)?;
                search_nodes = Some(spent);
                nodes += spent;
                s
            }
        };

        let (score_white, win_prob_white) = match terminal {
            Some(GameResult::WhiteWin) => (i32::MAX, 1.0),
            Some(GameResult::BlackWin) => (i32::MIN, 0.0),
            Some(GameResult::Draw) => (0, 0.5),
            None => {
                // The curve is read through `DEPTH_KERNEL`, not off the deepest iteration alone.
                // Adjacent positions rarely stop at the same depth, and the bar moving because the
                // *search* changed rather than because the *position* did is what put visible steps
                // on plies the engine itself would have played. See `DEPTH_KERNEL` for the numbers.
                let s = to_white_frame(&board, after.smoothed());
                (s, sigmoid(s as f64 / EVAL_SCALE))
            }
        };
        result = terminal;

        // A proof's line finished off by the engine that proved it, where the table lost the end of
        // it (`mate_of`). Same searcher, same budget; what it spends is counted like any search.
        let mut finish = |b: &mut B| -> anyhow::Result<Vec<GameMove>> {
            let (s, spent) = searcher.search(b, Breadth::Grading)?;
            nodes += spent;
            Ok(s.pv)
        };

        // A proof is read off the same score as the bar, while the board still stands on the
        // position it describes. Never on a terminal ply: that game is over, not about to be.
        let (mate, line) = if terminal.is_none() {
            mate_of(&mut board, score_white, &after.pv, &mut finish)?
        } else {
            (None, after.pv.clone())
        };

        // The line the search was looking at, read from the position this ply *reached* — so it is
        // the engine's answer to "and then what?", with its recommended reply first — and, on a
        // proven win, carried on to the end of the game. Taken here, while the board still sits on
        // that position and before the alternatives below step back off it.
        let pv = pv_ptn(&mut board, &line);

        // ...and the other line that search looked at. Same position, same search, same depth, so
        // its score needs no alignment against `score_white` — the two are the best and
        // second-best readings of one position. Spelled here for the same reason the PV is: the
        // board is standing on the position these moves are played in, which is what `short_ptn`
        // and `pv_ptn` need to get the count and the crush flag right.
        let runner_up = match after.runner_up() {
            Some((mv, cp, line)) => {
                let score_white = to_white_frame(&board, cp);
                let (mate, line) = mate_of(&mut board, score_white, line, &mut finish)?;
                Some(RunnerUp {
                    display: short_ptn(&board, mv),
                    score_white,
                    pv: pv_ptn(&mut board, &line),
                    mate,
                })
            }
            None => None,
        };

        // The same iteration-by-iteration record `align` grades from, put on the sheet's fixed White
        // footing so a consumer can replay any pairing depth (or any smoothing of the curve) without
        // re-searching. Read here, while the board still stands on the position `after` describes.
        let depth_scores: Vec<(usize, i32)> = if cfg.depth_scores {
            after.depths.iter().map(|&(d, s)| (d, to_white_frame(&board, s))).collect()
        } else {
            Vec::new()
        };

        // ---- Grading: how far the bar fell ----
        //
        // What the position the ply reached is worth to the mover, read as the bar reads it — the
        // pinned result for a ply that ended the game, and nothing at all for a search that finished
        // no iteration. See `bar_grade`.
        let played_value = match terminal {
            Some(result) => Some(terminal_value(result, player)),
            None => after.deepest().map(|_| -after.smoothed()),
        };
        let grade = bar_grade(&before, played_value, played_best);

        // ---- The other moves that were available ----
        //
        // One multi-PV search of the position this ply was played *in*, so every score on the list
        // sits on one footing (see `Alternatives`). It is a second search of a position already
        // searched once, which is why it runs only on the plies the caller's gates admit — and why
        // those gates are read here rather than left to the caller to apply afterwards.
        let win_prob_before = sigmoid(grade.cp_before as f64 / EVAL_SCALE);
        // Normalised off the grade's own endpoints, not off `cp_before - loss`: those two numbers
        // can come from different searches, so reconstructing the played value from them is what
        // this gate used to do wrong — and a ply whose loss read as thousands of cp was skipped
        // here for being far past the caller's threshold when it had barely given anything up.
        let loss_pct = grade.loss_win_pct().unwrap_or(f64::INFINITY);
        // How settled the game was, from the leader's side whichever that is, and on whichever
        // side of the ply reads more decided (see `alts_min_decided`). The mover's own probability
        // is the wrong reading here: a position one player has locked up is equally locked up for
        // the other, and the moves worth enumerating alternatives for come from both of them.
        //
        // Both sides of the ply, because a caller may gate a move on the position it *reached* —
        // and a ply that crossed the line while being played is exactly the one whose alternatives
        // it then has nothing to appeal on. `win_prob_white` is the same reading through the bar;
        // decidedness is symmetric, so the frame it is quoted in does not matter.
        let decided = win_prob_before
            .max(1.0 - win_prob_before)
            .max(win_prob_white.max(1.0 - win_prob_white));
        let alts = if cfg.alts > 0
            && decided >= cfg.alts_min_decided
            && loss_pct <= cfg.alts_max_loss_pct
            && cfg
                .alts_irregular
                .map_or(true, |gate| gate.admits(move_top, fcd, ocs_added))
        {
            board.reverse_move(rev_played);
            let (searched, alt_nodes) =
                searcher.search(&mut board, Breadth::Alternatives(cfg.alts))?;
            nodes += alt_nodes;

            let mut played_cp = None;
            let mut moves = Vec::with_capacity(searched.roots.len());
            for &(alt_mv, cp) in searched.roots.iter() {
                // A PV index that found no move comes back holding a window bound rather than an
                // evaluation; ranking against that would be meaningless.
                if !is_real_score(cp) {
                    continue;
                }
                if alt_mv == mv {
                    played_cp = Some(cp);
                    continue;
                }
                // Spelled before the move, while the source stack is still intact.
                let display = short_ptn(&board, alt_mv);
                let (facts, rev) = play_and_measure(
                    &mut board,
                    alt_mv,
                    player,
                    flats_before,
                    threat_before,
                    in_tak_before,
                    &mut storage,
                    &mut capture_targets,
                );
                board.reverse_move(rev);
                moves.push(Alternative { display, cp, facts });
            }
            board.do_move(mv);
            Some(Alternatives { played_cp, nodes: alt_nodes, moves })
        } else {
            None
        };

        // Did this ply hand the opponent a proven win? Either it ended the game in their favour, or
        // the search of the position it reached shows a forced win for them (they are to move there).
        let solved_loss = match terminal {
            Some(GameResult::WhiteWin) => player == Color::Black,
            Some(GameResult::BlackWin) => player == Color::White,
            Some(GameResult::Draw) => false,
            None => after.score >= WIN_SCORE - SOLVED_MARGIN,
        };

        // Would the best alternative have lost as well? Asked only on the plies where the answer
        // changes something, which is what keeps it to one extra search on one ply in a game or
        // three rather than on every ply of every game.
        let best_alt = match solved_loss.then(|| before.best_other_move(mv)) {
            Some(Some(alt)) => {
                board.reverse_move(rev_played);
                let out = best_alt_outcome(&mut board, searcher, alt, player, &mut nodes)?;
                board.do_move(mv);
                Some(out)
            }
            _ => None,
        };

        if !cfg.quiet {
            eprintln!(
                "analyzegame: ply {}/{} {} -> {:+.1} cp ({:.3})",
                i + 1,
                cfg.plies.len(),
                ptn,
                if terminal.is_some() { f64::NAN } else { score_white as f64 / 4.0 },
                win_prob_white
            );
        }

        out.push(PlyAnalysis {
            index: ply_offset + i + 1,
            // A `best` ply was never "given" to us, so there is no original spelling to echo back;
            // the display form is what the engine chose.
            ptn: if engine_chose { display.clone() } else { ptn.clone() },
            display,
            player,
            move_number,
            score_white,
            win_prob_white,
            terminal,
            call,
            defends_tak,
            completes_road,
            flats,
            fcd,
            ocs_added,
            move_top,
            largest_capture,
            largest_capture_after,
            pv,
            runner_up,
            mate,
            search_depth: after.deepest(),
            search_nodes,
            // Already computed above: the `--alts` gate reads it too.
            win_prob_before,
            solved_loss,
            best_alt,
            grade,
            played_best,
            alts,
            depth_scores,
        });

        flats_before = flats;
        before = after;
    }

    Ok(Analysis {
        plies: out,
        stats,
        result,
        nodes,
        seconds: start.elapsed().as_secs_f64(),
    })
}

/// What a movement had on top of the stack it moved, as the bot spells it.
fn move_top_str(p: Piece) -> &'static str {
    if p.is_cap() {
        "cap"
    } else if p.is_wall() {
        "wall"
    } else {
        "flat"
    }
}

/// A string for a JSON string literal. PTN never needs it; an engine's self-reported name might.
fn json_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            c if (c as u32) < 0x20 => vec![' '],
            c => vec![c],
        })
        .collect()
}

fn result_str(r: GameResult) -> &'static str {
    match r {
        GameResult::WhiteWin => "white",
        GameResult::BlackWin => "black",
        GameResult::Draw => "draw",
    }
}

fn player_stats_json(s: &PlayerStats) -> String {
    format!(
        "{{\"calls\": {}, \"flat_placements\": {}, \"wall_placements\": {}, \
         \"cap_placements\": {}, \"movements\": {}, \"largest_flat_lead\": {}, \
         \"largest_reserve_lead\": {}, \"largest_stack\": {}}}",
        s.calls,
        s.flat_placements,
        s.wall_placements,
        s.cap_placements,
        s.movements,
        s.largest_flat_lead,
        s.largest_reserve_lead,
        s.largest_stack
    )
}

/// The alternatives list, in human cp. Every move carries the same rarity fields the ply itself
/// reports, so a caller scores them with one code path — see `Alternatives` for what the list can
/// and cannot answer.
fn alts_json(a: &Alternatives) -> String {
    let moves: Vec<String> = a
        .moves
        .iter()
        .map(|m| {
            format!(
                "{{\"display\": \"{}\", \"cp\": {:.2}, \"fcd\": {}, \"ocs_added\": {}, \
                 \"move_top\": {}, \"largest_capture\": {}, \"largest_capture_after\": {}, \
                 \"call\": {}, \"defends_tak\": {}, \"completes_road\": {}, \"terminal\": {}}}",
                m.display,
                m.cp as f64 / 4.0,
                m.facts.fcd,
                m.facts.ocs_added,
                m.facts
                    .move_top
                    .map(|t| format!("\"{}\"", move_top_str(t)))
                    .unwrap_or_else(|| "null".to_string()),
                m.facts.largest_capture,
                m.facts.largest_capture_after,
                m.facts.call,
                m.facts.defends_tak,
                m.facts.completes_road,
                m.facts
                    .terminal
                    .map(|r| format!("\"{}\"", result_str(r)))
                    .unwrap_or_else(|| "null".to_string()),
            )
        })
        .collect();
    format!(
        "{{\"played_cp\": {}, \"nodes\": {}, \"moves\": [{}]}}",
        a.played_cp
            .map(|cp| format!("{:.2}", cp as f64 / 4.0))
            .unwrap_or_else(|| "null".to_string()),
        a.nodes,
        moves.join(", ")
    )
}

/// A proof as the JSON reports it — see `Mate`.
fn mate_json(m: Option<&Mate>) -> String {
    m.map(|m| {
        format!(
            "{{\"winner\": \"{}\", \"plies\": {}, \"kind\": {}}}",
            match m.winner {
                Color::White => "white",
                Color::Black => "black",
            },
            m.plies,
            match m.kind {
                Some(MateKind::Road) => "\"road\"",
                Some(MateKind::Flats) => "\"flats\"",
                None => "null",
            }
        )
    })
    .unwrap_or_else(|| "null".to_string())
}

fn to_json(cfg: &AnalyzeGameConfig, a: &Analysis, engine_id: &str) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str(&format!(
        "  \"size\": {}, \"half_komi\": {}, \"komi\": {}, \"nodes_per_ply\": {},\n",
        cfg.size,
        cfg.half_komi,
        cfg.half_komi as f64 / 2.0,
        cfg.nodes
    ));
    // Who searched, by the name the caller gave it (topaz when nobody else did), and how it
    // identified itself: the TEI `id name` line, version included, which is the nearest thing to a
    // build that the protocol carries.
    let engine_name = cfg.engine.as_ref().map_or("topaz", |e| e.name.as_str());
    s.push_str(&format!(
        "  \"engine\": {{\"name\": \"{}\", \"id\": \"{}\"}},\n",
        json_escape(engine_name),
        json_escape(engine_id)
    ));
    s.push_str(&format!(
        "  \"result\": {},\n",
        a.result
            .map(|r| format!("\"{}\"", result_str(r)))
            .unwrap_or_else(|| "null".to_string())
    ));
    s.push_str("  \"plies\": [\n");
    let mut rows: Vec<String> = Vec::with_capacity(a.plies.len());
    for p in &a.plies {
        // Terminal plies carry no meaningful cp; the bar is pinned via win_prob instead.
        let cp = if p.terminal.is_some() {
            "null".to_string()
        } else {
            format!("{:.2}", p.score_white as f64 / 4.0)
        };
        let g = &p.grade;
        rows.push(format!(
            "    {{\"index\": {}, \"move_number\": {}, \"player\": \"{}\", \"ptn\": \"{}\", \
             \"display\": \"{}\", \"cp_white\": {}, \"win_prob_white\": {:.6}, \"call\": {}, \
             \"defends_tak\": {}, \"completes_road\": {}, \"terminal\": {}, \"flats\": [{}, {}], \
             \"loss_cp\": {:.2}, \
             \"loss_exact\": {}, \"sharp_cp\": {}, \"fcd\": {}, \"ocs_added\": {}, \
             \"move_top\": {}, \"largest_capture\": {}, \"largest_capture_after\": {}, \
             \"pv\": [{}], \"runner_up\": {}, \"mate\": {}, \
             \"cp_before\": {:.2}, \
             \"cmp_played_cp\": {:.2}, \"cmp_other_cp\": {:.2}, \
             \"win_prob_before\": {:.6}, \
             \"solved_loss\": {}, \"best_alt\": {}, \
             \"played_best\": {}, \"search_depth\": {}, \"search_nodes\": {}{}{}}}",
            p.index,
            p.move_number,
            match p.player {
                Color::White => "white",
                Color::Black => "black",
            },
            p.ptn,
            p.display,
            cp,
            p.win_prob_white,
            p.call,
            p.defends_tak,
            p.completes_road,
            p.terminal
                .map(|r| format!("\"{}\"", result_str(r)))
                .unwrap_or_else(|| "null".to_string()),
            p.flats[0],
            p.flats[1],
            g.loss as f64 / 4.0,
            g.exact,
            g.sharp
                .map(|s| format!("{:.2}", s as f64 / 4.0))
                .unwrap_or_else(|| "null".to_string()),
            p.fcd,
            p.ocs_added,
            p.move_top
                .map(|t| format!("\"{}\"", move_top_str(t)))
                .unwrap_or_else(|| "null".to_string()),
            p.largest_capture,
            p.largest_capture_after,
            p.pv
                .iter()
                .map(|m| format!("\"{m}\""))
                .collect::<Vec<_>>()
                .join(", "),
            p.runner_up
                .as_ref()
                .map(|r| format!(
                    "{{\"display\": \"{}\", \"cp_white\": {:.2}, \"pv\": [{}], \"mate\": {}}}",
                    r.display,
                    r.score_white as f64 / 4.0,
                    r.pv.iter().map(|m| format!("\"{m}\"")).collect::<Vec<_>>().join(", "),
                    mate_json(r.mate.as_ref())
                ))
                .unwrap_or_else(|| "null".to_string()),
            mate_json(p.mate.as_ref()),
            g.cp_before as f64 / 4.0,
            g.cmp_played as f64 / 4.0,
            g.cmp_other as f64 / 4.0,
            p.win_prob_before,
            p.solved_loss,
            p.best_alt
                .as_ref()
                .map(|b| format!(
                    "{{\"display\": \"{}\", \"cp\": {:.2}, \"solved_loss\": {}}}",
                    b.display,
                    b.value as f64 / 4.0,
                    b.solved_loss
                ))
                .unwrap_or_else(|| "null".to_string()),
            p.played_best,
            p.search_depth
                .map(|d| d.to_string())
                .unwrap_or_else(|| "null".to_string()),
            p.search_nodes
                .map(|n| n.to_string())
                .unwrap_or_else(|| "null".to_string()),
            // Present only on the plies `--alts` was asked to spend a search on.
            p.alts
                .as_ref()
                .map(|a| format!(", \"alts\": {}", alts_json(a)))
                .unwrap_or_default(),
            if p.depth_scores.is_empty() {
                String::new()
            } else {
                format!(
                    ", \"depth_scores\": [{}]",
                    p.depth_scores
                        .iter()
                        .map(|&(d, s)| format!("[{}, {:.2}]", d, s as f64 / 4.0))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
        ));
    }
    s.push_str(&rows.join(",\n"));
    s.push_str("\n  ],\n");
    s.push_str(&format!(
        "  \"stats\": {{\"white\": {}, \"black\": {}}},\n",
        player_stats_json(&a.stats[0]),
        player_stats_json(&a.stats[1])
    ));
    s.push_str(&format!(
        "  \"nodes\": {}, \"seconds\": {:.3}\n}}\n",
        a.nodes, a.seconds
    ));
    s
}

/// The position to start from: the empty board, or `cfg.tps`. Komi comes from `cfg` either way — a
/// `!komi` rider on the TPS is overridden, since the caller knows the game's rules and the string
/// may not carry them.
fn start_board<B: TakBoard>(cfg: &AnalyzeGameConfig) -> anyhow::Result<B> {
    let board = match &cfg.tps {
        Some(tps) => B::try_from_tps(tps.trim())?,
        None => B::start_position(),
    };
    let board = board.with_komi(cfg.half_komi);
    Ok(match cfg.double_black_stack {
        Some(on) => board.with_double_black_stack(on),
        None => board,
    })
}

/// A whole game on topaz's own search.
fn analyze_native<B, E>(cfg: &AnalyzeGameConfig) -> anyhow::Result<Analysis>
where
    B: TakBoard,
    E: Evaluator<Game = B> + Default,
{
    let table = HashTable::new(TT_SIZE);
    let mut searcher = NativeSearch {
        eval: E::default(),
        table: &table,
        info: make_info(cfg.nodes, &table),
        nodes: cfg.nodes,
    };
    analyze_board(start_board::<B>(cfg)?, cfg, &mut searcher)
}

/// A whole game with another engine doing the searching, over TEI. Returns the engine's own
/// `id name` alongside the analysis.
fn analyze_tei<B: TakBoard>(
    cfg: &AnalyzeGameConfig,
    engine: &TeiEngineConfig,
) -> anyhow::Result<(Analysis, String)> {
    let board = start_board::<B>(cfg)?;
    let mut searcher = TeiSearch::start(
        engine,
        B::SIZE,
        cfg.half_komi,
        board.double_black_stack(),
        cfg.nodes,
        cfg.quiet,
    )?;
    let analysis = analyze_board(board, cfg, &mut searcher)?;
    Ok((analysis, searcher.id().to_string()))
}

pub fn run(cfg: AnalyzeGameConfig) -> anyhow::Result<()> {
    if cfg.plies.is_empty() {
        anyhow::bail!("no plies given");
    }
    let (analysis, engine_id) = match &cfg.engine {
        None => {
            let analysis = match cfg.size {
                4 => analyze_native::<Board4, NNUE4>(&cfg)?,
                5 => analyze_native::<Board5, NNUE5>(&cfg)?,
                6 => analyze_native::<Board6, NNUE6>(&cfg)?,
                n => anyhow::bail!("unsupported board size {n} (nets exist for 4x4, 5x5 and 6x6)"),
            };
            (analysis, "topaz".to_string())
        }
        Some(engine) => match cfg.size {
            4 => analyze_tei::<Board4>(&cfg, engine)?,
            5 => analyze_tei::<Board5>(&cfg, engine)?,
            6 => analyze_tei::<Board6>(&cfg, engine)?,
            n => anyhow::bail!("unsupported board size {n}"),
        },
    };
    let json = to_json(&cfg, &analysis, &engine_id);
    match &cfg.output {
        Some(path) => std::fs::write(path, &json)?,
        None => {
            let stdout = io::stdout();
            let mut lock = stdout.lock();
            lock.write_all(json.as_bytes())?;
            lock.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Position;

    /// White has a1-e1 and f1 to come: a road in one, White to move.
    const ROAD_IN_ONE: &str = "2,2,2,2,x2/x6/x6/x6/x6/1,1,1,1,1,x 1 5";

    /// A `more` with nothing to add.
    fn nothing(_: &mut Board6) -> anyhow::Result<Vec<GameMove>> {
        Ok(Vec::new())
    }

    fn mate(board: &mut Board6, score: i32, pv: &[GameMove]) -> Option<Mate> {
        mate_of(board, score, pv, &mut nothing).unwrap().0
    }

    #[test]
    fn a_mate_reads_its_distance_off_the_score_and_its_ending_off_the_line() {
        let mut board = Board6::try_from_tps(ROAD_IN_ONE).unwrap();
        let f1 = GameMove::try_from_ptn("f1", &board).unwrap();
        let m = mate(&mut board, WIN_SCORE - 1, &[f1]).unwrap();
        assert_eq!(m, Mate { winner: Color::White, plies: 1, kind: Some(MateKind::Road) });
        // The line is played back off the board, not left on it.
        assert!(board.game_result().is_none());
        assert_eq!(board.side_to_move(), Color::White);
    }

    #[test]
    fn a_line_that_stops_short_leaves_the_ending_unknown() {
        let mut board = Board6::try_from_tps(ROAD_IN_ONE).unwrap();
        assert_eq!(mate(&mut board, WIN_SCORE - 1, &[]).unwrap().kind, None);
        // ...and so does a line that ends the game for the other side than the score says.
        let f1 = GameMove::try_from_ptn("f1", &board).unwrap();
        assert_eq!(mate(&mut board, LOSE_SCORE + 1, &[f1]).unwrap().kind, None);
    }

    #[test]
    fn a_line_that_stops_short_is_finished_by_searching_where_it_stopped() {
        let mut board = Board6::try_from_tps(ROAD_IN_ONE).unwrap();
        let f1 = GameMove::try_from_ptn("f1", &board).unwrap();
        let mut asked = 0;
        let mut more = |b: &mut Board6| -> anyhow::Result<Vec<GameMove>> {
            asked += 1;
            assert_eq!(b.side_to_move(), Color::White, "asked from where the line ran out");
            Ok(vec![f1])
        };
        let (m, line) = mate_of(&mut board, WIN_SCORE - 1, &[], &mut more).unwrap();
        assert_eq!(m.unwrap().kind, Some(MateKind::Road));
        assert_eq!(line, vec![f1], "the finished line comes back for the PV");
        assert_eq!(asked, 1);
        assert!(board.game_result().is_none());
    }

    #[test]
    fn an_evaluation_is_not_a_mate() {
        let mut board = Board6::try_from_tps(ROAD_IN_ONE).unwrap();
        assert_eq!(mate(&mut board, 3000, &[]), None);
        assert_eq!(mate(&mut board, -(WIN_SCORE - SOLVED_MARGIN) + 1, &[]), None);
        assert_eq!(mate(&mut board, -(WIN_SCORE - 4), &[]).unwrap().winner, Color::Black);
    }

    /// A searched position worth `score` to the side to move, its root list best first.
    fn searched(score: i32, roots: &[i32]) -> Searched {
        let mv = GameMove::try_from_ptn("a1", &Board6::start_position()).unwrap();
        Searched {
            score,
            roots: roots.iter().map(|&s| (mv, s)).collect(),
            lines: Vec::new(),
            depths: vec![(9, score)],
            pv: Vec::new(),
        }
    }

    #[test]
    fn a_ply_gives_up_exactly_what_the_bar_falls() {
        // The mover's position read 200 before the ply and 80 after it: the endpoints are those two
        // bars, so the badge and the bar cannot tell different stories.
        let g = bar_grade(&searched(200, &[200, 150]), Some(80), false);
        assert_eq!((g.cmp_other, g.cmp_played, g.loss), (200, 80, 120));
        assert!(g.exact && g.sharp.is_none());
    }

    #[test]
    fn a_ply_whose_bar_rose_gave_up_nothing() {
        let g = bar_grade(&searched(200, &[200, 150]), Some(260), false);
        assert_eq!(g.loss, 0);
        assert_eq!(g.loss_win_pct(), Some(0.0));
    }

    #[test]
    fn a_position_with_no_bar_leaves_the_ply_ungraded() {
        // No completed iteration after the ply, or none before it: nothing to read, so no grade —
        // rather than a comparison against the zero an empty search falls back to.
        let g = bar_grade(&searched(200, &[200]), None, false);
        assert_eq!(g.loss_win_pct(), None);
        let g = bar_grade(&Searched::terminal(0), Some(80), false);
        assert_eq!(g.loss_win_pct(), None);
    }

    #[test]
    fn a_missed_forced_win_costs_what_the_position_was_still_worth() {
        // PlayTak 812697 ply 93, which is where this pair of fields came from. White played a move
        // worth +405 human cp — 98.3% — when a re-search proved a forced win was available. Missing it is real, but it cost the 1.7 points between 98.3%
        // and certainty, and nothing like the 88.9 the cp difference used to be normalised into.
        let g = Grade {
            cmp_other: WIN_SCORE - 8, // the alternative, proven
            cmp_played: 1620,         // +405 human cp, an ordinary evaluation
            cp_before: 832,           // +208 human cp: the parent's own search never saw the win
            exact: true,
            ..Default::default()
        };
        let lost = g.loss_win_pct().expect("an exact grade has a loss");
        assert!(lost < 2.0, "a 98.3% position should not read as a blunder, got {lost}");
    }

    #[test]
    fn the_engines_own_move_gives_up_nothing() {
        // A ply the engine chose is compared against the runner-up it beat, so the endpoints run the
        // other way and the clamp is what makes that a zero rather than a negative loss.
        let g = Grade { cmp_played: 400, cmp_other: 120, exact: true, ..Default::default() };
        assert_eq!(g.loss_win_pct(), Some(0.0));
    }

    #[test]
    fn an_unstable_runner_up_cannot_make_the_best_move_an_inaccuracy() {
        // PlayTak 812697 ply 48. Multi-PV re-searches each index from scratch, so the runner-up came
        // back *above* the move it lost to. That is a zero gap; left unclamped it reads as the best
        // move in the position having given ground, and the ply grades Inaccurate.
        let g = bar_grade(&searched(-159, &[-159, -93]), Some(-170), true);
        assert_eq!(g.loss, 0);
        assert_eq!(g.loss_win_pct(), Some(0.0));
        // ...and its endpoints are the parent's own root list, not the bar: the best move is
        // measured against the move it beat.
        assert_eq!((g.cmp_played, g.cmp_other), (-159, -159));
    }

    #[test]
    fn the_best_move_is_measured_against_the_runner_up() {
        let g = bar_grade(&searched(300, &[300, 120]), Some(250), true);
        assert_eq!((g.loss, g.sharp), (0, Some(180)));
        assert_eq!((g.cmp_played, g.cmp_other), (300, 120));
    }

    #[test]
    fn a_grade_with_nothing_to_compare_has_no_loss() {
        let g = Grade { cmp_played: 400, cmp_other: 120, exact: false, ..Default::default() };
        assert_eq!(g.loss_win_pct(), None);
    }

    #[test]
    fn a_single_weight_kernel_is_the_deepest_reading() {
        // The behaviour depth averaging replaced, kept reachable so the change is a knob and not a
        // rewrite: `&[1]` must reproduce exactly what reading the deepest iteration used to give.
        let ladder = [(9, 100), (10, 40), (11, 220)];
        assert_eq!(smooth_ladder(&ladder, 11, &[1]), Some(220));
        assert_eq!(smooth_ladder(&ladder, 10, &[1]), Some(40));
    }

    #[test]
    fn the_kernel_weights_the_deepest_reading_most() {
        // 3:2:1 over 220 / 40 / 100 = (660 + 80 + 100) / 6 = 140.
        let ladder = [(9, 100), (10, 40), (11, 220)];
        assert_eq!(smooth_ladder(&ladder, 11, &[3, 2, 1]), Some(140));
        // ...and the window follows `top` down the ladder when asked for a shallower reading.
        assert_eq!(smooth_ladder(&ladder, 10, &[3, 2, 1]), Some((40 * 3 + 100 * 2) / 5));
    }

    #[test]
    fn a_short_ladder_renormalises_instead_of_failing() {
        // Two completed iterations cannot fill a three-wide window. The weights that have readings
        // behind them are the ones that count: (220 * 3 + 40 * 2) / 5 = 148.
        let ladder = [(10, 40), (11, 220)];
        assert_eq!(smooth_ladder(&ladder, 11, &[3, 2, 1]), Some(148));
        // One iteration is just itself, and a depth that never completed has no reading at all.
        assert_eq!(smooth_ladder(&[(1, 60)], 1, &[3, 2, 1]), Some(60));
        assert_eq!(smooth_ladder(&ladder, 12, &[3, 2, 1]), None);
    }

    #[test]
    fn a_gap_in_the_ladder_stops_the_window() {
        // Only completed iterations are recorded, so depths need not be contiguous. Reaching past a
        // gap would average readings that are two apart as though they were adjacent.
        let ladder = [(8, 900), (10, 40), (11, 220)];
        assert_eq!(smooth_ladder(&ladder, 11, &[3, 2, 1]), Some(148));
    }

    #[test]
    fn a_proven_win_is_never_averaged_with_an_evaluation() {
        // A mate score is thousands of cp from any evaluation, so blending the two produces a number
        // that is neither a proof nor an eval — and the sigmoid is flat out there, so nothing
        // downstream would notice. The solved reading is reported alone.
        let solved_top = [(9, 100), (10, 120), (11, WIN_SCORE - 6)];
        assert_eq!(smooth_ladder(&solved_top, 11, &[3, 2, 1]), Some(WIN_SCORE - 6));

        // ...and a proof that appeared *below* the top ends the window there rather than being
        // mixed in, leaving the evaluations above it to stand on their own.
        let solved_below = [(9, WIN_SCORE - 8), (10, 120), (11, 220)];
        assert_eq!(smooth_ladder(&solved_below, 11, &[3, 2, 1]), Some((220 * 3 + 120 * 2) / 5));
    }

    #[test]
    fn averaging_rounds_away_from_zero_rather_than_toward_the_draw() {
        // Truncation would pull every averaged reading a fraction of a cp toward 0, which over a
        // whole game is a systematic bias toward "level" on exactly the balanced positions where the
        // sigmoid is steepest and a cp costs the most win probability.
        assert_eq!(smooth_ladder(&[(1, 1), (2, 2)], 2, &[1, 1]), Some(2)); // 1.5 -> 2
        assert_eq!(smooth_ladder(&[(1, -1), (2, -2)], 2, &[1, 1]), Some(-2)); // -1.5 -> -2
    }

}
