//! Whole-game analysis for TakAnalyzerBot's `/analyze` game sheet.
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
//! <move>` grades exactly one move, which is what TakAnalyzerBot's `scripts/positionAnalyzer.js`
//! uses to regression-test badge tuning against moves a human already has an opinion about. Ply
//! *numbering* is recovered from the position (`ply_offset`), not restarted at 1, so a ply analyzed
//! out of a TPS lands in the same opening/rarity bands it would have reached by replaying the game
//! to it. The ply `best` (`BEST_PLY`) stands in for a move where the caller would rather have the
//! engine's own choice than name one.
//!
//! **Move grading.** Every ply additionally carries the raw numbers a caller needs to award it a
//! badge — the thresholds themselves live in the bot, so they can be retuned without a rebuild:
//!   - `loss_cp` — how much worse than the best move, in the mover's frame. Exact when the search
//!     graded the move itself (`loss_exact`), otherwise inferred from the position it reached.
//!     **A diagnostic, not the grading input** — see `cmp_played_cp` below for what to grade on.
//!   - `cmp_played_cp` / `cmp_other_cp` — the two ends of that comparison, in the mover's frame and
//!     on one footing: the played move, and the move it was measured against (the best move, or the
//!     runner-up when the ply *was* the best move). Normalise both into win probability and subtract
//!     *there*. Subtracting them as cp is the one thing they must not be used for: a sibling
//!     re-search can prove a win the parent's search never saw, and a mate score minus an evaluation
//!     is not a quantity. See `Grade::cmp_played` for why `cp_before - loss_cp` cannot stand in.
//!   - `sharp_cp` — how much *better* than the best alternative to it. Negative whenever some other
//!     move was better, so "the only good move here" is simply a large positive value.
//!   - `fcd` — flat-count differential: how far the ply moved the mover's flat lead.
//!   - `cp_before` / `win_prob_before` — the position's value before the ply, mover's frame.
//!   - `solved_loss` — the ply handed the opponent a proven win.
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
//!
//! **How the loss is measured** (`Grading`). Two methods, because the obvious one is wrong:
//!
//! *Naive.* Difference a position's score against its child's. This does not work: the child search
//! sees a ply deeper, so the two scores have different horizons. Measured on plies the engine itself
//! picked — where the true loss is zero by definition — this showed swings of up to 27cp, enough to
//! badge a top move a blunder.
//!
//! *`Grading::Sibling`* (the original fix). Search the position the ply reached and the position the
//! *best* move would have reached, and compare those. Two children of the same parent, same budget,
//! same side to move, so the horizons match. Correct, but it doubles the searches.
//!
//! *`Grading::Aligned`* (the default). The horizon mismatch is a *parity* problem, not an inherent
//! one: a parent at depth `D` and its child at depth `D-1` look exactly the same distance down the
//! tree. `search` already records the score of every fully-completed iteration
//! (`SearchInfo::depth_scores`), so both readings come out of the one search the position needed
//! anyway — no second search, and no fixed depth limit either. For each adjacent pair the comparison
//! depth is `D = min(deepest(parent), deepest(child) + 1)`, so a position that ran out of nodes
//! early just gets compared shallower instead of being mismatched.
//!
//! This engine has no quiescence search — leaf nodes are a static eval behind a hard horizon (see
//! `alpha_beta`'s `depth == 0` arm) — so the parity effect is the dominant error term and aligning
//! it is worth more here than it would be in an engine that resolves captures at the leaf. What it
//! does *not* remove is search instability: the child is re-searched from scratch with a cleared TT,
//! so its move ordering, LMR and null-move decisions differ from the same subtree inside the
//! parent's search. `raw_cp` on the ply the engine itself chose is exactly that residual, and is
//! reported so it can be measured rather than assumed.
//!
//! The case alignment handles badly is a *lopsided* pair. `D` is capped by whichever position
//! searched shallower, so when the position before a ply reaches depth 7 and the position after it
//! reaches depth 10, the grade is computed from a 6-deep reading of a position the sheet draws at
//! depth 10 — and the sheet ends up showing a rising evaluation next to a Blunder badge. That gap is
//! measurable without any extra search (`MAX_GRADE_DRIFT`), and when it opens the ply is re-measured
//! the sibling way, which does not depend on alignment at all (`regraded` in the output marks them).
//!
//! How much of that gap is tolerated (`MAX_GRADE_DRIFT`) is the one number deciding how often a
//! badge lands a grade away from what a deeper search would say, and it is set from measurement:
//! below two win percentage points of drift the aligned grade is as good as either other method,
//! above it the method comes apart while the sibling measurement does not. It fires on 13-26% of
//! plies across the sample games, so most of the one-search-per-ply saving survives.
//!
//! A ply whose *parent* the search solved is the other way alignment fails, and it fails silently.
//! Solving stops the iteration, so the pair can come out at depth 2 — a proven win on one side and a
//! two-ply guess on the other — and the drift test cannot object, because win scores sit where the
//! sigmoid is flat and every difference out there reads as a couple of points. Hence `is_solved`: a
//! pairing that straddles the mate/evaluation boundary is rejected without consulting the drift.
//!
//! `sharp_cp` needs no extra search under either method: multi-PV 2 already returns the best and
//! second-best root moves with exact scores from the same search, at the same depth and for the same
//! side to move, so the gap between them is directly comparable.
//!
//! `Grading::Both` computes both and reports the sibling method alongside, for calibrating one
//! against the other.
//!
//! **The other moves that were available** (`--alts`). A caller grading a ply on how *hard it was to
//! see* eventually needs to know what else was on offer: a sacrifice is only remarkable if the quiet
//! moves were worse. `--alts N` asks for the position's best `N` root moves, each carrying the same
//! raw grading numbers the played ply does, so the caller can score them on its own tables. See
//! `Alternatives` for the shape and for what the list can and cannot answer.
//!
//! Enumerating them costs one extra search of the position, so it is gated: `--alts-min-win-prob`
//! and `--alts-max-loss` name the plies worth spending it on. Both are the *caller's* thresholds
//! handed to the engine — nothing here decides which plies matter, it only avoids searching the ones
//! the caller has already said it will ignore.

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
const SOLVED_MARGIN: i32 = 200;

