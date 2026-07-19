//! Self-play training-data generator for the "black stack setup" ruleset.
//!
//! Plays self-play games under the new rules (forced `2xx` double-placement opening, komi 0) and
//! writes every searched position — labelled with the engine's score and the eventual game result —
//! to the "TAK6" binpack format consumed by the external bullet trainer
//! (`bulletTrainer/bullet`, `examples/tak.rs` + `examples/tak_utils.rs`).
//!
//! ## Format (mirrors `tak_utils.rs`)
//! A file is a sequence of **chunks**; each chunk is an 8-byte `ChunkHeader` followed by a body of
//! whole **entries**. We emit exactly one entry per position with `plys_len = 0` (a full board
//! snapshot), which sidesteps the trainer reader's placement-only multi-ply replay.
//!
//! ```text
//! ChunkHeader  : magic u32 ("TAK6"), chunk_size u32        // little-endian, 8 bytes
//! EntryHeader  : caps[2] u8, white_to_move u8, extra u8,   // 10 bytes
//!                score i16, result u8, komi u8,
//!                data_len u8, plys_len u8
//! body         : data_len x PSquare(u8)                    // square | (validpiece << 6)
//! ```
//! All multi-byte fields are little-endian, matching the trainer's native-endian (x86) reads.

use crate::board::{Board6, TakBoard};
use crate::eval::{build_nn_repr, BoardData, NNUE6};
use crate::search::{search, SearchInfo};
use crate::transposition_table::HashTable;
use crate::{Color, GameMove, GameResult, Position};

use rand_core::{RngCore, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Instant;

/// "TAK6" as a big-endian u32; written little-endian to match the trainer's native-endian read.
const TAK_MAGIC: u32 = u32::from_be_bytes(*b"TAK6");
const ENTRY_HEADER_SIZE: usize = 10;
const CHUNK_HEADER_SIZE: usize = 8;
/// Flush a chunk once its body reaches roughly this size (closed only on entry boundaries).
const TARGET_CHUNK_BYTES: usize = 8 * 1024;
/// Hard depth cap; the node limit normally stops the search first.
const MAX_DEPTH: usize = 30;

#[derive(Clone, Debug)]
pub struct DataGenConfig {
    /// Total number of self-play games to generate.
    pub num_games: usize,
    /// Worker threads (one game at a time per thread).
    pub threads: usize,
    /// Search node cap per move (datagen wants fast, shallow searches).
    pub max_nodes: u64,
    /// Number of random **flat** opening plies at the start of each game, for diversity (ply 1's
    /// only legal moves are the forced double placements, so this also randomises White's opening
    /// square unless `corner_open` pins it). Ignored in book mode.
    pub random_plies: usize,
    /// Pin White's opening double-black-stack to a random corner instead of a random square, then
    /// play `random_plies - 1` further random flats (so `random_plies` stays the *total* opening
    /// ply count — note this differs from `balance`, where the corner open is an extra ply on top).
    /// A uniformly random ply 1 puts the stack on a corner only 4 times in 36; off-corner stacks
    /// exert far more influence and skew the data toward Black. Ignored in book mode.
    pub corner_open: bool,
    /// If set, a TPS opening book (one position per line). Each position seeds exactly one game,
    /// deterministically, so the **book size becomes the game count** and `num_games` /
    /// `random_plies` / `corner_open` are all ignored. Mirrors `balance --book`.
    pub book_path: Option<String>,
    /// Output `.bin` path.
    pub output: String,
    /// Transposition-table size (entries) per worker thread.
    pub tt_size: usize,
}

impl Default for DataGenConfig {
    fn default() -> Self {
        Self {
            num_games: 1000,
            threads: 4,
            max_nodes: 5000,
            random_plies: 6,
            corner_open: false,
            book_path: None,
            output: "data.bin".to_string(),
            tt_size: 1 << 20,
        }
    }
}

/// Append one position as a full-snapshot entry (`plys_len = 0`) to a chunk body buffer.
fn push_entry(body: &mut Vec<u8>, data: &BoardData, score: i16, result: u8) {
    body.push(data.caps[0]);
    body.push(data.caps[1]);
    body.push(data.white_to_move as u8);
    body.push(0); // extra_score
    body.extend_from_slice(&score.to_le_bytes());
    body.push(result);
    body.push(0); // komi (0 for this ruleset)
    body.push(data.data_len);
    body.push(0); // plys_len
    for i in 0..data.data_len as usize {
        // The engine's PieceSquare byte is exactly the trainer's PSquare byte.
        body.push(data.data[i].0);
    }
}

/// Wrap an accumulated entry body in a `ChunkHeader`.
fn make_chunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(CHUNK_HEADER_SIZE + body.len());
    out.extend_from_slice(&TAK_MAGIC.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    out
}

/// Map a finished game's result to the side-to-move-relative label {0 = loss, 1 = draw, 2 = win}.
fn result_for(white_to_move: bool, outcome: GameResult) -> u8 {
    match outcome {
        GameResult::Draw => 1,
        GameResult::WhiteWin => {
            if white_to_move {
                2
            } else {
                0
            }
        }
        GameResult::BlackWin => {
            if white_to_move {
                0
            } else {
                2
            }
        }
    }
}

pub fn new_rng() -> Xoshiro256PlusPlus {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).expect("failed to seed rng");
    Xoshiro256PlusPlus::from_seed(seed)
}

