# run-gembot.ps1 — launch a GemBot (Topaz) with auto-restart.
#
# The bot also auto-reconnects IN-PROCESS on telnet drops; this restart loop is belt-and-suspenders
# for hard crashes. Run one instance per bot (two terminals), or via run-gembots.ps1.
#
# Stage hosts/ports (playtak-api/server/README.md). Telnet is UNENCRYPTED — the login password is
# sent in plaintext, so over the public internet that is a (minor, bot-account) exposure:
#   Local: localhost          10000   (now)
#   Beta:  beta.playtak.com    10002   (after the DBS PR merges to dev)
#   Prod:  playtak.com         10000   (after it merges to main)
#
# Examples:
#   # local, unrated test
#   .\run-gembot.ps1 -User GemBot6 -Pass password -Size 6
#   # BETA, rated (calibrated caps are the per-size defaults below)
#   .\run-gembot.ps1 -User GemBot6 -Pass <beta-pw> -Size 6 -ServerHost beta.playtak.com -Port 10002 -Rated
#   .\run-gembot.ps1 -User GemBot5 -Pass <beta-pw> -Size 5 -ServerHost beta.playtak.com -Port 10002 -Rated

param(
  [Parameter(Mandatory)][string]$User,
  [Parameter(Mandatory)][string]$Pass,
  [ValidateSet(5, 6)][int]$Size = 6,
  # Calibrated starting node caps (2026-06-22, AbortDepth=1): 6x6~55, 5x5~220. Override as the live
  # ladder tells you (depth-quantized, so expect a few discrete rungs).
  [int]$Nodes = $(if ($Size -eq 5) { 220 } else { 55 }),
  [int]$AbortDepth = 1,
  [string]$ServerHost = "localhost",
  [int]$Port = 10000,
  [switch]$Rated,            # omit = unrated (test); -Rated = rated (deploy)
  [int]$HashMb = 128
)

$env:PLAYTAK_HOST = $ServerHost
$env:PLAYTAK_PORT = "$Port"
$env:PLAYTAK_USERNAME = $User
$env:PLAYTAK_PASSWORD = $Pass
$env:PLAYTAK_SIZE = "$Size"
$env:PLAYTAK_NODES = "$Nodes"
$env:PLAYTAK_ABORT_DEPTH = "$AbortDepth"
$env:PLAYTAK_HASH_MB = "$HashMb"
$env:PLAYTAK_OPENING = "1"        # Double Black Stack
$env:PLAYTAK_TIME = "180"         # 3 min base
$env:PLAYTAK_INC = "1"            # +1s * moveNum (with scaling)
$env:PLAYTAK_INC_SCALES = "1"     # 3+n increment scaling
$env:PLAYTAK_COLOR = "A"          # accept either color
$env:PLAYTAK_UNRATED = $(if ($Rated) { "0" } else { "1" })

$bin = Join-Path $PSScriptRoot "target\release\topaz.exe"
if (-not (Test-Path $bin)) { Write-Error "topaz.exe not found at $bin — build it first (cargo build --release)"; exit 1 }

Write-Host ("Launching {0}: size {1}, nodes {2}, abort {3}, {4}:{5}, rated={6}" -f `
    $User, $Size, $Nodes, $AbortDepth, $ServerHost, $Port, $Rated.IsPresent)
while ($true) {
  & $bin playtak
  Write-Host "$User exited (code $LASTEXITCODE); restarting in 5s..."
  Start-Sleep -Seconds 5
}
