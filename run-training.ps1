<#
.SYNOPSIS
  End-to-end NNUE retraining for the "black stack setup" ruleset, hands-off.

.DESCRIPTION
  Runs the full pipeline so you can start it once and walk away:
    1. Build the engine (release).
    2. Generate fresh self-play data under the new rules  -> data.bin   (engine `datagen`)
    3. Validate the dataset                                            (engine `checkdata`)
    4. Train a new network with the bullet trainer        -> quantised.bin
    5. Install the new net into the engine and rebuild.

  PREREQUISITE: the training step (4) needs a GPU toolkit for the chosen -Backend.
    -Backend cuda      NVIDIA, native CUDA backend     (needs the CUDA toolkit / nvcc)
    -Backend hip-cuda  NVIDIA via HIP (trainer default)
    -Backend hip       AMD ROCm
    -Backend cpu       no GPU; correctness only, far too slow for a real run
  If you don't have a GPU set up yet, run with -SkipTrain to do steps 1-3 now, then run
  the trainer yourself later (see the printed command).

.EXAMPLE
  .\run-training.ps1 -Smoke
  Tiny end-to-end test (200 games, 2 superbatches) to prove the whole pipeline works.

.EXAMPLE
  .\run-training.ps1 -Games 2000000 -Threads 8 -Backend cuda
  A full from-scratch run.

.EXAMPLE
  .\run-training.ps1 -SkipTrain
  Just build the engine and generate + validate the dataset.

.EXAMPLE
  .\run-training.ps1 -Shards datagenShards\doubleBlackStack
  Clean (trim) + concatenate every *.bin shard in that folder, auto-size the schedule, and train.
#>
[CmdletBinding()]
param(
    [int]$Games = 1000000,
    [int]$Threads = 8,
    [int]$Nodes = 5000,
    [int]$RandomPlies = 6,
    [string]$DataFile = "",
    [string]$BulletDir = "C:\Users\lance\Desktop\Tak\Bots\bulletTrainer\bullet",
    [ValidateSet("cuda", "hip", "hip-cuda", "cpu")]
    [string]$Backend = "cuda",
    [string]$Superbatches = "",   # "" -> "auto" when -Shards is used, else 240; "auto" sizes from dataset
    [int]$BatchesPerSuperbatch = 6104,
    [string]$Shards = "",         # dir of *.bin shards to clean + concatenate into -DataFile, then train
    [switch]$Smoke,
    [switch]$SkipDatagen,
    [switch]$SkipTrain
)

$ErrorActionPreference = "Stop"
$EngineDir = $PSScriptRoot
$EngineExe = Join-Path $EngineDir "target\release\topaz.exe"
if ([string]::IsNullOrEmpty($DataFile)) { $DataFile = Join-Path $EngineDir "data.bin" }

function Step($msg) { Write-Host "`n=== $msg ===" -ForegroundColor Cyan }
function Die($msg) { Write-Host "FAILED: $msg" -ForegroundColor Red; exit 1 }
function CheckExit($what) { if ($LASTEXITCODE -ne 0) { Die "$what (exit $LASTEXITCODE)" } }

# Resolve the superbatch default: auto-size when assembling shards, else the classic 240.
if ([string]::IsNullOrEmpty($Superbatches)) {
    $Superbatches = if ($Shards) { "auto" } else { "240" }
}

if ($Smoke) {
    Write-Host "SMOKE MODE: tiny end-to-end run to validate the pipeline." -ForegroundColor Yellow
    $Games = 200; $Superbatches = "2"; $BatchesPerSuperbatch = 50
}

$startedAt = Get-Date

# 1. Build the engine ---------------------------------------------------------
Step "1/5 Build engine (release)"
Push-Location $EngineDir
try { cargo build --release; CheckExit "engine build" } finally { Pop-Location }
if (-not (Test-Path $EngineExe)) { Die "engine binary not found at $EngineExe" }

# 2. Generate self-play data (or assemble shards) -----------------------------
if ($Shards) {
    Step "2/5 Assemble shards from $Shards -> $DataFile"
    if (-not (Test-Path $Shards)) { Die "shards dir not found: $Shards" }
    $dataResolved = (Resolve-Path -LiteralPath $DataFile -ErrorAction SilentlyContinue).Path
    $shardFiles = Get-ChildItem -Path $Shards -Filter *.bin -File |
        Where-Object { $_.FullName -ne $dataResolved }
    if (-not $shardFiles) { Die "no .bin shards found in $Shards" }
    $out = [System.IO.File]::Create($DataFile)   # start fresh
    try {
        foreach ($s in $shardFiles) {
            $msg = & $EngineExe trimdata $s.FullName
            CheckExit "trimdata"
            Write-Host "  + $($s.Name): $msg"
            $fs = [System.IO.File]::OpenRead($s.FullName)
            try { $fs.CopyTo($out) } finally { $fs.Close() }
        }
    } finally { $out.Close() }
    Write-Host "  assembled $($shardFiles.Count) shard(s) into $DataFile"
} elseif ($SkipDatagen) {
    Step "2/5 Generate data (SKIPPED, using existing $DataFile)"
    if (-not (Test-Path $DataFile)) { Die "-SkipDatagen set but $DataFile does not exist" }
} else {
    Step "2/5 Generate data ($Games games -> $DataFile)"
    & $EngineExe datagen -g $Games -t $Threads -n $Nodes -r $RandomPlies -o $DataFile
    CheckExit "datagen"
}