/// Play and serialize `num_games` games, sending finished chunks to the writer thread.
/// Read a TPS opening book (one position per line, blanks ignored).
fn load_book(path: &str) -> io::Result<Vec<String>> {
    let text = std::fs::read_to_string(path)?;
    let lines: Vec<String> = text
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("empty opening book: {path}"),
        ));
    }
    Ok(lines)
}

/// Build a game's starting board. In book mode, start from book entry `idx` (played exactly once —
/// the search is deterministic, so re-using an opening would just re-generate an identical game).
/// Otherwise build the opening from random flat plies, optionally pinning ply 1 to a corner.
fn start_board(
    book: &Option<Arc<Vec<String>>>,
    idx: usize,
    cfg: &DataGenConfig,
    rng: &mut impl RngCore,
) -> Board6 {
    if let Some(book) = book {
        return Board6::try_from_tps(&book[idx]).expect("datagen book contained an invalid tps");
    }

    let mut board = Board6::new(); // komi defaults to 0
    let mut plies = cfg.random_plies;

    // Pin White's opening double-black-stack to a random corner. All four corners are symmetric,
    // so the choice among them only averages out per-square quirks in the net. This consumes ply 1.
    if cfg.corner_open && plies > 0 {
        const CORNERS: [&str; 4] = ["2a1", "2f1", "2a6", "2f6"];
        let mv = GameMove::try_from_ptn_m(CORNERS[(rng.next_u32() % 4) as usize], 6, Color::White)
            .expect("valid corner double-placement");
        board.do_move(mv);
        plies -= 1;
    }

    // Random **flat** opening plies (no walls/caps/spreads — those are not realistic openings and
    // only distort the training distribution). The forced `2xx` at ply 1 is itself a flat placement,
    // so it is preserved when `corner_open` is off.
    for _ in 0..plies {
        if board.game_result().is_some() {
            break;
        }
        board.do_random_flat_move(rng);
    }
    board
}

