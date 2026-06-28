//! Self-play balance assessment for 5x5 (Board5/NNUE5 port of [`crate::balance`]).
//!
//! Plays many self-play games (the same net on both sides) and reports White/Black win rates and
//! win types (road vs flat) with a 95% CI. Used to compare the balance of the **black-stack** variant
//! (komi 0, black-stack net + black-stack book) against **standard** 5x5 (komi 0, standard net +
//! standard book). A result near 50/50 is balanced; far from it is not.
//!
//! Book positions are at `move_num >= 2`, so games start past the opening and the black-stack rule
//! never fires from a book — i.e. a standard book gives standard play. Komi is applied to the start
//! position. Output feeds the comparison info-sheets (`gen_sheets.py`).

use crate::board::{Board5, TakBoard};
use crate::datagen::new_rng;
use crate::datagen5::{do_random_flat_move, Tiltak};
use crate::eval::NNUE5;
use crate::search::{search, SearchInfo};
use crate::transposition_table::HashTable;
use crate::{Color, GameMove, GameResult, Position};

use std::fs::{self, File};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

const MAX_DEPTH: usize = 60;

#[derive(Clone, Debug)]
pub struct BalanceConfig {
    pub num_games: usize,
    pub threads: usize,
    pub max_nodes: u64,
    pub random_plies: usize,
    /// If set, force White's opening double-black-stack onto a random corner instead of a random
    /// square. The random plies then apply on top of that fixed opening.
    pub corner_open: bool,
    /// If set, a TPS opening book (one position per line). Each position is played exactly once,
    /// deterministically — the book size becomes the game count and `random_plies`/`corner_open`
    /// are ignored. See `topaz openings5`.
    pub book_path: Option<String>,
    /// Half-komi added to Black in the win check (0 for the black-stack variant and the standard
    /// 0-komi comparison). Applied to every game's starting position.
    pub komi: u8,
    /// If set, write each played game as PTN (with a `[TPS]` start tag) to this file. Forces
    /// single-threaded play; intended for inspecting a small sample (use a small book).
    pub ptnout: Option<String>,
    /// Generate the opening book internally (`num_games` openings) and break results down by opening
    /// archetype. Overrides `book_path`/`random_plies`.
    pub archetype_breakdown: bool,
    /// With `archetype_breakdown`: generate standard openings instead of black-stack.
    pub standard_book: bool,
    /// With `archetype_breakdown`: per-archetype relative weights (None = uniform).
    pub archetype_weights: Option<Vec<f64>>,
    /// Record PTN (when `ptnout` is set) for only the first `ptn_limit` games. Default is unlimited.
    pub ptn_limit: usize,
    pub tt_size: usize,
    /// If set, drive this tiltak TEI binary (self-play) instead of the embedded NNUE5 net — a
    /// cross-check of standard balance with the native standard engine. Use with `-a`/`--book`
    /// (book openings); the random-opening path forces the `2xx` move tiltak can't make.
    pub tiltak_path: Option<String>,
    /// Max plies before scoring a game a draw (used in tiltak mode to bound runaway games).
    pub max_plies: usize,
}

impl Default for BalanceConfig {
    fn default() -> Self {
        Self {
            num_games: 20000,
            threads: 4,
            max_nodes: 15000,
            random_plies: 4,
            corner_open: false,
            book_path: None,
            komi: 0,
            ptnout: None,
            archetype_breakdown: false,
            standard_book: false,
            archetype_weights: None,
            ptn_limit: usize::MAX,
            tt_size: 1 << 20,
            tiltak_path: None,
            max_plies: 200,
        }
    }
}

/// The classified outcome of a single game.
#[derive(Clone, Copy)]
enum Outcome {
    WhiteRoad,
    WhiteFlat,
    BlackRoad,
    BlackFlat,
    Draw,
}

