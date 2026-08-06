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
//!
//! Each ply is also reported twice: `ptn` exactly as it was given, and `display` in conventional
//! short PTN for the sheet.
//!
//! **Move grading.** Every ply additionally carries the raw numbers a caller needs to award it a
//! badge — the thresholds themselves live in the bot, so they can be retuned without a rebuild:
//!   - `loss_cp` — how much worse than the best move, in the mover's frame. Exact when the search
//!     graded the move itself (`loss_exact`), otherwise inferred from the position it reached.
//!   - `sharp_cp` — how much *better* than the best alternative to it. Negative whenever some other
//!     move was better, so "the only good move here" is simply a large positive value.
//!   - `fcd` — flat-count differential: how far the ply moved the mover's flat lead.
//!   - `cp_before` / `win_prob_before` — the position's value before the ply, mover's frame.
//!   - `solved_loss` — the ply handed the opponent a proven win.
//!   - `ocs_added` — how many squares the ply handed to the opponent (see `ocs_added` below).
//!   - `move_top` — for a stack movement, what was on top of the stack it moved: `"flat"`, `"wall"`
//!     or `"cap"`. `null` for a placement.
//!   - `defends_tak` — the mover was in tak coming into this ply and is not after it.
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
//! `sharp_cp` needs no extra search under either method: multi-PV 2 already returns the best and
//! second-best root moves with exact scores from the same search, at the same depth and for the same
//! side to move, so the gap between them is directly comparable.
//!
//! `Grading::Both` computes both and reports the sibling method alongside, for calibrating one
//! against the other.

use crate::board::{Bitboard, Board4, Board5, Board6, Piece, TakBoard};
use crate::eval::{Evaluator, LOSE_SCORE, NNUE4, NNUE5, NNUE6, WIN_SCORE};
use crate::search::{search, SearchInfo};
use crate::transposition_table::HashTable;
use crate::{Color, GameMove, GameResult, TimeBank};

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

#[derive(Clone)]
pub struct AnalyzeGameConfig {
    /// Board size (4, 5, or 6 — the sizes with nets).
    pub size: usize,
    /// Half-komi (2 komi == 4). Also selects the komi-specialized 6x6 net.
    pub half_komi: u8,
    /// Soft node budget per position. The hard cap is 4x this.
    pub nodes: u64,
    /// The game's plies in PTN, in order.
    pub plies: Vec<String>,
    /// JSON output path; `None` prints to stdout.
    pub output: Option<String>,
    /// Suppress the per-ply stderr progress line.
    pub quiet: bool,
    /// How to measure each ply's loss. `Aligned` is half the searches of `Sibling`.
    pub grading: Grading,
}

