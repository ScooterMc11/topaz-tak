# Eval-recalibration pod runbook (6×6 + gated 5×5)

Goal: fix the Black-favoured eval **skew** (the net finds good moves but scores even positions as
Black-ahead) by regenerating training data with a **corrected opening distribution** (ply-1 stack
always cornered, no center-stack, flats-only opening plies) and retraining from scratch with a
higher WDL weight so real game outcomes ground the eval.

Warm-start is **not possible** (no `weights.bin`/`optimiser_state` survived the old pods — only the
lossy `quantised*.bin` exports), so every retrain here is **from scratch**, matching the proven
pipeline.

**Order:** setup → **diagnostic (both sizes)** → decide → 6×6 regen+train+SPRT → *(gated)* 5×5.
Do the diagnostic first: if a net's crossover is already ~0 it needs no retrain, and if the gap is
pure *slope* (not a shifted crossover) it's an `eval_scale` fix, not a data problem.

Throughout: `internal cp = 4 × displayed cp`; `calibrate` reports **displayed cp** (what the engine
prints). Crossover < 0 ⇒ net over-favours Black by that many cp.

---

## 0. Local — build the single bundle, upload via Jupyter

One tarball holds all three repos (engine + trainer + racetrack) with the current-best nets already
embedded in `topaz-tak/src/`, so there is no net-copy step on the pod. It excludes `target/`, `.git`,
the size vaults, and the 11 GB `datagenShards`. Already built at
`C:\Users\lance\Desktop\Tak\Bots\pod-recal-bundle.tgz` (~25 MB) with:

```bash
cd /c/Users/lance/Desktop/Tak/Bots
tar czf pod-recal-bundle.tgz \
  --exclude='topaz-tak/target' --exclude='topaz-tak/.git' --exclude='topaz-tak/.claude' \
  --exclude='topaz-tak/4x4' --exclude='topaz-tak/5x5' --exclude='topaz-tak/6x6' \
  --exclude='topaz-tak/datagenShards*' --exclude='topaz-tak/recalShards*' \
  --exclude='topaz-tak/diagShards*' --exclude='topaz-tak/postShards*' \
  --exclude='topaz-tak/startPositionEvaluator' --exclude='topaz-tak/balancedOpeningLineCounterResults' \
  --exclude='topaz-tak/bot-manager-app' \
  --exclude='bulletTrainer/bullet/target' --exclude='bulletTrainer/.git' --exclude='bulletTrainer/bullet/.git' \
  --exclude='racetrack/target' --exclude='racetrack/.git' \
  --exclude='*.log' \
  topaz-tak bulletTrainer racetrack
```

Upload `pod-recal-bundle.tgz` through the Jupyter file browser. **racetrack is included** (needed for
the SPRT phases) — no separate transfer.

The embedded players are the current-best nets, already in the bundle and verified:
`topaz-tak/src/quantised.bin` (current 6×6, the one that measured ~−30 cp locally) and
`topaz-tak/src/quantised5.bin` (= 5×5 bs-v3). The trainer's compile-only bootstrap
(`checkpoints/test-240b/quantised.bin`) is included too, so nothing needs placeholdering.

---

## 1. Pod — extract + toolchain

```bash
cd /workspace
tar --no-same-owner -xzf pod-recal-bundle.tgz
find topaz-tak bulletTrainer racetrack -name '*.rs' -exec touch {} +   # defeat stale-mtime caching
```

```bash
cd /workspace/topaz-tak && ENGINE_DIR=/workspace/topaz-tak \
  BULLET_DIR=/workspace/bulletTrainer/bullet ./setup-cloud.sh
```

`setup-cloud.sh` installs Rust via rustup, but that only edits your shell profile — the CURRENT
terminal still won't have `cargo` on PATH (you'll hit `cargo: command not found`). Pull it into this
shell (covers the foreground diagnostic AND any `setsid nohup` launch, which inherits this PATH):