#[derive(Default)]
struct Tally {
    white_road: AtomicU64,
    white_flat: AtomicU64,
    black_road: AtomicU64,
    black_flat: AtomicU64,
    draws: AtomicU64,
    plies: AtomicU64,
    games: AtomicU64,
}

impl Tally {
    fn record(&self, outcome: Outcome, plies: u64) {
        match outcome {
            Outcome::WhiteRoad => &self.white_road,
            Outcome::WhiteFlat => &self.white_flat,
            Outcome::BlackRoad => &self.black_road,
            Outcome::BlackFlat => &self.black_flat,
            Outcome::Draw => &self.draws,
        }
        .fetch_add(1, Ordering::Relaxed);
        self.plies.fetch_add(plies, Ordering::Relaxed);
        self.games.fetch_add(1, Ordering::Relaxed);
    }

    fn to_result(&self) -> BalanceResult {
        BalanceResult {
            white_road: self.white_road.load(Ordering::Relaxed),
            white_flat: self.white_flat.load(Ordering::Relaxed),
            black_road: self.black_road.load(Ordering::Relaxed),
            black_flat: self.black_flat.load(Ordering::Relaxed),
            draws: self.draws.load(Ordering::Relaxed),
            games: self.games.load(Ordering::Relaxed),
            total_plies: self.plies.load(Ordering::Relaxed),
        }
    }
}

/// Aggregate outcome counts from a balance run.
#[derive(Debug, Clone, Copy)]
pub struct BalanceResult {
    pub white_road: u64,
    pub white_flat: u64,
    pub black_road: u64,
    pub black_flat: u64,
    pub draws: u64,
    pub games: u64,
    pub total_plies: u64,
}

impl BalanceResult {
    pub fn white_wins(&self) -> u64 {
        self.white_road + self.white_flat
    }
    pub fn black_wins(&self) -> u64 {
        self.black_road + self.black_flat
    }
}

/// Play one self-play game to completion and classify the result into the tally.
fn play_one(
    cfg: &BalanceConfig,
    table: &HashTable,
    rng: &mut impl rand_core::RngCore,
    start: Option<&str>,
    ptn_out: Option<&Mutex<File>>,
    tiltak: Option<&mut Tiltak>,
) -> (Outcome, u64) {
    let mut plies = 0u64;

    let mut board = match start {
        Some(tps) => Board5::try_from_tps(tps)
            .expect("balance book contained an invalid tps")
            .with_komi(cfg.komi),
        None => {
            let mut board = Board5::new().with_komi(cfg.komi);

            // Optionally pin White's opening double-black-stack to a (random) corner.
            if cfg.corner_open {
                const CORNERS: [&str; 4] = ["2a1", "2e1", "2a5", "2e5"];
                let mv =
                    GameMove::try_from_ptn_m(CORNERS[(rng.next_u32() % 4) as usize], 5, Color::White)
                        .expect("valid corner double-placement");
                board.do_move(mv);
                plies += 1;
            }

            // Random flat opening plies for diversity (no walls/caps), on top of any corner open.
            for _ in 0..cfg.random_plies {
                if board.game_result().is_some() {
                    break;
                }
                do_random_flat_move(&mut board, rng);
                plies += 1;
            }
            board
        }
    };

    let record = ptn_out.is_some();
    let start_tps = if record { Some(format!("{board:?}")) } else { None };
    let start_move_num = board.move_num();
    let white_first = board.side_to_move() == Color::White;
    let mut game_moves: Vec<GameMove> = Vec::new();

    // Best play to completion — driven by tiltak (TEI) if provided, else Topaz NNUE5 search.
    if let Some(tiltak) = tiltak {
        let _ = tiltak.new_game(5);
        while board.game_result().is_none() {
            if plies >= cfg.max_plies as u64 {
                break; // bounded draw
            }
            let tps = format!("{board:?}");
            let mv = match tiltak.bestmove(&tps, cfg.max_nodes) {
                Ok(Some((m, _))) => m,
                _ => break,
            };
            let gm = match GameMove::try_from_ptn(&mv, &board) {
                Some(g) => g,
                None => break,
            };
            if record {
                game_moves.push(gm);
            }
            board.do_move(gm);
            plies += 1;
        }
    } else {
        table.clear();
        while board.game_result().is_none() {
            let mut eval = NNUE5::default();
            let mut info = SearchInfo::new(MAX_DEPTH, table)
                .set_max_nodes(cfg.max_nodes, cfg.max_nodes)
                .quiet(true);
            let outcome = match search(&mut board, &mut eval, &mut info) {
                Some(o) => o,
                None => break,
            };
            let mv = match outcome.next() {
                Some(m) => m,
                None => break,
            };
            if record {
                game_moves.push(mv);
            }
            board.do_move(mv);
            plies += 1;
        }
    }

    let outcome = match board.game_result() {
        Some(GameResult::WhiteWin) => {
            if board.road(Color::White) {
                Outcome::WhiteRoad
            } else {
                Outcome::WhiteFlat
            }
        }
        Some(GameResult::BlackWin) => {
            if board.road(Color::Black) {
                Outcome::BlackRoad
            } else {
                Outcome::BlackFlat
            }
        }
        _ => Outcome::Draw,
    };

    if let (Some(file), Some(tps)) = (ptn_out, start_tps) {
        let result_str = match outcome {
            Outcome::WhiteRoad | Outcome::WhiteFlat => "1-0",
            Outcome::BlackRoad | Outcome::BlackFlat => "0-1",
            Outcome::Draw => "1/2-1/2",
        };
        let moves_str = moves_to_ptn(&game_moves, start_move_num, white_first);
        let block = format!(
            "[Site \"Topaz balance5\"]\n[TPS \"{tps}\"]\n[Komi \"{}\"]\n[Result \"{result_str}\"]\n\n{moves_str}{result_str}\n\n",
            cfg.komi
        );
        let _ = file.lock().unwrap().write_all(block.as_bytes());
    }

    (outcome, plies)
}

