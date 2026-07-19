//! Balanced-opening-line counter: a "breadth of viable openings" comparison between the
//! **black-stack** ruleset (forced `2xx` double-black-stack opening, komi 0) and the **2-komi**
//! ruleset (single swapped black flat opening, 2 komi / half-komi 4), both on 6x6.
//!
//! For each ruleset and each opening archetype we fix a canonical 2-ply opening, then enumerate
//! every continuation `plies` deep over **all placement moves** (flat / wall / cap). Every position
//! from the completed opening (ply 2) through the final ply is searched to a fixed `depth`.
//!
//! **What is counted: distinct end-state positions.** A balanced *line* is a `plies`-long move
//! sequence whose every position stayed in-band; but many lines transpose to the same final board,
//! so we count **distinct final positions**, de-duplicated under the **D4 symmetry group** (the 8
//! rotations/reflections) — mirror/rotated end-states collapse to one. So the reported number is the
//! count of distinct balanced *resulting positions*, not move orders.
//!
//! Two bands are supported, and either/both may be requested:
//!   - **Absolute** (`-b` / `cp_bound`): every position within `±cp_bound` of 0.
//!   - **Relative** (`-r` / `rel_bound`): every position within `±rel_bound` of an anchor, in a
//!     **fixed White-perspective frame** (`eval_white`). The anchor is the *archetype's own opening
//!     eval* by default, or — with `--relativeToTop` (`relative_to_top`) — the **per-ply top move**:
//!     each position's own eval, so a move stays in-band only if it is at most `rel_bound` worse than
//!     the best move there (a one-sided band; the fixed-anchor band is two-sided).
//!
//! When both bands are requested we report three per-archetype counts: `abs`, `rel`, and `both`
//! (the intersection) — each a distinct-end-state count over its own set of qualifying lines. With
//! `--requireBoth` (`require_both`) we instead report only the intersection and prune the moment
//! *either* band fails, which can trim the tree enough to reach deeper `plies`.
//!
//! Semantics: `plies` includes the two forced opening plies; `cp_bound`/`rel_bound` are human cp
//! (internal band = `*4`); each ruleset is judged by its own net (auto-selected by komi in `search`).
//! Absolute/combined modes skip an archetype whose base is already outside `±cp_bound`.
//!
//! **Parallelism.** Work is split into tasks and run by a per-ruleset worker pool (rulesets run
//! sequentially so a worker's transposition table never mixes komi values). Round 1 processes one
//! **ply-3 seed** task per legal ply-3 placement; for `plies >= 5` a seed does not recurse itself but
//! **spawns a ply-4 task** per in-band ply-4 child, which round 2 then DFS-expands. This keeps tasks
//! small and numerous so all cores stay busy through the tail (at `plies == 4` there are no ply-4
//! tasks — a seed expands its leaves inline, unchanged).
//!
//! **Reproducibility.** Every eval fully clears its transposition table first (`eval_white`), so the
//! depth-D score is a pure function of the position — the counts are **deterministic and identical
//! across runs and thread counts**. (A warm TT shared across a worker's searches would otherwise make
//! the score depend on nondeterministic task scheduling, swinging p6 counts by ~2x run to run.) The
//! cost is losing cross-search TT reuse, so each search is a bit slower.

use crate::board::{Board6, TakBoard};
use crate::eval::NNUE6;
use crate::move_gen::generate_all_place_moves;
use crate::search::{search, SearchInfo};
use crate::transposition_table::HashTable;
use crate::{Color, GameMove, Position, TimeBank};

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const SIZE: usize = 6;

/// Transposition-table size for every fixed-depth eval. Each eval fully clears the table first (see
/// `eval_white`), so it need only serve a single search; a uniform size keeps the eval a consistent
/// pure function of the position across phase-1 base checks and worker searches alike.
const BAL_TT_SIZE: usize = 1 << 20;

/// Canonical opening instance per archetype: `(name, white_opening_square, black_swap_square)`.
/// One representative per archetype (not all symmetric copies) — the first entry of each table in
/// `openings.rs`. Black-stack uses all four; 2-komi uses the first three (no gap-hug).
const ARCHS: [(&str, &str, &str); 4] = [
    ("diagonal corners", "a1", "f6"),
    ("adjacent corners", "a1", "a6"),
    ("hug", "a1", "a2"),
    ("gap hug", "a1", "c1"),
];