/// Win-probability scale. `sigmoid(internal_score / EVAL_SCALE)`, matching the trainer's convention
/// (`datagen.rs`), i.e. equivalently `sigmoid(human_cp / 100)`.
const EVAL_SCALE: f64 = 400.0;

/// How far the aligned grade's reading of a position may sit from the deepest reading of that same
/// position before the grade is thrown out and re-measured against a sibling (see part 3 of the
/// grading). In win percentage points, the unit the badge bands are defined in.
///
/// **Measured, not reasoned.** Graded against an 8x-deeper reference run over two whole games (117
/// gradable plies), the aligned grade is indistinguishable from the sibling and naive measurements
/// while the drift stays under two points — all three sit around a point of error — and comes apart
/// above it: mean error 3.5 points at two, 7.5 at three, 25 at five, against a sibling measurement
/// that stays near two throughout. So this is where the method stops being trustworthy, and it fires
/// on about a tenth of plies, which is what it costs in extra sibling searches.
///
/// It was 5.0, and being *above* the width of a whole band is what made it useless: the bands it
/// feeds are 2 and 3 points wide (Good 1-3, Inaccurate 3-6), so a grade could land two bands from
/// what a deeper search says and still pass.
const MAX_GRADE_DRIFT: f64 = 2.0;

/// Which method measures how much a ply gave up. See the module docs for what each one does and why
/// the naive parent-vs-child difference is not among them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Grading {
    /// Search the best alternative move and compare siblings. One extra search per ply.
    Sibling,
    /// Compare the parent at depth `D` against the child at depth `D-1`, both read off the searches
    /// the positions needed anyway. No extra search.
    Aligned,
    /// Compute both and report the sibling numbers alongside, for calibration.
    Both,
}

impl Grading {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "sibling" => Some(Self::Sibling),
            "aligned" => Some(Self::Aligned),
            "both" => Some(Self::Both),
            _ => None,
        }
    }
    /// Does this mode need the extra sibling search?
    fn needs_sibling(self) -> bool {
        matches!(self, Self::Sibling | Self::Both)
    }
    /// Does this mode need the depth curve?
    fn needs_aligned(self) -> bool {
        matches!(self, Self::Aligned | Self::Both)
    }
}

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
    /// How to measure each ply's loss. `Aligned` is half the searches of `Sibling`.
    pub grading: Grading,
    /// How many root moves to enumerate alongside a ply (see `Alternatives`). 0 turns the whole
    /// thing off, which is the default; the search caps it at `MAX_MULTI_PV`.
    pub alts: usize,
    /// Only plies where the mover's win probability *going in* is at least this get an alternatives
    /// list. The caller's threshold, not the engine's — see the module docs.
    pub alts_min_win_prob: f64,
    /// ...and only those that gave up at most this many **win percentage points**, on the same
    /// normalised footing the caller grades on (`loss_cp` against `cp_before`). A ply the loss
    /// method could not grade at all never qualifies.
    pub alts_max_loss_pct: f64,
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
            grading: Grading::Aligned,
            alts: 0,
            alts_min_win_prob: 0.0,
            alts_max_loss_pct: f64::INFINITY,
        }
    }
}

