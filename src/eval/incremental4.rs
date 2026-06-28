//! Size-4 (4x4) NNUE mapper. This is a self-contained duplicate of the 5x5
//! `incremental5.rs`, re-parameterised for a 16-square board with 15 reserve
//! pieces per side (15 flats + 0 capstones). It intentionally shares **nothing**
//! with the 5x5/6x6 modules so that 4x4 work cannot perturb the proven nets.
//!
//! **0-cap note:** a 4x4 board has no capstones, so the cap-encoding paths
//! (`caps[]`, `promote_cap`, the `WHITE_CAP`/`BLACK_CAP` location slot) are never
//! activated. They are kept present-but-inert so the module stays a mechanical,
//! line-for-line port of the 5x5 one (lowest engine<->trainer desync risk); the
//! corresponding net columns are simply dead weights. The smaller board already
//! makes the net much lighter than the 5x5 one (NUM_INPUTS 504 vs 738).
//!
//! Feature layout (must stay byte-identical with the 4x4 trainer's feature map):
//!   - `SQUARE_INPUTS = 16 * (6 + 2*STACK_DEPTH) = 416`; perspective split at 208.
//!   - side-to-move:  416, 417
//!   - reserve band A: `SQUARE_INPUTS + 8 + reserves`  (424..=439, reserves 0..=15)
//!   - reserve band B: `SQUARE_INPUTS + 44 + reserves` (460..=475)
//!   - `NUM_INPUTS = 504` (`SQUARE_INPUTS + 88`, mirroring the 5x5 padding).
//! The offsets deliberately mirror the 5x5 formula so only `SQUARES`/reserve
//! counts differ; this keeps the trainer port mechanical.

use crate::{pop_lowest, Piece};

pub const STACK_DEPTH: usize = 10;
pub const HIDDEN_SIZE: usize = 512;
pub const SCALE: i32 = 400;
pub const QA: i16 = 255;
pub const QB: i16 = 64;
pub const WHITE_FLAT: ValidPiece = ValidPiece(0);
pub const BLACK_FLAT: ValidPiece = ValidPiece(1);
pub const WHITE_WALL: ValidPiece = ValidPiece(2);
pub const BLACK_WALL: ValidPiece = ValidPiece(3);
pub const WHITE_CAP: ValidPiece = ValidPiece(4);
pub const BLACK_CAP: ValidPiece = ValidPiece(5);
const _ASS: () = assert!(
    WHITE_FLAT.flip_color().0 == BLACK_FLAT.0
        && BLACK_WALL.flip_color().0 == WHITE_WALL.0
        && BLACK_CAP.flip_color().0 == WHITE_CAP.0
);

/// Number of board squares (4x4).
const SQUARES: usize = 16;
/// Reserve pieces per side that the iterator counts down from (15 flats + 0 cap).
const RESERVE_INIT: usize = 15;
/// Max distinct (square, depth) entries: every piece of both sides = 2 * 15.
const DATA_LEN: usize = 30;