/// Format a move list as numbered PTN starting from `start_move_num`.
fn moves_to_ptn(moves: &[GameMove], start_move_num: usize, white_first: bool) -> String {
    let mut s = String::new();
    let mut num = start_move_num;
    let mut i = 0;
    if !white_first && i < moves.len() {
        s.push_str(&format!("{num}. -- {} ", moves[i].to_ptn::<Board5>()));
        i += 1;
        num += 1;
    }
    while i < moves.len() {
        let w = moves[i].to_ptn::<Board5>();
        i += 1;
        if i < moves.len() {
            let b = moves[i].to_ptn::<Board5>();
            i += 1;
            s.push_str(&format!("{num}. {w} {b} "));
        } else {
            s.push_str(&format!("{num}. {w} "));
        }
        num += 1;
    }
    s
}

/// Run the balance assessment, print a summary, and return the aggregate counts.
pub fn run(cfg: BalanceConfig) -> BalanceResult {
    assert!(cfg.threads >= 1, "need at least one thread");
    let start = Instant::now();

    let book: Option<Vec<String>> = cfg.book_path.as_ref().map(|path| {
        let data = fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("could not read opening book {path}: {e}"));
        data.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>()
    });

    let labeled = cfg.archetype_breakdown.then(|| match &cfg.archetype_weights {
        Some(w) => crate::openings5::generate_weighted(cfg.num_games, cfg.standard_book, w),
        None => crate::openings5::generate(cfg.num_games, cfg.standard_book),
    });
    let num_buckets = if cfg.archetype_breakdown {
        crate::openings5::num_archetypes(cfg.standard_book)
    } else {
        1
    };

    let schedule: Vec<(Option<String>, usize)> = if let Some(lab) = &labeled {
        lab.iter().map(|(tps, _, a)| (Some(tps.clone()), *a)).collect()
    } else if let Some(b) = &book {
        b.iter().map(|tps| (Some(tps.clone()), 0)).collect()
    } else {
        (0..cfg.num_games).map(|_| (None, 0)).collect()
    };
    let total_games = schedule.len();

    if cfg.archetype_breakdown {
        println!(
            "balance5: {} generated {} openings (per-archetype breakdown{}), {} threads, {} nodes/move (half-komi {})",
            total_games,
            if cfg.standard_book { "standard" } else { "black-stack" },
            if cfg.archetype_weights.is_some() { ", weighted" } else { "" },
            cfg.threads, cfg.max_nodes, cfg.komi,
        );
    } else if let Some(b) = &book {
        println!(
            "balance5: {} book positions from {} (1 deterministic game each), {} threads, {} nodes/move (half-komi {})",
            b.len(), cfg.book_path.as_deref().unwrap_or(""), cfg.threads, cfg.max_nodes, cfg.komi,
        );
    } else {
        println!(
            "balance5: {} games, {} threads, {} nodes/move, {} random flat plies{} (half-komi {})",
            cfg.num_games, cfg.threads, cfg.max_nodes, cfg.random_plies,
            if cfg.corner_open { ", White opens in a corner" } else { "" }, cfg.komi,
        );
    }

    let ptn_writer: Option<Arc<Mutex<File>>> = cfg.ptnout.as_ref().map(|path| {
        Arc::new(Mutex::new(
            File::create(path).unwrap_or_else(|e| panic!("could not create ptnout {path}: {e}")),
        ))
    });
    let threads = if ptn_writer.is_some() && cfg.ptn_limit >= total_games {
        1
    } else {
        cfg.threads
    };

    let schedule = Arc::new(schedule);
    let tallies: Arc<Vec<Tally>> = Arc::new((0..num_buckets).map(|_| Tally::default()).collect());
    let progress = Arc::new(AtomicU64::new(0));
    let base = total_games / threads;
    let extra = total_games % threads;
    let mut handles = Vec::new();
    let mut next = 0usize;
    for t in 0..threads {
        let count = base + if t < extra { 1 } else { 0 };
        let range = next..next + count;
        next += count;
        if count == 0 {
            continue;
        }
        let cfg = cfg.clone();
        let schedule = schedule.clone();
        let tallies = tallies.clone();
        let progress = progress.clone();
        let ptn_writer = ptn_writer.clone();
        handles.push(thread::spawn(move || {
            let mut rng = new_rng();
            // tiltak mode: one tiltak subprocess per worker, no TT needed (tiny placeholder table).
            let mut tiltak = cfg
                .tiltak_path
                .as_ref()
                .map(|p| Tiltak::spawn(p, 5, cfg.komi).expect("failed to spawn tiltak"));
            let table = HashTable::new(if tiltak.is_some() { 1 } else { cfg.tt_size });
            for i in range {
                let (start, bucket) = &schedule[i];
                let writer = if i < cfg.ptn_limit {
                    ptn_writer.as_deref()
                } else {
                    None
                };
                let (outcome, plies) =
                    play_one(&cfg, &table, &mut rng, start.as_deref(), writer, tiltak.as_mut());
                tallies[*bucket].record(outcome, plies);
                let done = progress.fetch_add(1, Ordering::Relaxed) + 1;
                if done % 1000 == 0 {
                    eprintln!("balance5: {done} games");
                }
            }
        }));
    }
    for h in handles {
        h.join().expect("balance worker panicked");
    }

    let per: Vec<BalanceResult> = tallies.iter().map(|t| t.to_result()).collect();
    let result = sum_results(&per);
    print_summary(&result, start.elapsed().as_secs_f64());
    if cfg.archetype_breakdown {
        println!("\n=== Per-archetype breakdown ===");
        for (i, r) in per.iter().enumerate() {
            println!(
                "\n[{}]  ({} games, {:.1}% of book)",
                crate::openings5::archetype_name(i),
                r.games,
                100.0 * r.games as f64 / result.games.max(1) as f64,
            );
            print_block(r);
        }
    }
    result
}