fn worker(
    games: std::ops::Range<usize>,
    book: Option<Arc<Vec<String>>>,
    cfg: &DataGenConfig,
    tx: mpsc::Sender<Vec<u8>>,
    games_done: &AtomicU64,
    positions_done: &AtomicU64,
) {
    let mut rng = new_rng();
    let table = HashTable::new(cfg.tt_size);
    let mut body: Vec<u8> = Vec::with_capacity(TARGET_CHUNK_BYTES + 256);
    // (snapshot, side-to-move-relative score) recorded before each played move.
    let mut positions: Vec<(BoardData, i16)> = Vec::new();

    for idx in games {
        table.clear();
        positions.clear();
        let mut board = start_board(&book, idx, cfg, &mut rng);

        // Play out, recording each searched position.
        while board.game_result().is_none() {
            let mut eval = NNUE6::default();
            let mut info = SearchInfo::new(MAX_DEPTH, &table)
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
            let data = build_nn_repr(&board);
            let score = outcome.score().clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            positions.push((data, score));
            board.do_move(mv);
        }

        // Backfill results now that the game is decided.
        let outcome = board.game_result().unwrap_or(GameResult::Draw);
        for (data, score) in positions.iter() {
            let result = result_for(data.white_to_move, outcome);
            push_entry(&mut body, data, *score, result);
        }
        positions_done.fetch_add(positions.len() as u64, Ordering::Relaxed);

        if body.len() >= TARGET_CHUNK_BYTES {
            if tx.send(make_chunk(&body)).is_err() {
                return;
            }
            body.clear();
        }

        let n = games_done.fetch_add(1, Ordering::Relaxed) + 1;
        if n % 200 == 0 {
            eprintln!(
                "datagen: {n} games, {} positions",
                positions_done.load(Ordering::Relaxed)
            );
        }
    }

    if !body.is_empty() {
        let _ = tx.send(make_chunk(&body));
    }
}

/// Generate a self-play dataset and write it to `cfg.output`.
pub fn run(cfg: DataGenConfig) -> io::Result<()> {
    assert!(cfg.threads >= 1, "need at least one thread");
    let start = Instant::now();

    // Book mode: every position seeds exactly one game, so the book size IS the game count.
    let book: Option<Arc<Vec<String>>> = match cfg.book_path.as_deref() {
        Some(path) => Some(Arc::new(load_book(path)?)),
        None => None,
    };
    let total_games = match &book {
        Some(b) => b.len(),
        None => cfg.num_games,
    };

    match &book {
        Some(b) => println!(
            "datagen: {} book positions from {} (1 deterministic game each), {} threads, {} nodes/move -> {}",
            b.len(),
            cfg.book_path.as_deref().unwrap_or(""),
            cfg.threads,
            cfg.max_nodes,
            cfg.output
        ),
        None => println!(
            "datagen: {} games, {} threads, {} nodes/move, {} random flat plies{} -> {}",
            total_games,
            cfg.threads,
            cfg.max_nodes,
            cfg.random_plies,
            if cfg.corner_open {
                ", White opens in a corner"
            } else {
                ""
            },
            cfg.output
        ),
    }

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let out_path = cfg.output.clone();
    let writer = thread::spawn(move || -> io::Result<u64> {
        let mut file = BufWriter::new(File::create(&out_path)?);
        let mut bytes = 0u64;
        while let Ok(chunk) = rx.recv() {
            file.write_all(&chunk)?;
            bytes += chunk.len() as u64;
        }
        file.flush()?;
        Ok(bytes)
    });

    let games_done = Arc::new(AtomicU64::new(0));
    let positions_done = Arc::new(AtomicU64::new(0));

    // Contiguous index ranges: in book mode these index the book, so every opening is used exactly
    // once across all workers.
    let base = total_games / cfg.threads;
    let extra = total_games % cfg.threads;
    let mut handles = Vec::new();
    let mut next = 0usize;
    for t in 0..cfg.threads {
        let count = base + if t < extra { 1 } else { 0 };
        let range = next..next + count;
        next += count;
        if count == 0 {
            continue;
        }
        let tx = tx.clone();
        let cfg = cfg.clone();
        let book = book.clone();
        let games_done = games_done.clone();
        let positions_done = positions_done.clone();
        handles.push(thread::spawn(move || {
            worker(range, book, &cfg, tx, &games_done, &positions_done);
        }));
    }
    drop(tx);

    for h in handles {
        h.join().expect("worker thread panicked");
    }
    let bytes = writer.join().expect("writer thread panicked")?;

    let secs = start.elapsed().as_secs_f64();
    let positions = positions_done.load(Ordering::Relaxed);
    println!(
        "datagen done: {} games, {} positions, {:.1} MB in {:.1}s ({:.0} pos/s) -> {}",
        games_done.load(Ordering::Relaxed),
        positions,
        bytes as f64 / (1024.0 * 1024.0),
        secs,
        positions as f64 / secs.max(1e-9),
        cfg.output,
    );
    Ok(())
}