impl Default for AnalyzeGameConfig {
    fn default() -> Self {
        Self {
            size: 6,
            half_komi: 0,
            nodes: 100_000,
            plies: Vec::new(),
            output: None,
            quiet: false,
            grading: Grading::Aligned,
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
    /// The position's value before the ply, on this method's own footing.
    cp_before: i32,
    /// `Aligned` only: the depth the comparison was made at (parent `D`, child `D-1`).
    depth: Option<usize>,
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
    /// The mover was in tak coming into this ply, and is not after it. The mirror of `call`, and
    /// like `call` it is never set on a game-ending ply: winning outright is not a defence.
    defends_tak: bool,
    /// On-board flat counts after the ply, `[white, black]`.
    flats: [i32; 2],
    /// Change in the mover's flat differential: +1 for a flat placement, 0 for a wall or capstone,
    /// and whatever a spread covers or buries.
    fcd: i32,
    /// How many squares this ply *handed to the opponent*: squares the opponent's piece tops after
    /// the ply that it did not top before. Deliberately **gross** — squares the ply took back the
    /// other way do not net it off, so a spread that uncovers one of the opponent's pieces still
    /// reads as having done so even if it buried another elsewhere.
    ///
    /// "Tops" means any piece — flat, wall or capstone. A stack the opponent's wall sits on is
    /// theirs, however little it does for their flat count.
    ocs_added: u32,
    /// What was on top of the stack a movement moved, or `None` for a placement.
    move_top: Option<Piece>,
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
}

impl Searched {
    /// A position that was never searched because the game had already ended there.
    fn terminal(score: i32) -> Self {
        Self { score, roots: Vec::new(), depths: Vec::new() }
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
            [(_, best), (_, second), ..] => Some(best - second),
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
    let score = search(board, eval, info).map(|o| o.score()).unwrap_or(0);
    Searched {
        score,
        roots: info.root_choices().to_vec(),
        depths: info.depth_scores().to_vec(),
    }
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
    let cp_before = parent.score_at(depth)?;
    let played_value = -child.score_at(depth - 1)?;
    let raw = cp_before - played_value;
    Some(Grade {
        loss: if played_best { 0 } else { raw.max(0) },
        raw,
        exact: true,
        sharp: if played_best { parent.root_gap() } else { None },
        cp_before,
        depth: Some(depth),
    })
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

fn analyze_board<B, E>(mut board: B, cfg: &AnalyzeGameConfig) -> anyhow::Result<Analysis>
where
    B: TakBoard,
    E: Evaluator<Game = B> + Default,
{
    let table = HashTable::new(TT_SIZE);
    let mut eval = E::default();
    let mut info = make_info(cfg.nodes, &table);
    let mut storage: Vec<GameMove> = Vec::with_capacity(128);

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

    // Every position is searched once, and that one search does double duty: its score is the bar
    // for the ply that *reached* the position, and its root move list grades the ply played *from*
    // it. So the loop carries the previous position's search forward — only the starting position
    // needs an extra one.
    let mut before = {
        let s = search_position(&mut board, &mut eval, &mut info);
        nodes += info.nodes() as u64;
        s
    };
    let mut flats_before = [0i32, 0i32];

    for (i, ptn) in cfg.plies.iter().enumerate() {
        if result.is_some() {
            anyhow::bail!("ply {} ({}) comes after the game already ended", i + 1, ptn);
        }
        let player = board.side_to_move();
        let move_number = board.move_num();
        // Board-aware on purpose: it fills in the crush flag for a wall smash written without the
        // optional `*`, which is how every PlayTak-derived move list writes them.
        let mv = GameMove::try_from_ptn(ptn, &board)
            .ok_or_else(|| anyhow::anyhow!("ply {} is not valid PTN: {}", i + 1, ptn))?;
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

        // Which squares the opponent tops going in, and what the ply is about to move. Both have to
        // be read before `do_move`, while the source stack is still intact.
        let opponent_before = board.bits().all_pieces(!player);
        let move_top = mv
            .is_stack_move()
            .then(|| board.index(mv.src_index()).top())
            .flatten();

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
            .map(|alt| {
                let rev = board.do_move(alt);
                let spent = info.nodes() as u64;
                let searched = search_position(&mut board, &mut eval, &mut info);
                nodes += (info.nodes() as u64).saturating_sub(spent);
                board.reverse_move(rev);
                // The sibling's score is from the opponent's view; negate for the mover's.
                -searched.score
            });

        // Display PTN. `to_ptn` already writes the conventional short form: a single-piece move
        // drops both the count and the drop-list (`1c2-1` -> `c2-`), as does any spread that puts
        // everything on one square (`3c2-3` -> `3c2-`). The one exception we add back: when that
        // single piece comes off a taller stack, the leading `1` is kept (`1b4-`) to make clear a
        // stack was split. Computed before the move, while the source stack is still intact.
        let display = {
            let short = mv.to_ptn::<B>();
            if mv.is_stack_move() && mv.number() == 1 && board.index(mv.src_index()).len() > 1 {
                format!("1{short}")
            } else {
                short
            }
        };

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

        board.do_move(mv);
        let terminal = board.game_result();

        // A move that *wins outright* is never a call: the opponent gets no turn, so there was
        // never a threat for them to answer. A game that merely stops here — resignation, timeout —
        // is different: the opponent did have their chance and didn't take it, so that last ply
        // counts like any other. Beyond that, a call is a *transition* into tak.
        let call = if terminal.is_some() {
            false
        } else {
            board.null_move();
            let threat_after = has_road_threat(&board, &mut storage);
            board.rev_null_move();
            threat_after && !threat_before
        };
        if call {
            stats[color_idx(player)].calls += 1;
        }

        // Defending tak is the mirror image, and gets the same treatment on a game-ending ply: a
        // move that just wins has not *answered* anything, so it defends nothing. The opponent is to
        // move now, so their road-completing move is read directly, with no null move needed.
        let defends_tak =
            terminal.is_none() && in_tak_before && !has_road_threat(&board, &mut storage);

        // Squares newly topped by an opponent piece. Gross, so squares taken back the other way in
        // the same spread do not cancel these out.
        let ocs_added = (board.bits().all_pieces(!player) & !opponent_before).pop_count();

        let flats = [
            board.bits().flat_score(Color::White) as i32,
            board.bits().flat_score(Color::Black) as i32,
        ];
        let reserves = [
            board.pieces_reserve(Color::White) as i32,
            board.pieces_reserve(Color::Black) as i32,
        ];
        for (idx, s) in stats.iter_mut().enumerate() {
            let other = 1 - idx;
            s.largest_flat_lead = s.largest_flat_lead.max(flats[idx] - flats[other]);
            // A reserve *lead* belongs to whoever has spent more, i.e. has fewer left.
            s.largest_reserve_lead = s.largest_reserve_lead.max(reserves[other] - reserves[idx]);
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
                let spent = info.nodes() as u64;
                let s = search_position(&mut board, &mut eval, &mut info);
                nodes += (info.nodes() as u64).saturating_sub(spent);
                s
            }
        };

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
            Some(GameResult::WhiteWin) => {
                if player == Color::White { WIN_SCORE } else { LOSE_SCORE }
            }
            Some(GameResult::BlackWin) => {
                if player == Color::Black { WIN_SCORE } else { LOSE_SCORE }
            }
            Some(GameResult::Draw) => 0,
            None => -after.score,
        };

        // Sharpness only means anything for a ply that *was* the best move: otherwise a better move
        // existed by definition, and no gap to the runner-up can make it the only good move.
        // `cp_before` still has to survive a ply the method could not grade — the bot normalises the
        // loss against it, and the sheet's win-probability bar reads it.
        let sibling = cfg.grading.needs_sibling().then(|| match alt_value {
            Some(alt) => {
                let raw = alt - played_value;
                Grade {
                    // Search is not perfectly consistent between siblings, so a ply can come out
                    // marginally "better than best"; that is a zero loss, not a negative one.
                    loss: if played_best { 0 } else { raw.max(0) },
                    raw,
                    exact: true,
                    sharp: if played_best { Some(-raw) } else { None },
                    cp_before: before_top,
                    depth: None,
                }
            }
            None => Grade { cp_before: before_top, ..Default::default() },
        });

        // A ply that ended the game has no child search to align against, so the pinned terminal
        // value stands in for it — that comparison needs no horizon matching to begin with.
        let aligned = cfg.grading.needs_aligned().then(|| {
            if terminal.is_some() {
                let raw = before_top - played_value;
                Grade {
                    loss: if played_best { 0 } else { raw.max(0) },
                    raw,
                    exact: true,
                    sharp: if played_best { before.root_gap() } else { None },
                    cp_before: before_top,
                    depth: None,
                }
            } else {
                align(&before, &after, played_best).unwrap_or(Grade {
                    cp_before: before_top,
                    ..Default::default()
                })
            }
        });

        // Flat-count differential: how far this ply moved the mover's flat lead. A flat placement is
        // +1, a wall or capstone 0, a spread whatever it covers and uncovers — including negative
        // when it buries the mover's own flat.
        let mover = color_idx(player);
        let opponent = 1 - mover;
        let fcd = (flats[mover] - flats[opponent]) - (flats_before[mover] - flats_before[opponent]);

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

        let ply = PlyAnalysis {
            index: i + 1,
            ptn: ptn.clone(),
            display,
            player,
            move_number,
            score_white,
            win_prob_white,
            terminal,
            call,
            defends_tak,
            flats,
            fcd,
            ocs_added,
            move_top,
            // Filled in below, once the primary grade is known.
            win_prob_before: 0.0,
            solved_loss,
            aligned,
            sibling,
            played_best,
        };
        out.push(PlyAnalysis {
            win_prob_before: sigmoid(ply.primary().cp_before as f64 / EVAL_SCALE),
            ..ply
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
         \"largest_reserve_lead\": {}}}",
        s.calls,
        s.flat_placements,
        s.wall_placements,
        s.cap_placements,
        s.movements,
        s.largest_flat_lead,
        s.largest_reserve_lead
    )
}

/// One grading method's numbers, in human cp. `raw_cp` is the unclamped comparison — see `Grade`.
fn grade_json(g: &Grade) -> String {
    format!(
        "{{\"loss_cp\": {:.2}, \"raw_cp\": {:.2}, \"exact\": {}, \"sharp_cp\": {}, \
         \"cp_before\": {:.2}, \"depth\": {}}}",
        g.loss as f64 / 4.0,
        g.raw as f64 / 4.0,
        g.exact,
        g.sharp
            .map(|s| format!("{:.2}", s as f64 / 4.0))
            .unwrap_or_else(|| "null".to_string()),
        g.cp_before as f64 / 4.0,
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
             \"defends_tak\": {}, \"terminal\": {}, \"flats\": [{}, {}], \"loss_cp\": {:.2}, \
             \"loss_exact\": {}, \"sharp_cp\": {}, \"fcd\": {}, \"ocs_added\": {}, \
             \"move_top\": {}, \"cp_before\": {:.2}, \"win_prob_before\": {:.6}, \
             \"solved_loss\": {}, \"played_best\": {}{}{}}}",
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
            g.cp_before as f64 / 4.0,
            p.win_prob_before,
            p.solved_loss,
            p.played_best,
            // Per-method detail, for calibrating one against the other. The flat fields above are
            // the contract; these are diagnostics and are only present for a method that ran.
            p.aligned
                .map(|g| format!(", \"aligned\": {}", grade_json(&g)))
                .unwrap_or_default(),
            p.sibling
                .map(|g| format!(", \"sibling\": {}", grade_json(&g)))
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

pub fn run(cfg: AnalyzeGameConfig) -> anyhow::Result<()> {
    if cfg.plies.is_empty() {
        anyhow::bail!("no plies given");
    }
    let analysis = match cfg.size {
        4 => analyze_board::<Board4, NNUE4>(Board4::new().with_komi(cfg.half_komi), &cfg)?,
        5 => analyze_board::<Board5, NNUE5>(Board5::new().with_komi(cfg.half_komi), &cfg)?,
        6 => analyze_board::<Board6, NNUE6>(Board6::new().with_komi(cfg.half_komi), &cfg)?,
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
