# run-ladder.ps1 - launch the full 6-bot skill ladder (Novice / Intermediate / Advanced x 5x5 / 6x6).
# Each bot runs in its own window with its own restart loop (delegates to run-gembot.ps1), posts a
# Double-Black-Stack seek, and waits for an opponent (you, in the browser) to accept. Six open seeks
# coexist fine - the bots only post seeks, they don't accept each other.
#
# Configs (2026-07-05, depth-6 base, DYNAMIC gap ladder). All levels share the per-size node cap
# (6x6=25000, 5x5=20000, AbortDepth=1) over a pool-16 MultiPV. Levels are separated by a
# DYNAMIC eval-gap that flexes with the bot's eval: GapMin at <= -50cp (losing => strongest), GapStd
# at 0cp (even), GapMax at >= +50cp (winning => weakest); linear between (kink at 0). These are MANUAL
# values (same for both sizes) - the earlier slatebot calibration was judged unrepresentative of real
# play; tune the triples live off ratings/feel (higher = weaker).
#
#   Local test (all accounts pw 'password', UNRATED):   .\run-ladder.ps1
#   Local rated:                                         .\run-ladder.ps1 -Rated
#   Just the 6x6 bots:                                   .\run-ladder.ps1 -Only AdvancedBot6,IntermediateBot6,NoviceBot6
#   BETA, rated (per-bot passwords via the {user} template):
#       .\run-ladder.ps1 -ServerHost beta.playtak.com -Port 10002 -Pass "{user}_S3cret!" -Rated
#
param(
  [string]$ServerHost = "localhost",
  [int]$Port = 10000,
  # Password TEMPLATE shared by all six. The token {user} is replaced by each bot's username, so one
  # flag gives every bot a distinct password (e.g. -Pass "{user}_S3cret!" => AdvancedBot6 logs in with
  # "AdvancedBot6_S3cret!"). With no {user} token it's a literal shared password (default, local dev).
  [string]$Pass = "password",
  [switch]$Rated,                 # omit = unrated (test); -Rated = rated
  [string[]]$Only,                # optional: launch only these bot names
  # Seconds to wait between launching each bot. Launching all six at once cold-starts six PowerShell
  # hosts + six topaz.exe simultaneously, which can freeze a modest machine (Defender scans each new
  # process). Staggering spreads that burst out. Set 0 for the old all-at-once behaviour.
  [int]$StaggerSeconds = 8
)

$here = $PSScriptRoot

# User, Size, Nodes (per-size: 6x6=500k, 5x5=400k -> ~depth 8/6 even at pool 16, so gap scores are
# reliable), dynamic eval-gap triple GapMin/GapStd/GapMax (cp), Slope (BiasSlope, per-level), MovePool.
# Both sizes slope 8; levels differ only by the gap triple.
$ladder = @(
  @{ User = "AdvancedBot6";     Size = 6; Nodes = 500000; GapMin = 20; GapStd = 30; GapMax = 40;  Slope = 8; Pool = 16 },
  @{ User = "IntermediateBot6"; Size = 6; Nodes = 500000; GapMin = 40; GapStd = 50; GapMax = 60;  Slope = 8; Pool = 16 },
  @{ User = "NoviceBot6";       Size = 6; Nodes = 500000; GapMin = 60; GapStd = 80; GapMax = 100; Slope = 8; Pool = 16 },
  @{ User = "AdvancedBot5";     Size = 5; Nodes = 400000; GapMin = 20; GapStd = 30; GapMax = 40;  Slope = 8; Pool = 16 },
  @{ User = "IntermediateBot5"; Size = 5; Nodes = 400000; GapMin = 40; GapStd = 50; GapMax = 60;  Slope = 8; Pool = 16 },
  @{ User = "NoviceBot5";       Size = 5; Nodes = 400000; GapMin = 60; GapStd = 80; GapMax = 100; Slope = 8; Pool = 16 }
)

function Launch($b) {
  $pass = $Pass.Replace('{user}', $b.User)   # per-bot password from the shared template
  $a = @("-NoExit", "-File", "$here\run-gembot.ps1", "-User", $b.User, "-Pass", $pass,
    "-Size", "$($b.Size)", "-Nodes", "$($b.Nodes)",
    "-EvalGapMin", "$($b.GapMin)", "-EvalGapStd", "$($b.GapStd)", "-EvalGapMax", "$($b.GapMax)",
    "-BiasSlope", ([double]$b.Slope).ToString([System.Globalization.CultureInfo]::InvariantCulture),
    "-MovePool", "$($b.Pool)", "-ServerHost", $ServerHost, "-Port", "$Port")
  if ($Rated) { $a += "-Rated" }
  Start-Process powershell -ArgumentList $a
}

$launched = @()
foreach ($b in $ladder) {
  if ($Only -and ($Only -notcontains $b.User)) { continue }
  if ($launched.Count -gt 0 -and $StaggerSeconds -gt 0) {
    Write-Host ("  waiting {0}s before next bot..." -f $StaggerSeconds)
    Start-Sleep -Seconds $StaggerSeconds
  }
  Launch $b
  $launched += $b.User
}
Write-Host ("Launched {0} ladder bot(s): {1}  ({2}:{3}, rated={4})" -f `
    $launched.Count, ($launched -join ", "), $ServerHost, $Port, $Rated.IsPresent)