pub static NNUE4: Network = unsafe {
    let bytes = include_bytes!("../quantised4.bin");
    assert!(bytes.len() == std::mem::size_of::<Network>());
    std::mem::transmute(*bytes)
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidPiece(pub u8);

impl ValidPiece {
    pub const fn without_color(self) -> u8 {
        self.0 >> 1
    }
    const fn flip_color(self) -> Self {
        Self(self.0 ^ 1) // Toggle bit 0
    }
    pub const fn promote_cap(self) -> Self {
        Self(self.0 | 4) // Set bit 2
    }
    pub const fn is_white(self) -> bool {
        (self.0 & 1) == 0
    }
    pub const fn color_index(self) -> usize {
        (self.0 & 1) as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PieceSquare(pub u8);

impl PieceSquare {
    pub fn new(square: usize, piece: u8) -> Self {
        Self((square as u8) | piece << 6)
    }
    pub fn square(self) -> u8 {
        self.0 & 63
    }
    pub fn piece(self) -> ValidPiece {
        let masked = 0b1100_0000 & self.0;
        ValidPiece(masked >> 6)
    }
    pub fn promote_wall(&mut self) {
        self.0 |= 128;
    }
    pub fn topaz_piece(self) -> Piece {
        match self.piece() {
            WHITE_FLAT => Piece::WhiteFlat,
            BLACK_FLAT => Piece::BlackFlat,
            WHITE_WALL => Piece::WhiteWall,
            BLACK_WALL => Piece::BlackWall,
            WHITE_CAP => Piece::WhiteCap,
            BLACK_CAP => Piece::BlackCap,
            _ => unimplemented!(),
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BoardData {
    pub caps: [u8; 2],
    pub data: [PieceSquare; DATA_LEN], // Each stack must be presented from top to bottom sequentially
    pub data_len: u8,
    pub white_to_move: bool,
}

impl BoardData {
    const SIZE: u8 = 4;
    const SYM_TABLE: [[u8; SQUARES]; 8] = Self::build_symmetry_table();
    pub fn new(
        caps: [u8; 2],
        data: [PieceSquare; DATA_LEN],
        data_len: u8,
        white_to_move: bool,
    ) -> Self {
        Self {
            caps,
            data,
            data_len,
            white_to_move,
        }
    }
    pub fn symmetry(mut self, idx: usize) -> Self {
        if idx == 0 {
            return self;
        }
        assert!(idx < 8);
        let table = &Self::SYM_TABLE[idx];
        if (self.caps[0] as usize) < SQUARES {
            self.caps[0] = table[self.caps[0] as usize];
        }
        if (self.caps[1] as usize) < SQUARES {
            self.caps[1] = table[self.caps[1] as usize];
        }
        for i in 0..(self.data_len as usize) {
            let old = self.data[i];
            self.data[i] = PieceSquare::new(table[old.square() as usize] as usize, old.piece().0);
        }
        self
    }
    const fn build_symmetry_table() -> [[u8; SQUARES]; 8] {
        [
            Self::transform(0),
            Self::transform(1),
            Self::transform(2),
            Self::transform(3),
            Self::transform(4),
            Self::transform(5),
            Self::transform(6),
            Self::transform(7),
        ]
    }
    const fn transform(rotation: usize) -> [u8; SQUARES] {
        let mut data = [(0, 0); SQUARES];
        let mut i = 0;
        while i < SQUARES {
            let (row, col) = Self::row_col(i as u8);
            data[i] = (row, col);
            i += 1;
        }
        match rotation {
            1 => Self::flip_ns(&mut data),
            2 => Self::flip_ew(&mut data),
            3 => Self::rotate(&mut data),
            4 => {
                Self::rotate(&mut data);
                Self::rotate(&mut data);
            }
            5 => {
                Self::rotate(&mut data);
                Self::rotate(&mut data);
                Self::rotate(&mut data);
            }
            6 => {
                Self::rotate(&mut data);
                Self::flip_ns(&mut data);
            }
            7 => {
                Self::rotate(&mut data);
                Self::flip_ew(&mut data);
            }
            _ => {}
        };
        let mut out = [0; SQUARES];
        let mut i = 0;
        while i < SQUARES {
            let (row, col) = data[i];
            out[i] = Self::index(row, col);
            i += 1;
        }
        out
    }
    const fn flip_ns(arr: &mut [(u8, u8); SQUARES]) {
        let mut i = 0;
        while i < SQUARES {
            let (row, _col) = &mut arr[i];
            *row = Self::SIZE - 1 - *row;
            i += 1;
        }
    }
    const fn flip_ew(arr: &mut [(u8, u8); SQUARES]) {
        let mut i = 0;
        while i < SQUARES {
            let (_row, col) = &mut arr[i];
            *col = Self::SIZE - 1 - *col;
            i += 1;
        }
    }
    const fn rotate(arr: &mut [(u8, u8); SQUARES]) {
        let mut i = 0;
        while i < SQUARES {
            let (row, col) = &mut arr[i];
            let new_row = Self::SIZE - 1 - *col;
            *col = *row;
            *row = new_row;
            i += 1;
        }
    }
    const fn row_col(index: u8) -> (u8, u8) {
        (index / Self::SIZE, index % Self::SIZE)
    }
    const fn index(row: u8, col: u8) -> u8 {
        row * Self::SIZE + col
    }
}

#[derive(Clone, Copy)]
pub struct TakSimple4 {}

impl TakSimple4 {
    pub const SQUARE_INPUTS: usize = SQUARES * (6 + 2 * STACK_DEPTH); // 416
    // Squares + Side + Reserves
    pub const NUM_INPUTS: usize = TakSimple4::SQUARE_INPUTS + 8 + 80; // Pad to 504
    /// Perspective split: own pieces in [0, SPLIT), opponent in [SPLIT, SQUARE_INPUTS).
    const SPLIT: usize = TakSimple4::SQUARE_INPUTS / 2; // 208
    /// Base of the second reserve one-hot band.
    const RES_B_BASE: usize = TakSimple4::SQUARE_INPUTS + 44; // 460

    pub fn handle_features<F: FnMut(usize, usize)>(&self, pos: &BoardData, mut f: F) {
        let mut reserves: [usize; 2] = [RESERVE_INIT, RESERVE_INIT];
        for (piece, square, depth_idx) in pos.into_iter() {
            let c = (piece.is_white() ^ pos.white_to_move) as usize; // 0 if matches, else 1
            reserves[c] -= 1;
            let location = usize::from(piece.without_color() + depth_idx);
            let sq = usize::from(square);

            let stm = [0, Self::SPLIT][c] + SQUARES * location + sq;
            let ntm = [Self::SPLIT, 0][c] + SQUARES * location + sq;
            f(stm, ntm);
        }
        if pos.white_to_move {
            // White to move
            f(
                Self::SQUARE_INPUTS + 8 + reserves[0],
                Self::SQUARE_INPUTS + 8 + reserves[1],
            );
            f(Self::RES_B_BASE + reserves[1], Self::RES_B_BASE + reserves[0]);
            f(Self::SQUARE_INPUTS, Self::SQUARE_INPUTS + 1);
        } else {
            // Black to move
            f(
                Self::SQUARE_INPUTS + 8 + reserves[1],
                Self::SQUARE_INPUTS + 8 + reserves[0],
            );
            f(Self::RES_B_BASE + reserves[0], Self::RES_B_BASE + reserves[1]);
            f(Self::SQUARE_INPUTS + 1, Self::SQUARE_INPUTS);
        }
    }
}

impl IntoIterator for BoardData {
    type Item = (ValidPiece, u8, u8);
    type IntoIter = TakBoardIter;
    fn into_iter(self) -> Self::IntoIter {
        TakBoardIter {
            board: self,
            idx: 0,
            last: u8::MAX,
            depth: 0,
        }
    }
}

pub struct TakBoardIter {
    board: BoardData,
    idx: usize,
    last: u8,
    depth: u8,
}

impl Iterator for TakBoardIter {
    type Item = (ValidPiece, u8, u8); // PieceType, Square, Depth
    fn next(&mut self) -> Option<Self::Item> {
        const DEPTH_TABLE: [u8; 10] = [0, 3, 4, 5, 6, 7, 8, 9, 10, 11];
        if self.idx > self.board.data.len() {
            return None;
        }
        let val = self.board.data[self.idx];
        let square = val.square();
        if (square as usize) >= SQUARES {
            return None;
        }
        let mut piece = val.piece();
        if square == self.last {
            self.depth += 1;
        } else {
            self.depth = 0;
            if self.board.caps[0] == square || self.board.caps[1] == square {
                piece = piece.promote_cap();
            }
        }
        self.idx += 1;
        self.last = square;
        Some((piece, square, DEPTH_TABLE[self.depth as usize]))
    }
}

/// A column of the feature-weights matrix.
/// Note the `align(64)`.
#[derive(Clone, Copy)]
#[repr(C, align(64))]
pub struct Accumulator {
    vals: [i16; HIDDEN_SIZE],
}

impl Accumulator {
    /// Initialised with bias so we can just efficiently
    /// operate on it afterwards.
    pub fn new(net: &Network) -> Self {
        net.feature_bias
    }

    pub fn from_old(old: &Self) -> Self {
        old.clone()
    }

    pub fn add_all(&mut self, features: &[u16], net: &Network) {
        for f in features {
            self.add_feature(*f as usize, net);
        }
    }

    pub fn remove_all(&mut self, features: &[u16], net: &Network) {
        for f in features {
            self.remove_feature(*f as usize, net);
        }
    }

    /// Add a feature to an accumulator.
    pub fn add_feature(&mut self, feature_idx: usize, net: &Network) {
        for (i, d) in self
            .vals
            .iter_mut()
            .zip(&net.feature_weights[feature_idx].vals)
        {
            *i += *d
        }
    }

    /// Remove a feature from an accumulator.
    pub fn remove_feature(&mut self, feature_idx: usize, net: &Network) {
        for (i, d) in self
            .vals
            .iter_mut()
            .zip(&net.feature_weights[feature_idx].vals)
        {
            *i -= *d
        }
    }
}

pub struct NNUE4 {
    white: (Incremental, Incremental),
    black: (Incremental, Incremental),
    pub(crate) tempo_offset: i32,
}

impl NNUE4 {
    pub fn incremental_eval(&mut self, takboard: BoardData) -> i32 {
        let (ours, theirs) = build_features(takboard);
        let (old_ours, old_theirs) = if takboard.white_to_move {
            (&self.white.0, &self.white.1)
        } else {
            (&self.black.0, &self.black.1)
        };
        // Ours
        let mut ours_acc = Accumulator::from_old(&old_ours.vec);
        ours.compute_diff(&old_ours.state, &mut ours_acc);
        let ours = Incremental {
            state: ours,
            vec: ours_acc,
        };
        // Theirs
        let mut theirs_acc = Accumulator::from_old(&old_theirs.vec);
        theirs.compute_diff(&old_theirs.state, &mut theirs_acc);
        let theirs = Incremental {
            state: theirs,
            vec: theirs_acc,
        };
        // Output
        let eval = NNUE4.evaluate(&ours.vec, &theirs.vec, ours.state.clone().into_iter());
        if takboard.white_to_move {
            self.white = (ours, theirs);
        } else {
            self.black = (ours, theirs);
        }
        eval
    }
    #[cfg(test)]
    pub(crate) fn manual_eval(takboard: BoardData) -> i32 {
        let (ours, theirs) = build_features(takboard);
        let ours = Incremental::fresh_new(&NNUE4, ours);
        let theirs = Incremental::fresh_new(&NNUE4, theirs);
        let eval = NNUE4.evaluate(&ours.vec, &theirs.vec, ours.state.clone().into_iter());
        eval
    }
}

impl Default for NNUE4 {
    fn default() -> Self {
        Self {
            white: (
                Incremental::fresh_empty(&NNUE4),
                Incremental::fresh_empty(&NNUE4),
            ),
            black: (
                Incremental::fresh_empty(&NNUE4),
                Incremental::fresh_empty(&NNUE4),
            ),
            tempo_offset: 100,
        }
    }
}

fn build_features(takboard: BoardData) -> (IncrementalState, IncrementalState) {
    let mut ours = IncrementalState::empty();
    let mut theirs = IncrementalState::empty();
    let simple = TakSimple4 {};
    simple.handle_features(&takboard, |x, y| {
        ours.add_feature(x as u16);
        theirs.add_feature(y as u16);
    });
    (ours, theirs)
}

#[inline]
pub fn screlu(x: i16) -> i32 {
    i32::from(x.clamp(0, QA as i16)).pow(2)
}

/// This is the quantised format that bullet outputs.
#[repr(C)]
pub struct Network {
    /// Column-Major `HIDDEN_SIZE x NUM_INPUTS` matrix.
    feature_weights: [Accumulator; TakSimple4::NUM_INPUTS],
    /// Vector with dimension `HIDDEN_SIZE`.
    feature_bias: Accumulator,
    /// Column-Major `1 x (2 * HIDDEN_SIZE)`
    /// matrix, we use it like this to make the
    /// code nicer in `Network::evaluate`.
    output_weights: [i16; 2 * HIDDEN_SIZE],
    /// Piece-Square Table for Input
    pqst: [i16; TakSimple4::NUM_INPUTS],
    /// Scalar output bias.
    output_bias: i16,
}

impl Network {
    /// Calculates the output of the network, starting from the already
    /// calculated hidden layer (done efficiently during makemoves).
    fn evaluate(&self, us: &Accumulator, them: &Accumulator, original: BitSetIterator) -> i32 {
        // Initialise output with bias.
        let mut sum = 0;
        let mut psqt_out = 0;

        // Side-To-Move Accumulator -> Output.
        for (&input, &weight) in us.vals.iter().zip(&self.output_weights[..HIDDEN_SIZE]) {
            sum += screlu(input) * i32::from(weight);
        }

        // Not-Side-To-Move Accumulator -> Output.
        for (&input, &weight) in them.vals.iter().zip(&self.output_weights[HIDDEN_SIZE..]) {
            sum += screlu(input) * i32::from(weight);
        }

        // Update Piece Square Table
        for idx in original {
            psqt_out += i32::from(self.pqst[idx as usize]);
        }
        // Apply eval scale.
        psqt_out *= SCALE;
        // Remove quantisation.
        let output =
            (sum / (QA as i32) + i32::from(self.output_bias)) * SCALE / (QA as i32 * QB as i32);
        psqt_out /= i32::from(QA);
        output + psqt_out
    }
}

// Sorry this naming convention is so bad
struct Incremental {
    state: IncrementalState,
    vec: Accumulator,
}

impl Incremental {
    fn fresh_empty(net: &Network) -> Self {
        let acc = Accumulator::new(net);
        let inc = IncrementalState::empty();
        Self {
            state: inc,
            vec: acc,
        }
    }
    fn fresh_new(net: &Network, data: IncrementalState) -> Self {
        let mut acc = Accumulator::new(net);
        for d in data.clone().into_iter() {
            acc.add_feature(d as usize, net);
        }
        Self {
            vec: acc,
            state: data,
        }
    }
}

struct BitSetIterator {
    bitset: [u64; Self::SIZE],
    idx: usize,
}

impl BitSetIterator {
    const SIZE: usize = 16;
    const END: usize = Self::SIZE - 1;
}

impl Iterator for BitSetIterator {
    type Item = u16;

    fn next(&mut self) -> Option<Self::Item> {
        while self.bitset[self.idx] == 0 {
            if self.idx >= Self::END {
                return None;
            }
            self.idx += 1
        }
        let b_idx = pop_lowest(&mut self.bitset[self.idx]) as u16;
        let out = self.idx as u16 * 64 + b_idx;
        Some(out)
    }
}

#[derive(Clone, PartialEq)]
struct IncrementalState {
    pub(crate) bitset: [u64; 16],
}

impl IntoIterator for IncrementalState {
    type Item = u16;

    type IntoIter = BitSetIterator;

    fn into_iter(self) -> Self::IntoIter {
        BitSetIterator {
            bitset: self.bitset,
            idx: 0,
        }
    }
}

impl IncrementalState {
    pub fn empty() -> Self {
        let bitset = [0; 16];
        Self { bitset }
    }
    pub fn from_vec(vec: Vec<u16>) -> Self {
        let mut bitset = [0; 16];
        for val in vec {
            let b_idx = (val / 64) as usize;
            let b_val = 1 << (val % 64);
            bitset[b_idx] |= b_val
        }
        Self { bitset }
    }
    pub fn add_feature(&mut self, val: u16) {
        let b_idx = (val / 64) as usize;
        let b_val = 1 << (val % 64);
        self.bitset[b_idx] |= b_val
    }
    pub fn compute_diff(&self, old: &Self, acc: &mut Accumulator) {
        for (idx, (n, o)) in self
            .bitset
            .iter()
            .copied()
            .zip(old.bitset.iter().copied())
            .enumerate()
        {
            let d = n ^ o; // Difference between bitsets
            if d != 0 {
                let mut sub = d & o; // Difference and Old
                let mut add = d & n; // Difference and New
                while sub != 0 {
                    let bit_idx = pop_lowest(&mut sub);
                    acc.remove_feature(idx * 64 + bit_idx as usize, &NNUE4);
                }
                while add != 0 {
                    let bit_idx = pop_lowest(&mut add);
                    acc.add_feature(idx * 64 + bit_idx as usize, &NNUE4);
                }
            }
        }
    }
}
