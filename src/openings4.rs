//! 4x4 black-stack opening-book generator (Board4 port of [`crate::openings5`]).
//!
//! The 4x4 board is small (16 squares, fills fast), so the book is **shallower** than the 5x5 one:
//!   - **4 plies** (not 6): plies 5-6 dropped — 6 flats already decides a 4x4 position.
//!   - **4 archetypes** (center-stack dropped): diagonal corners, adjacent corners, hug, gap-hug.
//!   - plies 3-4 use the proximity rule directly (no interior-only plies — the 4x4 interior is just
//!     the 2x2 b2..c3, too restrictive): ply 3 (white) within `MAX_STEPS` orthogonal steps of a white
//!     piece (= b2); ply 4 (black) within `MAX_STEPS` of a black piece (= w1).
//!
//! Same colour swap as 5x5: White's ply-1 double placement is BLACK, Black's ply-2 flat is (swapped)
//! WHITE. Emits a **TPS book** (move_num >= 2, for racetrack `--book-format tps`) + a parallel PTN
//! book — TPS because racetrack/tiltak (standard rules) reject the forced `2xx`. See auto-memory
//! `racetrack-tps-book-requirement`. D4-equivalent positions are de-duplicated.
//!
//! Enumerated D4-unique ceiling for this scheme = 207 positions (verified).

use crate::board::{Board4, TakBoard};
use crate::datagen::new_rng;
use crate::{GameMove, Position};

use rand_core::RngCore;
use std::collections::HashSet;
use std::fs;
use std::io;

const SIZE: usize = 4;

/// Max orthogonal (Manhattan) distance from an own-colour piece for ply 3-4 placements.
const MAX_STEPS: i32 = 3;

/// Ply 1-2 archetypes (White double-stack square, Black flat square), in 4x4 coordinates. Symmetry
/// de-duplication on the final 4-ply position collapses equivalents. Corners are a1/a4/d1/d4.
const DIAGONAL: &[(&str, &str)] = &[("a1", "d4"), ("d4", "a1"), ("a4", "d1"), ("d1", "a4")];
const ADJACENT: &[(&str, &str)] = &[
    ("a1", "a4"),
    ("a1", "d1"),
    ("a4", "a1"),
    ("a4", "d4"),
    ("d1", "d4"),
    ("d1", "a1"),
    ("d4", "d1"),
    ("d4", "a4"),
];
const HUG: &[(&str, &str)] = &[
    ("a1", "a2"),
    ("a1", "b1"),
    ("a4", "a3"),
    ("a4", "b4"),
    ("d1", "d2"),
    ("d1", "c1"),
    ("d4", "d3"),
    ("d4", "c4"),
];
const GAP_HUG: &[(&str, &str)] = &[
    ("a1", "c1"),
    ("a1", "a3"),
    ("a4", "c4"),
    ("a4", "a2"),
    ("d1", "b1"),
    ("d1", "d3"),
    ("d4", "b4"),
    ("d4", "d2"),
];
const CORNERS: [&str; 4] = ["a1", "a4", "d1", "d4"];

pub const NUM_ARCHETYPES: usize = 4;

/// Human-readable name for an archetype index (matches pick_instance's ordering).
pub fn archetype_name(i: usize) -> &'static str {
    match i {
        0 => "diagonal corners",
        1 => "adjacent corners",
        2 => "hug",
        3 => "gap hug",
        _ => "unknown",
    }
}

/// Number of archetypes for the given mode (4 black-stack, 3 standard).
pub fn num_archetypes(standard: bool) -> usize {
    if standard {
        STANDARD_ARCHETYPES
    } else {
        NUM_ARCHETYPES
    }
}

