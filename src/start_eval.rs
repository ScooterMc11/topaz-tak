//! Start-position evaluator: how each 6x6 opening *archetype* is evaluated as search depth grows,
//! compared between the **black-stack** ruleset (forced `2xx` double-black-stack opening, komi 0)
//! and the **2-komi** ruleset (single swapped black flat opening, 2 komi / half-komi 4).
//!
//! For each ruleset x archetype we build the canonical 2-ply opening position and then read the
//! whole eval-vs-depth curve from a **single** iterative-deepening search: depth 0 is the raw NNUE
//! static eval (no search) and depths `1..=max_depth` are the principal-variation scores captured as
//! each iteration completes (`SearchInfo::depth_scores`). One search yields every depth for free,
//! since iterative deepening computes them all on the way up.
//!
//! **Perspective.** At the root of every archetype it is White to move, so all scores land in one
//! White-positive frame directly; we still normalize by side-to-move defensively (flip if Black to
//! move) so a positive number always means "better for White". Reported cp is **human cp** = the
//! engine's internal cp divided by 4.
//!
//! **Net selection** is automatic by komi inside `search` (`eval.set_komi`), and we call `set_komi`
//! ourselves before the depth-0 static eval — so each ruleset is judged by its own net with no
//! rebuild dance.
//!
//! **Parallelism.** The 7 positions (4 black-stack + 3 two-komi) each run a deterministic
//! single-threaded search; a small worker pool runs several at once (one position per thread), so
//! wall-clock is roughly a single position's time rather than the sum. A live monitor polls each
//! search's `SearchProgress` (completed depth, PV score, cumulative nodes) and prints a block to
//! stderr every ~2s so a long deep iteration still shows advancing node counts.

use crate::board::{Board6, TakBoard};
use crate::eval::NNUE6;
use crate::search::{search, SearchInfo, SearchProgress};
use crate::transposition_table::HashTable;
use crate::{Color, GameMove, Position, TimeBank};

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const SIZE: usize = 6;

/// Transposition-table slots per worker. A generous table keeps deep searches fast and stable; with
/// at most 7 workers this is a few hundred MB total.
const TT_SLOTS: usize = 1 << 22;

/// Only depths at or above this are recorded/plotted. The search still deepens from depth 1, but the
/// first few plies are noisy and not meaningful for a balance comparison, so we drop them (and the
/// depth-0 static eval) from the results.
const MIN_RECORD_DEPTH: usize = 5;

/// A time bank large enough to never trigger the search's time-based stop (~11.5 days in ms); the
/// depth ceiling is what ends each search.
const NO_TIME_LIMIT_MS: u64 = 1_000_000_000;

/// Canonical opening instance per archetype: `(name, white_opening_square, black_swap_square)`. One
/// representative per archetype (the first entry of each `openings.rs` table). Black-stack uses all
/// four; 2-komi uses the first three (no gap-hug). Matches `balanced_lines::ARCHS`.
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
}

const RULESETS: [Ruleset; 2] = [
    Ruleset { key: "black_stack", label: "Black Stack", komi: 0, double: true, num_arch: 4 },
    Ruleset { key: "two_komi", label: "2-Komi", komi: 4, double: false, num_arch: 3 },
];

#[derive(Clone)]
pub struct StartEvalConfig {
    /// Maximum (ceiling) search depth. The curve runs from depth 0 (static eval) to this depth.
    pub max_depth: usize,
    /// Worker threads (0 = auto-detect, capped at the number of positions).
    pub threads: usize,
    /// JSON results output path.
    pub output: String,
}

impl Default for StartEvalConfig {
    fn default() -> Self {
        Self { max_depth: 30, threads: 0, output: "start_eval.json".to_string() }
    }
}

/// One unit of work: evaluate a single (ruleset, archetype) position across all depths.
#[derive(Clone, Copy)]
struct TaskSpec {
    /// Index into `RULESETS`.
    rs_idx: usize,
    /// Index into `ARCHS` (0..ruleset.num_arch).
    arch_idx: usize,
}

/// Result for one (ruleset, archetype): the eval-vs-depth curve in human cp (White frame).
struct ArchResult {
    /// `(depth, human_cp)` for depth 0 (static eval) through the deepest completed depth.
    points: Vec<(usize, f64)>,
}

/// PTN of the two opening plies for display, e.g. `2a1 f6` (black-stack) or `a1 f6` (2-komi).
fn opening_ptn(rs: &Ruleset, arch_idx: usize) -> String {
    let (_, white_sq, black_sq) = ARCHS[arch_idx];
    let first = if rs.double { format!("2{white_sq}") } else { white_sq.to_string() };
    format!("{first} {black_sq}")
}

