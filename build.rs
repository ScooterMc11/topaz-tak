use std::path::Path;

// Stage the 4x4 NNUE weights that `src/eval/incremental4.rs` embeds via `include_bytes!`.
//
// All weight files (`*.bin`) are gitignored and not committed, and the 4x4 net is still in
// progress. Because `incremental4.rs` embeds its net unconditionally at compile time, the whole
// crate historically refused to build unless a `quantised4.bin` was present -- even for people who
// only build/run 5x5 or 6x6. To decouple the build from the in-progress 4x4 net, we stage the
// embedded file into OUT_DIR here: copy the real net when `src/quantised4.bin` exists, otherwise
// synthesize a zero-filled placeholder of the exact expected size. A zero net makes 4x4 eval
// meaningless, but nothing relies on 4x4 yet, and the moment a real `src/quantised4.bin` appears it
// is used instead. (Only the 4x4 net is optional -- the real 6x6/5x5 nets are still embedded
// directly by their modules, so a missing one still fails loudly.)
fn main() {
    // size_of::<incremental4::Network>() for the current 4x4 layout: NUM_INPUTS(504) feature
    // accumulators of HIDDEN_SIZE(512) i16, plus bias/output/pqst vectors. `incremental4.rs`'s
    // `assert!(bytes.len() == size_of::<Network>())` guards against this constant drifting if the
    // net layout ever changes.
    const QUANTISED4_LEN: usize = 520_192;

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let staged = Path::new(&out_dir).join("quantised4.bin");
    let source = Path::new("src/quantised4.bin");

    println!("cargo:rerun-if-changed=src/quantised4.bin");
    println!("cargo:rerun-if-changed=build.rs");

    if source.exists() {
        std::fs::copy(&source, &staged).expect("failed to stage src/quantised4.bin into OUT_DIR");
    } else {
        std::fs::write(&staged, vec![0u8; QUANTISED4_LEN])
            .expect("failed to write placeholder quantised4.bin");
        println!(
            "cargo:warning=src/quantised4.bin not found; embedding a zero-filled placeholder \
             (4x4 NNUE eval is disabled until a real net is provided)"
        );
    }

    // 2-komi 6x6 net (used for half_komi == 4 games). Same idea, but the fallback is the STANDARD
    // 6x6 net -- not zeros -- so a build without the 2-komi file still works and 2-komi games simply
    // use the normal net. quantised.bin is required anyway (embedded directly by incremental.rs).
    let staged_2komi = Path::new(&out_dir).join("quantised-2-komi.bin");
    let source_2komi = Path::new("src/quantised-2-komi.bin");
    let standard = Path::new("src/quantised.bin");
    println!("cargo:rerun-if-changed=src/quantised-2-komi.bin");
    println!("cargo:rerun-if-changed=src/quantised.bin");
    if source_2komi.exists() {
        std::fs::copy(&source_2komi, &staged_2komi)
            .expect("failed to stage src/quantised-2-komi.bin into OUT_DIR");
    } else if standard.exists() {
        std::fs::copy(&standard, &staged_2komi)
            .expect("failed to stage fallback quantised.bin as quantised-2-komi.bin");
        println!(
            "cargo:warning=src/quantised-2-komi.bin not found; falling back to the standard \
             quantised.bin (2-komi games will use the standard 6x6 net)"
        );
    }
    // If neither exists, leave it absent: incremental.rs's include_bytes!(\"../quantised.bin\") will
    // surface the canonical missing-net error.
}