/// A decoded position, used only to validate the on-disk format in tests.
#[derive(Debug, PartialEq, Eq)]
pub struct DecodedPosition {
    pub caps: [u8; 2],
    pub white_to_move: bool,
    pub score: i16,
    pub result: u8,
    pub komi: u8,
    pub data: Vec<u8>,
}

/// Parse a "TAK6" byte stream back into positions (independent of the trainer crate).
pub fn parse_chunks(bytes: &[u8]) -> Vec<DecodedPosition> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + CHUNK_HEADER_SIZE <= bytes.len() {
        let magic = u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        assert_eq!(magic, TAK_MAGIC, "bad chunk magic");
        let chunk_size = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().unwrap()) as usize;
        i += CHUNK_HEADER_SIZE;
        let end = i + chunk_size;
        while i < end {
            let caps = [bytes[i], bytes[i + 1]];
            let white_to_move = bytes[i + 2] != 0;
            let score = i16::from_le_bytes([bytes[i + 4], bytes[i + 5]]);
            let result = bytes[i + 6];
            let komi = bytes[i + 7];
            let data_len = bytes[i + 8] as usize;
            let plys_len = bytes[i + 9] as usize;
            i += ENTRY_HEADER_SIZE;
            let data = bytes[i..i + data_len].to_vec();
            i += data_len + plys_len * 4;
            out.push(DecodedPosition {
                caps,
                white_to_move,
                score,
                result,
                komi,
                data,
            });
        }
        i = end;
    }
    out
}

/// Parse a "TAK6" file's bytes and return a human-readable summary, validating each entry's
/// fields. Useful for sanity-checking a generated dataset before a long training run.
pub fn summarize(bytes: &[u8]) -> String {
    let positions = parse_chunks(bytes);
    let mut wdl = [0u64; 3]; // loss / draw / win (side-to-move relative)
    let mut max_data = 0usize;
    let mut invalid = 0u64;
    let (mut smin, mut smax) = (i16::MAX, i16::MIN);
    for p in &positions {
        if p.result <= 2 {
            wdl[p.result as usize] += 1;
        } else {
            invalid += 1;
        }
        if p.data.len() > 62 || p.komi != 0 {
            invalid += 1;
        }
        max_data = max_data.max(p.data.len());
        smin = smin.min(p.score);
        smax = smax.max(p.score);
    }
    format!(
        "positions={} results[L/D/W]={:?} score=[{},{}] max_stack_bytes={} invalid={}",
        positions.len(),
        wdl,
        smin,
        smax,
        max_data,
        invalid
    )
}

