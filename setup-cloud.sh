#!/usr/bin/env bash
#
# Prepare a fresh Linux cloud GPU box (RunPod / Vast / etc.) to run the Tak NNUE training pipeline.
# Installs the Rust toolchain + build deps, verifies the GPU/CUDA toolkit, and checks that both
# repos and the engine net are in place. Idempotent — safe to re-run.
#
# This does NOT fetch the repos: your engine branch and edited bullet trainer are local/unpushed,
# so upload both directories to this box first, e.g.:
#   runpodctl receive <code>            # if you sent them with `runpodctl send`
#   rsync -avz ./topaz-tak ./bulletTrainer  user@box:~/    # or scp/rsync from your machine
#
# Then:  ENGINE_DIR=~/topaz-tak BULLET_DIR=~/bulletTrainer/bullet ./setup-cloud.sh
set -euo pipefail

ENGINE_DIR="${ENGINE_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
BULLET_DIR="${BULLET_DIR:-$ENGINE_DIR/../bulletTrainer/bullet}"

say()  { printf '\n=== %s ===\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }

SUDO=""
if [ "$(id -u)" -ne 0 ] && have sudo; then SUDO=sudo; fi

say "System packages"
if have apt-get; then
  $SUDO apt-get update -y
  $SUDO apt-get install -y git build-essential pkg-config curl ca-certificates
else
  echo "No apt-get; ensure git, a C toolchain, pkg-config and curl are installed."
fi

say "Rust toolchain"
if ! have cargo; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi
# shellcheck disable=SC1090
source "$HOME/.cargo/env" 2>/dev/null || true
cargo --version || { echo "Rust install failed — open a new shell and re-source ~/.cargo/env"; exit 1; }

say "GPU / CUDA check"
if have nvidia-smi; then nvidia-smi || true; else echo "WARNING: nvidia-smi not found — is this a GPU box?"; fi
if have nvcc; then
  nvcc --version | tail -n1
else
  echo "WARNING: nvcc (CUDA toolkit) not found. bullet's GPU backend compiles CUDA kernels and"
  echo "needs the full toolkit, not just drivers. Use a CUDA-toolkit template or install it."
fi

say "Repositories"
[ -f "$ENGINE_DIR/Cargo.toml" ] || { echo "Engine repo not found at ENGINE_DIR=$ENGINE_DIR (upload it, then re-run)."; exit 1; }
echo "ENGINE_DIR=$ENGINE_DIR"
if [ -f "$BULLET_DIR/Cargo.toml" ]; then
  echo "BULLET_DIR=$BULLET_DIR"
else
  echo "NOTE: bullet trainer not found at BULLET_DIR=$BULLET_DIR."
  echo "      That's fine for a datagen-only box (run with --skip-train). Needed only for training."
fi

say "Engine net (required to build both engine and trainer)"
if [ -f "$ENGINE_DIR/src/quantised.bin" ]; then
  echo "Found $ENGINE_DIR/src/quantised.bin"
else
  echo "WARNING: $ENGINE_DIR/src/quantised.bin is missing."
  echo "The engine embeds it via include_bytes!, and run-training.sh bootstraps the trainer's"
  echo "embed net from it. Upload your existing quantised.bin to $ENGINE_DIR/src/ before training."
fi

say "Setup complete"
cat <<EOF
Next steps (from $ENGINE_DIR):
  chmod +x run-training.sh
  ENGINE_DIR=$ENGINE_DIR BULLET_DIR=$BULLET_DIR ./run-training.sh --smoke --backend cuda   # validate
  ENGINE_DIR=$ENGINE_DIR BULLET_DIR=$BULLET_DIR ./run-training.sh --backend cuda            # full run
EOF