fn sum_results(results: &[BalanceResult]) -> BalanceResult {
    let mut s = BalanceResult {
        white_road: 0,
        white_flat: 0,
        black_road: 0,
        black_flat: 0,
        draws: 0,
        games: 0,
        total_plies: 0,
    };
    for r in results {
        s.white_road += r.white_road;
        s.white_flat += r.white_flat;
        s.black_road += r.black_road;
        s.black_flat += r.black_flat;
        s.draws += r.draws;
        s.games += r.games;
        s.total_plies += r.total_plies;
    }
    s
}

/// Print the win/loss/draw breakdown for one result block (no header, no timing).
fn print_block(r: &BalanceResult) {
    let n = r.games.max(1) as f64;
    let w = r.white_wins();
    let b = r.black_wins();
    let pct = |x: u64| 100.0 * x as f64 / n;
    let split = |road: u64, flat: u64| {
        let tot = (road + flat).max(1) as f64;
        (100.0 * road as f64 / tot, 100.0 * flat as f64 / tot)
    };
    let (wr, wf) = split(r.white_road, r.white_flat);
    let (br, bf) = split(r.black_road, r.black_flat);
    let p = w as f64 / n;
    let ci = 1.96 * (p * (1.0 - p) / n).sqrt() * 100.0;
    let roads = r.white_road + r.black_road;
    let flats = r.white_flat + r.black_flat;
    let decisive = (roads + flats).max(1) as f64;

    println!(
        "  White wins: {:>6} ({:>5.1}%)   [road {} ({:.1}%), flat {} ({:.1}%)]",
        w, pct(w), r.white_road, pct(r.white_road), r.white_flat, pct(r.white_flat)
    );
    println!(
        "  Black wins: {:>6} ({:>5.1}%)   [road {} ({:.1}%), flat {} ({:.1}%)]",
        b, pct(b), r.black_road, pct(r.black_road), r.black_flat, pct(r.black_flat)
    );
    println!("  Draws:      {:>6} ({:>5.1}%)", r.draws, pct(r.draws));
    println!("  White win types: road {:.1}%, flat {:.1}%  (of White's wins)", wr, wf);
    println!("  Black win types: road {:.1}%, flat {:.1}%  (of Black's wins)", br, bf);
    println!(
        "  Win types:  road {} ({:.1}% of wins), flat {} ({:.1}% of wins)",
        roads, 100.0 * roads as f64 / decisive, flats, 100.0 * flats as f64 / decisive
    );
    println!(
        "  White win rate: {:.1}% ± {:.1}% (95% CI)   White-Black: {:+.1}%",
        pct(w), ci, pct(w) - pct(b)
    );
}

fn print_summary(r: &BalanceResult, secs: f64) {
    println!("\n=== Balance over {} games ===", r.games);
    print_block(r);
    let avg_plies = r.total_plies as f64 / r.games.max(1) as f64;
    println!(
        "  Avg game length: {:.1} plies ({:.1} moves)   ({:.0} games/sec)",
        avg_plies,
        avg_plies / 2.0,
        r.games.max(1) as f64 / secs.max(1e-9)
    );
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn balance5_accounts_for_all_games() {
        let cfg = BalanceConfig {
            num_games: 2,
            threads: 2,
            max_nodes: 100,
            random_plies: 4,
            corner_open: true,
            book_path: None,
            komi: 0,
            ptnout: None,
            archetype_breakdown: false,
            standard_book: false,
            archetype_weights: None,
            ptn_limit: usize::MAX,
            tt_size: 1 << 20,
            tiltak_path: None,
            max_plies: 200,
        };
        let r = run(cfg);
        assert_eq!(r.games, 2);
        assert_eq!(
            r.white_road + r.white_flat + r.black_road + r.black_flat + r.draws,
            2
        );
        assert!(r.total_plies > 0);
    }
}
