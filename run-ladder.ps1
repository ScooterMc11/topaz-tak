# run-ladder.ps1 - launch the full 6-bot skill ladder (Novice / Intermediate / Advanced x 5x5 / 6x6).
# Each bot runs in its own window with its own restart loop (delegates to run-gembot.ps1), posts a
# Double-Black-Stack seek, and waits for an opponent (you, in the browser) to accept. Six open seeks
# coexist fine - the bots only post seeks, they don't accept each other.
#
# Provisional calibrated configs (2026-06-29). All levels share the Advanced per-size node cap; the
# lower levels weaken purely via Temperature + MovePool. These are STARTING points (offline racetrack,
# +/-~70 Elo noise) - the live ladder is the real anchor, so expect to nudge them once rated.
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
  [string[]]$Only                 # optional: launch only these bot names
)

$here = $PSScriptRoot

# User, Size, Nodes (calibrated Advanced cap, shared by all levels), Temperature, MovePool.
$ladder = @(
  @{ User = "AdvancedBot6";     Size = 6; Nodes = 55;  Temp = 0;   Pool = 5 },
  @{ User = "IntermediateBot6"; Size = 6; Nodes = 55;  Temp = 70;  Pool = 5 },
  @{ User = "NoviceBot6";       Size = 6; Nodes = 55;  Temp = 135; Pool = 8 },
  @{ User = "AdvancedBot5";     Size = 5; Nodes = 220; Temp = 0;   Pool = 5 },
  @{ User = "IntermediateBot5"; Size = 5; Nodes = 220; Temp = 165; Pool = 5 },
  @{ User = "NoviceBot5";       Size = 5; Nodes = 220; Temp = 320; Pool = 8 }
)

function Launch($b) {
  $pass = $Pass.Replace('{user}', $b.User)   # per-bot password from the shared template
  $a = @("-NoExit", "-File", "$here\run-gembot.ps1", "-User", $b.User, "-Pass", $pass,
    "-Size", "$($b.Size)", "-Nodes", "$($b.Nodes)", "-Temperature", "$($b.Temp)",
    "-MovePool", "$($b.Pool)", "-ServerHost", $ServerHost, "-Port", "$Port")
  if ($Rated) { $a += "-Rated" }
  Start-Process powershell -ArgumentList $a
}

$launched = @()
foreach ($b in $ladder) {
  if ($Only -and ($Only -notcontains $b.User)) { continue }
  Launch $b
  $launched += $b.User
}
Write-Host ("Launched {0} ladder bot(s): {1}  ({2}:{3}, rated={4})" -f `
    $launched.Count, ($launched -join ", "), $ServerHost, $Port, $Rated.IsPresent)