```bash
source "$HOME/.cargo/env"
cargo --version    # sanity: should print a version
```

The GPU training in Phase 5 also needs **`CUDA_PATH`** — bullet's default backend compiles CUDA
kernels and its build script panics (`CUDA_PATH is not defined`) without it. `setup-cloud.sh` only
*checks* for `nvcc`; set the path in the shell you train from (adjust if your toolkit dir differs):

```bash
which nvcc && nvcc --version | tail -1        # confirm the toolkit (not just the runtime) exists
export CUDA_PATH=/usr/local/cuda
export PATH="$CUDA_PATH/bin:$PATH"
```

---

## 2. Pod — stash the baseline nets (for the SPRT gate later)

Nets are already embedded, so there's nothing to copy in. Just save the ORIGINAL nets now, before
Phase 5 overwrites `src/` with the recal net — Phase 6/7 SPRT needs them as the baseline.

```bash
cd /workspace/topaz-tak
cp src/quantised.bin  baseline-6s.bin
cp src/quantised5.bin baseline-5s.bin
```

---

## 3. Pod — DIAGNOSTIC (both sizes)

Small corner-open self-play runs with each current-best net; the script auto-runs `calibrate`.
`~5–10 min` each.

```bash
cd /workspace/topaz-tak
SIZE=6 GAMES=4000 NODES=10000 THREADS=26 SHARDS=4 \
  OUTDIR=diagShards6 CONCAT=diag6.bin ./run-recal-datagen.sh 2>&1 | tee diag6.log
```

```bash
cd /workspace/topaz-tak
SIZE=5 GAMES=4000 NODES=10000 THREADS=26 SHARDS=4 \
  OUTDIR=diagShards5 CONCAT=diag5.bin ./run-recal-datagen.sh 2>&1 | tee diag5.log
```

**Read the `White scores 50% at net eval = … cp` line in each.**
- `≈ 0` → that size is already calibrated; **skip its retrain**.
- clearly `< 0` (e.g. the ~−30 cp seen locally for 6×6) → genuine Black skew; retrain it.
- rows where actual `White%` ≫ `implied%` with a ~0 crossover → *slope* issue → tell me, that's an
  `eval_scale` retune (change `eval_scale` in the trainer), not a data fix.

Decide from `diag5.log` whether to run Phase 7 (5×5).

---

## 4. Pod — 6×6 full corrected regen

First **measure throughput** on this pod, then size the full run to your budget.

```bash
cd /workspace/topaz-tak
./target/release/topaz datagen -c -g 2000 -t 26 -n 8000 -o /tmp/thr.bin
# note the "… pos/s" it prints, then:  seconds ≈ target_positions / pos_per_s
rm -f /tmp/thr.bin
```

Target ~35 M positions (≈500 k games at ~70 pos/game) — a defensible "full-ish" from-scratch set
that also leaves session time for 5×5; scale `GAMES` up (→ ~1 M for ~70 M positions) if Phase 6 SPRT
shows a strength regression, or down if the pod budget is tight (≥ ~300 k games stays usable).
Launch **detached** so it survives a kernel restart:

```bash
cd /workspace/topaz-tak
setsid nohup env SIZE=6 GAMES=500000 NODES=8000 THREADS=26 SHARDS=40 \
  OUTDIR=recalShards6 CONCAT=data_recal_6.bin ./run-recal-datagen.sh > recal6.log 2>&1 &
echo "PID $!  — tail -f recal6.log"
```

Resumable: re-run the same line after any interruption; finished shards are skipped. It ends with a
`checkdata` (`invalid=0`) and a `calibrate` of the **input** distribution.

---

## 5. Pod — 6×6 train (higher WDL), install, re-verify

Train from scratch. `TAK_WDL=0.7` leans the target on real outcomes to correct the skew; `TAK_NET_ID`
keeps the checkpoint from overwriting anything.