/// Stream a "TAK6" file chunk-by-chunk and return a validation summary, using O(1) memory
/// (one chunk in flight) so it scales to arbitrarily large datasets. Tolerates a truncated tail.
pub fn summarize_file(path: &str) -> io::Result<String> {
    let mut f = BufReader::new(File::open(path)?);
    let mut wdl = [0u64; 3]; // loss / draw / win (side-to-move relative)
    let mut total = 0u64;
    let mut invalid = 0u64;
    let mut max_data = 0usize;
    let mut smin = i16::MAX;
    let mut smax = i16::MIN;
    let mut truncated = false;
    let mut hdr = [0u8; CHUNK_HEADER_SIZE];
    loop {
        match f.read_exact(&mut hdr) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        let magic = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
        if magic != TAK_MAGIC {
            return Ok(format!("ERROR: bad chunk magic (corrupt file); positions read so far={total}"));
        }
        let chunk_size = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
        let mut body = vec![0u8; chunk_size];
        match f.read_exact(&mut body) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                truncated = true;
                break;
            }
            Err(e) => return Err(e),
        }
        let mut i = 0;
        while i + ENTRY_HEADER_SIZE <= body.len() {
            let score = i16::from_le_bytes([body[i + 4], body[i + 5]]);
            let result = body[i + 6];
            let komi = body[i + 7];
            let data_len = body[i + 8] as usize;
            let plys_len = body[i + 9] as usize;
            let entry_end = i + ENTRY_HEADER_SIZE + data_len + plys_len * 4;
            if entry_end > body.len() {
                invalid += 1;
                break;
            }
            if result <= 2 {
                wdl[result as usize] += 1;
            } else {
                invalid += 1;
            }
            if data_len > 62 || komi != 0 {
                invalid += 1;
            }
            max_data = max_data.max(data_len);
            smin = smin.min(score);
            smax = smax.max(score);
            total += 1;
            i = entry_end;
        }
    }
    if total == 0 {
        smin = 0;
        smax = 0;
    }
    let trunc = if truncated { " (WARNING: file truncated at tail)" } else { "" };
    Ok(format!(
        "positions={total} results[L/D/W]={:?} score=[{},{}] max_stack_bytes={max_data} invalid={invalid}{trunc}",
        wdl, smin, smax
    ))
}

