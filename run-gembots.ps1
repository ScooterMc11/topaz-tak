# run-gembots.ps1 — launch BOTH GemBots (each in its own window, each with its own restart loop).
#
# Local test (default):   .\run-gembots.ps1
# BETA, rated:            .\run-gembots.ps1 -ServerHost beta.playtak.com -Port 10002 -Pass6 <pw6> -Pass5 <pw5> -Rated
# Prod, rated:            .\run-gembots.ps1 -ServerHost playtak.com      -Port 10000 -Pass6 <pw6> -Pass5 <pw5> -Rated
#
# Per-size calibrated node caps (6x6=55, 5x5=220, AbortDepth=1) are the defaults in run-gembot.ps1.

param(
  [string]$ServerHost = "localhost",
  [int]$Port = 10000,
  [string]$Pass6 = "password",
  [string]$Pass5 = "password",
  [switch]$Rated
)

$here = $PSScriptRoot
function Launch([string]$user, [int]$size, [string]$pass) {
  $a = @("-NoExit", "-File", "$here\run-gembot.ps1", "-User", $user, "-Pass", $pass,
    "-Size", "$size", "-ServerHost", $ServerHost, "-Port", "$Port")
  if ($Rated) { $a += "-Rated" }
  Start-Process powershell -ArgumentList $a
}

Launch "GemBot6" 6 $Pass6
Launch "GemBot5" 5 $Pass5
Write-Host ("Launched GemBot6 + GemBot5 in separate windows ({0}:{1}, rated={2})." -f $ServerHost, $Port, $Rated.IsPresent)