/// A ruleset to evaluate.
#[derive(Clone, Copy)]
struct Ruleset {
    key: &'static str,
    label: &'static str,
    /// Half-komi passed to `with_komi` (0 = black-stack, 4 = 2-komi). Also selects the net.
    komi: u8,
    /// Whether ply 1 is the forced `2xx` double-black-stack (`true`) or a single swapped flat.
    double: bool,
    /// Number of archetypes used (4 black-stack, 3 two-komi).
    num_arch: usize,
    /// Base index into the flat `ArchAgg` vector.
    agg_base: usize,
}

const RULESETS: [Ruleset; 2] = [
    Ruleset { key: "black_stack", label: "Black Stack", komi: 0, double: true, num_arch: 4, agg_base: 0 },
    Ruleset { key: "two_komi", label: "2-Komi", komi: 4, double: false, num_arch: 3, agg_base: 4 },
];

const NUM_AGGS: usize = 7; // 4 black-stack + 3 two-komi

#[derive(Clone)]
pub struct BalancedLinesConfig {
    /// Fixed search depth for every position eval.
    pub depth: usize,
    /// Total plies per line, **including** the 2 forced opening plies (must be >= 2).
    pub plies: usize,
    /// Human-facing absolute cp bound (internal band = `*4`). `None` = absolute band not requested.
    pub cp_bound: Option<i32>,
    /// Human-facing relative cp bound (internal band = `*4`). `None` = relative band not requested.
    pub rel_bound: Option<i32>,
    /// If set, the relative band is anchored per-ply to the **top move** (the position's own eval),
    /// not the fixed opening eval. Requires `rel_bound`.
    pub relative_to_top: bool,
    /// If set, count only lines that are BOTH absolutely and relatively balanced (prune when either
    /// fails). Requires both `cp_bound` and `rel_bound`.
    pub require_both: bool,
    /// Worker threads (0 = auto-detect).
    pub threads: usize,
    /// JSON results output path.
    pub output: String,
}

impl Default for BalancedLinesConfig {
    fn default() -> Self {
        Self {
            depth: 9,
            plies: 7,
            cp_bound: Some(30),
            rel_bound: None,
            relative_to_top: false,
            require_both: false,
            threads: 0,
            output: "balanced_lines.json".to_string(),
        }
    }
}

/// The internal (×4) bands plus the "which bands are active" flags, shared by every node check.
#[derive(Clone, Copy)]
struct Bounds {
    abs_bound: Option<i32>,
    rel_bound: Option<i32>,
    want_abs: bool,
    want_rel: bool,
    /// Relative band anchored to the per-ply top move (position eval) instead of the opening eval.
    rel_to_top: bool,
    /// Count only the intersection (absolutely AND relatively balanced), pruning when either fails.
    require_both: bool,
}

impl Bounds {
    fn from_cfg(cfg: &BalancedLinesConfig) -> Self {
        let abs_bound = cfg.cp_bound.map(|b| b * 4);
        let rel_bound = cfg.rel_bound.map(|b| b * 4);
        Self {
            abs_bound,
            rel_bound,
            want_abs: abs_bound.is_some(),
            want_rel: rel_bound.is_some(),
            rel_to_top: cfg.relative_to_top,
            require_both: cfg.require_both,
        }
    }
    /// Output mode: `require_both` collapses to a single "both" metric regardless of the two bands.
    fn mode(&self) -> &'static str {
        if self.require_both {
            return "both";
        }
        match (self.want_abs, self.want_rel) {
            (true, false) => "absolute",
            (false, true) => "relative",
            (true, true) => "combined",
            (false, false) => "none",
        }
    }
}

/// Per-node bundle: the active bands plus this archetype's opening-eval anchor (White frame).
#[derive(Clone, Copy)]
struct NodeParams {
    b: Bounds,
    base_white: i32,
}

/// Update the running "still absolute / still relative" flags for a **child** with White-frame eval
/// `child_white`, chosen by the mover at its parent node. `parent_white` is the parent position's own
/// depth-D eval and `parent_is_white` its side to move.
///
/// Absolute: `|child_white| <= abs_bound`. Relative: with `rel_to_top`, one-sided — the child is at
/// most `rel_bound` worse than the parent's eval in the mover's frame (i.e. within `rel_bound` of the
/// top move, since the parent eval is the value of its best move); otherwise two-sided around the
/// fixed opening eval `base_white`.
#[inline]
fn child_flags(
    child_white: i32,
    still_abs: bool,
    still_rel: bool,
    p: NodeParams,
    parent_white: i32,
    parent_is_white: bool,
) -> (bool, bool) {
    let a = p.b.abs_bound.map_or(true, |b| child_white.abs() <= b);
    let r = p.b.rel_bound.map_or(true, |b| {
        if p.b.rel_to_top {
            let delta = if parent_is_white {
                parent_white - child_white
            } else {
                child_white - parent_white
            };
            delta <= b
        } else {
            (child_white - p.base_white).abs() <= b
        }
    });
    (still_abs && a, still_rel && r)
}