/// Black's first-reply square to White's double-black-stack on a corner. 4x4 port of
/// [`crate::openings5::pick_black_reply`]: weights = `[diagonal, adjacent, hug, gap_hug]`, returns
/// `None` for a non-corner White opening (caller falls back to search). Self-seeds its RNG.
pub fn pick_black_reply(white_sq: &str, weights: &[f64]) -> Option<String> {
    if !CORNERS.iter().any(|c| *c == white_sq) {
        return None;
    }
    let tables: [&[(&str, &str)]; 4] = [DIAGONAL, ADJACENT, HUG, GAP_HUG];
    let mut w = [0.0f64; 4];
    for (i, slot) in w.iter_mut().enumerate() {
        *slot = weights.get(i).copied().unwrap_or(0.0).max(0.0);
    }
    let total: f64 = w.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let mut rng = new_rng();
    let mut r = (rng.next_u32() as f64 / u32::MAX as f64) * total;
    let mut arch = 0usize;
    for (i, wi) in w.iter().enumerate() {
        arch = i;
        if r < *wi {
            break;
        }
        r -= *wi;
    }
    let opts: Vec<&str> = tables[arch]
        .iter()
        .filter(|(ws, _)| *ws == white_sq)
        .map(|(_, bs)| *bs)
        .collect();
    if opts.is_empty() {
        return None;
    }
    Some(opts[(rng.next_u32() as usize) % opts.len()].to_string())
}

pub struct OpeningsConfig {
    pub count: usize,
    pub tps_path: String,
    pub ptn_path: String,
    /// Standard-Tak mode: ply 1 is a single swapped flat instead of the `2xx` double-black-stack,
    /// and only the diagonal-corner / adjacent-corner / hug archetypes are used.
    pub standard: bool,
}

impl Default for OpeningsConfig {
    fn default() -> Self {
        Self {
            count: 207,
            tps_path: "4s_black_stack_openings.tps".to_string(),
            ptn_path: "4s_black_stack_openings.ptn".to_string(),
            standard: false,
        }
    }
}

/// Number of ply 1-2 archetypes used in standard mode (diagonal corners, adjacent corners, hug).
const STANDARD_ARCHETYPES: usize = 3;

/// All 16 squares, as "<file><rank>" strings.
fn all_squares() -> Vec<String> {
    let mut v = Vec::with_capacity(SIZE * SIZE);
    for file in 'a'..=('a' as u8 + SIZE as u8 - 1) as char {
        for rank in 1..=SIZE {
            v.push(format!("{file}{rank}"));
        }
    }
    v
}

#[cfg(test)]
fn choose<'a, R: RngCore>(slice: &'a [(&str, &str)], rng: &mut R) -> (&'a str, &'a str) {
    slice[(rng.next_u32() as usize) % slice.len()]
}

/// Pick a (White-stack, Black-flat) square pair for the given archetype.
#[cfg(test)]
fn pick_instance<R: RngCore>(arch: usize, rng: &mut R) -> (String, String) {
    match arch {
        0 => {
            let (w, b) = choose(DIAGONAL, rng);
            (w.to_string(), b.to_string())
        }
        1 => {
            let (w, b) = choose(ADJACENT, rng);
            (w.to_string(), b.to_string())
        }
        2 => {
            let (w, b) = choose(HUG, rng);
            (w.to_string(), b.to_string())
        }
        _ => {
            let (w, b) = choose(GAP_HUG, rng);
            (w.to_string(), b.to_string())
        }
    }
}

/// Apply one ply to the board by constructing the move from its PTN. `double` marks the forced
/// ply-1 `2xx` placement. `do_move` handles the move_num==1 swap for plies 1-2.
fn apply_ply(board: &mut Board4, sq: &str, double: bool) {
    let ptn = if double {
        format!("2{sq}")
    } else {
        sq.to_string()
    };
    let mv = GameMove::try_from_ptn_m(&ptn, SIZE, board.side_to_move())
        .expect("constructed a valid ptn placement");
    board.do_move(mv);
}

/// (file, rank) zero-based coordinates of a "<file><rank>" square, e.g. "c4" -> (2, 3).
fn coords(sq: &str) -> (i32, i32) {
    let file = sq.as_bytes()[0] as i32 - b'a' as i32;
    let rank = sq[1..].parse::<i32>().unwrap() - 1;
    (file, rank)
}

/// Whether two squares are within `steps` orthogonal (Manhattan) steps of each other.
fn within_steps(a: &str, b: &str, steps: i32) -> bool {
    let (af, ar) = coords(a);
    let (bf, br) = coords(b);
    (af - bf).abs() + (ar - br).abs() <= steps
}

