#![allow(clippy::style)]

use anyhow::{anyhow, ensure, Result};
pub use board::{Bitboard, BitboardStorage, Piece, Stack, TakBoard};
pub use move_gen::{generate_all_moves, GameMove, RevGameMove};

#[cfg(all(feature = "evaluation", feature = "random"))]
pub mod balance;
#[cfg(all(feature = "evaluation", feature = "random"))]
pub mod balance4;
#[cfg(all(feature = "evaluation", feature = "random"))]
pub mod balance5;
#[cfg(feature = "evaluation")]
pub mod analyze_game;
#[cfg(feature = "evaluation")]
pub mod balanced_lines;
pub mod board;
#[cfg(feature = "evaluation")]
pub mod start_eval;
#[cfg(all(feature = "evaluation", feature = "random"))]
pub mod datagen;
#[cfg(all(feature = "evaluation", feature = "random"))]
pub mod datagen4;
#[cfg(all(feature = "evaluation", feature = "random"))]
pub mod datagen5;
#[cfg(feature = "evaluation")]
pub mod eval;
pub mod move_gen;
#[cfg(all(feature = "evaluation", feature = "random"))]
pub mod openings;
#[cfg(all(feature = "evaluation", feature = "random"))]
pub mod openings4;
#[cfg(all(feature = "evaluation", feature = "random"))]
pub mod openings5;
pub mod proof;
#[cfg(feature = "evaluation")]
pub mod search;
#[cfg(feature = "evaluation")]
pub mod transposition_table;

use crate::board::{Board4, Board5, Board6, Board7};
#[cfg(feature = "evaluation")]
use crate::search::SearchHyper;

#[cfg(feature = "evaluation")]
#[derive(Clone)]
pub struct GameInitializer {
    pub hash_size: usize,
    pub max_depth: usize,
    pub komi: u8,
    pub add_noise: bool,
    pub num_threads: usize,
    pub max_nodes: i64,
    pub multi_pv: usize,
    pub hyper: SearchHyper,
    /// Iterative-deepening depth below which time/node aborts are NOT honored (the search always
    /// completes at least this depth). The default 6 imposes a ~depth-6 floor (~4k nodes on 6x6),
    /// so low `max_nodes` caps don't bite. Lower it (e.g. 1) to let small node caps take effect.
    pub early_abort_depth: usize,
    /// Engine-weakening temperature for softmax move-selection over the top `multi_pv` root moves
    /// (0 = off => always play the best move). Higher => more random among the top moves. Used to
    /// space the bot ladder below the node-cap floor; see `search::select_softmax`.
    pub temperature: f64,
    /// Engine-weakening eval-gap tolerance in centipawns (0 = off). When > 0, prune every root move
    /// more than this far below the best, then pick uniformly among the survivors. A harder-edged
    /// alternative to `temperature`: it bounds the worst-case blunder and never plays a clearly-worse
    /// move even from a wide pool. Takes precedence over `temperature` when both are set. See
    /// `search::select_gap`.
    pub eval_gap: i32,
    /// Dynamic eval-gap (0 = off). When `eval_gap_std > 0`, the effective gap flexes with the bot's
    /// eval between these three anchors (min at <= -50cp / losing, std at 0cp / even, max at >= +50cp
    /// / winning; see `search::dynamic_gap`). Takes precedence over the static `eval_gap`.
    pub eval_gap_min: i32,
    pub eval_gap_std: i32,
    pub eval_gap_max: i32,
    /// Linear rank bias applied when picking among the eval-gap survivors (0 = uniform; higher tilts
    /// toward the WORSE eligible moves, worst:best odds = 1 + bias_slope; see `search::select_gap`).
    pub bias_slope: f64,
    /// Absolute "always-allow" floor in centipawns: any root move scoring at least this stays eligible
    /// even if it falls outside the eval-gap band, so a winning-but-not-best move can still be played.
    /// Only ever widens the pool (effective floor = `min(best - gap, always_allow)`) and never
    /// overrides the decisive-best or near-loss guards. `i32::MAX` = disabled. See `search::select_gap`.
    pub always_allow: i32,
}

#[cfg(feature = "evaluation")]
impl GameInitializer {
    pub fn new(hash_size: usize, max_depth: usize, komi: u8, add_noise: bool) -> Self {
        Self {
            hash_size,
            max_depth,
            komi,
            add_noise,
            max_nodes: -1,
            num_threads: 1,
            multi_pv: 1,
            hyper: SearchHyper::default(),
            early_abort_depth: 6,
            temperature: 0.0,
            eval_gap: 0,
            eval_gap_min: 0,
            eval_gap_std: 0,
            eval_gap_max: 0,
            bias_slope: 0.0,
            always_allow: i32::MAX,
        }
    }
    pub fn get_board<E: eval::Evaluator + Default>(&self) -> (E::Game, E) {
        (E::Game::start_position().with_komi(self.komi), E::default())
    }
    pub fn get_board_tps<E: eval::Evaluator + Default>(&self, tps: &str) -> (E::Game, E) {
        (
            E::Game::try_from_tps(tps).unwrap().with_komi(self.komi),
            E::default(),
        )
    }
    // pub fn various_search(&mut self, g: TakGame) {
    //     match g {
    //         TakGame::Standard5(board) => {
    //             if self.num_threads == 1 {
    //                 if self.use_nn {
    //                     unimplemented!()
    //                 } else {
    //                     todo!()
    //                 }
    //             } else {
    //                 todo!()
    //             }
    //         }
    //         TakGame::Standard6(board) => todo!(),
    //         TakGame::Standard7(board) => todo!(),
    //     }
    // }
}