/// A path is worth extending while it can still reach a leaf that scores a requested count. With
/// `require_both` only the intersection is wanted, so both flags must survive; otherwise absolute
/// needs `still_abs` or relative needs `still_rel`.
#[inline]
fn should_explore(b: Bounds, still_abs: bool, still_rel: bool) -> bool {
    if b.require_both {
        still_abs && still_rel
    } else {
        (b.want_abs && still_abs) || (b.want_rel && still_rel)
    }
}

/// Per-(ruleset, archetype) accumulator, shared across worker threads. The three sets hold the
/// **distinct D4-canonical end-state keys** for each band; the reported count is `set.len()`.
struct ArchAgg {
    name: &'static str,
    abs_set: Mutex<HashSet<u64>>,
    rel_set: Mutex<HashSet<u64>>,
    both_set: Mutex<HashSet<u64>>,
    evals: AtomicU64,
    tasks_done: AtomicU64,
    tasks_total: AtomicU64,
    /// Base position outside the absolute band -> archetype contributes 0 (absolute/combined modes).
    dead: AtomicBool,
    /// Opening eval anchor, White-perspective, internal cp (set in phase 1, read by workers).
    base_white: AtomicI32,
}

impl ArchAgg {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            abs_set: Mutex::new(HashSet::new()),
            rel_set: Mutex::new(HashSet::new()),
            both_set: Mutex::new(HashSet::new()),
            evals: AtomicU64::new(0),
            tasks_done: AtomicU64::new(0),
            tasks_total: AtomicU64::new(0),
            dead: AtomicBool::new(false),
            base_white: AtomicI32::new(0),
        }
    }
    fn abs_count(&self) -> u64 {
        self.abs_set.lock().unwrap().len() as u64
    }
    fn rel_count(&self) -> u64 {
        self.rel_set.lock().unwrap().len() as u64
    }
    fn both_count(&self) -> u64 {
        self.both_set.lock().unwrap().len() as u64
    }
}

/// A unit of parallel work.
enum Task {
    /// A ply-3 placement to evaluate and expand (round 1).
    Seed { arch_idx: usize, ply3: GameMove },
    /// An already-in-band interior node (path from base) whose subtree of `plies_left` plies is to be
    /// DFS-expanded inline (round 2). `node_white` is this node's own eval (the relative-to-top anchor
    /// for its children).
    Node {
        arch_idx: usize,
        moves: Vec<GameMove>,
        plies_left: usize,
        node_white: i32,
        still_abs: bool,
        still_rel: bool,
    },
}

/// Per-task progress reporter. Leaf keys are accumulated in **task-local sets**, then merged into the
/// archetype's shared sets on `flush` (once per task, not per leaf — cheap locking). Evals flush
/// immediately: each is a full fixed-depth search, so a live eval counter is the run's liveness signal.
struct Prog<'a> {
    agg: &'a ArchAgg,
    want_abs: bool,
    want_rel: bool,
    require_both: bool,
    abs: HashSet<u64>,
    rel: HashSet<u64>,
    both: HashSet<u64>,
}

impl<'a> Prog<'a> {
    fn new(agg: &'a ArchAgg, b: Bounds) -> Self {
        Self {
            agg,
            want_abs: b.want_abs,
            want_rel: b.want_rel,
            require_both: b.require_both,
            abs: HashSet::new(),
            rel: HashSet::new(),
            both: HashSet::new(),
        }
    }
    fn add_eval(&self) {
        self.agg.evals.fetch_add(1, Ordering::Relaxed);
    }
    /// Record a balanced end-state `key` for whichever bands this leaf satisfies. With `require_both`
    /// only the intersection set is populated (single-metric output).
    fn add_leaf(&mut self, key: u64, still_abs: bool, still_rel: bool) {
        if self.require_both {
            if still_abs && still_rel {
                self.both.insert(key);
            }
            return;
        }
        if self.want_abs && still_abs {
            self.abs.insert(key);
        }
        if self.want_rel && still_rel {
            self.rel.insert(key);
        }
        if self.want_abs && self.want_rel && still_abs && still_rel {
            self.both.insert(key);
        }
    }
    fn flush(&mut self) {
        if !self.abs.is_empty() {
            self.agg.abs_set.lock().unwrap().extend(self.abs.drain());
        }
        if !self.rel.is_empty() {
            self.agg.rel_set.lock().unwrap().extend(self.rel.drain());
        }
        if !self.both.is_empty() {
            self.agg.both_set.lock().unwrap().extend(self.both.drain());
        }
    }
}

