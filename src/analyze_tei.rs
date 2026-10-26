//! Another engine doing `analyzegame`'s searching, over TEI.
//!
//! `analyze_game` asks one question of whatever searches for it — "search this position, ranking
//! this many root moves" (`PositionSearch`) — and everything it builds from the answer, the board
//! facts, the grading and the JSON, is its own. This module answers that question by driving a TEI
//! engine as a child process, so a sheet from another engine is graded by exactly the rules a topaz
//! sheet is. Takana is the caller; Syntaks is the engine it was written for, and the only one it has
//! been run against.
//!
//! **One process for the whole game**, set up once and then asked per position:
//!
//! ```text
//! teinewgame <size>          clears the engine's table and histories, so every position is
//! position tps <tps>         searched from scratch and the sheet is a pure function of the game
//! isready / readyok
//! go nodes <n>
//! info … (one line per multi-PV index at the end of every completed depth)
//! bestmove …
//! ```
//!
//! `teinewgame` goes in its **bare** form, `teinewgame 6`. The current TEI spec writes
//! `teinewgame size 6 halfkomi 4`, and Syntaks reads the word `size` as an invalid board size and
//! skips the reset — silently, which would let one position's table leak into the next.
//!
//! **Positions go in as TPS, never as a move list.** Every move-spelling question then disappears
//! except for the moves that come *back*, and those are parsed against topaz's own board (crush flags
//! inferred, legality checked) the same way a ply from a PTN is.
//!
//! **Scores go through the engine's own win probability.** Every gate in `analyze_game` (the drift
//! guard, the `--alts` gate, the solved test) and every threshold in Takana reads topaz's scale,
//! `sigmoid(internal / 400)`. Each info line's `wdl` is read as an expected score, `(W + D/2) /
//! (W + D + L)`, and mapped back onto that scale as `400 · logit(p)`, so another engine's 60% sits
//! exactly where topaz's 60% does. A line without `wdl` falls back to its `cp` read as human cp
//! (`internal / 4`). `mate n` becomes a proven win or loss in topaz's terms, `WIN_SCORE` minus the
//! distance in plies.
//!
//! **The depth ladder** is read off the multi-PV 1 line at each completed depth, which is what
//! topaz records as `depth_scores`. Lines marked `upperbound` / `lowerbound` are window bounds, not
//! scores, and are dropped; the repeat an engine prints when it stops is the same depth again and
//! simply overwrites it.

use crate::analyze_game::{Breadth, PositionSearch, Searched, SOLVED_MARGIN};
use crate::board::TakBoard;
use crate::eval::{LOSE_SCORE, WIN_SCORE};
use crate::{Color, GameMove, RevGameMove};
use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// Which engine to run, and how.
#[derive(Clone, Debug)]
pub struct TeiEngineConfig {
    /// The executable.
    pub command: String,
    /// What to call it in the JSON (`engine.name`). The engine's own `id name` is reported beside it.
    pub name: String,
    /// `setoption` pairs the caller wants on top of the ones this module sets itself — Takana's
    /// `HumanWDL true`, for instance. Applied after the defaults and before the game's own settings,
    /// so they can tune the engine but not change the game it is analyzing.
    pub options: Vec<(String, String)>,
}

/// An option the engine announced, as far as this module needs to know it.
struct AnnouncedOption {
    min: Option<i64>,
    max: Option<i64>,
}

pub struct TeiSearch {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// The engine's `id name`, e.g. `syntaks 0.1.0`.
    id: String,
    size: usize,
    nodes: u64,
    /// The `MultiPV` the engine currently has, so it is only re-sent when it changes.
    multi_pv: usize,
    line: String,
}