#[cfg(feature = "evaluation")]
impl std::default::Default for GameInitializer {
    fn default() -> Self {
        Self {
            hash_size: 2 << 22,
            max_depth: 128,
            komi: 0,
            num_threads: 1,
            add_noise: false,
            max_nodes: -1,
            multi_pv: 1,
            hyper: SearchHyper::default(),
            early_abort_depth: 6,
            temperature: 0.0,
            eval_gap: 0,
            eval_gap_min: 0,
            eval_gap_std: 0,
            eval_gap_max: 0,
            bias_slope: 0.0,
            always_allow: i32::MAX,
        }
    }
}

/// Represents a player's color.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Color {
    White = 0,
    Black = 1,
}

impl std::ops::Not for Color {
    type Output = Color;

    #[inline]
    fn not(self) -> Self {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        fmt.write_str(match *self {
            Color::White => "White",
            Color::Black => "Black",
        })
    }
}

/// The result of a game after it has finished.
#[derive(PartialEq, Eq, Clone, Debug, Copy)]
pub enum GameResult {
    WhiteWin = 0,
    BlackWin = 1,
    Draw = 2,
}

impl GameResult {
    /// Returns WhiteWin for white, BlackWin for black
    #[inline]
    pub fn win_by(color: Color) -> Self {
        match color {
            Color::White => Self::WhiteWin,
            Color::Black => Self::BlackWin,
        }
    }
}