/// One grading method's verdict on one ply, in the mover's frame and in internal cp.
#[derive(Clone, Copy, Default)]
struct Grade {
    /// How much the ply gave up. Clamped at 0, and pinned to 0 for the engine's own best move.
    loss: i32,
    /// The same comparison *before* clamping. For `Aligned` on a ply the engine itself chose this is
    /// the method's error — it should be 0 and isn't — which is the whole reason it is reported.
    /// (For `Sibling` on such a ply the reference is the runner-up instead, so this is just
    /// `-sharp`, not an error term.)
    raw: i32,
    /// False when the method had nothing to grade against; `loss` is then a meaningless 0.
    exact: bool,
    /// Gap back to the runner-up. `None` unless the ply was the engine's best move.
    sharp: Option<i32>,
    /// The value of the position before the ply — the deepest the search got, **not** whatever depth
    /// the loss happened to be measured at.
    ///
    /// These are deliberately different numbers. The loss needs matched horizons, which can force it
    /// down to a shallow pairing; the *position's value* has no such constraint and should always be
    /// the best estimate available, because that is what the bar draws and what decides how settled
    /// the game already was. Reading it at the comparison depth instead made a resolved position look
    /// live, which let a lost position keep collecting Fatal badges.
    cp_before: i32,
    /// The value this method assigned to the move actually played, on its own footing. Compared
    /// against the deepest reading of the same position to detect a grade the search has outgrown.
    played_value: i32,
    /// The two ends of the comparison this grade is about, in the mover's frame and — this is the
    /// whole point — **on one footing**, from one method's reading of both.
    ///
    /// `cmp_played` is the move that was played, `cmp_other` the move it is measured against. Which
    /// move that is follows `played_best`, and the two cases are mutually exclusive: a ply that was
    /// the engine's own choice is compared against the **runner-up** and gives up nothing, and a ply
    /// that was not is compared against the **best move** and is not sharp. So one pair of endpoints
    /// carries both `loss` and `sharp`.
    ///
    /// They exist because `loss` and `cp_before` cannot be used to recover them. `cp_before` is
    /// deliberately the *deepest* reading of the position before the ply (see above) while `loss`
    /// can be measured by a different search entirely — the sibling regrade re-searches from
    /// scratch — so `cp_before - loss` is a mix of two readings and can land nowhere near the value
    /// the ply actually reached. Consumers normalise these two into win probability and subtract
    /// *there*; that is the only subtraction that is meaningful when one end is a proven win and the
    /// other is an evaluation, because the sigmoid saturates instead of running off the scale.
    ///
    /// **On a `played_best` ply, `cmp_other` never sits above `cmp_played`.** It is the runner-up, so
    /// the pair runs the other way and the ply gave up nothing by definition — which is exactly what
    /// pinning `loss` to 0 says. The clamp that guarantees it is not cosmetic: each multi-PV index is
    /// a full re-search with the earlier PVs excluded at the root, and the sibling method searches
    /// the two moves independently, so either can hand back a runner-up that scored *above* the move
    /// it lost to. Left alone that reads as the best move in the position having given ground, which
    /// is how a Great move becomes an Inaccuracy.
    cmp_played: i32,
    cmp_other: i32,
    /// `Aligned` only: the depth the comparison was made at (parent `D`, child `D-1`).
    depth: Option<usize>,
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
    /// The other root moves, best first, excluding the played one.
    moves: Vec<Alternative>,
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
    /// the number with meaning. TakAnalyzerBot's `src/badges.js` picks between them on `move_top`
    /// and scores each against its own table.
    ///
    /// Always at least 1 for a flat-led spread — the carry keeps its order, so the leading flat is
    /// the last piece dropped and the final square is always the mover's.
    largest_capture_after: u32,
    /// The principal variation from the position this ply reached, in short PTN — the engine's
    /// recommended reply first, then the line it expects to follow. Empty for a terminal ply, and
    /// truncated at any move the position cannot actually play (see `pv_ptn`).
    pv: Vec<String>,
    /// Deepest fully-completed iteration of the search of the position this ply reached. `None` for
    /// a terminal ply, which is pinned rather than searched. The grading compares against the ply
    /// *before*, so a position that searched much shallower than its predecessor drags the whole
    /// comparison down to its own depth — which is worth being able to see.
    search_depth: Option<usize>,
    /// The mover's win probability before this ply, from the primary grade's `cp_before`.
    win_prob_before: f64,
    /// This ply handed the opponent a proven win.
    solved_loss: bool,
    /// The depth-aligned grade, when computed.
    aligned: Option<Grade>,
    /// The sibling-search grade, when computed.
    sibling: Option<Grade>,
    /// Whether this ply was the engine's own best move. The aligned method's `raw` is only an error
    /// term on these plies, so the comparison script needs to know which they are.
    played_best: bool,
    /// The aligned grade was discarded and the ply re-measured against a sibling, because the depth
    /// alignment landed on a reading the deeper search contradicts (see part 3 of the grading).
    regraded: bool,
    /// The other moves available here, when this ply passed the `--alts` gates.
    alts: Option<Alternatives>,
}

