# run-gembot.ps1 - launch a GemBot (Topaz) with auto-restart.
#
# The bot also auto-reconnects IN-PROCESS on telnet drops; this restart loop is belt-and-suspenders
# for hard crashes. Run one instance per bot (two terminals), or via run-gembots.ps1.
#
# Stage hosts/ports (playtak-api/server/README.md). Telnet is UNENCRYPTED - the login password is
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
  # Node caps (2026-07-07, AbortDepth=1): 6x6=25000, 5x5=20000. The 12s/move safety bank caps per-move
  # time. Override as the live ladder tells you.
  [int]$Nodes = $(if ($Size -eq 5) { 20000 } else { 25000 }),
  # Fixed search depth (iterative-deepening ceiling). When > 0, the bot searches to exactly this
  # depth regardless of board complexity and the Nodes cap is IGNORED (a 12s/move safety bank still
  # prevents flagging). 0 = use the Nodes cap instead. e.g. Novice 4, Intermediate 6, Advanced 8.
  [int]$Depth = 0,
  [int]$AbortDepth = 1,
  # Seek base time in seconds; the increment scales with move number (the "+n"). Per-size TC:
  # 5x5 = 7+n (420s base), 6x6 = 10+n (600s base).
  [int]$Time = $(if ($Size -eq 5) { 420 } else { 600 }),
  # Ladder weakening. The ACTIVE knob is EvalGap (calibrated 2026-07-05, gap-based ladder; live ladder
  # is the real anchor): prune every root move more than EvalGap centipawns below the best, then pick
  # uniformly among the survivors (bounds the worst-case blunder; never plays a clearly-worse move from
  # a wide pool). 0 = full strength. Takes precedence over Temperature. See run-ladder.ps1 for values.
  [int]$EvalGap = 0,
  # Dynamic eval-gap (the ACTIVE ladder knob). When EvalGapStd > 0 the gap flexes with the bot's eval:
  # EvalGapMin at <= -50cp (losing => strongest), EvalGapStd at 0cp (even), EvalGapMax at >= +50cp
  # (winning => weakest); linear between. Takes precedence over the static EvalGap. See run-ladder.ps1.
  [int]$EvalGapMin = 0,
  [int]$EvalGapStd = 0,
  [int]$EvalGapMax = 0,
  # Linear rank bias among the eval-gap survivors (decimal ok): 0 = uniform, higher tilts selection
  # toward the WORSE eligible moves (worst:best odds = 1 + BiasSlope). See run-ladder.ps1 for the value.
  [double]$BiasSlope = 0,
  # Absolute "always-allow" floor (cp): any root move scoring >= this stays eligible even if it falls
  # outside the eval-gap band (so a winning-but-not-best move can be played). Only widens the pool;
  # never overrides a solved/decisive position or plays a losing move. Large default = disabled, so
  # standalone / ladder launches are unaffected unless set. The bot-manager passes the real value.
  [int]$AlwaysAllow = 1000000,
  # Courtesy undo: how many times per game the bot may offer to take back an opponent move that hung
  # an instant road while a non-losing alternative was available. It requests the undo, says so in
  # chat, and plays the winning move if the opponent says "gg" or goes quiet for a minute (the wait
  # is capped by its own clock, so it can never flag over this). 0 = off, the default, so standalone
  # / ladder launches are unaffected. The bot-manager passes the per-level cap.
  [int]$UndoOffers = 0,
  # Legacy softmax knob (superseded by EvalGap; kept for experiments). Higher => weaker.
  [int]$Temperature = 0,
  [int]$MovePool = 8,        # MultiPV pool the gap/temperature selection samples within (max 16)
  [string]$ServerHost = "localhost",
  [int]$Port = 10000,
  [switch]$Rated,            # omit = unrated (test); -Rated = rated (deploy)
  # 48MB is plenty at these node caps (55-220/move barely touches the TT) and keeps 6 bots light on
  # an 8GB machine. Bump it for standalone full-strength analysis.
  [int]$HashMb = 48,
  # Windows priority class for topaz.exe. BelowNormal keeps the desktop responsive when several bots
  # run at once, without starving a rated bot into flagging on time (which Idle can). Set to "Normal"
  # to disable. Valid: Idle, BelowNormal, Normal, AboveNormal, High.
  [string]$Priority = "BelowNormal"
)