/// Pick a random unoccupied square within `MAX_STEPS` orthogonal steps of at least one reference
/// square. Returns `None` if no such square exists (caller retries with a new opening).
#[cfg(test)]
fn pick_free_near<R: RngCore>(
    pool: &[String],
    occupied: &[String],
    refs: &[String],
    rng: &mut R,
) -> Option<String> {
    let cands: Vec<&String> = pool
        .iter()
        .filter(|s| !occupied.iter().any(|o| o == *s))
        .filter(|s| refs.iter().any(|r| within_steps(s, r, MAX_STEPS)))
        .collect();
    if cands.is_empty() {
        return None;
    }
    Some(cands[(rng.next_u32() as usize) % cands.len()].clone())
}

/// The (White-stack, Black-flat) seed pairs for an archetype index.
fn archetype_table(arch: usize) -> &'static [(&'static str, &'static str)] {
    match arch {
        0 => DIAGONAL,
        1 => ADJACENT,
        2 => HUG,
        _ => GAP_HUG,
    }
}

/// Build the explicit 4-ply opening `w1, b2, p3, p4` (squares already chosen). Returns `(tps, ptn)`,
/// or `None` if any placement is illegal (e.g. duplicate square). Deterministic — no RNG.
fn place_opening(w1: &str, b2: &str, p3: &str, p4: &str, standard: bool) -> Option<(String, String)> {
    // Squares must be distinct.
    let sqs = [w1, b2, p3, p4];
    for i in 0..4 {
        for j in (i + 1)..4 {
            if sqs[i] == sqs[j] {
                return None;
            }
        }
    }
    let mut board = Board4::new(); // komi 0
    let mut ptn: Vec<String> = Vec::with_capacity(4);

    // ply 1: White's opening -> a black piece at w1 (double placement, or single flat in standard).
    if standard {
        apply_ply(&mut board, w1, false);
        ptn.push(w1.to_string());
    } else {
        apply_ply(&mut board, w1, true);
        ptn.push(format!("2{w1}"));
    }
    // ply 2: Black's (swapped) white flat at b2.
    apply_ply(&mut board, b2, false);
    ptn.push(b2.to_string());
    // ply 3: White flat at p3 (must be within MAX_STEPS of a white piece = b2).
    apply_ply(&mut board, p3, false);
    ptn.push(p3.to_string());
    // ply 4: Black flat at p4 (must be within MAX_STEPS of a black piece = w1).
    apply_ply(&mut board, p4, false);
    ptn.push(p4.to_string());

    // Defensive: flats-only over 4 plies cannot make a road or fill the board, so this never trips.
    if board.game_result().is_some() {
        return None;
    }
    Some((format!("{board:?}"), ptn.join(" ")))
}

/// Exhaustively enumerate every D4-unique opening of a single archetype (deduped within the
/// archetype). Deterministic order: instance table order, then p3 / p4 in board-square order.
/// Returns `(canonical_key, tps, ptn)` per unique position.
fn enumerate_archetype(arch: usize, all_sq: &[String], standard: bool) -> Vec<(String, String, String)> {
    let mut local_seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for &(w1, b2) in archetype_table(arch) {
        // p3 (white) within MAX_STEPS of b2; p4 (black) within MAX_STEPS of w1.
        for p3 in all_sq.iter().filter(|s| within_steps(s, b2, MAX_STEPS)) {
            for p4 in all_sq.iter().filter(|s| within_steps(s, w1, MAX_STEPS)) {
                if let Some((tps, ptn)) = place_opening(w1, b2, p3, p4, standard) {
                    let key = canonical_key(&tps);
                    if local_seen.insert(key.clone()) {
                        out.push((key, tps, ptn));
                    }
                }
            }
        }
    }
    out
}