# 3. Validate the dataset -----------------------------------------------------
Step "3/5 Validate dataset"
$summary = & $EngineExe checkdata $DataFile
CheckExit "checkdata"
Write-Host $summary
if ($summary -notmatch "invalid=0") { Die "dataset reported invalid entries: $summary" }

# Suggest (and, if requested, apply) a superbatch count sized for ~24 epochs.
if ($summary -match "positions=(\d+)") {
    $positions = [int64]$Matches[1]
    if ($positions -gt 0) {
        $suggested = [math]::Max(1, [int][math]::Round($positions * 24 / 100000000.0))
        Write-Host "  ~$positions positions -> suggested -Superbatches $suggested (~24 epochs)"
        if ($Superbatches -eq "auto") {
            $Superbatches = "$suggested"
            Write-Host "  using -Superbatches $Superbatches (auto)"
        }
    }
}
if ($Superbatches -eq "auto") {
    Write-Host "  (could not parse position count; defaulting -Superbatches to 240)"
    $Superbatches = "240"
}

# 4. Train --------------------------------------------------------------------
if ($SkipTrain) {
    Step "4/5 Train (SKIPPED)"
    Write-Host "To train later, run from '$BulletDir':" -ForegroundColor Yellow
    Write-Host "  `$env:TAK_DATA='$DataFile'; `$env:TAK_SUPERBATCHES=$Superbatches; `$env:TAK_BATCHES=$BatchesPerSuperbatch"
    Write-Host "  cargo run -p bullet_lib --release --example tak   # add backend feature flags as needed"
    Write-Host "`nPipeline (steps 1-3) complete." -ForegroundColor Green
    exit 0
}

Step "4/5 Train ($Superbatches superbatches, backend=$Backend)"

# The trainer's tak_utils.rs embeds checkpoints/test-240b/quantised.bin via include_bytes! (for
# its unused inference helpers), so that file must exist for the trainer to *build*. It's the same
# architecture/size as the engine's net, so bootstrap it from the engine's quantised.bin if absent.
$embed = Join-Path $BulletDir "checkpoints\test-240b\quantised.bin"
if (-not (Test-Path $embed)) {
    $engineNet = Join-Path $EngineDir "src\quantised.bin"
    if (-not (Test-Path $engineNet)) { Die "trainer needs $embed but engine net $engineNet is missing too" }
    New-Item -ItemType Directory -Force (Split-Path $embed) | Out-Null
    Copy-Item $engineNet $embed
    Write-Host "Bootstrapped trainer embed net: $embed"
}

$featureArgs = switch ($Backend) {
    "cuda" { @("--no-default-features", "--features", "cuda") }
    "hip" { @("--no-default-features", "--features", "hip") }
    "hip-cuda" { @() }   # trainer default
    "cpu" { @("--no-default-features", "--features", "cpu") }
}
$env:TAK_DATA = $DataFile
$env:TAK_SUPERBATCHES = "$Superbatches"
$env:TAK_BATCHES = "$BatchesPerSuperbatch"
Push-Location $BulletDir
try {
    cargo run -p bullet_lib --release --example tak @featureArgs
    CheckExit "bullet trainer"
} finally { Pop-Location }

# 5. Install the new net and rebuild -----------------------------------------
Step "5/5 Install new network"
$ckptRoot = Join-Path $BulletDir "checkpoints"
if (-not (Test-Path $ckptRoot)) { Die "no checkpoints directory at $ckptRoot" }
$net = Get-ChildItem -Path $ckptRoot -Recurse -Filter "quantised.bin" |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($null -eq $net) { Die "no quantised.bin produced under $ckptRoot (quantisation may have overflowed)" }
Write-Host "Newest checkpoint: $($net.FullName)"

if ($Smoke) {
    # A smoke run trains a throwaway toy net; never install it over the real one.
    Write-Host "SMOKE MODE: not installing the toy net. Pipeline validated end-to-end." -ForegroundColor Green
    exit 0
}

# Back up the current net before overwriting, so a bad run is always recoverable.
$dest = Join-Path $EngineDir "src\quantised.bin"
if (Test-Path $dest) {
    Copy-Item $dest "$dest.prev" -Force
    Write-Host "Backed up previous net to $dest.prev"
}
Copy-Item $net.FullName $dest -Force

Step "Rebuild engine with new network"
Push-Location $EngineDir
try { cargo build --release; CheckExit "engine rebuild" } finally { Pop-Location }

$elapsed = (Get-Date) - $startedAt
Write-Host "`nDONE in $([int]$elapsed.TotalMinutes) min. New net installed at src\quantised.bin (previous saved as quantised.bin.prev)." -ForegroundColor Green
Write-Host "Sanity-check it: echo `"tei`nteinewgame 6`nposition startpos`ngo depth 8`" | .\target\release\topaz.exe" -ForegroundColor Green