/// Build a search config that always completes a full fixed-depth iteration: `abort_depth = depth+1`
/// disables the depth-based early break, and a huge time bank disables the time-based stop.
fn make_info<'a>(depth: usize, table: &'a HashTable) -> SearchInfo<'a> {
    SearchInfo::new(depth, table)
        .quiet(true)
        .abort_depth(depth + 1)
        .time_bank(TimeBank::flat(6_000_000))
}

/// Fixed-depth search score normalized to a **fixed White-perspective frame**. `search()` returns the
/// score from the side-to-move's perspective and the mover alternates, so we flip the sign when Black
/// is to move. This makes signed comparisons across plies (the relative band) coherent; the absolute
/// band uses `|white|`, which is unchanged by the flip.
///
/// **Reproducibility:** the transposition table is fully cleared before every search, so each eval is
/// a *pure function* of the position (and depth/net) — independent of what this worker searched
/// before. Without this, a warm TT shared across a worker's searches makes the depth-D score depend
/// on nondeterministic task scheduling, so the same config yields different counts run to run.
fn eval_white(board: &mut Board6, eval: &mut NNUE6, info: &mut SearchInfo) -> i32 {
    info.clear_tt_full();
    let s = search(board, eval, info).map(|o| o.score()).unwrap_or(0);
    if board.side_to_move() == Color::White {
        s
    } else {
        -s
    }
}

/// Apply the fixed 2-ply opening for `arch_idx`. Mirrors `openings.rs::apply_ply`: `do_move` handles
/// the move_num==1 colour swap for both plies, and `2xx` (double) draws two black flats for ply 1.
fn apply_opening(board: &mut Board6, arch_idx: usize, double: bool) {
    let (_, white_sq, black_sq) = ARCHS[arch_idx];
    let p1 = if double { format!("2{white_sq}") } else { white_sq.to_string() };
    let m1 = GameMove::try_from_ptn_m(&p1, SIZE, board.side_to_move()).expect("valid opening ptn");
    board.do_move(m1);
    let m2 = GameMove::try_from_ptn_m(black_sq, SIZE, board.side_to_move()).expect("valid swap ptn");
    board.do_move(m2);
}

/// A board reached by the archetype opening plus `extra` moves.
fn build_board(rs: &Ruleset, arch_idx: usize, extra: &[GameMove]) -> Board6 {
    let mut board = Board6::new().with_komi(rs.komi);
    apply_opening(&mut board, arch_idx, rs.double);
    for &mv in extra {
        board.do_move(mv);
    }
    board
}

// ---- D4-canonical end-state key (symmetry-collapsing dedup) ----

/// Canonical key of a position under the dihedral group D4 (4 rotations x 2 reflections). Operates on
/// the Debug/TPS board grid; stack contents are rotation-invariant, so permuting cells by each D4
/// transform and taking the lexicographic minimum yields one representative per symmetry class. Side
/// to move / move number are ignored (constant across all end-states at a fixed ply). Mirrors
/// `openings.rs::canonical_key` (duplicated to keep this module free of the `random` feature gate).
fn canonical_board_key(board: &Board6) -> u64 {
    let tps = format!("{board:?}");
    let board_part = tps.split(' ').next().unwrap_or(&tps);
    let grid: Vec<Vec<String>> = board_part
        .split('/')
        .map(|row| row.split(',').map(|c| c.to_string()).collect())
        .collect();

    let mut current = grid;
    let mut best: Option<String> = None;
    for _ in 0..4 {
        for candidate in [&current, &flip_h(&current)] {
            let s = serialize(candidate);
            if best.as_ref().map_or(true, |b| &s < b) {
                best = Some(s);
            }
        }
        current = rot90(&current);
    }
    let mut h = DefaultHasher::new();
    best.unwrap().hash(&mut h);
    h.finish()
}

fn serialize(grid: &[Vec<String>]) -> String {
    grid.iter().map(|row| row.join(",")).collect::<Vec<_>>().join("/")
}

fn flip_h(grid: &[Vec<String>]) -> Vec<Vec<String>> {
    grid.iter().map(|row| row.iter().rev().cloned().collect()).collect()
}