/// Analyse the **eval calibration** of a "TAK6" dataset: does the net's score at a position predict
/// the actual game outcome, and — crucially — is there a systematic White/Black skew?
///
/// Every entry stores a side-to-move-relative search `score` and the STM-relative game `result`.
/// A White/Black bias is invisible in the STM frame (it cancels as the mover alternates), so we
/// convert **both** to a fixed **White frame** and bin positions by the net's White-frame eval,
/// then measure White's actual score in each bin. The eval where White actually scores 50% is the
/// key number:
///   - ≈ 0  → the net is calibrated (its zero-point is a true 50/50 position);
///   - < 0  → the net **over-favours Black**: it scores genuinely even positions as Black-ahead by
///            that many cp (this is the skew we're hunting);
///   - > 0  → the net over-favours White.
///
/// Bins also print the sigmoid-implied win% (`sigmoid(internal_score / 400)`) next to the actual
/// win%, so a *scale* problem (right zero-point, wrong slope) is visible as a consistent
/// steeper/flatter actual curve rather than a shifted crossover.
///
/// `score` is internal (SCALE = 400/pawn); displayed "cp" = internal / 4, matching the engine's
/// `info … score cp` output. Streams the file in O(1) memory, so it scales to the 11 GB datasets.
pub fn calibrate_file(path: &str) -> io::Result<String> {
    use std::collections::BTreeMap;

    // Displayed-cp bin width and tail clamp; binning is done in internal units (= 4× displayed) to
    // avoid a double round-off. Use floor division (`div_euclid`) so the zero boundary is clean:
    // plain `/` truncates toward zero, which would make the bin at 0 straddle it (double width) and
    // mislabel every negative bin.
    const BIN_CP: i32 = 25;
    const CLAMP_CP: i32 = 300;
    const BIN_INT: i32 = BIN_CP * 4; // internal score units per bin
    const CLAMP_INT: i32 = CLAMP_CP * 4;
    const EVAL_SCALE: f64 = 400.0; // internal score → win prob via sigmoid(score / EVAL_SCALE)

    // bin index (in displayed cp / BIN_CP) → (count, white_result_sum, internal_score_sum)
    let mut bins: BTreeMap<i32, (u64, f64, f64)> = BTreeMap::new();
    let mut total = 0u64;
    let mut white_sum = 0.0f64;
    let mut truncated = false;

    let mut f = BufReader::new(File::open(path)?);
    let mut hdr = [0u8; CHUNK_HEADER_SIZE];
    loop {
        match f.read_exact(&mut hdr) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        let magic = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
        if magic != TAK_MAGIC {
            return Ok(format!("ERROR: bad chunk magic (corrupt file); positions read so far={total}"));
        }
        let chunk_size = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
        let mut body = vec![0u8; chunk_size];
        match f.read_exact(&mut body) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                truncated = true;
                break;
            }
            Err(e) => return Err(e),
        }
        let mut i = 0;
        while i + ENTRY_HEADER_SIZE <= body.len() {
            let white_to_move = body[i + 2] != 0;
            let score = i16::from_le_bytes([body[i + 4], body[i + 5]]) as i32;
            let result = body[i + 6];
            let data_len = body[i + 8] as usize;
            let plys_len = body[i + 9] as usize;
            let entry_end = i + ENTRY_HEADER_SIZE + data_len + plys_len * 4;
            if entry_end > body.len() {
                break;
            }
            i = entry_end;

            // Skip mate-coded / invalid scores (the trainer filters |score| > 9999 too).
            if result > 2 || score.abs() > 9999 {
                continue;
            }

            // Convert score and result into the White frame.
            let white_score = if white_to_move { score } else { -score };
            let stm_wr = result as f64 / 2.0; // 0=loss, 0.5=draw, 1=win (STM)
            let white_wr = if white_to_move { stm_wr } else { 1.0 - stm_wr };

            let bin = white_score.clamp(-CLAMP_INT, CLAMP_INT).div_euclid(BIN_INT);
            let e = bins.entry(bin).or_insert((0, 0.0, 0.0));
            e.0 += 1;
            e.1 += white_wr;
            e.2 += white_score as f64;
            total += 1;
            white_sum += white_wr;
        }
    }

    if total == 0 {
        return Ok("calibrate: no valid positions".to_string());
    }

    let sigmoid = |x: f64| 1.0 / (1.0 + (-x).exp());

    let mut out = String::new();
    out.push_str(&format!(
        "calibrate: {path}\n{total} positions (White frame){}\n\n",
        if truncated { "  (WARNING: file truncated at tail)" } else { "" }
    ));
    out.push_str("  net eval (cp)     positions     White%    implied%\n");

    // For the crossover, walk bins in ascending eval and find where actual White% crosses 0.5,
    // interpolating between adjacent bin centres (in displayed cp).
    let mut prev: Option<(f64, f64)> = None; // (bin_centre_cp, white_frac)
    let mut crossover: Option<f64> = None;
    for (&bin, &(count, wr_sum, score_sum)) in &bins {
        let frac = wr_sum / count as f64;
        let mean_internal = score_sum / count as f64;
        let implied = sigmoid(mean_internal / EVAL_SCALE);
        let centre_cp = mean_internal / 4.0;
        let lo = bin * BIN_CP;
        let label = if bin <= -CLAMP_CP / BIN_CP {
            format!("<= {:>4}", -CLAMP_CP)
        } else if bin >= CLAMP_CP / BIN_CP {
            format!(">= {:>4}", CLAMP_CP)
        } else {
            format!("[{:>4},{:>4})", lo, lo + BIN_CP)
        };
        out.push_str(&format!(
            "  {label:<14} {count:>10}     {:>5.1}%    {:>5.1}%\n",
            100.0 * frac,
            100.0 * implied
        ));
        if crossover.is_none() {
            if let Some((pc, pf)) = prev {
                if (pf - 0.5).signum() != (frac - 0.5).signum() && frac != pf {
                    let t = (0.5 - pf) / (frac - pf);
                    crossover = Some(pc + t * (centre_cp - pc));
                }
            }
        }
        prev = Some((centre_cp, frac));
    }

    out.push_str(&format!("\noverall White score: {:.2}%\n", 100.0 * white_sum / total as f64));
    match crossover {
        Some(c) => {
            out.push_str(&format!("White scores 50% at net eval = {c:+.1} cp\n"));
            if c.abs() < 5.0 {
                out.push_str("  → |crossover| < 5 cp: net is essentially calibrated (no White/Black skew).\n");
            } else if c < 0.0 {
                out.push_str(&format!(
                    "  → crossover < 0: net OVER-FAVOURS BLACK by ~{:.0} cp (scores even positions as Black-ahead).\n",
                    -c
                ));
            } else {
                out.push_str(&format!(
                    "  → crossover > 0: net over-favours White by ~{c:.0} cp.\n"
                ));
            }
        }
        None => out.push_str("White never crosses 50% within the measured eval range (widen the data's eval spread).\n"),
    }
    out.push_str(
        "Compare White% vs implied% per row: a consistent gap with a ~0 crossover = a scale (slope) issue, not a skew.\n",
    );

    Ok(out)
}