/// Apply the fixed 2-ply opening for `arch_idx`. Mirrors `balanced_lines::apply_opening`: `do_move`
/// handles the move_num==1 colour swap for both plies, and `2xx` draws two black flats for ply 1.
fn build_opening_board(rs: &Ruleset, arch_idx: usize) -> Board6 {
    let (_, white_sq, black_sq) = ARCHS[arch_idx];
    let mut board = Board6::new().with_komi(rs.komi);
    let p1 = if rs.double { format!("2{white_sq}") } else { white_sq.to_string() };
    let m1 = GameMove::try_from_ptn_m(&p1, SIZE, board.side_to_move()).expect("valid opening ptn");
    board.do_move(m1);
    let m2 = GameMove::try_from_ptn_m(black_sq, SIZE, board.side_to_move()).expect("valid swap ptn");
    board.do_move(m2);
    board
}

/// Run one task to completion: depth-0 static eval + a single search to `max_depth`, filling the
/// shared result slot and updating `progress` as depths complete.
fn run_task(spec: TaskSpec, cfg: &StartEvalConfig, result: &Mutex<ArchResult>, progress: &Arc<SearchProgress>) {
    let rs = &RULESETS[spec.rs_idx];
    let mut board = build_opening_board(rs, spec.arch_idx);

    // Fixed White frame: root is White to move for every archetype, but normalize defensively.
    let stm_white = board.side_to_move() == Color::White;
    let to_human = |internal: i32| -> f64 {
        let w = if stm_white { internal } else { -internal };
        w as f64 / 4.0
    };

    let mut points: Vec<(usize, f64)> = Vec::with_capacity(cfg.max_depth + 1);

    // One iterative-deepening search to `max_depth`; per-depth PV scores are captured inside
    // (`depth_scores`). The ruleset's own net is selected by komi inside `search`. We record only
    // depths >= MIN_RECORD_DEPTH (shallow plies are noisy); there is no depth-0 static-eval point.
    let mut eval = NNUE6::default();
    let table = HashTable::new(TT_SLOTS);
    let mut info = SearchInfo::new(cfg.max_depth, &table)
        .quiet(true)
        .abort_depth(cfg.max_depth + 1) // disable the depth-based early break
        .time_bank(TimeBank::flat(NO_TIME_LIMIT_MS)) // disable the time-based stop
        .report_progress(progress.clone());
    search(&mut board, &mut eval, &mut info);

    for &(depth, internal) in info.depth_scores() {
        if depth >= MIN_RECORD_DEPTH {
            points.push((depth, to_human(internal)));
        }
    }

    result.lock().unwrap().points = points;
}