impl TeiSearch {
    /// Start the engine and set it up for one game. Fails — rather than analyzing the wrong game —
    /// when the engine cannot be told the game's komi or opening.
    pub fn start(
        cfg: &TeiEngineConfig,
        size: usize,
        half_komi: u8,
        double_black_stack: bool,
        nodes: u64,
        quiet: bool,
    ) -> Result<Self> {
        let mut child = Command::new(&cfg.command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(if quiet { Stdio::null() } else { Stdio::inherit() })
            .spawn()
            .with_context(|| format!("could not start {}", cfg.command))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin for {}", cfg.name))?;
        let stdout = BufReader::new(
            child.stdout.take().ok_or_else(|| anyhow!("no stdout for {}", cfg.name))?,
        );
        let mut tei = Self {
            child,
            stdin,
            stdout,
            id: cfg.name.clone(),
            size,
            nodes,
            multi_pv: 1,
            line: String::new(),
        };

        tei.send("tei")?;
        let mut options: BTreeMap<String, AnnouncedOption> = BTreeMap::new();
        loop {
            let line = tei.read_line()?;
            if line == "teiok" {
                break;
            }
            if let Some(name) = line.strip_prefix("id name ") {
                tei.id = name.trim().to_string();
            } else if let Some(rest) = line.strip_prefix("option name ") {
                let (name, option) = parse_option(rest);
                options.insert(name.to_ascii_lowercase(), option);
            }
        }
        let has = |name: &str| options.contains_key(&name.to_ascii_lowercase());

        // One thread: a multi-threaded search is not deterministic, and the sheet has to be.
        // A small table: it is cleared before every position, so its size is only a cost.
        let mut settings: Vec<(String, String)> = Vec::new();
        for (name, value) in [("Threads", "1"), ("Hash", "16"), ("MateScores", "true")] {
            if has(name) {
                settings.push((name.to_string(), value.to_string()));
            }
        }
        settings.extend(cfg.options.iter().cloned());

        // The game itself. Each of these is refused rather than skipped when the engine can't take
        // it: an engine that never heard the komi analyzes a different game.
        match options.get("halfkomi") {
            Some(o) => {
                let k = i64::from(half_komi);
                if o.min.map_or(false, |m| k < m) || o.max.map_or(false, |m| k > m) {
                    bail!(
                        "{} supports half-komi {} to {}, and this game is {}",
                        tei.id,
                        o.min.unwrap_or(k),
                        o.max.unwrap_or(k),
                        k
                    );
                }
                settings.push(("HalfKomi".to_string(), k.to_string()));
            }
            None if half_komi != 0 => bail!("{} has no HalfKomi option", tei.id),
            None => {}
        }
        if has("DoubleBlackStack") {
            settings.push(("DoubleBlackStack".to_string(), double_black_stack.to_string()));
        } else if double_black_stack {
            bail!("{} has no DoubleBlackStack option", tei.id);
        }
        if !has("MultiPV") {
            bail!("{} has no MultiPV option, which the grading needs", tei.id);
        }
        settings.push(("MultiPV".to_string(), "2".to_string()));
        tei.multi_pv = 2;

        for (name, value) in &settings {
            tei.send(&format!("setoption name {name} value {value}"))?;
        }
        tei.send("isready")?;
        tei.wait_for("readyok")?;
        Ok(tei)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    fn send(&mut self, command: &str) -> Result<()> {
        writeln!(self.stdin, "{command}")
            .and_then(|_| self.stdin.flush())
            .with_context(|| format!("{} stopped reading its input", self.id))
    }

    fn read_line(&mut self) -> Result<String> {
        self.line.clear();
        let n = self
            .stdout
            .read_line(&mut self.line)
            .with_context(|| format!("could not read from {}", self.id))?;
        if n == 0 {
            bail!("{} exited unexpectedly", self.id);
        }
        Ok(self.line.trim().to_string())
    }

    fn wait_for(&mut self, token: &str) -> Result<()> {
        while self.read_line()? != token {}
        Ok(())
    }
}

impl Drop for TeiSearch {
    fn drop(&mut self) {
        // Ask nicely; an engine that is mid-search or wedged gets killed instead of outliving us.
        let _ = writeln!(self.stdin, "quit").and_then(|_| self.stdin.flush());
        for _ in 0..50 {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `MultiPV type spin default 1 min 1 max 2048` → the name and its bounds. Names may contain spaces,
/// so the name runs up to the `type` keyword.
fn parse_option(rest: &str) -> (String, AnnouncedOption) {
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let type_at = tokens.iter().position(|&t| t == "type").unwrap_or(tokens.len());
    let name = tokens[..type_at].join(" ");
    let after = |key: &str| {
        tokens[type_at..]
            .iter()
            .position(|&t| t == key)
            .and_then(|i| tokens.get(type_at + i + 1))
            .and_then(|v| v.parse().ok())
    };
    (name, AnnouncedOption { min: after("min"), max: after("max") })
}

/// One `info` line with a score, as far as the grading needs it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InfoLine {
    pub multipv: usize,
    pub depth: usize,
    pub nodes: u64,
    /// In topaz's internal scale, side to move's frame.
    pub score: i32,
    /// False for a fail-high / fail-low bound, which is not a score.
    pub exact: bool,
    pub pv: Vec<String>,
}

/// Parse an `info` line; `None` for one without a depth and a score (`info string`, currmove…).
pub(crate) fn parse_info(line: &str) -> Option<InfoLine> {
    let mut tokens = line.split_whitespace();
    if tokens.next()? != "info" {
        return None;
    }
    let (mut multipv, mut depth, mut nodes) = (1usize, None, 0u64);
    let (mut cp, mut mate, mut wdl) = (None::<f64>, None::<i64>, None::<(f64, f64, f64)>);
    let mut exact = true;
    let mut pv = Vec::new();
    while let Some(t) = tokens.next() {
        match t {
            "multipv" => multipv = tokens.next()?.parse().ok()?,
            "depth" => depth = tokens.next()?.parse().ok(),
            "nodes" => nodes = tokens.next()?.parse().unwrap_or(0),
            "score" => {}
            "cp" => cp = tokens.next()?.parse().ok(),
            "mate" => mate = tokens.next()?.parse().ok(),
            // The spec's own spelling of a proof, `score solved win|loss|draw <plies>`.
            "solved" => {
                let outcome = tokens.next()?;
                let plies: i64 = tokens.next()?.parse().ok()?;
                let moves = (plies + 1) / 2;
                match outcome {
                    "win" => mate = Some(moves.max(1)),
                    "loss" => mate = Some(-moves.max(1)),
                    _ => cp = Some(0.0),
                }
            }
            "upperbound" | "lowerbound" => exact = false,
            "wdl" => {
                let w = tokens.next()?.parse().ok()?;
                let d = tokens.next()?.parse().ok()?;
                let l = tokens.next()?.parse().ok()?;
                wdl = Some((w, d, l));
            }
            "pv" => {
                pv = tokens.by_ref().map(str::to_string).collect();
            }
            // `string` runs to the end of the line and carries nothing to read.
            "string" => break,
            _ => {}
        }
    }
    let score = internal_score(cp, mate, wdl)?;
    Some(InfoLine { multipv, depth: depth?, nodes, score, exact, pv })
}

/// An engine's score on topaz's internal scale (see the module docs).
fn internal_score(cp: Option<f64>, mate: Option<i64>, wdl: Option<(f64, f64, f64)>) -> Option<i32> {
    if let Some(n) = mate {
        // `mate n` counts the mover's own moves: a win in n moves is 2n - 1 plies away, and a loss
        // in n is 2n.
        return Some(if n > 0 {
            WIN_SCORE - (2 * n as i32 - 1)
        } else {
            LOSE_SCORE + 2 * (-n) as i32
        });
    }
    if let Some((w, d, l)) = wdl {
        let total = w + d + l;
        if total > 0.0 {
            // Clamped half a permille inside each end: the engine rounds to whole permille, so 0
            // and 1000 mean "under half a permille", not certainty.
            let p = ((w + d / 2.0) / total).clamp(0.0005, 0.9995);
            return Some((400.0 * (p / (1.0 - p)).ln()).round() as i32);
        }
    }
    // Human cp otherwise — except that topaz's own TEI writes a proof as the raw score itself
    // (`cp 9998`, see `readable_eval`), which read as human cp would land four times past any win.
    cp.map(|cp| {
        if cp.abs() >= f64::from(WIN_SCORE - SOLVED_MARGIN) {
            (cp.round() as i32).clamp(LOSE_SCORE, WIN_SCORE)
        } else {
            ((cp * 4.0).round() as i32).clamp(LOSE_SCORE + SOLVED_MARGIN + 1, WIN_SCORE - SOLVED_MARGIN - 1)
        }
    })
}

/// A line of moves in the engine's spelling, parsed against `board` one move at a time and stopped
/// at the first one that is unreadable or illegal there, or that ends the game. Leaves the board as
/// it found it.
fn parse_line<B: TakBoard>(board: &mut B, moves: &[String]) -> Vec<GameMove> {
    let mut out = Vec::with_capacity(moves.len());
    let mut played: Vec<RevGameMove> = Vec::with_capacity(moves.len());
    for text in moves {
        let Some(mv) = parse_move(board, text) else { break };
        if !board.legal_move(mv) {
            break;
        }
        out.push(mv);
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

/// One move in the engine's spelling. The one place spellings differ that matters: under double
/// black stack, an engine may write White's first move as a bare square (Syntaks does), where topaz
/// reads a bare square as a single flat and wants `2a1`.
fn parse_move<B: TakBoard>(board: &B, text: &str) -> Option<GameMove> {
    let bare_square = text.len() == 2 && text.as_bytes()[0].is_ascii_lowercase();
    if bare_square
        && board.move_num() == 1
        && board.side_to_move() == Color::White
        && board.double_black_stack()
    {
        return GameMove::try_from_ptn(&format!("2{text}"), board);
    }
    GameMove::try_from_ptn(text, board)
}

/// Fold one search's `info` lines into the shape `analyze_game` grades from.
pub(crate) fn fold_search<B: TakBoard>(board: &mut B, infos: &[InfoLine]) -> (Searched, u64) {
    let mut ladder: BTreeMap<usize, i32> = BTreeMap::new();
    let mut latest: BTreeMap<usize, &InfoLine> = BTreeMap::new();
    let mut nodes = 0u64;
    for info in infos {
        nodes = nodes.max(info.nodes);
        if !info.exact {
            continue;
        }
        if info.multipv == 1 {
            ladder.insert(info.depth, info.score);
        }
        latest.insert(info.multipv, info);
    }

    let mut roots = Vec::with_capacity(latest.len());
    let mut lines = Vec::with_capacity(latest.len());
    for info in latest.values() {
        let line = parse_line(board, &info.pv);
        let Some(&first) = line.first() else { continue };
        roots.push((first, info.score));
        lines.push(line);
    }
    let pv = lines.first().cloned().unwrap_or_default();
    let score = ladder
        .values()
        .next_back()
        .copied()
        .or_else(|| roots.first().map(|&(_, s)| s))
        .unwrap_or(0);
    (
        Searched {
            score,
            roots,
            lines,
            depths: ladder.into_iter().collect(),
            pv,
        },
        nodes,
    )
}

impl<B: TakBoard> PositionSearch<B> for TeiSearch {
    fn search(&mut self, board: &mut B, breadth: Breadth) -> Result<(Searched, u64)> {
        debug_assert_eq!(B::SIZE, self.size);
        let multi_pv = match breadth {
            Breadth::Grading => 2,
            Breadth::Alternatives(n) => n,
        };
        if multi_pv != self.multi_pv {
            self.send(&format!("setoption name MultiPV value {multi_pv}"))?;
            self.multi_pv = multi_pv;
        }
        self.send(&format!("teinewgame {}", self.size))?;
        // topaz's `Debug` for a board is its TPS: `x,x,…/…  <side> <move>`.
        self.send(&format!("position tps {:?}", board))?;
        self.send("isready")?;
        self.wait_for("readyok")?;
        self.send(&format!("go nodes {}", self.nodes))?;

        let mut infos = Vec::new();
        loop {
            let line = self.read_line()?;
            if line.starts_with("bestmove") {
                break;
            }
            if let Some(info) = parse_info(&line) {
                infos.push(info);
            }
        }
        Ok(fold_search(board, &infos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board6;
    use crate::Position;

    #[test]
    fn info_lines_parse() {
        let line = "info multipv 2 depth 11 seldepth 14 time 298 nodes 200000 nps 668954 score cp 37 \
                    wdl 546 61 393 hashfull 88 pv c3 d4 Cc4";
        let info = parse_info(line).unwrap();
        assert_eq!(info.multipv, 2);
        assert_eq!(info.depth, 11);
        assert_eq!(info.nodes, 200_000);
        assert!(info.exact);
        assert_eq!(info.pv, vec!["c3", "d4", "Cc4"]);
        // Expected score 0.5765 → 400 · logit ≈ 124.
        assert!((info.score - 124).abs() <= 1, "{}", info.score);
        assert!(parse_info("info string hello").is_none());
        assert!(!parse_info("info depth 3 score cp 10 upperbound pv a1").unwrap().exact);
    }

    #[test]
    fn mate_scores_are_proofs() {
        let win = parse_info("info depth 5 score mate 2 wdl 1000 0 0 pv a1").unwrap();
        assert_eq!(win.score, WIN_SCORE - 3);
        let loss = parse_info("info depth 5 score mate -3 wdl 0 0 1000 pv a1").unwrap();
        assert_eq!(loss.score, LOSE_SCORE + 6);
    }

    #[test]
    fn topaz_style_proofs_in_cp_are_read_as_proofs() {
        // topaz's TEI writes a proof as the raw score (`readable_eval`), everything else / 4.
        assert_eq!(parse_info("info depth 1 score cp 9998 pv a1").unwrap().score, 9998);
        assert_eq!(parse_info("info depth 1 score cp -9997 pv a1").unwrap().score, -9997);
        assert_eq!(parse_info("info depth 1 score cp 25 pv a1").unwrap().score, 100);
        // A huge evaluation that is not a proof stays short of the proven band.
        let big = parse_info("info depth 1 score cp 3000 pv a1").unwrap().score;
        assert!(big < WIN_SCORE - SOLVED_MARGIN, "{big}");
    }

    #[test]
    fn wdl_extremes_stay_below_proofs() {
        let p = parse_info("info depth 5 score cp 900 wdl 1000 0 0 pv a1").unwrap();
        assert!(p.score > 2000 && p.score < WIN_SCORE - 200, "{}", p.score);
    }

    #[test]
    fn a_bare_first_move_is_the_double_stack_under_dbs() {
        let mut board = Board6::start_position().with_komi(4).with_double_black_stack(true);
        let line = parse_line(&mut board, &["a1".to_string(), "f6".to_string()]);
        assert_eq!(line.len(), 2);
        assert_eq!(line[0].number(), 2);
        // ...and the same text is a single flat under the swap.
        let mut swap = Board6::start_position().with_komi(0).with_double_black_stack(false);
        let line = parse_line(&mut swap, &["a1".to_string()]);
        assert_eq!(line.len(), 1);
        assert_ne!(line[0].number(), 2);
    }

    #[test]
    fn tps_round_trips_through_debug() {
        let mut board = Board6::start_position().with_double_black_stack(true);
        for m in ["2a1", "f6", "c3", "d4", "c4", "d3", "c4-"] {
            let mv = GameMove::try_from_ptn(m, &board).unwrap();
            assert!(board.legal_move(mv), "{m}");
            board.do_move(mv);
        }
        let tps = format!("{:?}", board);
        let back = Board6::try_from_tps(&tps).unwrap();
        assert_eq!(format!("{:?}", back), tps);
    }

    #[test]
    fn the_ladder_keeps_exact_multipv1_lines_and_the_latest_root_per_index() {
        let mut board = Board6::start_position().with_komi(4).with_double_black_stack(false);
        let infos: Vec<InfoLine> = [
            "info multipv 1 depth 1 nodes 10 score cp 10 pv a1",
            "info multipv 2 depth 1 nodes 10 score cp 5 pv b1",
            "info multipv 1 depth 2 nodes 40 score cp 20 lowerbound pv a1",
            "info multipv 1 depth 2 nodes 50 score cp 15 pv c3",
            "info multipv 2 depth 2 nodes 50 score cp 12 pv d4",
            "info multipv 1 depth 2 nodes 60 score cp 15 pv c3",
        ]
        .iter()
        .map(|l| parse_info(l).unwrap())
        .collect();
        let (s, nodes) = fold_search(&mut board, &infos);
        assert_eq!(nodes, 60);
        assert_eq!(s.depths, vec![(1, 40), (2, 60)]);
        assert_eq!(s.score, 60);
        assert_eq!(s.roots.len(), 2);
        assert_eq!(s.roots[0].0, GameMove::try_from_ptn("c3", &board).unwrap());
        assert_eq!(s.roots[1].1, 48);
    }
}