fn rot90(grid: &[Vec<String>]) -> Vec<Vec<String>> {
    let n = grid.len();
    (0..n).map(|r| (0..n).map(|c| grid[n - 1 - c][r].clone()).collect()).collect()
}

/// Inline DFS of an already-in-band node's subtree for `plies_left` plies. At a leaf (`plies_left == 0`)
/// the final position's D4-canonical key is recorded for whichever bands the path still satisfies.
#[allow(clippy::too_many_arguments)]
fn dfs_inline(
    board: &mut Board6,
    eval: &mut NNUE6,
    info: &mut SearchInfo,
    plies_left: usize,
    p: NodeParams,
    node_white: i32,
    still_abs: bool,
    still_rel: bool,
    prog: &mut Prog,
) {
    if plies_left == 0 {
        let key = canonical_board_key(board);
        prog.add_leaf(key, still_abs, still_rel);
        return;
    }
    let parent_is_white = board.side_to_move() == Color::White;
    let mut moves: Vec<GameMove> = Vec::with_capacity(128);
    generate_all_place_moves(board, &mut moves);
    for mv in moves {
        let rev = board.do_move(mv);
        if board.game_result().is_none() {
            let w = eval_white(board, eval, info);
            prog.add_eval();
            let (na, nr) = child_flags(w, still_abs, still_rel, p, node_white, parent_is_white);
            if should_explore(p.b, na, nr) {
                // The child's own eval `w` becomes the relative-to-top anchor for its children.
                dfs_inline(board, eval, info, plies_left - 1, p, w, na, nr, prog);
            }
        }
        board.reverse_move(rev);
    }
}

