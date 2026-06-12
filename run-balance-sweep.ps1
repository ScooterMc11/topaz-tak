<#
.SYNOPSIS
  Black-stack balance consistency sweep.

  Runs `topaz balance` under several opening regimes and node counts and prints a side-by-side
  summary of the White-Black gap. The point is robustness: if the lean is stable across opening
  depth, opening source, and search strength, it is a property of the RULE, not an artifact.

  Regimes:
    - random flat plies r=4 / r=6 / r=8   (opening-depth stability)
    - random flat plies r=6 at higher nodes (search-strength stability)
    - fixed TPS book, one deterministic game per position (reproducible, clean openings)

.EXAMPLE
  # Quick mechanics check (tiny, ~1 min):
  ./run-balance-sweep.ps1 -Games 40 -Nodes 300 -StrengthNodes 600

.EXAMPLE
  # Real run (sized for ~1.5% CI per config); pick Games to trade CI vs runtime.
  # At ~2 games/sec (8 threads, 15k nodes): Games=5000 ~= 42 min per 15k-node config.
  ./run-balance-sweep.ps1 -Games 5000
#>
param(
    [int]$Games = 5000,
    [int]$Nodes = 15000,
    [int]$StrengthNodes = 60000,
    [int]$Threads = 8,
    [string]$Bin = "./target/release/topaz.exe",
    [string]$Out = "balance-sweep-results.txt"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $Bin)) { throw "Engine binary not found: $Bin (run 'cargo build --release' first)" }

# A book sized to the game count, so the book-mode run plays each position exactly once.
$book = "sweep_book_$Games.tps"
Write-Host "Generating $Games-position book -> $book"
& $Bin openings -n $Games -t $book -p "sweep_book_$Games.ptn" | Out-Null

$configs = @(
    @{ name = "random r=4  (n=$Nodes)";        args = @("balance", "-g", "$Games", "-n", "$Nodes",         "-t", "$Threads", "-r", "4") }
    @{ name = "random r=6  (n=$Nodes)";        args = @("balance", "-g", "$Games", "-n", "$Nodes",         "-t", "$Threads", "-r", "6") }
    @{ name = "random r=8  (n=$Nodes)";        args = @("balance", "-g", "$Games", "-n", "$Nodes",         "-t", "$Threads", "-r", "8") }
    @{ name = "book        (n=$Nodes)";        args = @("balance", "--book", $book,  "-n", "$Nodes",         "-t", "$Threads") }
)

"Balance consistency sweep - $(Get-Date)"        | Tee-Object $Out
"Bin=$Bin  Games=$Games  Threads=$Threads"        | Tee-Object $Out -Append
""                                                | Tee-Object $Out -Append

$summary = @()
foreach ($c in $configs) {
    "=== $($c.name) ===" | Tee-Object $Out -Append
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    # Capture stdout only (the summary lives there); stderr progress streams to the console.
    $output = & $Bin @($c.args)
    $sw.Stop()
    $output | Tee-Object $Out -Append
    $wb = ($output | Select-String "White-Black").ToString().Trim()
    $summary += [pscustomobject]@{
        Config  = $c.name
        Minutes = [math]::Round($sw.Elapsed.TotalMinutes, 1)
        Result  = $wb
    }
    "" | Tee-Object $Out -Append
}

"===== SUMMARY (White-Black gap per regime) =====" | Tee-Object $Out -Append
$summary | Format-Table -AutoSize | Out-String | Tee-Object $Out -Append
Write-Host "`nFull output written to $Out"