impl PlyAnalysis {
    /// The grade the exported `loss_cp` / `sharp_cp` / `cp_before` fields come from. `Both` reports
    /// the aligned numbers as primary and the sibling ones alongside.
    fn primary(&self) -> Grade {
        self.aligned.or(self.sibling).unwrap_or_default()
    }
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

/// The gap between two internal-cp scores expressed in win percentage points, which is how far apart
/// they are in the terms the badges are graded on rather than in raw cp. The same cp gap matters far
/// more in a balanced position than a decided one, and this is what says so.
///
/// **Blind out where the win scores live**, and unavoidably so: the sigmoid is flat there, so the
/// whole span from a comfortable win to a proven one compresses into the last few percentage points.
/// Callers using this to decide whether two readings *disagree* have to rule out a solved/unsolved
/// pairing separately — see `is_solved`.
fn drift_win_pct(a: i32, b: i32) -> f64 {
    (sigmoid(a as f64 / EVAL_SCALE) - sigmoid(b as f64 / EVAL_SCALE)).abs() * 100.0
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
struct Searched {
    score: i32,
    roots: Vec<(GameMove, i32)>,
    /// `(depth, score)` for every fully-completed iteration, ascending. Empty for a terminal
    /// position, which is pinned rather than searched.
    depths: Vec<(usize, i32)>,
    /// The principal variation of the deepest iteration — the line the score is *about*, best move
    /// first. Empty for a terminal position.
    pv: Vec<GameMove>,
}

impl Searched {
    /// A position that was never searched because the game had already ended there.
    fn terminal(score: i32) -> Self {
        Self { score, roots: Vec::new(), depths: Vec::new(), pv: Vec::new() }
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
    /// This position's score at a specific depth.
    fn score_at(&self, depth: usize) -> Option<i32> {
        self.depths.iter().find(|&&(d, _)| d == depth).map(|&(_, s)| s)
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
        depths: info.depth_scores().to_vec(),
        pv: outcome.map(|o| o.pv).unwrap_or_default(),
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

/// Grade a ply by comparing the parent at depth `D` against the child at depth `D-1`, which put the
/// same horizon under both. `D` is the deepest pairing both searches actually completed, so a
/// position that ran out of nodes early is compared shallower rather than mismatched.
///
/// `parent` is in the mover's frame; `child` is in the opponent's, hence the negation. Returns
/// `None` when no valid pairing exists — the child needs a completed depth 1, so the parent needs 2.
fn align(parent: &Searched, child: &Searched, played_best: bool) -> Option<Grade> {
    let depth = parent.deepest()?.min(child.deepest()? + 1);
    if depth < 2 {
        return None;
    }
    // The *reference* is the parent at the matched depth; the position's reported value is its
    // deepest reading. See `Grade::cp_before` for why those must not be the same number.
    let reference = parent.score_at(depth)?;
    let played_value = -child.score_at(depth - 1)?;
    let raw = reference - played_value;
    let sharp = if played_best { parent.root_gap() } else { None };
    // A ply the engine chose is compared against the runner-up, and both ends of *that* comparison
    // come from the parent's own root list rather than from the aligned pairing — same search, same
    // depth, so `top()` and the gap back from it are exactly `roots[0]` and `roots[1]`. Every other
    // ply is compared against the best move, which is the aligned pairing itself.
    let (cmp_played, cmp_other) = match sharp {
        Some(gap) => (parent.top(), parent.top() - gap.max(0)),
        None if played_best => (parent.top(), parent.top()),
        None => (played_value, reference),
    };
    Some(Grade {
        loss: if played_best { 0 } else { raw.max(0) },
        raw,
        exact: true,
        sharp,
        cp_before: parent.top(),
        played_value,
        cmp_played,
        cmp_other,
        depth: Some(depth),
    })
}

/// What a sibling search says the move it played is worth, in the **mover's** frame — `None` when
/// the search came back with nothing worth comparing against.
///
/// Two ways that happens, and both would otherwise pass silently into a cp subtraction and come out
/// looking like a real grade. A budget too small to finish even one iteration leaves `Searched::score`
/// at the zero `search_position` falls back to, which reads as a dead-level position rather than as
/// "no answer". And a PV that found no move at all comes back holding one of the search's window
/// sentinels, two orders of magnitude outside the range a score can occupy.
fn sibling_value(searched: &Searched) -> Option<i32> {
    (searched.deepest().is_some() && is_real_score(searched.score)).then(|| -searched.score)
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

/// Play `alt` and report what it is worth to the mover, undoing it before returning.
///
/// The terminal arm is not a nicety. A move that wins outright leaves a *finished* position, and
/// searching one completes no iteration at all — so `sibling_value` refuses it, correctly, and
/// without this arm the strongest alternative there can be, the one that simply wins, comes back as
/// "no answer". The ply that declined it is then ungraded, which on the sheet is a move with no
/// badge rather than the mistake it was.
fn alternative_value<B, E>(
    board: &mut B,
    eval: &mut E,
    info: &mut SearchInfo,
    alt: GameMove,
    player: Color,
    nodes: &mut u64,
) -> Option<i32>
where
    B: TakBoard,
    E: Evaluator<Game = B>,
{
    let rev = board.do_move(alt);
    let value = match board.game_result() {
        Some(result) => Some(terminal_value(result, player)),
        None => {
            let searched = search_position(board, eval, info);
            *nodes += info.nodes() as u64;
            // The search scores the position from the *opponent's* view — they are to move there —
            // so `sibling_value` negates it into the mover's.
            sibling_value(&searched)
        }
    };
    board.reverse_move(rev);
    value
}

/// The comparison endpoints for a sibling measurement, as `(played, other)`.
///
/// The clamp is the whole reason this is a function rather than a tuple literal: on a ply the engine
/// itself chose, `alt` is the **runner-up**, and two independently searched siblings can perfectly
/// well come back with the runner-up ahead. That is a zero gap, not a negative one — the same clamp
/// `loss` and `sharp` already carry, and without it the best move in the position grades as having
/// given ground. See `Grade::cmp_played`.
fn sibling_endpoints(played_value: i32, alt: i32, played_best: bool) -> (i32, i32) {
    (played_value, if played_best { alt.min(played_value) } else { alt })
}

/// Should this aligned grade be thrown away and the ply re-measured against a sibling? See part 3 of
/// the grading for what each arm is guarding against.
///
/// `deepest_played` is the value of the position the ply reached at the deepest depth its own search
/// completed — the number the sheet draws. The aligned grade read that same position at `D-1`, so the
/// two are readings of one position and are entitled to agree.
fn distrust_aligned(g: &Grade, deepest_played: i32) -> bool {
    // Nothing to trust: the method had no usable pair at all.
    if !g.exact {
        return true;
    }
    // The parent's half of the comparison, recovered: `raw` is what the two sides differed by, so
    // adding the played side back gives the reference it was measured against.
    let reference = g.raw + g.played_value;
    is_solved(reference) != is_solved(g.played_value)
        || is_solved(g.played_value) != is_solved(deepest_played)
        || drift_win_pct(g.played_value, deepest_played) >= MAX_GRADE_DRIFT
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

fn analyze_board<B, E>(mut board: B, cfg: &AnalyzeGameConfig) -> anyhow::Result<Analysis>
where
    B: TakBoard,
    E: Evaluator<Game = B> + Default,
{
    let table = HashTable::new(TT_SIZE);
    let mut eval = E::default();
    let mut info = make_info(cfg.nodes, &table);
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
    // `info.nodes()` is this search's own count, not a lifetime total: `SearchInfo::start_search`
    // resets the stats, so every search's cost is read straight off it and added here.
    let mut before = {
        let s = search_position(&mut board, &mut eval, &mut info);
        nodes += info.nodes() as u64;
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

        // ---- Grading, part 1: value the alternative, from here ----
        //
        // Only the sibling method needs this, and only it pays for it: one extra search per ply.
        // Which sibling depends on what was played. If the ply *was* the best move, the alternative
        // is the second-best; if it wasn't, the alternative is the best move.
        //
        // The position's own value before the ply, in the mover's frame — the "how won/lost was this
        // already?" number.
        let before_top = before.top();
        let played_best = before.best_move() == Some(mv);
        let alt_value = cfg
            .grading
            .needs_sibling()
            .then(|| before.best_other_move(mv))
            .flatten()
            .and_then(|alt| {
                alternative_value(&mut board, &mut eval, &mut info, alt, player, &mut nodes)
            });

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

        // Play it and measure it. `rev_played` is kept so the grading can step back to this
        // position if it needs the sibling comparison after all (see part 3), and so the
        // alternatives below can be played from it.
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
        let after = match terminal {
            Some(GameResult::WhiteWin) => {
                Searched::terminal(if player == Color::White { WIN_SCORE } else { LOSE_SCORE })
            }
            Some(GameResult::BlackWin) => {
                Searched::terminal(if player == Color::Black { WIN_SCORE } else { LOSE_SCORE })
            }
            Some(GameResult::Draw) => Searched::terminal(0),
            None => {
                let s = search_position(&mut board, &mut eval, &mut info);
                nodes += info.nodes() as u64;
                s
            }
        };

        // The line the search was looking at, read from the position this ply *reached* — so it is
        // the engine's answer to "and then what?", with its recommended reply first. Taken here,
        // while the board still sits on that position and before the regrade below borrows it back.
        let pv = pv_ptn(&mut board, &after.pv);

        let (score_white, win_prob_white) = match terminal {
            Some(GameResult::WhiteWin) => (i32::MAX, 1.0),
            Some(GameResult::BlackWin) => (i32::MIN, 0.0),
            Some(GameResult::Draw) => (0, 0.5),
            None => {
                let s = to_white_frame(&board, after.score);
                (s, sigmoid(s as f64 / EVAL_SCALE))
            }
        };
        result = terminal;

        // ---- Grading, part 2: measure what the ply gave up ----
        //
        // The played ply's own value, in the mover's frame. A ply that ended the game is pinned
        // rather than searched.
        let played_value = match terminal {
            Some(result) => terminal_value(result, player),
            None => -after.score,
        };

        // Sharpness only means anything for a ply that *was* the best move: otherwise a better move
        // existed by definition, and no gap to the runner-up can make it the only good move.
        // `cp_before` still has to survive a ply the method could not grade — the bot normalises the
        // loss against it, and the sheet's win-probability bar reads it.
        let sibling = cfg.grading.needs_sibling().then(|| match alt_value {
            Some(alt) => {
                let raw = alt - played_value;
                let (cmp_played, cmp_other) = sibling_endpoints(played_value, alt, played_best);
                Grade {
                    // Search is not perfectly consistent between siblings, so a ply can come out
                    // marginally "better than best"; that is a zero loss, not a negative one.
                    loss: if played_best { 0 } else { raw.max(0) },
                    raw,
                    exact: true,
                    sharp: if played_best { Some(-raw) } else { None },
                    cp_before: before_top,
                    played_value,
                    // Both ends come from sibling searches of the two moves — one footing by
                    // construction, whichever move `alt` turned out to be.
                    cmp_played,
                    cmp_other,
                    depth: None,
                }
            }
            None => Grade {
                cp_before: before_top,
                played_value,
                cmp_played: played_value,
                cmp_other: played_value,
                ..Default::default()
            },
        });

        // A ply that ended the game has no child search to align against, so the pinned terminal
        // value stands in for it — that comparison needs no horizon matching to begin with.
        let aligned = cfg.grading.needs_aligned().then(|| {
            if terminal.is_some() {
                let raw = before_top - played_value;
                let sharp = if played_best { before.root_gap() } else { None };
                // A ply that ended the game has no search of its own to be read at any depth: its
                // value is the pinned result. So the two ends are the position before it and the
                // outcome it produced, and they are only comparable once normalised — in cp, a
                // game-losing ply reads as a loss of `before_top + WIN_SCORE`, which is not a
                // quantity. In win probability it reads as "gave up whatever was left", which is.
                let (cmp_played, cmp_other) = match sharp {
                    Some(gap) => (before_top, before_top - gap.max(0)),
                    None if played_best => (before_top, before_top),
                    None => (played_value, before_top),
                };
                Grade {
                    loss: if played_best { 0 } else { raw.max(0) },
                    raw,
                    exact: true,
                    sharp,
                    cp_before: before_top,
                    played_value,
                    cmp_played,
                    cmp_other,
                    depth: None,
                }
            } else {
                align(&before, &after, played_best).unwrap_or(Grade {
                    cp_before: before_top,
                    played_value,
                    cmp_played: played_value,
                    cmp_other: played_value,
                    ..Default::default()
                })
            }
        });

        // ---- Grading, part 3: distrust an aligned grade the deeper search contradicts ----
        //
        // The aligned comparison reads this position at `D-1`, but the sheet's bar draws it at the
        // deepest depth the search reached. Those normally agree closely. When they don't — because
        // the *previous* position searched shallower than this one, dragging `D` down with it — the
        // grade is computed from a reading the engine itself has since revised, and the sheet ends
        // up showing a rising evaluation next to a Blunder badge.
        //
        // The same applies when there is no aligned pair to be had at all. A position the search
        // *solves* stops iterating the moment it proves the win, so it can finish only depth 1 —
        // and a pair needs the parent at 2 or more. Those plies were coming out ungraded, which on
        // the sheet is simply a move with no badge on it.
        //
        // A solved position is the awkward case even when it *does* leave a usable pair, because it
        // leaves a shallow one: stopping at depth 2 or 3 puts a proven win on one side of the
        // comparison and a two-ply guess on the other. Subtracting those measures nothing, and the
        // drift test cannot see it — win scores sit where the sigmoid is flat, so a mate against a
        // merely winning evaluation reads as a few points and passes. Hence the explicit scale
        // check: a pairing that straddles the solved/evaluation boundary is rejected on sight,
        // whatever the drift says.
        //
        // The sibling comparison does not depend on depth alignment at all, which makes it exactly
        // the right authority when alignment is the thing that failed. It costs the extra search
        // this method exists to avoid, and at the drift threshold this is tuned to it fires on
        // 13-26% of plies across the sample games — the price of a grade that survives comparison
        // with a deeper search.
        //
        // What it does *not* do is put the two sides on one scale. Given its own full budget on one
        // position, the re-search can prove a win the parent's multi-PV search never came close to
        // seeing, and then the sibling is a mate score while the played move is an ordinary
        // evaluation. That is a real finding — the ply missed a forced win — but it is only a
        // measurable one in win probability, which is why the grade carries its endpoints (see
        // `Grade::cmp_played`) rather than leaving a consumer to rebuild them out of cp.
        let mut sibling = sibling;
        let mut aligned = aligned;
        let mut regraded = false;
        if cfg.grading == Grading::Aligned && terminal.is_none() {
            let distrust = aligned.map_or(false, |g| distrust_aligned(&g, played_value));
            if distrust {
                if let Some(alt) = before.best_other_move(mv) {
                    board.reverse_move(rev_played);
                    let value =
                        alternative_value(&mut board, &mut eval, &mut info, alt, player, &mut nodes);
                    board.do_move(mv);

                    // A sibling search that came back with nothing leaves the distrusted aligned
                    // grade in place rather than replacing it with a comparison against a
                    // placeholder: a grade the deeper search argues with still beats no grade.
                    if let Some(alt_value) = value {
                        let raw = alt_value - played_value;
                        let (cmp_played, cmp_other) =
                            sibling_endpoints(played_value, alt_value, played_best);
                        sibling = Some(Grade {
                            loss: if played_best { 0 } else { raw.max(0) },
                            raw,
                            exact: true,
                            sharp: if played_best { Some(-raw) } else { None },
                            cp_before: before_top,
                            played_value,
                            // The pair this regrade exists to produce: two children of one parent,
                            // each given the full budget, so they are on one footing however far
                            // apart they land. This is where a re-search can prove a win the
                            // parent's own multi-PV search never saw — `cmp_other` is then a mate
                            // score and `cmp_played` an evaluation, and subtracting them as cp is
                            // exactly the thing the endpoints exist to stop a consumer doing.
                            cmp_played,
                            cmp_other,
                            depth: None,
                        });
                        // `primary()` prefers the aligned grade, so the discredited one has to go.
                        aligned = None;
                        regraded = true;
                    }
                }
            }
        }

        // ---- The other moves that were available ----
        //
        // One multi-PV search of the position this ply was played *in*, so every score on the list
        // sits on one footing (see `Alternatives`). It is a second search of a position already
        // searched once, which is why it runs only on the plies the caller's gates admit — and why
        // those gates are read here rather than left to the caller to apply afterwards.
        let grade = aligned.or(sibling).unwrap_or_default();
        let win_prob_before = sigmoid(grade.cp_before as f64 / EVAL_SCALE);
        // Normalised off the grade's own endpoints, not off `cp_before - loss`: those two numbers
        // can come from different searches, so reconstructing the played value from them is what
        // this gate used to do wrong — and a ply whose loss read as thousands of cp was skipped
        // here for being far past the caller's threshold when it had barely given anything up.
        let loss_pct = grade.loss_win_pct().unwrap_or(f64::INFINITY);
        let alts = if cfg.alts > 0
            && win_prob_before >= cfg.alts_min_win_prob
            && loss_pct <= cfg.alts_max_loss_pct
        {
            board.reverse_move(rev_played);
            let mut alt_info = make_info(cfg.nodes, &table)
                .set_multi_pv(cfg.alts)
                .full_root_moves(true);
            let searched = search_position(&mut board, &mut eval, &mut alt_info);
            nodes += alt_info.nodes() as u64;

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
            Some(Alternatives { played_cp, moves })
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
            search_depth: after.deepest(),
            // Already computed above: the `--alts` gate reads it too.
            win_prob_before,
            solved_loss,
            aligned,
            sibling,
            played_best,
            regraded,
            alts,
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
        "{{\"played_cp\": {}, \"moves\": [{}]}}",
        a.played_cp
            .map(|cp| format!("{:.2}", cp as f64 / 4.0))
            .unwrap_or_else(|| "null".to_string()),
        moves.join(", ")
    )
}

/// One grading method's numbers, in human cp. `raw_cp` is the unclamped comparison — see `Grade`.
fn grade_json(g: &Grade) -> String {
    format!(
        "{{\"loss_cp\": {:.2}, \"raw_cp\": {:.2}, \"exact\": {}, \"sharp_cp\": {}, \
         \"cp_before\": {:.2}, \"cmp_played_cp\": {:.2}, \"cmp_other_cp\": {:.2}, \"depth\": {}}}",
        g.loss as f64 / 4.0,
        g.raw as f64 / 4.0,
        g.exact,
        g.sharp
            .map(|s| format!("{:.2}", s as f64 / 4.0))
            .unwrap_or_else(|| "null".to_string()),
        g.cp_before as f64 / 4.0,
        g.cmp_played as f64 / 4.0,
        g.cmp_other as f64 / 4.0,
        g.depth
            .map(|d| d.to_string())
            .unwrap_or_else(|| "null".to_string()),
    )
}

fn to_json(cfg: &AnalyzeGameConfig, a: &Analysis) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str(&format!(
        "  \"size\": {}, \"half_komi\": {}, \"komi\": {}, \"nodes_per_ply\": {},\n",
        cfg.size,
        cfg.half_komi,
        cfg.half_komi as f64 / 2.0,
        cfg.nodes
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
        let g = p.primary();
        rows.push(format!(
            "    {{\"index\": {}, \"move_number\": {}, \"player\": \"{}\", \"ptn\": \"{}\", \
             \"display\": \"{}\", \"cp_white\": {}, \"win_prob_white\": {:.6}, \"call\": {}, \
             \"defends_tak\": {}, \"completes_road\": {}, \"terminal\": {}, \"flats\": [{}, {}], \
             \"loss_cp\": {:.2}, \
             \"loss_exact\": {}, \"sharp_cp\": {}, \"fcd\": {}, \"ocs_added\": {}, \
             \"move_top\": {}, \"largest_capture\": {}, \"largest_capture_after\": {}, \
             \"pv\": [{}], \
             \"cp_before\": {:.2}, \
             \"cmp_played_cp\": {:.2}, \"cmp_other_cp\": {:.2}, \
             \"win_prob_before\": {:.6}, \
             \"solved_loss\": {}, \"played_best\": {}, \"search_depth\": {}, \"regraded\": {}{}{}{}}}",
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
            g.cp_before as f64 / 4.0,
            g.cmp_played as f64 / 4.0,
            g.cmp_other as f64 / 4.0,
            p.win_prob_before,
            p.solved_loss,
            p.played_best,
            p.search_depth
                .map(|d| d.to_string())
                .unwrap_or_else(|| "null".to_string()),
            p.regraded,
            // Per-method detail, for calibrating one against the other. The flat fields above are
            // the contract; these are diagnostics and are only present for a method that ran.
            p.aligned
                .map(|g| format!(", \"aligned\": {}", grade_json(&g)))
                .unwrap_or_default(),
            p.sibling
                .map(|g| format!(", \"sibling\": {}", grade_json(&g)))
                .unwrap_or_default(),
            // Present only on the plies `--alts` was asked to spend a search on.
            p.alts
                .as_ref()
                .map(|a| format!(", \"alts\": {}", alts_json(a)))
                .unwrap_or_default(),
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
    Ok(board.with_komi(cfg.half_komi))
}

pub fn run(cfg: AnalyzeGameConfig) -> anyhow::Result<()> {
    if cfg.plies.is_empty() {
        anyhow::bail!("no plies given");
    }
    let analysis = match cfg.size {
        4 => analyze_board::<Board4, NNUE4>(start_board(&cfg)?, &cfg)?,
        5 => analyze_board::<Board5, NNUE5>(start_board(&cfg)?, &cfg)?,
        6 => analyze_board::<Board6, NNUE6>(start_board(&cfg)?, &cfg)?,
        n => anyhow::bail!("unsupported board size {n} (nets exist for 4x4, 5x5 and 6x6)"),
    };
    let json = to_json(&cfg, &analysis);
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

    /// An aligned grade with the two readings a `distrust_aligned` decision turns on: what the
    /// parent was worth at the comparison depth, and what the played move was worth one below it.
    fn graded(reference: i32, played_value: i32) -> Grade {
        Grade {
            loss: (reference - played_value).max(0),
            raw: reference - played_value,
            exact: true,
            sharp: None,
            cp_before: reference,
            played_value,
            cmp_played: played_value,
            cmp_other: reference,
            depth: Some(8),
        }
    }

    #[test]
    fn agreeing_readings_are_trusted() {
        // The aligned reading of the position the ply reached and the deepest reading of that same
        // position are a couple of flats apart: ordinary search wobble, and the grade stands.
        let g = graded(40, 10);
        assert!(!distrust_aligned(&g, 14));
    }

    #[test]
    fn a_revised_reading_is_distrusted() {
        // Same pair, but the position's own deeper search has since moved it far enough to change
        // the grade by a band or more.
        let g = graded(40, 10);
        assert!(distrust_aligned(&g, 120));
    }

    #[test]
    fn a_grade_with_no_pair_is_distrusted() {
        let g = Grade { cp_before: 40, played_value: 10, ..Default::default() };
        assert!(distrust_aligned(&g, 10));
    }

    #[test]
    fn a_mate_against_an_evaluation_is_distrusted() {
        // The position before the ply is a proven win; the ply's own reading is an ordinary
        // evaluation, and the deeper search agrees with it. Something really was given up here, but
        // subtracting a mate score from an evaluation cannot say how much, so the ply goes to a
        // sibling search rather than standing. The drift test cannot reach this on its own: both
        // readings of the position the ply reached agree, so there is no drift to find.
        let g = graded(WIN_SCORE - 20, 3200);
        assert!(drift_win_pct(g.played_value, 3200) < MAX_GRADE_DRIFT);
        assert!(distrust_aligned(&g, 3200));
    }

    #[test]
    fn a_mate_the_deeper_search_confirms_is_distrusted() {
        // The other way the pairing straddles the boundary: the aligned reading of the position the
        // ply reached is an ordinary evaluation, but its own deeper search has since proved the win.
        // Nothing was given up at all, and the sigmoid is flat enough out there to hide the gap.
        let mate = WIN_SCORE - 20;
        let g = graded(mate, 1400);
        assert!(distrust_aligned(&g, mate));
    }

    #[test]
    fn two_mates_are_not_distrusted_for_being_mates() {
        // Both sides proven wins, both readings agreeing: nothing was given up and the check must
        // not invent a loss out of the mate distances.
        let g = graded(WIN_SCORE - 12, WIN_SCORE - 20);
        assert!(!distrust_aligned(&g, WIN_SCORE - 18));
    }

    #[test]
    fn a_missed_forced_win_costs_what_the_position_was_still_worth() {
        // PlayTak 812697 ply 93, which is where this pair of fields came from. White played a move
        // worth +405 human cp — 98.3%, and the bar goes *up* — when a sibling re-search proved a
        // forced win was available. Missing it is real, but it cost the 1.7 points between 98.3%
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

        // The reconstruction this replaced, kept as the thing not to go back to: `cp_before` comes
        // from the parent's search and `cmp_other` from a sibling re-search that outran it, so the
        // implied played value lands 7500cp below where the ply actually was.
        let loss_cp = g.cmp_other - g.cmp_played;
        assert!(drift_win_pct(g.cp_before, g.cp_before - loss_cp) > 80.0);
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
        let played_best = true;
        let (cmp_played, cmp_other) = sibling_endpoints(-159, -93, played_best);
        let g = Grade { cmp_played, cmp_other, exact: true, ..Default::default() };
        assert_eq!(g.loss_win_pct(), Some(0.0));

        // ...and a ply that was *not* the best move keeps the alternative exactly as searched, since
        // there the alternative being ahead is the whole point.
        assert_eq!(sibling_endpoints(-159, -93, false), (-159, -93));
    }

    #[test]
    fn a_grade_with_nothing_to_compare_has_no_loss() {
        let g = Grade { cmp_played: 400, cmp_other: 120, exact: false, ..Default::default() };
        assert_eq!(g.loss_win_pct(), None);
    }

    #[test]
    fn a_sibling_search_that_finished_nothing_is_rejected() {
        // No completed iteration: `Searched::score` is the zero the search falls back to, which
        // would otherwise read as a dead-level position rather than as "no answer".
        assert_eq!(sibling_value(&Searched::terminal(0)), None);
    }

    #[test]
    fn a_sibling_search_holding_a_window_bound_is_rejected() {
        let sentinel = Searched {
            score: 250_000,
            roots: Vec::new(),
            depths: vec![(1, 250_000)],
            pv: Vec::new(),
        };
        assert_eq!(sibling_value(&sentinel), None);

        // ...and an ordinary one comes back negated into the mover's frame.
        let ok = Searched { score: -1620, roots: Vec::new(), depths: vec![(9, -1620)], pv: Vec::new() };
        assert_eq!(sibling_value(&ok), Some(1620));
    }
}