/// Build one 4-ply opening for the given archetype using RNG (kept for tests / `pick_black_reply`-
/// style sampling). Returns `(tps, ptn)` on success.
#[cfg(test)]
fn build_opening<R: RngCore>(
    arch: usize,
    all_sq: &[String],
    standard: bool,
    rng: &mut R,
) -> Option<(String, String)> {
    let (w1, b2) = pick_instance(arch, rng);
    let occupied = vec![w1.clone(), b2.clone()];
    let white_refs = vec![b2.clone()];
    let black_refs = vec![w1.clone()];
    let p3 = pick_free_near(all_sq, &occupied, &white_refs, rng)?;
    let mut occ2 = occupied.clone();
    occ2.push(p3.clone());
    let p4 = pick_free_near(all_sq, &occ2, &black_refs, rng)?;
    place_opening(&w1, &b2, &p3, &p4, standard)
}

/// Canonical key of a position under the dihedral group D4, for rotational/reflectional dedup.
/// Operates on the Debug/TPS grid (`<rows> <side> <num>`), where each row has SIZE cells.
fn canonical_key(tps: &str) -> String {
    let board_part = tps.split(' ').next().unwrap_or(tps);
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
    best.unwrap()
}

fn serialize(grid: &[Vec<String>]) -> String {
    grid.iter()
        .map(|row| row.join(","))
        .collect::<Vec<_>>()
        .join("/")
}

/// Mirror left<->right: new[r][c] = old[r][n-1-c].
fn flip_h(grid: &[Vec<String>]) -> Vec<Vec<String>> {
    grid.iter()
        .map(|row| row.iter().rev().cloned().collect())
        .collect()
}

/// Rotate 90 degrees: new[r][c] = old[n-1-c][r].
fn rot90(grid: &[Vec<String>]) -> Vec<Vec<String>> {
    let n = grid.len();
    (0..n)
        .map(|r| (0..n).map(|c| grid[n - 1 - c][r].clone()).collect())
        .collect()
}

/// Generate `count` unique (D4-deduped) openings with per-archetype relative `weights` (length must
/// equal the active archetype count). Returns `(tps, ptn, archetype_index)` per opening.
///
/// Unlike the 5x5 generator (which randomly samples a huge space), the 4x4 space is small enough to
/// **enumerate exhaustively and deterministically**. Each archetype's full D4-unique set is computed,
/// then interleaved by smooth weighted round-robin with a GLOBAL dedup (a position reachable from
/// several archetypes is assigned to whichever pulls it first). If `count` >= the total ceiling, the
/// complete book is returned. Determinism matters: `balance -a` plays each opening exactly once.
pub fn generate_weighted(
    count: usize,
    standard: bool,
    weights: &[f64],
) -> Vec<(String, String, usize)> {
    let num_arch = num_archetypes(standard);
    assert_eq!(weights.len(), num_arch, "expected {num_arch} archetype weights");
    let total_w: f64 = weights.iter().sum();
    assert!(total_w > 0.0, "archetype weights must sum to > 0");

    let all_sq = all_squares();
    let lists: Vec<Vec<(String, String, String)>> = (0..num_arch)
        .map(|a| enumerate_archetype(a, &all_sq, standard))
        .collect();
    let mut idx = vec![0usize; num_arch];
    let mut global_seen: HashSet<String> = HashSet::new();
    let mut credit = vec![0.0f64; num_arch];
    let mut out: Vec<(String, String, usize)> = Vec::with_capacity(count);

    while out.len() < count {
        // Choose this slot's archetype by smooth weighted round-robin.
        for (a, w) in weights.iter().enumerate() {
            credit[a] += w;
        }
        // Among archetypes that still have unseen positions, pick the highest-credit one.
        let pick = (0..num_arch)
            .filter(|&a| {
                // Has at least one not-yet-emitted (globally unique) position remaining.
                lists[a][idx[a]..].iter().any(|(k, _, _)| !global_seen.contains(k))
            })
            .max_by(|&x, &y| credit[x].partial_cmp(&credit[y]).unwrap());
        let pick = match pick {
            Some(p) => p,
            None => break, // every archetype exhausted: full ceiling reached
        };
        credit[pick] -= total_w;

        // Advance past already-emitted positions, then take the next unique one.
        while idx[pick] < lists[pick].len() && global_seen.contains(&lists[pick][idx[pick]].0) {
            idx[pick] += 1;
        }
        if idx[pick] < lists[pick].len() {
            let (key, tps, ptn) = lists[pick][idx[pick]].clone();
            idx[pick] += 1;
            global_seen.insert(key);
            out.push((tps, ptn, pick));
        }
    }
    out
}