/// Compact human number, e.g. 45600 -> "45.6K", 2_300_000 -> "2.3M".
fn human(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Flat list of every (ruleset, archetype) task, in a stable order (all black-stack, then 2-komi).
fn build_tasks() -> Vec<TaskSpec> {
    let mut tasks = Vec::new();
    for (rs_idx, rs) in RULESETS.iter().enumerate() {
        for arch_idx in 0..rs.num_arch {
            tasks.push(TaskSpec { rs_idx, arch_idx });
        }
    }
    tasks
}

/// Print one progress block for all tasks to stderr: overall elapsed + node rate, then per position
/// the completed depth, current White-frame score (human cp), and cumulative nodes.
fn print_progress(
    tasks: &[TaskSpec],
    progress: &[Arc<SearchProgress>],
    done: &[AtomicBool],
    cfg: &StartEvalConfig,
    start: Instant,
    node_rate: f64,
) {
    let mut out = io::stderr().lock();
    let total_nodes: u64 = progress.iter().map(|p| p.nodes.load(Ordering::Relaxed)).sum();
    let _ = writeln!(
        out,
        "--- {:>7.1}s --- {} nodes, {}/s ---",
        start.elapsed().as_secs_f64(),
        human(total_nodes),
        human(node_rate as u64),
    );
    for (i, spec) in tasks.iter().enumerate() {
        let rs = &RULESETS[spec.rs_idx];
        let name = ARCHS[spec.arch_idx].0;
        let depth = progress[i].depth.load(Ordering::Relaxed);
        let score = progress[i].score.load(Ordering::Relaxed) as f64 / 4.0;
        let nodes = progress[i].nodes.load(Ordering::Relaxed);
        let tag = if done[i].load(Ordering::Relaxed) { "DONE" } else { "...." };
        let _ = writeln!(
            out,
            "  [{:<11} {:<16}] d {:>2}/{:<2}  {:>+7.1} cp  {:>7} nodes  {}",
            rs.label,
            name,
            depth,
            cfg.max_depth,
            score,
            human(nodes),
            tag,
        );
    }
    let _ = out.flush();
}

/// Write the JSON results: config + per-ruleset per-archetype `points` (already human cp, White
/// frame). Hand-rolled to avoid a serde dependency (matches `balanced_lines`).
fn write_json(cfg: &StartEvalConfig, tasks: &[TaskSpec], results: &[Mutex<ArchResult>]) -> io::Result<()> {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str(&format!(
        "  \"config\": {{\"max_depth\": {}, \"min_depth\": {}, \"threads\": {}, \"note\": \"cp is human (internal/4), White-positive frame; depths {}+ only\"}},\n",
        cfg.max_depth, MIN_RECORD_DEPTH, cfg.threads, MIN_RECORD_DEPTH
    ));
    s.push_str("  \"rulesets\": {\n");
    for (ri, rs) in RULESETS.iter().enumerate() {
        let mut arch_json: Vec<String> = Vec::new();
        for (i, spec) in tasks.iter().enumerate() {
            if spec.rs_idx != ri {
                continue;
            }
            let points = &results[i].lock().unwrap().points;
            let pts: Vec<String> =
                points.iter().map(|(d, cp)| format!("{{\"depth\": {}, \"cp\": {:.2}}}", d, cp)).collect();
            arch_json.push(format!(
                "{{\"name\": \"{}\", \"opening\": \"{}\", \"points\": [{}]}}",
                ARCHS[spec.arch_idx].0,
                opening_ptn(rs, spec.arch_idx),
                pts.join(", ")
            ));
        }
        let comma = if ri + 1 < RULESETS.len() { "," } else { "" };
        s.push_str(&format!(
            "    \"{}\": {{\"label\": \"{}\", \"komi\": {}, \"archetypes\": [{}]}}{}\n",
            rs.key,
            rs.label,
            rs.komi,
            arch_json.join(", "),
            comma
        ));
    }
    s.push_str("  }\n}\n");
    std::fs::write(&cfg.output, s)?;
    Ok(())
}

/// Run the whole experiment: both rulesets, all archetypes, with a live monitor, then write JSON.
pub fn run(cfg: StartEvalConfig) -> io::Result<()> {
    let tasks = build_tasks();
    let n = tasks.len();

    let nworkers = if cfg.threads > 0 {
        cfg.threads
    } else {
        thread::available_parallelism().map(|p| p.get()).unwrap_or(4)
    }
    .max(1)
    .min(n);

    let results: Arc<Vec<Mutex<ArchResult>>> =
        Arc::new((0..n).map(|_| Mutex::new(ArchResult { points: Vec::new() })).collect());
    let progress: Arc<Vec<Arc<SearchProgress>>> =
        Arc::new((0..n).map(|_| Arc::new(SearchProgress::default())).collect());
    let done: Arc<Vec<AtomicBool>> = Arc::new((0..n).map(|_| AtomicBool::new(false)).collect());

    eprintln!(
        "starteval: max_depth={} positions={} workers={} -> {}",
        cfg.max_depth, n, nworkers, cfg.output
    );
    eprintln!(
        "starteval: cp shown is human (internal/4), White-positive; recording depths {}+.",
        MIN_RECORD_DEPTH
    );
    eprintln!(
        "starteval: NOTE each position is one single-threaded search to depth {}; deep runs take \
         hours. Watch the nodes/s rate below for liveness.",
        cfg.max_depth
    );

    // Live monitor.
    let start = Instant::now();
    let monitor_done = Arc::new(AtomicBool::new(false));
    let monitor = {
        let progress = progress.clone();
        let done = done.clone();
        let monitor_done = monitor_done.clone();
        let cfg = cfg.clone();
        let tasks = tasks.clone();
        thread::spawn(move || {
            let mut prev_nodes = 0u64;
            let mut prev_t = Instant::now();
            while !monitor_done.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(2000));
                if monitor_done.load(Ordering::Relaxed) {
                    break;
                }
                let cur: u64 = progress.iter().map(|p| p.nodes.load(Ordering::Relaxed)).sum();
                let dt = prev_t.elapsed().as_secs_f64().max(1e-3);
                let rate = (cur.saturating_sub(prev_nodes)) as f64 / dt;
                prev_nodes = cur;
                prev_t = Instant::now();
                print_progress(&tasks, &progress, &done, &cfg, start, rate);
            }
        })
    };

    // Worker pool over a shared task-index queue.
    let queue: Arc<Mutex<VecDeque<usize>>> = Arc::new(Mutex::new((0..n).collect()));
    let mut handles = Vec::with_capacity(nworkers);
    for _ in 0..nworkers {
        let queue = queue.clone();
        let results = results.clone();
        let progress = progress.clone();
        let done = done.clone();
        let cfg = cfg.clone();
        let tasks = tasks.clone();
        handles.push(thread::spawn(move || loop {
            let idx = {
                let mut q = queue.lock().unwrap();
                q.pop_front()
            };
            let Some(idx) = idx else { break };
            run_task(tasks[idx], &cfg, &results[idx], &progress[idx]);
            done[idx].store(true, Ordering::Relaxed);
        }));
    }
    for h in handles {
        let _ = h.join();
    }

    monitor_done.store(true, Ordering::Relaxed);
    let _ = monitor.join();
    let avg_rate = progress.iter().map(|p| p.nodes.load(Ordering::Relaxed)).sum::<u64>() as f64
        / start.elapsed().as_secs_f64().max(1e-3);
    print_progress(&tasks, &progress, &done, &cfg, start, avg_rate);

    write_json(&cfg, &tasks, &results)?;
    eprintln!("starteval: done in {:.1}s. wrote {}", start.elapsed().as_secs_f64(), cfg.output);
    Ok(())
}
