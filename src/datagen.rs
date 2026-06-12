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
use crate::{GameResult, Position};

use rand_core::SeedableRng;
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
    /// Number of random plies at the start of each game for opening diversity (ply 0's only legal
    /// moves are the forced double placements, so this also randomises White's opening square).
    pub random_plies: usize,
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

pub(crate) fn new_rng() -> Xoshiro256PlusPlus {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).expect("failed to seed rng");
    Xoshiro256PlusPlus::from_seed(seed)
}

/// Play and serialize `num_games` games, sending finished chunks to the writer thread.
fn worker(
    num_games: usize,
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

    for _ in 0..num_games {
        table.clear();
        positions.clear();
        let mut board = Board6::new(); // komi defaults to 0

        // Random opening for diversity (includes the forced ply-0 double placement).
        for _ in 0..cfg.random_plies {
            if board.game_result().is_some() {
                break;
            }
            board.do_random_move(&mut rng);
        }

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
    println!(
        "datagen: {} games, {} threads, {} nodes/move, {} random plies -> {}",
        cfg.num_games, cfg.threads, cfg.max_nodes, cfg.random_plies, cfg.output
    );

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

    let base = cfg.num_games / cfg.threads;
    let extra = cfg.num_games % cfg.threads;
    let mut handles = Vec::new();
    for t in 0..cfg.threads {
        let games = base + if t < extra { 1 } else { 0 };
        if games == 0 {
            continue;
        }
        let tx = tx.clone();
        let cfg = cfg.clone();
        let games_done = games_done.clone();
        let positions_done = positions_done.clone();
        handles.push(thread::spawn(move || {
            worker(games, &cfg, tx, &games_done, &positions_done);
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
