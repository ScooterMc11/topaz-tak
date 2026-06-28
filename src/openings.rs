//! Black-stack opening-book generator for the SPRT runner (racetrack).
//!
//! Emits a set of legal black-stack opening positions, **6 plies deep**, in two parallel files:
//!   - a **TPS book** (one position per line) for racetrack `--book-format tps`
//!   - a **PTN book** (one move sequence per line) for human readability
//!
//! Why TPS instead of a move list: racetrack arbitrates every game with stock `tiltak` (standard
//! Tak rules), which rejects the forced `2xx` double placement. Baking the opening into a TPS start
//! position (at `move_num >= 2`) sidesteps this — Topaz's black-stack rule only fires at
//! `move_num == 1` (see `move_gen.rs`), so from these positions both engines play identical standard
//! Tak. See auto-memory `racetrack-tps-book-requirement`.
//!
//! Opening structure (per the experiment's guidelines):
//!   - ply 1: White's forced `2xx` double-black-stack
//!   - ply 2: Black's single swapped flat
//!   - plies 1-2 come from 5 archetypes: diagonal corners, adjacent corners, hug, gap-hug,
//!     center stack (White on a central square, Black on a corner)
//!   - plies 3-4: flat placements restricted to the interior 4x4 (no edge, no walls/caps)
//!   - plies 5-6: flat placements within 4 orthogonal steps (Manhattan distance) of one of the
//!     moving player's own-colour pieces (no walls/caps). Note the opening swap: White's ply-1
//!     double placement is a BLACK stack, and Black's ply-2 flat is a (swapped) WHITE flat.
//!
//! Positions equivalent under the board's 8 rotations/reflections (the dihedral group D4) are
//! de-duplicated, keeping one representative per class.

use crate::board::{Board6, TakBoard};
use crate::datagen::new_rng;
use crate::{GameMove, Position};

use rand_core::RngCore;
use std::collections::HashSet;
use std::fs;
use std::io;

const SIZE: usize = 6;

/// Max orthogonal (Manhattan) distance from an own-colour piece for ply 5-6 placements.
const MAX_STEPS: i32 = 4;

/// Ply 1-2 archetypes (White double-stack square, Black flat square). All geometric instances are
/// listed; symmetry de-duplication on the final 6-ply position collapses equivalents.
const DIAGONAL: &[(&str, &str)] = &[("a1", "f6"), ("f6", "a1"), ("a6", "f1"), ("f1", "a6")];
const ADJACENT: &[(&str, &str)] = &[
    ("a1", "a6"),
    ("a1", "f1"),
    ("a6", "a1"),
    ("a6", "f6"),
    ("f1", "f6"),
    ("f1", "a1"),
    ("f6", "f1"),
    ("f6", "a6"),
];
const HUG: &[(&str, &str)] = &[
    ("a1", "a2"),
    ("a1", "b1"),
    ("a6", "a5"),
    ("a6", "b6"),
    ("f1", "f2"),
    ("f1", "e1"),
    ("f6", "f5"),
    ("f6", "e6"),
];
const GAP_HUG: &[(&str, &str)] = &[
    ("a1", "c1"),
    ("a1", "a3"),
    ("a6", "c6"),
    ("a6", "a4"),
    ("f1", "d1"),
    ("f1", "f3"),
    ("f6", "d6"),
    ("f6", "f4"),
];
const CENTERS: [&str; 4] = ["c3", "c4", "d3", "d4"];
const CORNERS: [&str; 4] = ["a1", "a6", "f1", "f6"];

pub const NUM_ARCHETYPES: usize = 5;

/// Human-readable name for an archetype index (matches pick_instance's ordering).
pub fn archetype_name(i: usize) -> &'static str {
    match i {
        0 => "diagonal corners",
        1 => "adjacent corners",
        2 => "hug",
        3 => "gap hug",
        4 => "center stack",
        _ => "unknown",
    }
}

/// Number of archetypes for the given mode (5 black-stack, 3 standard).
pub fn num_archetypes(standard: bool) -> usize {
    if standard {
        STANDARD_ARCHETYPES
    } else {
        NUM_ARCHETYPES
    }
}