/// Trim a "TAK6" file in place, removing any incomplete/corrupt tail so that it ends exactly on a
/// chunk boundary. Makes a partially-written shard (e.g. from a killed pod) safe to concatenate
/// anywhere. Streams the file, so it works on arbitrarily large datasets.
pub fn trim_file(path: &str) -> io::Result<String> {
    let mut f = BufReader::new(File::open(path)?);
    let file_len = f.get_ref().metadata()?.len();
    let mut good_end: u64 = 0; // bytes up to and including the last fully-valid chunk
    let mut chunks = 0u64;
    let mut hdr = [0u8; CHUNK_HEADER_SIZE];
    let mut body: Vec<u8> = Vec::new();
    loop {
        match f.read_exact(&mut hdr) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        let magic = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
        if magic != TAK_MAGIC {
            break; // corrupt mid-stream: keep everything before this header
        }
        let chunk_size = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
        body.resize(chunk_size, 0);
        match f.read_exact(&mut body) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break, // truncated body
            Err(e) => return Err(e),
        }
        good_end += (CHUNK_HEADER_SIZE + chunk_size) as u64;
        chunks += 1;
    }
    if good_end == file_len {
        Ok(format!("OK: already clean ({chunks} chunks, {file_len} bytes) — nothing to trim"))
    } else {
        OpenOptions::new().write(true).open(path)?.set_len(good_end)?;
        Ok(format!(
            "TRIMMED: removed {} bytes of incomplete tail; kept {chunks} chunks ({good_end} bytes)",
            file_len - good_end
        ))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn magic_and_header_sizes() {
        // "TAK6" big-endian, written little-endian on disk.
        assert_eq!(TAK_MAGIC, 0x5441_4B36);
        assert_eq!(TAK_MAGIC.to_le_bytes(), [0x36, 0x4B, 0x41, 0x54]);
        assert_eq!(ENTRY_HEADER_SIZE, 10);
        assert_eq!(CHUNK_HEADER_SIZE, 8);
    }

    #[test]
    fn entry_roundtrip_matches_board() {
        // A rich position (caps, walls, multi-piece stacks) exercising the full snapshot encoding.
        let tps = "2,2,2,21,12,x/x4,2,x/x4,2C,x/1,2,12,122211C,x2/x2,1S,1,12,1/x3,2S,1,1 1 19";
        let board = Board6::try_from_tps(tps).unwrap();
        let data = build_nn_repr(&board);

        let mut body = Vec::new();
        push_entry(&mut body, &data, -123, 2);
        let chunk = make_chunk(&body);

        let parsed = parse_chunks(&chunk);
        assert_eq!(parsed.len(), 1);
        let p = &parsed[0];
        assert_eq!(p.caps, data.caps);
        assert_eq!(p.white_to_move, data.white_to_move);
        assert_eq!(p.score, -123);
        assert_eq!(p.result, 2);
        assert_eq!(p.komi, 0);
        assert_eq!(p.data.len(), data.data_len as usize);
        for i in 0..data.data_len as usize {
            assert_eq!(p.data[i], data.data[i].0, "PSquare byte {i} mismatch");
        }
    }

    #[test]
    fn multiple_entries_and_chunks_roundtrip() {
        let board = Board6::try_from_tps("x6/x6/x6/x2,2,x3/1,x5/x6 1 3").unwrap();
        let data = build_nn_repr(&board);
        let mut all = Vec::new();
        // Two chunks, three entries total, with varied score/result labels.
        let mut body1 = Vec::new();
        push_entry(&mut body1, &data, 0, 1);
        push_entry(&mut body1, &data, 9999, 2);
        all.extend_from_slice(&make_chunk(&body1));
        let mut body2 = Vec::new();
        push_entry(&mut body2, &data, -5000, 0);
        all.extend_from_slice(&make_chunk(&body2));

        let parsed = parse_chunks(&all);
        assert_eq!(parsed.len(), 3);
        assert_eq!((parsed[0].score, parsed[0].result), (0, 1));
        assert_eq!((parsed[1].score, parsed[1].result), (9999, 2));
        assert_eq!((parsed[2].score, parsed[2].result), (-5000, 0));
    }

    #[test]
    fn summarize_file_streams() {
        let board = Board6::try_from_tps("x6/x6/x6/x2,2,x3/1,x5/x6 1 3").unwrap();
        let data = build_nn_repr(&board);
        let mut all = Vec::new();
        let mut b1 = Vec::new();
        push_entry(&mut b1, &data, 10, 2);
        push_entry(&mut b1, &data, -10, 0);
        all.extend_from_slice(&make_chunk(&b1));
        let mut b2 = Vec::new();
        push_entry(&mut b2, &data, 0, 1);
        all.extend_from_slice(&make_chunk(&b2));

        let path = std::env::temp_dir().join(format!("datagen_summ_{}.bin", std::process::id()));
        std::fs::write(&path, &all).unwrap();
        let s = summarize_file(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(s.contains("positions=3"), "{s}");
        assert!(s.contains("results[L/D/W]=[1, 1, 1]"), "{s}");
        assert!(s.contains("invalid=0"), "{s}");
    }
    #[test]
    fn trim_removes_truncated_tail() {
        let board = Board6::try_from_tps("x6/x6/x6/x2,2,x3/1,x5/x6 1 3").unwrap();
        let data = build_nn_repr(&board);
        let mut bytes = Vec::new();
        let mut b1 = Vec::new();
        push_entry(&mut b1, &data, 5, 2);
        push_entry(&mut b1, &data, -5, 0);
        bytes.extend_from_slice(&make_chunk(&b1));
        let good_len = bytes.len();
        // append a chunk with its last 3 bytes chopped off (simulating a killed write)
        let mut b2 = Vec::new();
        push_entry(&mut b2, &data, 1, 1);
        let bad = make_chunk(&b2);
        bytes.extend_from_slice(&bad[..bad.len() - 3]);

        let path = std::env::temp_dir().join(format!("datagen_trim_{}.bin", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let msg = trim_file(path.to_str().unwrap()).unwrap();
        assert!(msg.contains("TRIMMED"), "{msg}");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), good_len as u64);
        // now clean: 2 positions, no truncation warning, and trimming again is a no-op
        let s = summarize_file(path.to_str().unwrap()).unwrap();
        assert!(s.contains("positions=2") && !s.contains("truncated"), "{s}");
        assert!(trim_file(path.to_str().unwrap()).unwrap().contains("already clean"));
        std::fs::remove_file(&path).ok();
    }
    #[test]
    fn result_perspective() {
        assert_eq!(result_for(true, GameResult::WhiteWin), 2);
        assert_eq!(result_for(true, GameResult::BlackWin), 0);
        assert_eq!(result_for(false, GameResult::WhiteWin), 0);
        assert_eq!(result_for(false, GameResult::BlackWin), 2);
        assert_eq!(result_for(true, GameResult::Draw), 1);
        assert_eq!(result_for(false, GameResult::Draw), 1);
    }
}
