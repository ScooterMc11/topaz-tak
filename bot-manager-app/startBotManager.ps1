# startBotManager.ps1 - launch the local Bot Manager web app (backend + frontend dev servers).
#
# Opens two windows: the Express backend (127.0.0.1:4317) and the Vite frontend (127.0.0.1:5173),
# then opens the UI in your browser. From the panel you configure and start/stop the 6 ladder bots.
# Double-click startBotManager.cmd (which calls this) - no terminal needed. On first run it installs
# dependencies automatically. Pass -Install to force a fresh reinstall.
#
#   Double-click:  startBotManager.cmd
#   From a shell:  .\startBotManager.ps1            # start (auto-installs deps if missing)
#                  .\startBotManager.ps1 -Install   # force reinstall deps, then start
#
param(
  [switch]$Install,        # force npm install in server/ and web/ even if node_modules exist
  [switch]$NoBrowser       # don't auto-open the browser
)

$here = $PSScriptRoot      # this script lives inside bot-manager-app/
$server = Join-Path $here "server"
$web = Join-Path $here "web"

if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
  # Common cause on this machine: nvm-windows is installed but no version is active, so its PATH
  # entry (C:\nvm4w\nodejs) points at a missing symlink. Try to self-heal by activating a version.
  $nvm = if ($env:NVM_HOME) { Join-Path $env:NVM_HOME "nvm.exe" } else { $null }
  if ($nvm -and (Test-Path $nvm)) {
    Write-Host "npm not on PATH; nvm-windows detected. Activating a Node version..." -ForegroundColor Yellow
    $list = & $nvm list
    # Prefer the version nvm marks current (*), else the first installed.
    $ver = ($list | Where-Object { $_ -match '\*\s*(\d+\.\d+\.\d+)' } | ForEach-Object { $Matches[1] } | Select-Object -First 1)
    if (-not $ver) { $ver = ($list | ForEach-Object { if ($_ -match '(\d+\.\d+\.\d+)') { $Matches[1] } } | Select-Object -First 1) }
    if ($ver) { & $nvm use $ver | Out-Host }
  }
  if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
    Write-Error ("npm still not found. If you use nvm-windows, run this in an ELEVATED PowerShell " +
      "(Run as Administrator): nvm use 22.14.0  - then retry. Otherwise install Node.js LTS: https://nodejs.org/")
    exit 1
  }
}

# Install deps if forced (-Install) or missing (first run). Double-click users never need a flag.
if ($Install -or -not (Test-Path (Join-Path $server "node_modules"))) {
  Write-Host "Installing backend deps..." -ForegroundColor Cyan
  Push-Location $server; npm install; $ok = $?; Pop-Location
  if (-not $ok) { Write-Error "backend npm install failed."; exit 1 }
}
if ($Install -or -not (Test-Path (Join-Path $web "node_modules"))) {
  Write-Host "Installing frontend deps..." -ForegroundColor Cyan
  Push-Location $web; npm install; $ok = $?; Pop-Location
  if (-not $ok) { Write-Error "frontend npm install failed."; exit 1 }
}

# Each dev server in its own window so you can see logs / Ctrl-C independently.
Write-Host "Starting backend (127.0.0.1:4317)..." -ForegroundColor Green
Start-Process powershell -ArgumentList "-NoExit", "-Command", "Set-Location '$server'; npm run dev"

Write-Host "Starting frontend (127.0.0.1:5173)..." -ForegroundColor Green
Start-Process powershell -ArgumentList "-NoExit", "-Command", "Set-Location '$web'; npm run dev"

if (-not $NoBrowser) {
  Start-Sleep -Seconds 4   # give Vite a moment to bind
  Start-Process "http://127.0.0.1:5173"
}

Write-Host ""
Write-Host "Bot Manager is starting:" -ForegroundColor Cyan
Write-Host "  UI:      http://127.0.0.1:5173"
Write-Host "  Backend: http://127.0.0.1:4317"
Write-Host "Close the two dev-server windows (or Ctrl-C in each) to stop the app." -ForegroundColor DarkGray
Write-Host "Stopping the backend window also stops any bots it launched." -ForegroundColor DarkGray