impl std::ops::Not for GameResult {
    type Output = Self;
    #[inline]
    fn not(self) -> Self {
        match self {
            GameResult::WhiteWin => GameResult::BlackWin,
            GameResult::BlackWin => GameResult::WhiteWin,
            GameResult::Draw => GameResult::Draw,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TimeBank {
    pub goal_time: u64,
    pub max_time: u64,
}

impl TimeBank {
    pub fn init(total_res: u64, total_time: u64, _increment: u64) -> Self {
        let use_time = if total_res > 40 {
            total_time / 15
        } else if total_res > 20 {
            total_time / 10
        } else {
            total_time / 6
        };
        Self {
            goal_time: use_time,
            max_time: use_time,
        }
    }
    pub fn flat(use_time: u64) -> Self {
        Self {
            goal_time: use_time,
            max_time: use_time,
        }
    }
}

pub trait Position {
    type Move: Eq + Clone + std::fmt::Debug;
    type ReverseMove;
    fn start_position() -> Self
    where
        Self: Sized;
    fn side_to_move(&self) -> Color;
    fn generate_moves(&self, moves: &mut Vec<Self::Move>);
    fn do_move(&mut self, mv: Self::Move) -> Self::ReverseMove;
    fn reverse_move(&mut self, mv: Self::ReverseMove);
    fn game_result(&self) -> Option<GameResult>;
}

#[non_exhaustive]
pub enum TakGame {
    Standard4(Board4),
    Standard5(Board5),
    Standard6(Board6),
    Standard7(Board7),
}

impl TakGame {
    pub fn try_from_tps(tps: &str) -> Result<Self> {
        let size = tps.chars().filter(|&c| c == '/').count() + 1;
        match size {
            4 => Ok(TakGame::Standard4(Board4::try_from_tps(tps)?)),
            5 => Ok(TakGame::Standard5(Board5::try_from_tps(tps)?)),
            6 => Ok(TakGame::Standard6(Board6::try_from_tps(tps)?)),
            7 => Ok(TakGame::Standard7(Board7::try_from_tps(tps)?)),
            _ => Err(anyhow!("Unknown game size: {}", size)),
        }
    }
    pub fn try_new(size: usize) -> Result<Self> {
        match size {
            4 => Ok(TakGame::Standard4(Board4::new())),
            5 => Ok(TakGame::Standard5(Board5::new())),
            6 => Ok(TakGame::Standard6(Board6::new())),
            7 => Ok(TakGame::Standard7(Board7::new())),
            _ => Err(anyhow!("Unknown game size: {}", size)),
        }
    }
    pub fn try_new_with_komi(size: usize, half_flats: u8) -> Result<Self> {
        match size {
            4 => Ok(TakGame::Standard4(Board4::new().with_komi(half_flats))),
            5 => Ok(TakGame::Standard5(Board5::new().with_komi(half_flats))),
            6 => Ok(TakGame::Standard6(Board6::new().with_komi(half_flats))),
            7 => Ok(TakGame::Standard7(Board7::new().with_komi(half_flats))),
            _ => Err(anyhow!("Unknown game size: {}", size)),
        }
    }
}

impl Position for TakGame {
    type Move = GameMove;

    type ReverseMove = RevGameMove;

    fn start_position() -> Self
    where
        Self: Sized,
    {
        TakGame::Standard6(Board6::new())
    }

    fn side_to_move(&self) -> Color {
        match self {
            TakGame::Standard4(board) => board.side_to_move(),
            TakGame::Standard5(board) => board.side_to_move(),
            TakGame::Standard6(board) => board.side_to_move(),
            TakGame::Standard7(board) => board.side_to_move(),
        }
    }

    fn generate_moves(&self, moves: &mut Vec<Self::Move>) {
        match self {
            TakGame::Standard4(board) => board.generate_moves(moves),
            TakGame::Standard5(board) => board.generate_moves(moves),
            TakGame::Standard6(board) => board.generate_moves(moves),
            TakGame::Standard7(board) => board.generate_moves(moves),
        }
    }

    fn do_move(&mut self, mv: Self::Move) -> Self::ReverseMove {
        match self {
            TakGame::Standard4(board) => board.do_move(mv),
            TakGame::Standard5(board) => board.do_move(mv),
            TakGame::Standard6(board) => board.do_move(mv),
            TakGame::Standard7(board) => board.do_move(mv),
        }
    }

    fn reverse_move(&mut self, mv: Self::ReverseMove) {
        match self {
            TakGame::Standard4(board) => board.reverse_move(mv),
            TakGame::Standard5(board) => board.reverse_move(mv),
            TakGame::Standard6(board) => board.reverse_move(mv),
            TakGame::Standard7(board) => board.reverse_move(mv),
        }
    }

    fn game_result(&self) -> Option<GameResult> {
        match self {
            TakGame::Standard4(board) => board.game_result(),
            TakGame::Standard5(board) => board.game_result(),
            TakGame::Standard6(board) => board.game_result(),
            TakGame::Standard7(board) => board.game_result(),
        }
    }
}

#[derive(Default, Debug)]
pub struct TeiGo {
    pub wtime: Option<u64>,
    pub btime: Option<u64>,
    pub winc: Option<u64>,
    pub binc: Option<u64>,
    pub depth: Option<usize>,
    pub nodes: Option<u64>,
    pub movetime: Option<u64>,
    pub infinite: bool,
}

impl TeiGo {
    pub fn from_go_str(s: &str) -> Self {
        let mut out = Self::default();
        let mut iter = s.split_whitespace();
        while let Some(keyword) = iter.next() {
            match keyword {
                "wtime" => out.wtime = iter.next().and_then(|x| x.parse::<u64>().ok()),
                "btime" => out.btime = iter.next().and_then(|x| x.parse::<u64>().ok()),
                "winc" => out.winc = iter.next().and_then(|x| x.parse::<u64>().ok()),
                "binc" => out.binc = iter.next().and_then(|x| x.parse::<u64>().ok()),
                "depth" => out.depth = iter.next().and_then(|x| x.parse::<usize>().ok()),
                "nodes" => out.nodes = iter.next().and_then(|x| x.parse::<u64>().ok()),
                "movetime" => out.movetime = iter.next().and_then(|x| x.parse::<u64>().ok()),
                "infinite" => out.infinite = true,
                _ => {}
            }
        }
        out
    }
}

#[derive(Debug)]
pub enum TeiCommand {
    Stop,
    Quit,
    Go(TeiGo),
    Position(String),
    NewGame(usize),
    /// New half-komi for the running game worker. Sent by the TEI loop when `setoption HalfKomi`
    /// is processed, so a worker (spawned once per connection) picks up a komi change when the GUI
    /// switches games without reconnecting. Carries the half-komi value.
    SetKomi(u8),
}

pub fn execute_moves_check_valid(board: &mut Board6, ptn_slice: &[&str]) -> Result<Vec<GameMove>> {
    let mut moves: Vec<GameMove> = Vec::new();
    let mut made_moves = Vec::new();
    for m_str in ptn_slice {
        moves.clear();
        let m =
            GameMove::try_from_ptn(m_str, board).ok_or_else(|| anyhow!("Invalid ptn string"))?;
        generate_all_moves(board, &mut moves);
        ensure!(moves.iter().any(|&x| x == m), "Illegal move attempted");
        board.do_move(m);
        made_moves.push(m);
    }
    Ok(made_moves)
}

pub fn perft<P: Position>(board: &mut P, depth: u16) -> u64 {
    if depth == 0 {
        1
    } else {
        let mut moves = vec![];
        board.generate_moves(&mut moves);
        moves
            .into_iter()
            .map(|mv| {
                let reverse_move = board.do_move(mv);
                let num_moves = perft(board, depth - 1);
                board.reverse_move(reverse_move);
                num_moves
            })
            .sum()
    }
}

fn pop_lowest(x: &mut u64) -> u32 {
    let highest_index = x.trailing_zeros();
    if highest_index == 64 {
        0
    } else {
        let value = 1 << highest_index;
        *x ^= value;
        highest_index
    }
}