/// Process one task. `Seed` evaluates its ply-3 node and then either counts leaves inline (`plies<=4`)
/// or spawns a ply-4 `Node` task per in-band child (`plies>=5`, collected into `spawns`). `Node`
/// DFS-expands its subtree inline.
#[allow(clippy::too_many_arguments)]
fn process_task(
    task: Task,
    rs: &Ruleset,
    cfg: &BalancedLinesConfig,
    bounds: Bounds,
    aggs: &[ArchAgg],
    eval: &mut NNUE6,
    info: &mut SearchInfo,
    spawns: &Mutex<Vec<Task>>,
) {
    match task {
        Task::Seed { arch_idx, ply3 } => {
            let agg = &aggs[rs.agg_base + arch_idx];
            let params = NodeParams { b: bounds, base_white: agg.base_white.load(Ordering::Relaxed) };
            let mut board = build_board(rs, arch_idx, &[ply3]);
            let mut prog = Prog::new(agg, bounds);
            if board.game_result().is_none() {
                let w = eval_white(&mut board, eval, info);
                prog.add_eval();
                // ply-3's parent is the base position (always White to move; its eval is `base_white`).
                // Base is in-band for every requested band, so the running flags start `true`.
                let (na, nr) = child_flags(w, true, true, params, params.base_white, true);
                if should_explore(bounds, na, nr) {
                    let remaining = cfg.plies - 3; // plies still to play after ply 3
                    if remaining == 0 {
                        // plies == 3: ply 3 is the end-state.
                        let key = canonical_board_key(&board);
                        prog.add_leaf(key, na, nr);
                    } else if remaining == 1 {
                        // plies == 4: expand ply-4 leaves inline (no ply-4 tasks). `w` = ply-3 anchor.
                        dfs_inline(&mut board, eval, info, 1, params, w, na, nr, &mut prog);
                    } else {
                        // plies >= 5: spawn a ply-4 task per in-band ply-4 child (`w` = ply-3 anchor).
                        let ply3_is_white = board.side_to_move() == Color::White;
                        let mut moves: Vec<GameMove> = Vec::with_capacity(128);
                        generate_all_place_moves(&board, &mut moves);
                        let mut new_tasks: Vec<Task> = Vec::new();
                        for mv in moves {
                            let rev = board.do_move(mv);
                            if board.game_result().is_none() {
                                let w4 = eval_white(&mut board, eval, info);
                                prog.add_eval();
                                let (na4, nr4) = child_flags(w4, na, nr, params, w, ply3_is_white);
                                if should_explore(bounds, na4, nr4) {
                                    new_tasks.push(Task::Node {
                                        arch_idx,
                                        moves: vec![ply3, mv],
                                        plies_left: remaining - 1,
                                        node_white: w4,
                                        still_abs: na4,
                                        still_rel: nr4,
                                    });
                                }
                            }
                            board.reverse_move(rev);
                        }
                        if !new_tasks.is_empty() {
                            agg.tasks_total.fetch_add(new_tasks.len() as u64, Ordering::Relaxed);
                            spawns.lock().unwrap().extend(new_tasks);
                        }
                    }
                }
            }
            prog.flush();
            agg.tasks_done.fetch_add(1, Ordering::Relaxed);
        }
        Task::Node { arch_idx, moves, plies_left, node_white, still_abs, still_rel } => {
            let agg = &aggs[rs.agg_base + arch_idx];
            let params = NodeParams { b: bounds, base_white: agg.base_white.load(Ordering::Relaxed) };
            let mut board = build_board(rs, arch_idx, &moves);
            let mut prog = Prog::new(agg, bounds);
            dfs_inline(&mut board, eval, info, plies_left, params, node_white, still_abs, still_rel, &mut prog);
            prog.flush();
            agg.tasks_done.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Run a worker pool over a static task queue; return any tasks the workers spawned (ply-4 `Node`s).
fn run_round(
    tasks: VecDeque<Task>,
    rs: &Ruleset,
    cfg: &BalancedLinesConfig,
    bounds: Bounds,
    aggs: &Arc<Vec<ArchAgg>>,
) -> Vec<Task> {
    if tasks.is_empty() {
        return Vec::new();
    }
    let nworkers = if cfg.threads > 0 {
        cfg.threads
    } else {
        thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
    }
    .max(1)
    .min(tasks.len());

    let queue = Arc::new(Mutex::new(tasks));
    let spawns = Arc::new(Mutex::new(Vec::<Task>::new()));
    let mut handles = Vec::with_capacity(nworkers);
    for _ in 0..nworkers {
        let queue = queue.clone();
        let spawns = spawns.clone();
        let aggs = aggs.clone();
        let cfg = cfg.clone();
        let rs = *rs;
        handles.push(thread::spawn(move || {
            let table = HashTable::new(BAL_TT_SIZE);
            let mut eval = NNUE6::default();
            let mut info = make_info(cfg.depth, &table);
            loop {
                let task = {
                    let mut q = queue.lock().unwrap();
                    q.pop_front()
                };
                let Some(task) = task else { break };
                process_task(task, &rs, &cfg, bounds, &aggs, &mut eval, &mut info, &spawns);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    let collected = std::mem::take(&mut *spawns.lock().unwrap());
    collected
}

/// Enumerate one ruleset: base-check every archetype and seed ply-3 tasks (phase 1), then run round 1
/// (seeds) and round 2 (any spawned ply-4 tasks).
fn run_ruleset(rs: &Ruleset, cfg: &BalancedLinesConfig, bounds: Bounds, aggs: &Arc<Vec<ArchAgg>>) {
    let mut seeds: VecDeque<Task> = VecDeque::new();
    {
        let table = HashTable::new(BAL_TT_SIZE);
        let mut eval = NNUE6::default();
        let mut info = make_info(cfg.depth, &table);
        for arch_idx in 0..rs.num_arch {
            let agg = &aggs[rs.agg_base + arch_idx];
            let mut board = Board6::new().with_komi(rs.komi);
            apply_opening(&mut board, arch_idx, rs.double);

            if board.game_result().is_some() {
                agg.dead.store(true, Ordering::Relaxed);
                continue;
            }
            let base_white = eval_white(&mut board, &mut eval, &mut info);
            agg.evals.fetch_add(1, Ordering::Relaxed);
            agg.base_white.store(base_white, Ordering::Relaxed);

            if bounds.want_abs && base_white.abs() > bounds.abs_bound.unwrap() {
                agg.dead.store(true, Ordering::Relaxed);
                continue;
            }
            if cfg.plies <= 2 {
                // The opening itself is the whole (single) balanced end-state.
                let key = canonical_board_key(&board);
                if bounds.require_both {
                    agg.both_set.lock().unwrap().insert(key);
                } else {
                    if bounds.want_abs {
                        agg.abs_set.lock().unwrap().insert(key);
                    }
                    if bounds.want_rel {
                        agg.rel_set.lock().unwrap().insert(key);
                    }
                    if bounds.want_abs && bounds.want_rel {
                        agg.both_set.lock().unwrap().insert(key);
                    }
                }
                continue;
            }
            let mut moves: Vec<GameMove> = Vec::with_capacity(128);
            generate_all_place_moves(&board, &mut moves);
            agg.tasks_total.store(moves.len() as u64, Ordering::Relaxed);
            for mv in moves {
                seeds.push_back(Task::Seed { arch_idx, ply3: mv });
            }
        }
    }
    if seeds.is_empty() {
        return;
    }

    // Round 1: ply-3 seeds (may spawn ply-4 tasks). Round 2: those ply-4 tasks.
    let nodes = run_round(seeds, rs, cfg, bounds, aggs);
    if !nodes.is_empty() {
        run_round(VecDeque::from(nodes), rs, cfg, bounds, aggs);
    }
}

/// Compact human number, e.g. 45600 -> "45.6K", 2_300_000 -> "2.3M".
fn human(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Sum of searches (evals) run so far across all archetypes — the run's main liveness signal.
fn total_evals(aggs: &[ArchAgg]) -> u64 {
    aggs.iter().map(|a| a.evals.load(Ordering::Relaxed)).sum()
}

/// Print one progress block for all archetypes to stderr, adapting to the active mode. Counts are the
/// distinct-end-state set sizes so far (they advance at task boundaries); `rate` is searches/sec.
fn print_progress(aggs: &[ArchAgg], start: Instant, b: Bounds, rate: f64) {
    let mut out = io::stderr().lock();
    let evals = total_evals(aggs);
    let _ = writeln!(
        out,
        "--- {:>6.1}s --- {} searches, {:.1}/s ---",
        start.elapsed().as_secs_f64(),
        human(evals),
        rate,
    );
    for rs in RULESETS.iter() {
        let mut parts: Vec<String> = Vec::new();
        for arch_idx in 0..rs.num_arch {
            let a = &aggs[rs.agg_base + arch_idx];
            let short: String = a.name.split(' ').next().unwrap_or(a.name).to_string();
            if a.dead.load(Ordering::Relaxed) {
                parts.push(format!("{short} DEAD"));
                continue;
            }
            let done = a.tasks_done.load(Ordering::Relaxed);
            let total = a.tasks_total.load(Ordering::Relaxed);
            let counts = if b.require_both {
                human(a.both_count())
            } else {
                match (b.want_abs, b.want_rel) {
                    (true, true) => format!(
                        "A:{} R:{} B:{}",
                        human(a.abs_count()),
                        human(a.rel_count()),
                        human(a.both_count())
                    ),
                    (true, false) => human(a.abs_count()),
                    (false, true) => human(a.rel_count()),
                    (false, false) => String::new(),
                }
            };
            parts.push(format!("{short} {counts} {}/{}", done, total));
        }
        let _ = writeln!(out, "  [{}] {}", rs.label, parts.join("  |  "));
    }
    let _ = out.flush();
}

/// Format a JSON `Option<i32>` (value or `null`).
fn opt_num(v: Option<i32>) -> String {
    v.map(|x| x.to_string()).unwrap_or_else(|| "null".to_string())
}

/// Write the JSON results file. `abs`/`rel`/`both` are distinct-end-state counts. Every archetype is
/// written (including 0-count and `dead`); the renderer decides which to show based on `mode`.
fn write_json(cfg: &BalancedLinesConfig, b: Bounds, aggs: &[ArchAgg]) -> io::Result<()> {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str(&format!(
        "  \"config\": {{\"depth\": {}, \"plies\": {}, \"cp_bound\": {}, \"rel_bound\": {}, \
         \"cp_bound_internal\": {}, \"rel_bound_internal\": {}, \"relative_to_top\": {}, \
         \"require_both\": {}}},\n",
        cfg.depth,
        cfg.plies,
        opt_num(cfg.cp_bound),
        opt_num(cfg.rel_bound),
        opt_num(b.abs_bound),
        opt_num(b.rel_bound),
        cfg.relative_to_top,
        cfg.require_both,
    ));
    s.push_str(&format!("  \"mode\": \"{}\",\n", b.mode()));
    s.push_str("  \"dedup\": \"distinct end-state positions (D4 symmetry-collapsed)\",\n");
    s.push_str("  \"rulesets\": {\n");
    for (ri, rs) in RULESETS.iter().enumerate() {
        let mut arch_json: Vec<String> = Vec::new();
        for arch_idx in 0..rs.num_arch {
            let a = &aggs[rs.agg_base + arch_idx];
            let opening_cp = a.base_white.load(Ordering::Relaxed) as f64 / 4.0;
            arch_json.push(format!(
                "{{\"name\": \"{}\", \"dead\": {}, \"opening_cp\": {:.1}, \"abs\": {}, \"rel\": {}, \"both\": {}}}",
                a.name,
                a.dead.load(Ordering::Relaxed),
                opening_cp,
                a.abs_count(),
                a.rel_count(),
                a.both_count(),
            ));
        }
        let comma = if ri + 1 < RULESETS.len() { "," } else { "" };
        s.push_str(&format!(
            "    \"{}\": {{\"label\": \"{}\", \"archetypes\": [{}]}}{}\n",
            rs.key,
            rs.label,
            arch_json.join(", "),
            comma
        ));
    }
    s.push_str("  }\n}\n");
    std::fs::write(&cfg.output, s)?;
    Ok(())
}

/// Grand total of the "primary" metric for the end-of-run summary: the intersection count under
/// `require_both`, else the absolute count when the absolute band is active, else the relative count.
fn grand_total(aggs: &[ArchAgg], b: Bounds) -> u64 {
    aggs.iter()
        .map(|a| {
            if b.require_both {
                a.both_count()
            } else if b.want_abs {
                a.abs_count()
            } else {
                a.rel_count()
            }
        })
        .sum()
}

/// Run the whole experiment: both rulesets, all archetypes, with a live progress monitor, then
/// write the JSON results.
pub fn run(cfg: BalancedLinesConfig) -> io::Result<()> {
    assert!(cfg.plies >= 2, "plies must be >= 2 (it includes the 2 opening plies)");
    let bounds = Bounds::from_cfg(&cfg);
    assert!(
        bounds.want_abs || bounds.want_rel,
        "provide at least one of cp_bound (-b) or rel_bound (-r)"
    );
    assert!(
        !bounds.rel_to_top || bounds.want_rel,
        "--relativeToTop (-rtt) requires a relative bound (-r)"
    );
    assert!(
        !bounds.require_both || (bounds.want_abs && bounds.want_rel),
        "--requireBoth (-rb) requires both an absolute bound (-b) and a relative bound (-r)"
    );

    let mut agg_vec: Vec<ArchAgg> = Vec::with_capacity(NUM_AGGS);
    for rs in RULESETS.iter() {
        for arch_idx in 0..rs.num_arch {
            agg_vec.push(ArchAgg::new(ARCHS[arch_idx].0));
        }
    }
    let aggs = Arc::new(agg_vec);

    eprintln!(
        "balancedlines: mode={} depth={} plies={} cp_bound={} rel_bound={} -> {}",
        bounds.mode(),
        cfg.depth,
        cfg.plies,
        opt_num(cfg.cp_bound),
        opt_num(cfg.rel_bound),
        cfg.output
    );
    eprintln!("balancedlines: counting DISTINCT end-state positions (D4 symmetry-collapsed).");
    eprintln!("balancedlines: evals are TT-isolated -> results are deterministic (same across runs/threads).");
    if cfg.depth >= 10 {
        eprintln!(
            "balancedlines: NOTE depth {} means each position is a full fixed-depth search (seconds \
             each; ~10s+ at depth 12). With several enumerated plies the run can take hours to days — \
             watch the 'searches/s' rate below to gauge feasibility.",
            cfg.depth
        );
    }

    // Live monitor thread.
    let start = Instant::now();
    let done = Arc::new(AtomicBool::new(false));
    let monitor = {
        let aggs = aggs.clone();
        let done = done.clone();
        thread::spawn(move || {
            let mut prev_evals = 0u64;
            let mut prev_t = Instant::now();
            while !done.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1500));
                if done.load(Ordering::Relaxed) {
                    break;
                }
                let cur = total_evals(&aggs);
                let dt = prev_t.elapsed().as_secs_f64().max(1e-3);
                let rate = (cur.saturating_sub(prev_evals)) as f64 / dt;
                prev_evals = cur;
                prev_t = Instant::now();
                print_progress(&aggs, start, bounds, rate);
            }
        })
    };

    // Rulesets run sequentially so per-worker transposition tables never mix komi values.
    for rs in RULESETS.iter() {
        run_ruleset(rs, &cfg, bounds, &aggs);
    }

    done.store(true, Ordering::Relaxed);
    let _ = monitor.join();
    let avg_rate = total_evals(&aggs) as f64 / start.elapsed().as_secs_f64().max(1e-3);
    print_progress(&aggs, start, bounds, avg_rate);

    write_json(&cfg, bounds, &aggs)?;
    let metric_label = if bounds.require_both {
        "both"
    } else if bounds.want_abs {
        "absolute"
    } else {
        "relative"
    };
    eprintln!(
        "balancedlines: done in {:.1}s. total distinct ({}) end-states = {}. wrote {}",
        start.elapsed().as_secs_f64(),
        metric_label,
        grand_total(&aggs, bounds),
        cfg.output
    );
    Ok(())
}