$env:PLAYTAK_HOST = $ServerHost
$env:PLAYTAK_PORT = "$Port"
$env:PLAYTAK_USERNAME = $User
$env:PLAYTAK_PASSWORD = $Pass
$env:PLAYTAK_SIZE = "$Size"
$env:PLAYTAK_NODES = "$Nodes"
$env:PLAYTAK_DEPTH = "$Depth"     # >0 = fixed-depth search (Nodes ignored); 0 = use Nodes cap
$env:PLAYTAK_ABORT_DEPTH = "$AbortDepth"
$env:PLAYTAK_TEMPERATURE = "$Temperature"
$env:PLAYTAK_EVAL_GAP = "$EvalGap"
$env:PLAYTAK_EVAL_GAP_MIN = "$EvalGapMin"
$env:PLAYTAK_EVAL_GAP_STD = "$EvalGapStd"
$env:PLAYTAK_EVAL_GAP_MAX = "$EvalGapMax"
# Invariant format so the decimal separator is always '.' (Rust f64 parse), regardless of locale.
$env:PLAYTAK_BIAS_SLOPE = $BiasSlope.ToString([System.Globalization.CultureInfo]::InvariantCulture)
$env:PLAYTAK_ALWAYS_ALLOW = "$AlwaysAllow"
$env:PLAYTAK_UNDO_OFFERS = "$UndoOffers"
$env:PLAYTAK_MOVE_POOL = "$MovePool"
$env:PLAYTAK_HASH_MB = "$HashMb"
$env:PLAYTAK_OPENING = "1"        # Double Black Stack
$env:PLAYTAK_TIME = "$Time"       # base time (s): 5x5=420 (7+n), 6x6=600 (10+n)
$env:PLAYTAK_INC = "1"            # base increment; +1s * moveNum with scaling
$env:PLAYTAK_INC_SCALES = "1"     # increment scales with move number (the "+n")
$env:PLAYTAK_COLOR = "A"          # accept either color
$env:PLAYTAK_UNRATED = $(if ($Rated) { "0" } else { "1" })

$bin = Join-Path $PSScriptRoot "target\release\topaz.exe"
if (-not (Test-Path $bin)) { Write-Error "topaz.exe not found at $bin - build it first (cargo build --release)"; exit 1 }

$searchDesc = if ($Depth -gt 0) { "depth $Depth (fixed)" } else { "nodes $Nodes" }
Write-Host ("Launching {0}: size {1}, {2}, abort {3}, gap {4}, dyn-gap {5}/{6}/{7}, slope {8}, always-allow {9}, temp {10}, pool {11}, time {12}s, hash {13}MB, prio {14}, {15}:{16}, rated={17}, undo-offers {18}" -f `
    $User, $Size, $searchDesc, $AbortDepth, $EvalGap, $EvalGapMin, $EvalGapStd, $EvalGapMax, $BiasSlope, $AlwaysAllow, $Temperature, $MovePool, $Time, $HashMb, $Priority, $ServerHost, $Port, $Rated.IsPresent, $UndoOffers)
while ($true) {
  # Start in the SAME console (-NoNewWindow) so output stays visible and the child inherits the
  # PLAYTAK_* env vars set above; -PassThru lets us drop its priority before it starts searching.
  $p = Start-Process -FilePath $bin -ArgumentList 'playtak' -NoNewWindow -PassThru
  try { $p.PriorityClass = [System.Diagnostics.ProcessPriorityClass]::$Priority }
  catch { Write-Host "Could not set priority '$Priority': $_" }
  $p.WaitForExit()
  Write-Host "$User exited (code $($p.ExitCode)); restarting in 5s..."
  Start-Sleep -Seconds 5
}