/// Black's first-reply square to White's double-black-stack on a corner (PlayTak GemBot use). White's
/// opening square `white_sq` ("a1"-style) is matched against the corner archetype tables; an archetype
/// is sampled by `weights` (= `[diagonal, adjacent, hug, gap_hug]`, the center stack is unused here),
/// then one of that archetype's 1-2 mirror Black squares for `white_sq` is picked uniformly. Returns
/// `None` if `white_sq` is not a corner (caller should fall back to a normal search) or if the weights
/// are unusable. Self-seeds its own RNG.
pub fn pick_black_reply(white_sq: &str, weights: &[f64]) -> Option<String> {
    if !CORNERS.iter().any(|c| *c == white_sq) {
        return None;
    }
    let tables: [&[(&str, &str)]; 4] = [DIAGONAL, ADJACENT, HUG, GAP_HUG];
    // Up to 4 weights (diag, adj, hug, gap-hug); missing/negative entries treated as 0.
    let mut w = [0.0f64; 4];
    for (i, slot) in w.iter_mut().enumerate() {
        *slot = weights.get(i).copied().unwrap_or(0.0).max(0.0);
    }
    let total: f64 = w.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let mut rng = new_rng();
    // Weighted archetype pick.
    let mut r = (rng.next_u32() as f64 / u32::MAX as f64) * total;
    let mut arch = 0usize;
    for (i, wi) in w.iter().enumerate() {
        arch = i;
        if r < *wi {
            break;
        }
        r -= *wi;
    }
    // Black squares for this corner in the chosen archetype (1-2 mirror options).
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
    /// Standard-Tak mode (for the 2-komi comparison book): ply 1 is a single swapped flat instead
    /// of the `2xx` double-black-stack, and only the diagonal-corner / adjacent-corner / hug
    /// archetypes are used (no gap-hug or center-stack).
    pub standard: bool,
}

impl Default for OpeningsConfig {
    fn default() -> Self {
        Self {
            count: 1000,
            tps_path: "6s_black_stack_openings.tps".to_string(),
            ptn_path: "6s_black_stack_openings.ptn".to_string(),
            standard: false,
        }
    }
}

/// Number of ply 1-2 archetypes used in standard mode (diagonal corners, adjacent corners, hug).
const STANDARD_ARCHETYPES: usize = 3;

/// All 36 squares, as "<file><rank>" strings.
fn all_squares() -> Vec<String> {
    let mut v = Vec::with_capacity(SIZE * SIZE);
    for file in 'a'..=('a' as u8 + SIZE as u8 - 1) as char {
        for rank in 1..=SIZE {
            v.push(format!("{file}{rank}"));
        }
    }
    v
}

/// The interior 4x4 (b2..e5): squares that are not on the board edge.
fn interior_squares() -> Vec<String> {
    let mut v = Vec::with_capacity((SIZE - 2) * (SIZE - 2));
    for file in 'b'..=('a' as u8 + SIZE as u8 - 2) as char {
        for rank in 2..=(SIZE - 1) {
            v.push(format!("{file}{rank}"));
        }
    }
    v
}

fn choose<'a, R: RngCore>(slice: &'a [(&str, &str)], rng: &mut R) -> (&'a str, &'a str) {
    slice[(rng.next_u32() as usize) % slice.len()]
}

/// Pick a (White-stack, Black-flat) square pair for the given archetype.
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
        3 => {
            let (w, b) = choose(GAP_HUG, rng);
            (w.to_string(), b.to_string())
        }
        _ => {
            let w = CENTERS[(rng.next_u32() as usize) % CENTERS.len()];
            let b = CORNERS[(rng.next_u32() as usize) % CORNERS.len()];
            (w.to_string(), b.to_string())
        }
    }
}

/// Apply one ply to the board by constructing the move from its PTN. `double` marks the forced
/// ply-1 `2xx` placement. The active player's own colour is used; `do_move` handles the move_num==1
/// swap for plies 1-2.
fn apply_ply(board: &mut Board6, sq: &str, double: bool) {
    let ptn = if double {
        format!("2{sq}")
    } else {
        sq.to_string()
    };
    let mv = GameMove::try_from_ptn_m(&ptn, SIZE, board.side_to_move())
        .expect("constructed a valid ptn placement");
    board.do_move(mv);
}