**Size `TAK_SUPERBATCHES` to the dataset**, not to a fixed number: the trainer processes
`superbatches × batches × 16384` positions total, so a fixed count means wildly different epochs for
different dataset sizes. The original ~280M-position run was `240 × 6104` ≈ 86 epochs. Match that
per-position exposure: **`superbatches ≈ 0.86 × (positions in millions)`** (read the `positions=` count
from the datagen `checkdata`). E.g. ~70M positions → **`TAK_SUPERBATCHES=60`**; ~280M → 240. Using the
old 240 on a 70M set is ~340 epochs and risks overfitting the smaller corrected set. Replace the `-60`
in the paths below with whatever count you use.

**CRITICAL — use `--no-default-features --features cuda`.** The trainer's *default* backend is
`hip-cuda` (`bullet_hip_backend`), which links the CUDA libs but does NOT run on the GPU here — it
silently executes on CPU (symptom: `nvidia-smi` shows ~0% util and <500 MiB used *during* training,
and pos/sec sits around a few hundred k). The dedicated `cuda` backend (`bullet_cuda_backend`) is the
one that actually uses the GPU — it's what `run-training.sh --backend cuda` (the original runs) built.
`CUDA_PATH` must be set (Phase 1) for either to compile.

```bash
cd /workspace/bulletTrainer/bullet
TAK_DATA=/workspace/topaz-tak/data_recal_6.bin \
TAK_WDL=0.7 TAK_NET_ID=recal6_v1 \
TAK_SUPERBATCHES=60 TAK_BATCHES=6104 \
  cargo run -p bullet_lib --release --no-default-features --features cuda --example tak
```