/// Generate `count` unique openings with uniform archetype weighting.
pub fn generate(count: usize, standard: bool) -> Vec<(String, String, usize)> {
    generate_weighted(count, standard, &vec![1.0; num_archetypes(standard)])
}

/// Generate the book and write both files. Returns the number of unique openings written.
pub fn run(cfg: OpeningsConfig) -> io::Result<usize> {
    let openings = generate(cfg.count, cfg.standard);

    // Self-check: every emitted TPS must parse back through Topaz's own parser.
    for (tps, _, _) in &openings {
        if Board4::try_from_tps(tps).is_err() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("generated TPS failed to round-trip: {tps}"),
            ));
        }
    }

    let tps_blob = openings
        .iter()
        .map(|(t, _, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let ptn_blob = openings
        .iter()
        .map(|(_, p, _)| p.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&cfg.tps_path, format!("{tps_blob}\n"))?;
    fs::write(&cfg.ptn_path, format!("{ptn_blob}\n"))?;
    println!(
        "openings4: wrote {} unique positions -> {} (tps), {} (ptn)",
        openings.len(),
        cfg.tps_path,
        cfg.ptn_path
    );
    Ok(openings.len())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn opening_has_four_plies_and_round_trips() {
        let mut rng = new_rng();
        let all_sq = all_squares();
        let (tps, ptn) = build_opening(0, &all_sq, false, &mut rng).unwrap();
        let moves: Vec<&str> = ptn.split(' ').collect();
        assert_eq!(moves.len(), 4);
        assert!(moves[0].starts_with('2'));
        let board = Board4::try_from_tps(&tps).unwrap();
        assert!(board.move_num() >= 2);
    }

    #[test]
    fn plies_3_4_within_three_steps_of_same_colour() {
        let mut rng = new_rng();
        let all_sq = all_squares();
        for _ in 0..300 {
            let arch = (rng.next_u32() as usize) % NUM_ARCHETYPES;
            let (_, ptn) = build_opening(arch, &all_sq, false, &mut rng).unwrap();
            let m: Vec<&str> = ptn.split(' ').collect();
            assert_eq!(m.len(), 4);
            let w1 = &m[0][1..]; // strip leading '2' (black ref)
            let (b2, p3, p4) = (m[1], m[2], m[3]); // b2 = white ref
            assert!(
                within_steps(p3, b2, MAX_STEPS),
                "p3 {p3} not within {MAX_STEPS} of white {b2}"
            );
            assert!(
                within_steps(p4, w1, MAX_STEPS),
                "p4 {p4} not within {MAX_STEPS} of black {w1}"
            );
        }
    }

    #[test]
    fn enumeration_is_exhaustive_and_deterministic() {
        // The 4x4 space is enumerated exhaustively, so requesting more than exist returns exactly
        // the full D4-unique ceiling, and two runs are byte-identical (no RNG). `balance -a` relies
        // on this determinism (each opening played once).
        let a = generate(100_000, false);
        let b = generate(100_000, false);
        assert_eq!(a.len(), b.len(), "non-deterministic count");
        assert_eq!(a, b, "non-deterministic ordering/content");
        // Asking for fewer returns a deterministic prefix of the same enumeration.
        let prefix = generate(10, false);
        assert_eq!(prefix.len(), 10);
        assert_eq!(&prefix[..], &a[..10]);
        eprintln!("openings4 exhaustive black-stack ceiling = {}", a.len());
        // Sanity: a healthy book, well above the per-archetype minimum.
        assert!(a.len() >= 120, "unexpectedly small book: {}", a.len());
    }

    #[test]
    fn generated_openings_are_unique_and_valid() {
        let cfg = OpeningsConfig {
            count: 50,
            tps_path: std::env::temp_dir().join("topaz4_test_openings.tps").to_string_lossy().into_owned(),
            ptn_path: std::env::temp_dir().join("topaz4_test_openings.ptn").to_string_lossy().into_owned(),
            standard: false,
        };
        let n = run(cfg).unwrap();
        assert_eq!(n, 50);
    }
}