/// Pick a random square from `pool` that is not already occupied.
fn pick_free<R: RngCore>(pool: &[String], occupied: &[String], rng: &mut R) -> Option<String> {
    let free: Vec<&String> = pool
        .iter()
        .filter(|s| !occupied.iter().any(|o| o == *s))
        .collect();
    if free.is_empty() {
        return None;
    }
    Some(free[(rng.next_u32() as usize) % free.len()].clone())
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

/// Pick a random unoccupied square that is within `MAX_STEPS` orthogonal steps of at least one
/// reference square. Returns `None` if no such square exists (caller retries with a new opening).
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

/// Build one 6-ply opening for the given archetype. Returns `(tps, ptn)` on success.
fn build_opening<R: RngCore>(
    arch: usize,
    all_sq: &[String],
    interior_sq: &[String],
    standard: bool,
    rng: &mut R,
) -> Option<(String, String)> {
    let (w1, b2) = pick_instance(arch, rng);
    let mut board = Board6::new(); // komi 0
    let mut occupied: Vec<String> = Vec::with_capacity(6);
    let mut ptn: Vec<String> = Vec::with_capacity(6);

    // Reference squares by piece colour, for the ply 5-6 proximity constraint. The opening swap
    // means White's ply-1 placement is BLACK and Black's ply-2 flat is WHITE.
    let mut white_refs: Vec<String> = Vec::new();
    let mut black_refs: Vec<String> = Vec::new();

    // ply 1: White's opening -> a black piece at w1. Standard mode: a single swapped flat ("a1").
    // Black-stack mode: the forced double placement ("2a1").
    if standard {
        apply_ply(&mut board, &w1, false);
        ptn.push(w1.clone());
    } else {
        apply_ply(&mut board, &w1, true);
        ptn.push(format!("2{w1}"));
    }
    black_refs.push(w1.clone());
    occupied.push(w1);

    // ply 2: Black's move -> a (swapped) white flat at b2.
    apply_ply(&mut board, &b2, false);
    ptn.push(b2.clone());
    white_refs.push(b2.clone());
    occupied.push(b2);

    // plies 3-4: white then black flats, interior only.
    let p3 = pick_free(interior_sq, &occupied, rng)?;
    apply_ply(&mut board, &p3, false);
    ptn.push(p3.clone());
    white_refs.push(p3.clone());
    occupied.push(p3);

    let p4 = pick_free(interior_sq, &occupied, rng)?;
    apply_ply(&mut board, &p4, false);
    ptn.push(p4.clone());
    black_refs.push(p4.clone());
    occupied.push(p4);

    // plies 5-6: white then black flats, each within MAX_STEPS orthogonal steps of one of the
    // moving player's own-colour pieces.
    let p5 = pick_free_near(all_sq, &occupied, &white_refs, rng)?;
    apply_ply(&mut board, &p5, false);
    ptn.push(p5.clone());
    occupied.push(p5);

    let p6 = pick_free_near(all_sq, &occupied, &black_refs, rng)?;
    apply_ply(&mut board, &p6, false);
    ptn.push(p6.clone());
    occupied.push(p6);

    // Defensive: flats-only over 6 plies cannot make a road or fill the board, so this never trips.
    if board.game_result().is_some() {
        return None;
    }

    Some((format!("{board:?}"), ptn.join(" ")))
}

/// Canonical key of a position under the dihedral group D4 (4 rotations x 2 reflections), used for
/// rotational/reflectional de-duplication. Operates on the Debug/TPS grid (`<rows> <side> <num>`),
/// where rows run rank6..rank1 and each row has exactly SIZE comma-separated cells ("x" = empty).
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
/// equal the active archetype count: 5 black-stack, 3 standard). Archetypes are interleaved by
/// smooth weighted round-robin, so the output order is mixed (not grouped) and hits the requested
/// proportions. Returns `(tps, ptn, archetype_index)` per opening.
pub fn generate_weighted(
    count: usize,
    standard: bool,
    weights: &[f64],
) -> Vec<(String, String, usize)> {
    let num_arch = num_archetypes(standard);
    assert_eq!(weights.len(), num_arch, "expected {num_arch} archetype weights");
    let total_w: f64 = weights.iter().sum();
    assert!(total_w > 0.0, "archetype weights must sum to > 0");

    let mut rng = new_rng();
    let all_sq = all_squares();
    let interior_sq = interior_squares();
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<(String, String, usize)> = Vec::with_capacity(count);
    let mut credit = vec![0.0f64; num_arch];
    let max_attempts = count.saturating_mul(1000).max(100_000);
    let mut attempts = 0usize;

    while out.len() < count {
        // Choose this slot's archetype by smooth weighted round-robin.
        for (a, w) in weights.iter().enumerate() {
            credit[a] += w;
        }
        let pick = (0..num_arch)
            .max_by(|&x, &y| credit[x].partial_cmp(&credit[y]).unwrap())
            .unwrap();
        credit[pick] -= total_w;

        // Generate a unique opening of that archetype (retry on dup/collision).
        let mut added = false;
        while !added {
            if attempts >= max_attempts {
                eprintln!(
                    "openings: stopped after {attempts} attempts with {} unique positions (target {})",
                    out.len(),
                    count
                );
                return out;
            }
            attempts += 1;
            if let Some((tps, ptn)) = build_opening(pick, &all_sq, &interior_sq, standard, &mut rng) {
                if seen.insert(canonical_key(&tps)) {
                    out.push((tps, ptn, pick));
                    added = true;
                }
            }
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
        if Board6::try_from_tps(tps).is_err() {
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
        "openings: wrote {} unique positions -> {} (tps), {} (ptn)",
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
    fn generated_openings_are_unique_and_valid() {
        let cfg = OpeningsConfig {
            count: 50,
            tps_path: std::env::temp_dir()
                .join("topaz_test_openings.tps")
                .to_string_lossy()
                .into_owned(),
            ptn_path: std::env::temp_dir()
                .join("topaz_test_openings.ptn")
                .to_string_lossy()
                .into_owned(),
            standard: false,
        };
        let n = run(cfg).unwrap();
        assert_eq!(n, 50);
    }

    #[test]
    fn opening_has_six_plies_and_round_trips() {
        let mut rng = new_rng();
        let all_sq = all_squares();
        let interior_sq = interior_squares();
        let (tps, ptn) = build_opening(0, &all_sq, &interior_sq, false, &mut rng).unwrap();
        // 6 plies in the ptn line, first is the forced double placement.
        let moves: Vec<&str> = ptn.split(' ').collect();
        assert_eq!(moves.len(), 6);
        assert!(moves[0].starts_with('2'));
        // TPS parses, and the board is past the special-move zone (move_num >= 2).
        let board = Board6::try_from_tps(&tps).unwrap();
        assert!(board.move_num() >= 2);
    }

    #[test]
    fn symmetric_positions_share_a_canonical_key() {
        // A position and its left-right mirror must canonicalize identically.
        let mut rng = new_rng();
        let all_sq = all_squares();
        let interior_sq = interior_squares();
        let (tps, _) = build_opening(4, &all_sq, &interior_sq, false, &mut rng).unwrap();
        let board_part = tps.split(' ').next().unwrap();
        let grid: Vec<Vec<String>> = board_part
            .split('/')
            .map(|row| row.split(',').map(|c| c.to_string()).collect())
            .collect();
        let mirrored = format!("{} 1 4", serialize(&flip_h(&grid)));
        assert_eq!(canonical_key(&tps), canonical_key(&mirrored));
    }

    #[test]
    fn interior_squares_exclude_edges() {
        let interior = interior_squares();
        assert_eq!(interior.len(), 16);
        for sq in &interior {
            let file = sq.chars().next().unwrap();
            let rank: usize = sq[1..].parse().unwrap();
            assert!(('b'..='e').contains(&file) && (2..=5).contains(&rank), "{sq} is on an edge");
        }
    }

    #[test]
    fn plies_5_6_are_near_same_colour_pieces() {
        let mut rng = new_rng();
        let all_sq = all_squares();
        let interior_sq = interior_squares();
        for _ in 0..300 {
            let arch = (rng.next_u32() as usize) % NUM_ARCHETYPES;
            let (_, ptn) = build_opening(arch, &all_sq, &interior_sq, false, &mut rng).unwrap();
            let m: Vec<&str> = ptn.split(' ').collect();
            assert_eq!(m.len(), 6);
            let w1 = &m[0][1..]; // strip the leading '2' off the double placement
            let (b2, p3, p4, p5, p6) = (m[1], m[2], m[3], m[4], m[5]);
            // White's own-colour refs before ply 5: the swapped white flat (b2) and p3.
            assert!(
                within_steps(p5, b2, MAX_STEPS) || within_steps(p5, p3, MAX_STEPS),
                "p5 {p5} not within {MAX_STEPS} steps of white pieces {b2}/{p3}"
            );
            // Black's own-colour refs before ply 6: the black stack (w1) and p4.
            assert!(
                within_steps(p6, w1, MAX_STEPS) || within_steps(p6, p4, MAX_STEPS),
                "p6 {p6} not within {MAX_STEPS} steps of black pieces {w1}/{p4}"
            );
        }
    }

    #[test]
    fn standard_mode_uses_single_flat_opening() {
        let mut rng = new_rng();
        let all_sq = all_squares();
        let interior_sq = interior_squares();
        for _ in 0..100 {
            let arch = (rng.next_u32() as usize) % STANDARD_ARCHETYPES;
            let (tps, ptn) = build_opening(arch, &all_sq, &interior_sq, true, &mut rng).unwrap();
            let m: Vec<&str> = ptn.split(' ').collect();
            assert_eq!(m.len(), 6);
            // ply 1 is a single swapped flat, not a 2-stack.
            assert!(
                !m[0].starts_with('2'),
                "standard ply1 should be a single flat, got {}",
                m[0]
            );
            let board = Board6::try_from_tps(&tps).unwrap();
            assert!(board.move_num() >= 2);
        }
    }
}