Confirm the GPU actually engaged: `nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv`
during training must show **>1500 MiB used** and non-zero util. If it doesn't, you're on the CPU
backend — stop and re-check the feature flags. (On GPU this run is ~20–30 min; on CPU it's hours.)

Install the new net, rebuild, and **re-diagnose** — the crossover should move toward 0:

```bash
cd /workspace/topaz-tak
cp /workspace/bulletTrainer/bullet/checkpoints/recal6_v1-60/quantised.bin 6x6-recal-v1.bin
cp 6x6-recal-v1.bin src/quantised.bin && touch src/eval/incremental.rs
cargo build --release

SIZE=6 GAMES=4000 NODES=10000 THREADS=26 SHARDS=4 \
  OUTDIR=postShards6 CONCAT=post6.bin ./run-recal-datagen.sh 2>&1 | tee post6.log
# success = "White scores 50% at net eval" now ≈ 0 (was ~−30)
```

---

## 6. Pod — 6×6 SPRT (strength gate: new vs old)

Build one binary per net (baseline = the original net stashed in Phase 2, candidate = recal-v1), then
race them on a **no-center-stack** book. Baseline is listed FIRST so positive Elo / "SPRT passed" =
improvement.

```bash
cd /workspace/topaz-tak
# candidate binary (recal net is currently embedded from Phase 5)
cp target/release/topaz topaz-6s-recal-v1
# baseline binary (original net stashed in Phase 2)
cp baseline-6s.bin src/quantised.bin && touch src/eval/incremental.rs && cargo build --release
cp target/release/topaz topaz-6s-old
# restore the recal net as the working embed
cp 6x6-recal-v1.bin src/quantised.bin && touch src/eval/incremental.rs && cargo build --release

# build racetrack (SPRT arbiter) if not already built
[ -x /workspace/racetrack/target/release/racetrack ] || ( cd /workspace/racetrack && cargo build --release )

# no-center-stack 6×6 book
./target/release/topaz openings -n 3000 --no-center-stack -t 6s_ncs_openings.tps -p 6s_ncs_openings.ptn
book_count=$(wc -l < 6s_ncs_openings.tps); games=$(( book_count * 2 ))

RT=/workspace/racetrack/target/release/racetrack
"$RT" -s 6 --komi 0 -c 24 -g "$games" \
  --book 6s_ncs_openings.tps --book-format tps --ptnout sprt6.ptn \
  --format sprt --sprt elo0=-10 elo1=0 alpha=0.05 beta=0.05 \
  --engine path=./topaz-6s-old \
  --engine path=./topaz-6s-recal-v1 \
  --all-engines tc=8+0.8
```

`elo0=-10 elo1=0` accepts the recal net as long as it is **not more than 10 Elo weaker** — the aim is
calibration without a real strength loss (raise `GAMES` in Phase 4 if it fails). Promote on pass:
download `6x6-recal-v1.bin` off the pod and drop it in your local `6x6/nets/` vault.

---

## 7. Pod — 5×5 (ONLY if Phase 3 `diag5.log` showed a skew)

Symmetric to 6×6; the self-play player is the embedded 5×5 bs-v3 net (already in `src/quantised5.bin`).

```bash
cd /workspace/topaz-tak
# full regen (5×5 is smaller/faster; ~55 pos/game)
setsid nohup env SIZE=5 GAMES=400000 NODES=8000 THREADS=26 SHARDS=40 \
  OUTDIR=recalShards5 CONCAT=data_recal_5.bin ./run-recal-datagen.sh > recal5.log 2>&1 &
```

```bash
# train (5×5 trainer)
cd /workspace/bulletTrainer/bullet
TAK_DATA=/workspace/topaz-tak/data_recal_5.bin \
TAK_WDL=0.7 TAK_NET_ID=recal5_v1 \
TAK_SUPERBATCHES=30 TAK_BATCHES=300 \
  cargo run -p bullet_lib --release --example tak5   # add backend feature flags
```

```bash
# install + re-diagnose
cd /workspace/topaz-tak
cp /workspace/bulletTrainer/bullet/checkpoints/recal5_v1-30/quantised.bin 5x5-recal-v1.bin
cp 5x5-recal-v1.bin src/quantised5.bin && touch src/eval/incremental5.rs && cargo build --release
SIZE=5 GAMES=4000 NODES=10000 THREADS=26 SHARDS=4 \
  OUTDIR=postShards5 CONCAT=post5.bin ./run-recal-datagen.sh 2>&1 | tee post5.log
```

```bash
# SPRT vs old bs-v3 (net-vs-net), no-center-stack book
cd /workspace/topaz-tak
cp target/release/topaz topaz-5s-recal-v1
# baseline binary (original bs-v3 net stashed in Phase 2)
cp baseline-5s.bin src/quantised5.bin && touch src/eval/incremental5.rs && cargo build --release
cp target/release/topaz topaz-5s-old
# restore the recal net as the working embed
cp 5x5-recal-v1.bin src/quantised5.bin && touch src/eval/incremental5.rs && cargo build --release

./target/release/topaz openings5 -n 2000 --no-center-stack -t 5s_ncs_openings.tps -p 5s_ncs_openings.ptn
SPRT=1 ENGINE_B=/workspace/topaz-tak/topaz-5s-old TOPAZ=/workspace/topaz-tak/topaz-5s-recal-v1 \
  BOOK=/workspace/topaz-tak/5s_ncs_openings.tps ELO0=-10 ELO1=0 \
  ./run-sprt5.sh
```

Promote on pass: download `5x5-recal-v1.bin` off the pod into your local `5x5/nets/` vault.

---

## Success criteria

1. **`calibrate` crossover** moves from clearly-negative to ≈ 0 (post vs pre) — the skew is fixed.
2. **SPRT** shows the recal net is within the Elo margin of the old net — calibration didn't cost
   real strength.

If (1) holds but (2) fails, raise `GAMES` (more data) and/or nudge `TAK_WDL` down toward 0.5.
If the diagnostic showed a *slope* gap with a ~0 crossover instead, stop — that's an `eval_scale`
retune, not this data pipeline.
